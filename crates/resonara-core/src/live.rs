//! Control-thread preparation and bounded, lock-free audio graph handoff.
use crate::{Controls, Engine, OutputSample, PluginOwner, Project, Result};
use std::{
    collections::HashMap,
    marker::PhantomData,
    ptr,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicPtr, Ordering},
    },
};

struct Slots {
    pending: AtomicPtr<Engine>,
    retired: AtomicPtr<Engine>,
}
impl Drop for Slots {
    fn drop(&mut self) {
        // The last Arc proves that neither endpoint can access these slots.
        for slot in [&mut self.pending, &mut self.retired] {
            let pointer = *slot.get_mut();
            if !pointer.is_null() {
                unsafe {
                    drop(Box::from_raw(pointer));
                }
            }
        }
    }
}

/// Retain on the creating thread. The backend must destroy/join the renderer
/// before dropping this owner, just as for Engine::take_plugin_owners.
pub struct Playback {
    slots: Arc<Slots>,
    owners: HashMap<usize, Vec<(usize, PluginOwner)>>,
    project: Project,
    current: usize,
    device_rate: u32,
    pub controls: Arc<Controls>,
    _owner_thread: PhantomData<Rc<()>>,
}

/// The sole audio-thread endpoint. Swaps boxes without allocating or freeing
/// engines; their destruction and plugin lifecycle stay on the control thread.
pub struct PlaybackRenderer {
    active: Box<Engine>,
    slots: Arc<Slots>,
}

impl Playback {
    pub fn new(
        project: &Project,
        device_rate: u32,
        start: u64,
        metronome: bool,
    ) -> Result<(Self, PlaybackRenderer)> {
        let controls = Arc::new(Controls::new(project));
        controls.metronome.store(metronome, Ordering::Relaxed);
        let mut engine = Box::new(Engine::try_new(
            project,
            controls.clone(),
            device_rate,
            start,
        )?);
        let mut owners = HashMap::new();
        owners.insert(
            (&*engine as *const Engine) as usize,
            engine.take_keyed_plugin_owners(),
        );
        let slots = Arc::new(Slots {
            pending: AtomicPtr::new(ptr::null_mut()),
            retired: AtomicPtr::new(ptr::null_mut()),
        });
        let current = (&*engine as *const Engine) as usize;
        let renderer = PlaybackRenderer {
            active: engine,
            slots: slots.clone(),
        };
        Ok((
            Self {
                slots,
                owners,
                project: project.clone(),
                current,
                device_rate,
                controls,
                _owner_thread: PhantomData,
            },
            renderer,
        ))
    }

    /// Prepare before publishing. A failure leaves the live graph untouched.
    /// Pending rapid edits coalesce; the callback always adopts a complete graph.
    pub fn update(&mut self, project: &Project) -> Result<()> {
        project.validate_routing()?;
        self.collect_retired();
        if same_structure(&self.project, project) {
            for (control, track) in self.controls.tracks.iter().zip(&project.tracks) {
                control.set(track);
            }
            for (control, bus) in self.controls.buses.iter().zip(&project.buses) {
                control.set(bus);
            }
            for (control, send) in self.controls.send_gains.iter().zip(
                project
                    .channel_routings()
                    .flat_map(|routing| &routing.sends),
            ) {
                control.store(send.gain.to_bits(), Ordering::Relaxed);
            }
            self.controls
                .master
                .store(project.master.to_bits(), Ordering::Relaxed);
            for (control, insert) in self.controls.insert_bypasses.iter().zip(
                project
                    .channel_routings()
                    .flat_map(|routing| &routing.inserts),
            ) {
                control.store(insert.bypass, Ordering::Relaxed);
            }
            // Report the edited session immediately, without waiting for the
            // callback. A bypassed missing insert remains a dry placeholder.
            let owners = &self.owners[&self.current];
            let missing = project
                .channel_routings()
                .flat_map(|routing| &routing.inserts)
                .enumerate()
                .filter(|(slot, insert)| {
                    !insert.bypass
                        && matches!(insert.kind, crate::InsertKind::Clap { .. })
                        && !owners.iter().any(|(index, _)| index == slot)
                })
                .count() as u32;
            self.controls
                .unavailable_plugins
                .store(missing, Ordering::Relaxed);
            self.project = project.clone();
            return Ok(());
        }
        let controls = Controls::new(project);
        controls.metronome.store(
            self.controls.metronome.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        let start = self.controls.position.load(Ordering::Relaxed);
        let mut engine = Box::new(Engine::try_new(
            project,
            Arc::new(controls),
            self.device_rate,
            start,
        )?);
        // Preparation used private transport atomics and never rewound the live
        // clock. Join them only after all fallible work has succeeded.
        let controls = Arc::get_mut(&mut engine.controls).expect("unpublished controls are unique");
        controls.position = self.controls.position.clone();
        controls.seek = self.controls.seek.clone();
        controls.playing = self.controls.playing.clone();
        controls.error = self.controls.error.clone();
        controls.metronome = self.controls.metronome.clone();
        let next_controls = engine.controls.clone();
        self.close_editors()?;
        let owners = engine.take_keyed_plugin_owners();
        let pointer = Box::into_raw(engine);
        self.owners.insert(pointer as usize, owners);
        // swap transfers sole ownership of the old pending box to this thread;
        // a concurrent consumer either takes it first or takes the new box.
        self.current = pointer as usize;
        let superseded = self.slots.pending.swap(pointer, Ordering::AcqRel);
        self.dispose(superseded);
        self.controls = next_controls;
        self.project = project.clone();
        Ok(())
    }

    /// Slot is the flattened track-then-bus insert index in this generation.
    /// Replacements close windows before retiring their exact DSP instances.
    pub fn open_editor(&self, slot: usize) -> Result<bool> {
        let owner = self
            .owners
            .get(&self.current)
            .and_then(|owners| owners.iter().find(|(index, _)| *index == slot));
        match owner {
            Some((_, owner)) => Ok(owner.open_editor()?),
            None => Ok(false),
        }
    }
    pub fn has_open_editors(&self) -> Result<bool> {
        for owners in self.owners.values() {
            for (_, owner) in owners {
                if owner.editor_is_open()? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    pub fn close_editors(&self) -> Result<()> {
        for owners in self.owners.values() {
            for (_, owner) in owners {
                owner.close_editor()?;
            }
        }
        Ok(())
    }
    /// Poll owner callbacks and snapshot GUI changes without rebuilding DSP.
    pub fn poll_plugins(
        &mut self,
        force: bool,
    ) -> Result<Vec<(usize, crate::plugins::ClapInsert)>> {
        for owners in self.owners.values() {
            for (_, owner) in owners {
                owner.service_main_thread()?;
            }
        }
        let mut changes = Vec::new();
        if let Some(owners) = self.owners.get(&self.current) {
            for (slot, owner) in owners {
                let previous = self
                    .project
                    .channel_routings()
                    .flat_map(|r| &r.inserts)
                    .nth(*slot);
                if let Some(crate::Insert {
                    kind: crate::InsertKind::Clap { plugin },
                    ..
                }) = previous
                    && let Some(next) = crate::plugins::snapshot_live(owner, plugin, force)?
                    && next != *plugin
                {
                    changes.push((*slot, next));
                }
            }
        }
        for (slot, next) in &changes {
            if let Some(insert) = self
                .project
                .tracks
                .iter_mut()
                .map(|t| &mut t.routing)
                .chain(self.project.buses.iter_mut().map(|b| &mut b.routing))
                .flat_map(|r| &mut r.inserts)
                .nth(*slot)
            {
                insert.kind = crate::InsertKind::Clap {
                    plugin: next.clone(),
                };
            }
        }
        Ok(changes)
    }
    pub fn collect_retired(&mut self) {
        let retired = self.slots.retired.swap(ptr::null_mut(), Ordering::AcqRel);
        self.dispose(retired);
    }

    fn dispose(&mut self, pointer: *mut Engine) {
        if !pointer.is_null() {
            // Removed atomically from a mailbox; no renderer can access it.
            unsafe {
                drop(Box::from_raw(pointer));
            }
            self.owners.remove(&(pointer as usize));
        }
    }
}
impl Drop for Playback {
    fn drop(&mut self) {
        self.collect_retired();
        let pending = self.slots.pending.swap(ptr::null_mut(), Ordering::AcqRel);
        self.dispose(pending);
        // Backend shutdown has already destroyed the active realtime proxies.
        self.owners.clear();
    }
}
impl PlaybackRenderer {
    /// Call at the start of a device callback/producer block. If reclamation
    /// has not caught up, keep rendering the current graph instead of waiting.
    pub fn apply_pending(&mut self) {
        if !self.slots.retired.load(Ordering::Acquire).is_null() {
            return;
        }
        let next = self.slots.pending.swap(ptr::null_mut(), Ordering::AcqRel);
        if next.is_null() {
            return;
        }
        // The mailbox grants this endpoint exclusive ownership of the box.
        let mut next = unsafe { Box::from_raw(next) };
        next.continue_from(&self.active);
        let old = std::mem::replace(&mut self.active, next);
        self.slots
            .retired
            .store(Box::into_raw(old), Ordering::Release);
    }
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.active
    }
    pub fn render<T: OutputSample>(&mut self, output: &mut [T], channels: usize) {
        self.apply_pending();
        self.active.render(output, channels);
    }
}

// Metadata and already-atomic mixer values do not invalidate the prepared graph.
fn same_structure(a: &Project, b: &Project) -> bool {
    a.sample_rate == b.sample_rate
        && a.tempo == b.tempo
        && a.time_signature == b.time_signature
        && a.tracks.len() == b.tracks.len()
        && a.buses.len() == b.buses.len()
        && a.tracks.iter().zip(&b.tracks).all(|(a, b)| {
            same_routing(&a.routing, &b.routing)
                && a.clips.len() == b.clips.len()
                && a.clips.iter().zip(&b.clips).all(|(a, b)| {
                    a.start == b.start
                        && a.frames == b.frames
                        && a.source_offset == b.source_offset
                        && Arc::ptr_eq(&a.samples, &b.samples)
                })
        })
        && a.buses
            .iter()
            .zip(&b.buses)
            .all(|(a, b)| a.id == b.id && same_routing(&a.routing, &b.routing))
}
fn same_routing(a: &crate::ChannelRouting, b: &crate::ChannelRouting) -> bool {
    a.output == b.output
        && a.inserts.len() == b.inserts.len()
        && a.inserts
            .iter()
            .zip(&b.inserts)
            .all(|(a, b)| a.kind == b.kind)
        && a.sends.len() == b.sends.len()
        && a.sends.iter().zip(&b.sends).all(|(a, b)| {
            a.target == b.target && a.pre_fader == b.pre_fader && a.enabled == b.enabled
        })
}
