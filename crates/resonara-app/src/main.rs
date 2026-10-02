mod animation;
mod counter;
mod fader;
mod insert_slot;
mod knob;
mod meter;
mod native_editor;
mod profiling;
mod ruler;
mod send_knob;
mod send_slot;
#[cfg(test)]
mod tests;
mod timeline;
mod ui;
mod wave;
mod workbench;
use resonara_core::{BusId, BusKind};
use routing::{RoutingMenu, RoutingTarget};
use workbench::{RulerDrag, TrackMenu};

use resonara_core::{Clip, Project, Result};
#[cfg(not(test))]
use resonara_platform::Audio;
use scarlet_ui::{
    WindowContext, WindowId,
    event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent},
    file_dialog::{
        FileDialog, FileDialogError, FileDialogFilter, FileDialogHandle, FileDialogMode,
        FileDialogOutcome, FileDialogResult,
    },
    prelude::*,
    vstack,
};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, atomic::Ordering, mpsc},
    time::Instant,
};
#[cfg(test)]
use tests::TestAudio as Audio;
use ui::*;
macro_rules! row { ($($v:expr),* $(,)?) => { HStack::new(Children(vec![$(Box::new($v) as Box<dyn View>),*])) }; }
mod channel_strip;
mod insert_editor;
mod inspector;
mod routing;
mod send_editor;

#[derive(Clone)]
struct Snapshot {
    project: Project,
    selected: usize,
    selected_bus: Option<BusId>,
    clip: Option<usize>,
    version: u64,
    path: Option<PathBuf>,
}
struct History {
    state: Snapshot,
    label: String,
    restore_path: bool,
}
#[derive(Clone, Copy, PartialEq)]
enum FileAction {
    Import,
    Open,
    Save,
    Export,
}
#[derive(Clone, PartialEq)]
enum Dialog {
    None,
    File(FileAction),
    Native(FileAction),
    ConfirmOpen,
    ConfirmClose,
    Help,
    InsertValue(RoutingTarget, usize),
    InsertPicker(RoutingTarget),
    InsertActions(RoutingTarget, usize),
    ClapEditor(RoutingTarget, usize),
    ClapPicker(RoutingTarget),
    SendPicker(RoutingTarget, Option<usize>),
    SendActions(RoutingTarget, usize),
    SendLevel(RoutingTarget, usize),
}
#[derive(Clone)]
struct FileEntry {
    name: String,
    path: PathBuf,
    directory: bool,
}
#[derive(Clone, Copy, PartialEq)]
enum DragMode {
    Move,
    Left,
    Right,
}
struct Drag {
    track: usize,
    clip: usize,
    x: f32,
    original: Clip,
    before: Snapshot,
    mode: DragMode,
    moved: bool,
}
struct Channel {
    gain: State<f32>,
    gain_normalized: State<f32>,
    pan: State<f32>,
    meter: State<String>,
    peak: State<meter::StereoMeter>,
    focused: State<bool>,
    pan_focused: State<bool>,
    dragging_gain: State<bool>,
    dragging_pan: State<bool>,
    canvas: SgfxCanvasHandle,
    playhead_mesh: SgfxMeshHandle,
    frame: State<Arc<SgfxCanvasFrame>>,
    mesh: Arc<SgfxMesh>,
}
struct IoResult {
    action: FileAction,
    path: PathBuf,
    version: u64,
    result: std::result::Result<Option<Project>, String>,
}
struct Model {
    project: Project,
    audio: Option<Audio>,
    // Keep stopped native editor/DSP sessions alive for resume, and retain
    // naturally finished outputs for SAS's separately queued hardware tail.
    retired_audio: Option<Audio>,
    selected: usize,
    selected_bus: Option<BusId>,
    clip: Option<usize>,
    undo: Vec<History>,
    redo: Vec<History>,
    version: u64,
    saved_version: u64,
    next_version: u64,
    drag: Option<Drag>,
    mixer_before: Option<Snapshot>,
    peaks: wave::Peaks,
    channels: Vec<Channel>,
    bus_channels: Vec<Channel>,
    io: Option<mpsc::Receiver<IoResult>>,
    picker: Option<(FileAction, FileDialogHandle)>,
    current_path: Option<PathBuf>,
}
#[derive(Clone)]
struct Daw {
    model: Rc<RefCell<Model>>,
    revision: State<u64>,
    size: State<Size>,
    arrangement_size: State<Size>,
    status: State<String>,
    clock: State<String>,
    time_format: State<timeline::Format>,
    tempo_input: State<String>,
    signature_input: State<String>,
    track_menu: State<Option<TrackMenu>>,
    menu_choice: State<usize>,
    routing_menu: State<Option<RoutingMenu>>,
    routing_value: State<String>,
    insert_focus: Rc<RefCell<std::collections::HashMap<(RoutingTarget, usize), State<bool>>>>,
    plugin_fields: Rc<RefCell<Vec<(u32, State<String>)>>>,
    plugin_catalog: Rc<RefCell<resonara_core::plugins::ClapCatalog>>,
    native_editor: Rc<RefCell<Option<(RoutingTarget, usize, resonara_core::plugins::ClapEditor)>>>,
    native_edit_group: Rc<Cell<bool>>,
    native_edit_time: Rc<Cell<Instant>>,
    send_controls:
        Rc<RefCell<std::collections::HashMap<(RoutingTarget, usize), routing::SendControl>>>,
    follow_playhead: State<bool>,
    follow_suspended: Rc<Cell<bool>>,
    ruler_drag: Rc<RefCell<Option<RulerDrag>>>,
    metronome: State<bool>,
    cursor: State<String>,
    range_end: State<String>,
    track_name: State<String>,
    master: State<f32>,
    master_dragging: State<bool>,
    master_focus: State<bool>,
    master_peak: State<meter::StereoMeter>,
    master_meter: State<String>,
    dialog: State<Dialog>,
    path: State<String>,
    filename: State<String>,
    files: State<Vec<FileEntry>>,
    file_selected: State<Option<usize>>,
    dialog_error: State<String>,
    inspector: State<bool>,
    inspector_details: State<bool>,
    inspector_fader_focus: State<bool>,
    inspector_pan_focus: State<bool>,
    inspector_fraction: State<f32>,
    mixer_fraction: State<f32>,
    mixer_visible: State<bool>,
    snap: State<bool>,
    tool: State<usize>,
    view_start: State<f64>,
    view_span: State<f64>,
    focus: State<bool>,
    window_id: Rc<Cell<Option<WindowId>>>,
    close_after_save: Rc<Cell<bool>>,
    open_after_save: Rc<Cell<bool>>,
    last_meter: Rc<Cell<Instant>>,
    playhead: State<f64>,
    last_playhead: Rc<Cell<Instant>>,
    last_frame: Rc<Cell<u64>>,
    profiler: Rc<RefCell<profiling::Profiler>>,
}
fn state<T: 'static>(id: u64, value: T) -> State<T> {
    State::new(StateId::new(id as u32), value)
}
impl Daw {
    fn new(project: Project) -> Self {
        let span = (project.duration() as f64 / project.sample_rate as f64 * 1.20).max(5.);
        let master = project.master;
        let s = Self {
            model: Rc::new(RefCell::new(Model {
                project,
                audio: None,
                retired_audio: None,
                selected: 0,
                selected_bus: None,
                clip: None,
                undo: vec![],
                redo: vec![],
                version: 1,
                saved_version: 1,
                next_version: 2,
                drag: None,
                mixer_before: None,
                peaks: wave::Peaks::default(),
                channels: vec![],
                bus_channels: vec![],
                io: None,
                picker: None,
                current_path: None,
            })),
            revision: state(1, 0),
            size: state(2, Size::new(1280., 790.)),
            arrangement_size: state(3, Size::new(1044., 420.)),
            status: state(
                4,
                "Ready · select a region, drag to move, or drag an edge to trim".into(),
            ),
            clock: state(5, "00:00.000".into()),
            time_format: state(30, timeline::Format::Bars),
            tempo_input: state(31, String::new()),
            signature_input: state(35, String::new()),
            track_menu: state(32, None),
            menu_choice: state(33, 0),
            routing_menu: state(37, None),
            routing_value: state(38, String::new()),
            insert_focus: Rc::new(RefCell::new(std::collections::HashMap::new())),
            plugin_fields: Rc::new(RefCell::new(vec![])),
            plugin_catalog: Rc::new(RefCell::new(Default::default())),
            native_editor: Rc::new(RefCell::new(None)),
            native_edit_group: Rc::new(Cell::new(false)),
            native_edit_time: Rc::new(Cell::new(Instant::now())),
            send_controls: Rc::new(RefCell::new(std::collections::HashMap::new())),
            follow_playhead: state(34, false),
            follow_suspended: Rc::new(Cell::new(false)),
            ruler_drag: Rc::new(RefCell::new(None)),
            metronome: state(36, false),
            cursor: state(6, "0.000".into()),
            range_end: state(7, "1.000".into()),
            track_name: state(8, String::new()),
            master: state(9, master),
            master_dragging: state(10, false),
            master_focus: state(24, false),
            master_peak: state(25, meter::StereoMeter::default()),
            master_meter: state(28, meter::StereoMeter::default().summary()),
            dialog: state(11, Dialog::None),
            path: state(12, String::new()),
            filename: state(13, String::new()),
            files: state(14, vec![]),
            file_selected: state(15, None),
            dialog_error: state(16, String::new()),
            inspector: state(17, true),
            inspector_details: state(39, false),
            inspector_fader_focus: state(40, false),
            inspector_pan_focus: state(41, false),
            inspector_fraction: state(26, 0.185),
            mixer_fraction: state(27, 0.),
            mixer_visible: state(18, true),
            snap: state(19, true),
            tool: state(20, 0),
            view_start: state(21, 0.),
            view_span: state(22, span),
            focus: state(23, false),
            window_id: Rc::new(Cell::new(None)),
            close_after_save: Rc::new(Cell::new(false)),
            open_after_save: Rc::new(Cell::new(false)),
            last_meter: Rc::new(Cell::new(Instant::now())),
            playhead: state(29, 0.),
            last_playhead: Rc::new(Cell::new(Instant::now())),
            last_frame: Rc::new(Cell::new(0)),
            profiler: Rc::new(RefCell::new(profiling::Profiler::from_env())),
        };
        s.refresh(true);
        s
    }
    fn snapshot(m: &Model) -> Snapshot {
        Snapshot {
            project: m.project.clone(),
            selected: m.selected,
            selected_bus: m.selected_bus,
            clip: m.clip,
            version: m.version,
            path: m.current_path.clone(),
        }
    }
    fn restore(m: &mut Model, s: Snapshot) {
        m.project = s.project;
        m.selected_bus = s.selected_bus.filter(|id| m.project.bus(*id).is_some());
        m.selected = s.selected.min(m.project.tracks.len().saturating_sub(1));
        m.clip = s.clip.filter(|i| {
            m.project
                .tracks
                .get(m.selected)
                .is_some_and(|t| *i < t.clips.len())
        });
        m.version = s.version;
        m.current_path = s.path;
    }
    fn history(m: &mut Model, before: Snapshot, label: &str) {
        m.undo.push(History {
            state: before,
            label: label.into(),
            restore_path: false,
        });
        if m.undo.len() > 100 {
            m.undo.remove(0);
        }
        m.redo.clear();
        m.version = m.next_version;
        m.next_version += 1;
    }
    fn changed(&self) {
        self.revision.update(|v| *v = v.wrapping_add(1));
    }
    fn dirty(&self) -> bool {
        let m = self.model.borrow();
        m.version != m.saved_version
    }
    fn seconds(&self, s: &str) -> Result<u64> {
        let v: f64 = s.parse()?;
        if !v.is_finite() || !(0.0..=86400.).contains(&v) {
            return Err("Time must be between 0 and 86400 seconds".into());
        }
        Ok((v * self.model.borrow().project.sample_rate as f64).round() as u64)
    }
    fn submit_tempo(&self) {
        let parsed = self
            .tempo_input
            .get()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && (20.0..=400.0).contains(v));
        let Some(tempo) = parsed else {
            self.status.set("Enter a BPM between 20 and 400".into());
            self.tempo_input
                .set(format!("{}", self.model.borrow().project.tempo));
            return;
        };
        if self.model.borrow().project.tempo != tempo {
            self.edit("Set tempo", |m| {
                m.project.tempo = tempo;
                Ok(())
            });
        } else {
            self.tempo_input.set(format!("{tempo}"));
        }
    }
    fn submit_signature(&self) {
        let text = self.signature_input.get();
        let parsed = text
            .trim()
            .split_once('/')
            .and_then(|(n, d)| {
                Some(resonara_core::TimeSignature {
                    numerator: n.trim().parse().ok()?,
                    denominator: d.trim().parse().ok()?,
                })
            })
            .filter(|m| m.valid());
        if let Some(meter) = parsed {
            if self.model.borrow().project.time_signature != meter {
                self.edit("Set time signature", |m| {
                    m.project.time_signature = meter;
                    Ok(())
                });
            } else {
                self.signature_input
                    .set(format!("{}/{}", meter.numerator, meter.denominator));
            }
        } else {
            self.status
                .set("Meter: 1–32 beats, denominator 1, 2, 4, 8, 16 or 32".into());
            let meter = self.model.borrow().project.time_signature;
            self.signature_input
                .set(format!("{}/{}", meter.numerator, meter.denominator));
        }
    }
    fn cycle_time_format(&self) {
        self.time_format.set(self.time_format.get().next());
        self.refresh(true);
    }
    fn refresh(&self, waveforms: bool) {
        let mut m = self.model.borrow_mut();
        self.tempo_input.set(format!("{}", m.project.tempo));
        self.signature_input.set(format!(
            "{}/{}",
            m.project.time_signature.numerator, m.project.time_signature.denominator
        ));
        self.clock.set(self.time_format.get().position(
            self.playhead.get(),
            m.project.tempo,
            m.project.sample_rate,
            m.project.time_signature,
        ));
        m.selected = m.selected.min(m.project.tracks.len().saturating_sub(1));
        m.selected_bus = m.selected_bus.filter(|id| m.project.bus(*id).is_some());
        if let Some(bus) = m.selected_bus.and_then(|id| m.project.bus(id)) {
            self.track_name.set(bus.name.clone());
        } else if let Some(t) = m.project.tracks.get(m.selected) {
            self.track_name.set(t.name.clone());
        } else {
            self.track_name.set(String::new());
        }
        self.master.set(m.project.master);
        let len = m.project.tracks.len();
        while m.channels.len() < len {
            let i = m.channels.len();
            let id = 1000 + i as u64 * 10;
            m.channels.push(Channel::new(id));
        }
        m.channels.truncate(len);
        for i in 0..len {
            m.channels[i].gain.set(m.project.tracks[i].gain);
            m.channels[i]
                .gain_normalized
                .set(fader::gain_to_fraction(m.project.tracks[i].gain));
            m.channels[i].pan.set(m.project.tracks[i].pan);
        }
        Self::refresh_bus_channels(&mut m);
        if waveforms {
            if self.profiler.borrow().active() {
                self.profiler.borrow_mut().waveform_refreshes += 1;
            }
            let width = (self.arrangement_size.get().width - HEADER).max(200.);
            let start = self.view_start.get();
            let span = self.view_span.get();
            let Model {
                project,
                peaks,
                channels,
                selected,
                clip,
                ..
            } = &mut *m;
            peaks.retain(&project.tracks);
            for (i, t) in project.tracks.iter().enumerate() {
                channels[i].mesh = wave::base(
                    t,
                    i,
                    if i == *selected { *clip } else { None },
                    width,
                    ROW,
                    start,
                    span,
                    self.time_format.get().step(
                        span,
                        project.tempo,
                        project.sample_rate,
                        project.time_signature,
                    ),
                    project.sample_rate,
                    peaks,
                    channels[i].mesh.handle(),
                    self.last_frame.get().wrapping_add(1),
                );
            }
        }
        drop(m);
        self.update_frames();
        self.changed();
    }
    fn animate_playhead(&self, position: f64) {
        if self.playhead.get() != position {
            self.playhead.set(position);
        }
        let m = self.model.borrow();
        let clock = self.time_format.get().position(
            position,
            m.project.tempo,
            m.project.sample_rate,
            m.project.time_signature,
        );
        if self.clock.get() != clock {
            self.clock.set(clock);
        }
        drop(m);
        self.follow_position(position);
        if self.profiler.borrow().active() {
            self.profiler.borrow_mut().playhead_updates += 1;
        }
    }
    fn update_frames(&self) {
        let m = self.model.borrow();
        let width = (self.arrangement_size.get().width - HEADER).max(200.);
        let rev = self.last_frame.get().wrapping_add(1);
        self.last_frame.set(rev);
        for (i, c) in m.channels.iter().enumerate() {
            c.frame.set(wave::frame(
                c.mesh.clone(),
                width,
                ROW,
                self.view_start.get(),
                self.view_span.get(),
                -1.,
                i == m.selected,
                rev,
                c.playhead_mesh,
            ));
        }
    }
    fn finish_mix(&self) {
        let mut m = self.model.borrow_mut();
        if let Some(before) = m.mixer_before.take() {
            let changed = before
                .project
                .buses
                .iter()
                .zip(&m.project.buses)
                .any(|(a, b)| {
                    a.gain != b.gain
                        || a.pan != b.pan
                        || a.mute != b.mute
                        || a.routing.sends != b.routing.sends
                })
                || before.project.master != m.project.master
                || before
                    .project
                    .tracks
                    .iter()
                    .zip(&m.project.tracks)
                    .any(|(a, b)| {
                        a.gain != b.gain
                            || a.pan != b.pan
                            || a.mute != b.mute
                            || a.routing.sends != b.routing.sends
                            || a.solo != b.solo
                    });
            if changed {
                Self::history(&mut m, before, "Mixer change");
            }
        }
        drop(m);
        self.changed();
    }
    fn edit(&self, label: &str, f: impl FnOnce(&mut Model) -> Result<()>) {
        self.poll_native_editors(true);
        self.native_edit_group.set(false);
        self.track_menu.set(None);
        self.routing_menu.set(None);
        if self.busy() {
            return;
        }
        self.finish_mix();
        let mut m = self.model.borrow_mut();
        // Retain a stopped editor across unrelated edits. Its slot is scoped to
        // this project layout, so invalidate it if the edit replaces that insert.
        let editor_insert = self
            .native_editor
            .borrow()
            .as_ref()
            .map(|(target, slot, _)| {
                (
                    *target,
                    *slot,
                    target
                        .get(&m.project)
                        .and_then(|r| r.inserts.get(*slot))
                        .map(|i| i.kind.clone()),
                )
            });
        let before = Self::snapshot(&m);
        match f(&mut m).and_then(|()| Self::sync_audio(&mut m)) {
            Ok(()) => {
                Self::history(&mut m, before, label);
                self.status.set(Self::edit_status(&m, label.into()));
            }
            Err(e) => {
                Self::restore(&mut m, before);
                self.status.set(format!("Could not {label}: {e}"));
            }
        }
        let invalidate_editor = editor_insert.is_some_and(|(target, slot, previous)| {
            previous
                != target
                    .get(&m.project)
                    .and_then(|r| r.inserts.get(slot))
                    .map(|i| i.kind.clone())
        });
        drop(m);
        if invalidate_editor {
            // The old state was captured before the edit. Do not snapshot the
            // retired editor into a slot that now belongs to a different insert.
            self.native_editor.borrow_mut().take();
        }
        self.refresh(true);
    }
    fn sync_audio(m: &mut Model) -> Result<()> {
        if let Some(audio) = m.audio.as_mut().or(m.retired_audio.as_mut()) {
            audio.update(&m.project)?;
        }
        Ok(())
    }
    fn edit_status(m: &Model, label: String) -> String {
        let missing = m.audio.as_ref().map_or(0, |a| {
            a.controls.unavailable_plugins.load(Ordering::Relaxed)
        });
        if missing == 0 {
            label
        } else {
            format!("{label} · {missing} unavailable CLAP insert(s) bypassed")
        }
    }
    fn undo(&self, redo: bool) {
        self.poll_native_editors(true);
        self.native_edit_group.set(false);
        if self.busy() {
            return;
        }
        self.finish_mix();
        let mut m = self.model.borrow_mut();
        let item = if redo { m.redo.pop() } else { m.undo.pop() };
        if let Some(h) = item {
            let current = Self::snapshot(&m);
            let label = h.label;
            let restore_path = h.restore_path;
            let current_path = m.current_path.clone();
            let rejected = h.state.clone();
            Self::restore(&mut m, h.state);
            if !restore_path {
                m.current_path = current_path;
            }
            if let Err(error) = Self::sync_audio(&mut m) {
                Self::restore(&mut m, current);
                let item = History {
                    state: rejected,
                    label,
                    restore_path,
                };
                if redo {
                    m.redo.push(item);
                } else {
                    m.undo.push(item);
                }
                self.status.set(format!("Could not restore edit: {error}"));
                drop(m);
                self.refresh(true);
                return;
            }
            let next = History {
                state: current,
                label: label.clone(),
                restore_path,
            };
            if redo {
                m.undo.push(next);
            } else {
                m.redo.push(next);
            }
            self.status.set(Self::edit_status(
                &m,
                format!("{}: {label}", if redo { "Redid" } else { "Undid" }),
            ));
        }
        drop(m);
        self.refresh(true);
    }
    fn choose(&self, index: usize, clip: Option<usize>) {
        self.insert_focus.borrow_mut().clear();
        self.routing_menu.set(None);
        self.finish_mix();
        let mut m = self.model.borrow_mut();
        if index < m.project.tracks.len() {
            m.selected = index;
            m.selected_bus = None;
            m.clip = clip;
        }
        drop(m);
        self.refresh(true);
    }
    fn select(&self, delta: isize) {
        let m = self.model.borrow();
        let n = m.project.tracks.len();
        let i = (m.selected as isize + delta).clamp(0, n.saturating_sub(1) as isize) as usize;
        drop(m);
        self.choose(i, None);
    }
    fn mix_normalized(&self, index: usize, position: f32) {
        self.mix(index, Some(fader::gain_from_fraction(position)), None, None);
    }
    fn mix(&self, index: usize, gain: Option<f32>, pan: Option<f32>, toggle: Option<bool>) {
        let mut m = self.model.borrow_mut();
        if (m.io.is_some() || m.picker.is_some()) || index >= m.project.tracks.len() {
            return;
        }
        if m.mixer_before.is_none() {
            m.mixer_before = Some(Self::snapshot(&m));
        }
        let t = &mut m.project.tracks[index];
        if let Some(v) = gain {
            t.gain = v;
        }
        if let Some(v) = pan {
            t.pan = v;
        }
        if let Some(solo) = toggle {
            if solo {
                t.solo = !t.solo;
            } else {
                t.mute = !t.mute;
            }
        }
        if let Some(a) = &m.audio {
            if let Some(c) = a.controls.tracks.get(index) {
                c.set(&m.project.tracks[index]);
            }
        }
        drop(m);
        if toggle.is_some() {
            self.finish_mix();
        }
        self.refresh(toggle.is_some());
    }
    fn master_change(&self, value: f32) {
        let mut m = self.model.borrow_mut();
        if m.io.is_some() || m.picker.is_some() {
            return;
        }
        if m.mixer_before.is_none() {
            m.mixer_before = Some(Self::snapshot(&m));
        }
        m.project.master = value;
        if let Some(a) = &m.audio {
            a.controls.master.store(value.to_bits(), Ordering::Relaxed);
        }
        drop(m);
        self.master.set(value);
        self.changed();
    }
    fn play(&self) {
        if self.model.borrow().audio.is_some() {
            self.stop_audio(true);
            return;
        }
        self.poll_native_editors(true);
        self.finish_mix();
        self.reset_meters();
        let result = self.seconds(&self.cursor.get()).and_then(|start| {
            let mut m = self.model.borrow_mut();
            if m.project.duration() == 0 && !self.metronome.get() {
                return Err("Import an audio file first, or enable the metronome".into());
            }
            let start = if m.project.duration() > 0 && start >= m.project.duration() {
                0
            } else {
                start
            };
            let a = if let Some(mut audio) = m.retired_audio.take() {
                audio.update(&m.project)?;
                if audio.resume(start, self.metronome.get()) {
                    audio
                } else {
                    drop(audio);
                    Audio::start_with_metronome(&m.project, start, self.metronome.get())?
                }
            } else {
                Audio::start_with_metronome(&m.project, start, self.metronome.get())?
            };
            let missing = a.controls.unavailable_plugins.load(Ordering::Relaxed);
            self.status.set(if missing == 0 {
                format!("Playing · {}", a.device)
            } else {
                format!(
                    "Playing · {} · {missing} unavailable CLAP insert(s) bypassed",
                    a.device
                )
            });
            m.audio = Some(a);
            self.playhead
                .set(start as f64 / m.project.sample_rate as f64);
            self.follow_suspended.set(false);
            Ok(())
        });
        if let Err(e) = result {
            self.status.set(format!("Playback unavailable: {e}"));
        }
        if self.model.borrow().audio.is_some() {
            self.animate_playhead(self.playhead.get());
        }
        self.changed();
    }
    fn toggle_metronome(&self) {
        let enabled = !self.metronome.get();
        self.metronome.set(enabled);
        if let Some(audio) = &self.model.borrow().audio {
            audio.controls.metronome.store(enabled, Ordering::Relaxed);
        }
        self.changed();
    }
    fn snap_grid_for(&self, project: &Project) -> timeline::SnapGrid {
        timeline::snap_grid(
            self.time_format.get(),
            self.view_span.get(),
            (self.arrangement_size.get().width - HEADER).max(200.),
            project.tempo,
            project.sample_rate,
            project.time_signature,
        )
    }
    fn snap_position(&self, seconds: f64) -> f64 {
        if self.snap.get() {
            self.snap_grid_for(&self.model.borrow().project)
                .position(seconds)
        } else {
            seconds
        }
    }
    fn stop_audio(&self, message: bool) {
        self.finish_audio(message, false);
    }
    fn finish_audio(&self, message: bool, preserve_device_tail: bool) {
        self.poll_native_editors(true);
        let mut m = self.model.borrow_mut();
        if m.retired_audio
            .as_ref()
            .is_some_and(|a| !a.has_open_editors())
        {
            m.retired_audio = None;
        }
        if let Some(a) = m.audio.take() {
            let pos =
                a.controls.position.load(Ordering::Relaxed) as f64 / m.project.sample_rate as f64;
            self.playhead.set(pos);
            self.cursor.set(timeline::seconds_input(pos));
            self.clock.set(self.time_format.get().position(
                pos,
                m.project.tempo,
                m.project.sample_rate,
                m.project.time_signature,
            ));
            a.pause();
            if preserve_device_tail
                || (a.has_open_editors() && !a.controls.error.load(Ordering::Relaxed))
            {
                m.retired_audio = Some(a);
            }
        }
        if message {
            self.status.set("Stopped".into());
        }
        drop(m);
        self.update_meters(None, 0.);
        self.update_frames();
        self.changed();
    }
    fn reset_meters(&self) {
        for channel in self
            .model
            .borrow()
            .channels
            .iter()
            .chain(&self.model.borrow().bus_channels)
        {
            channel.peak.set(meter::StereoMeter::default());
            channel.meter.set(meter::StereoMeter::default().summary());
        }
        self.master_peak.set(meter::StereoMeter::default());
        self.master_meter
            .set(meter::StereoMeter::default().summary());
    }
    /// Consume independent L/R interval maxima once and update bars and text
    /// from that same snapshot. No audio callback work or locks occur here.
    fn update_meters(&self, controls: Option<&resonara_core::Controls>, elapsed: f32) {
        if self.profiler.borrow().active() {
            self.profiler.borrow_mut().meter_updates += 1;
        }
        let update = |state: &State<meter::StereoMeter>, text: &State<String>, input: [f32; 2]| {
            let old = state.get();
            let mut next = old;
            next.advance(input, elapsed);
            if next != old {
                let summary = next.summary();
                state.set(next);
                if text.get() != summary {
                    text.set(summary);
                }
            }
        };
        for (index, channel) in self.model.borrow().channels.iter().enumerate() {
            let input = controls
                .and_then(|c| c.tracks.get(index))
                .map_or([0.; 2], |track| {
                    // Retain the legacy mono telemetry field's reset semantics too.
                    track.peak.swap(0, Ordering::Relaxed);
                    [
                        f32::from_bits(track.peak_left.swap(0, Ordering::Relaxed)),
                        f32::from_bits(track.peak_right.swap(0, Ordering::Relaxed)),
                    ]
                });
            update(&channel.peak, &channel.meter, input);
        }
        for (index, channel) in self.model.borrow().bus_channels.iter().enumerate() {
            let input = controls
                .and_then(|c| c.buses.get(index))
                .map_or([0.; 2], |bus| {
                    bus.peak.swap(0, Ordering::Relaxed);
                    [
                        f32::from_bits(bus.peak_left.swap(0, Ordering::Relaxed)),
                        f32::from_bits(bus.peak_right.swap(0, Ordering::Relaxed)),
                    ]
                });
            update(&channel.peak, &channel.meter, input);
        }
        let input = controls.map_or([0.; 2], |c| {
            [
                f32::from_bits(c.master_peak_left.swap(0, Ordering::Relaxed)),
                f32::from_bits(c.master_peak_right.swap(0, Ordering::Relaxed)),
            ]
        });
        update(&self.master_peak, &self.master_meter, input);
    }
    fn seek(&self, seconds: f64) {
        let was_playing = self.model.borrow().audio.is_some();
        let seconds = seconds.max(0.);
        let direct = {
            let m = self.model.borrow();
            let start = (seconds * m.project.sample_rate as f64).round() as u64;
            if let Some(audio) = m.audio.as_ref() {
                audio.resume(start, self.metronome.get())
            } else {
                m.retired_audio
                    .as_ref()
                    .is_some_and(|audio| audio.seek(start))
            }
        };
        if !direct {
            self.stop_audio(false);
        }
        self.playhead.set(seconds);
        self.cursor.set(timeline::seconds_input(seconds));
        self.animate_playhead(seconds);
        self.update_frames();
        if was_playing && !direct {
            self.play();
        }
    }
    fn release_meters(&self, elapsed: f32) {
        let release = |state: &State<meter::StereoMeter>| {
            let old = state.get();
            let mut next = old;
            next.release(elapsed);
            if next != old {
                state.set(next);
            }
        };
        for channel in self
            .model
            .borrow()
            .channels
            .iter()
            .chain(&self.model.borrow().bus_channels)
        {
            release(&channel.peak);
        }
        release(&self.master_peak);
    }
    fn split(&self) {
        if self.model.borrow().selected_bus.is_some() {
            return;
        }
        let at = match self.seconds(&self.cursor.get()) {
            Ok(v) => v,
            Err(e) => {
                self.status.set(e.to_string());
                return;
            }
        };
        self.edit("Split region", |m| {
            m.project.split(m.selected, at)?;
            m.clip = m.project.tracks[m.selected]
                .clips
                .iter()
                .position(|c| c.start == at);
            Ok(())
        });
    }
    fn duplicate(&self) {
        if self.model.borrow().selected_bus.is_some() {
            self.status.set("Select an audio track to duplicate".into());
            return;
        }
        self.edit("Duplicate track", |m| {
            let mut t = m
                .project
                .tracks
                .get(m.selected)
                .ok_or("Select a track")?
                .clone();
            t.name.push_str(" copy");
            m.project.tracks.insert(m.selected + 1, t);
            m.selected += 1;
            m.clip = None;
            Ok(())
        });
    }
    fn delete(&self, track: bool) {
        if self.model.borrow().selected_bus.is_some() {
            self.status
                .set("Use Delete bus in the inspector to remove this bus".into());
            return;
        }
        self.edit(
            if track {
                "Delete track"
            } else {
                "Delete region"
            },
            |m| {
                let t = m
                    .project
                    .tracks
                    .get_mut(m.selected)
                    .ok_or("Select a track")?;
                if !track {
                    if let Some(i) = m.clip {
                        t.clips.remove(i);
                        m.clip = None;
                    } else {
                        return Err(
                            "Select a region, or right-click its track header to delete the track"
                                .into(),
                        );
                    }
                } else {
                    m.project.tracks.remove(m.selected);
                    m.selected = m.selected.min(m.project.tracks.len().saturating_sub(1));
                    m.clip = None;
                }
                Ok(())
            },
        );
    }
    fn trim_range(&self) {
        if self.model.borrow().selected_bus.is_some() {
            return;
        }
        let r = self
            .seconds(&self.cursor.get())
            .and_then(|a| self.seconds(&self.range_end.get()).map(|b| (a, b)));
        match r {
            Ok((a, b)) => self.edit("Trim track to range", |m| {
                m.project.trim(m.selected, a, b)?;
                m.clip = None;
                Ok(())
            }),
            Err(e) => self.status.set(e.to_string()),
        }
    }
    fn zoom(&self, factor: f64) {
        let old = self.view_span.get();
        let span = (old * factor).clamp(0.05, 86400.);
        let center = self
            .playhead
            .get()
            .clamp(self.view_start.get(), self.view_start.get() + old);
        let ratio = (center - self.view_start.get()) / old;
        self.view_span.set(span);
        self.view_start.set((center - ratio * span).max(0.));
        self.refresh(true);
    }
    fn fit(&self) {
        let m = self.model.borrow();
        self.view_span
            .set((m.project.duration() as f64 / m.project.sample_rate as f64 * 1.15).max(1.));
        self.view_start.set(0.);
        drop(m);
        self.refresh(true);
    }
    fn timeline_event(&self, index: usize, e: &Event) -> bool {
        if self.busy() {
            return true;
        }
        let width = (self.arrangement_size.get().width - HEADER).max(200.);
        let seconds =
            |x: f32| self.view_start.get() + x as f64 / width as f64 * self.view_span.get();
        match e {
            Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                ..
            }) => {
                self.finish_mix();
                self.focus.set(true);
                let at = seconds(*x as f32).max(0.);
                let mut m = self.model.borrow_mut();
                if index >= m.project.tracks.len() {
                    return false;
                }
                let rate = m.project.sample_rate;
                let was_playing = m.audio.is_some();
                let frame = (at * rate as f64) as u64;
                let hit = m.project.tracks[index]
                    .clips
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, c)| frame >= c.start && frame <= c.start + c.frames as u64)
                    .map(|(i, _)| i);
                m.selected = index;
                m.selected_bus = None;
                self.routing_menu.set(None);
                m.clip = hit;
                if let Some(ci) = hit {
                    let c = m.project.tracks[index].clips[ci].clone();
                    let start = (c.start as f64 / rate as f64 - self.view_start.get())
                        / self.view_span.get()
                        * width as f64;
                    let end = ((c.start + c.frames as u64) as f64 / rate as f64
                        - self.view_start.get())
                        / self.view_span.get()
                        * width as f64;
                    let mode = if (*x as f64 - start).abs() < 7. {
                        DragMode::Left
                    } else if (*x as f64 - end).abs() < 7. {
                        DragMode::Right
                    } else {
                        DragMode::Move
                    };
                    let before = Self::snapshot(&m);
                    m.drag = Some(Drag {
                        track: index,
                        clip: ci,
                        x: *x as f32,
                        original: c,
                        before,
                        mode,
                        moved: false,
                    });
                }
                drop(m);
                // Selecting/dragging a region preserves transport. A click in
                // empty space (or the scissors tool) explicitly seeks instead.
                if !was_playing || hit.is_none() || self.tool.get() == 1 {
                    self.seek(self.snap_position(at));
                }
                if self.tool.get() == 1 && hit.is_some() {
                    self.model.borrow_mut().drag = None;
                    self.split();
                } else {
                    self.refresh(true);
                }
                true
            }
            Event::Mouse(MouseEvent::Moved { x, .. }) => {
                let mut m = self.model.borrow_mut();
                let Some(mut drag) = m.drag.take() else {
                    return false;
                };
                if (*x as f32 - drag.x).abs() < 3. && !drag.moved {
                    m.drag = Some(drag);
                    return true;
                }
                drag.moved = true;
                let rate = m.project.sample_rate;
                let delta = (*x as f32 - drag.x) as f64 / width as f64 * self.view_span.get();
                let grid = self.snap_grid_for(&m.project);
                let quantize = |s: f64| {
                    if self.snap.get() {
                        grid.position(s)
                    } else {
                        (s * rate as f64).round() / rate as f64
                    }
                };
                let c = &mut m.project.tracks[drag.track].clips[drag.clip];
                *c = drag.original.clone();
                match drag.mode {
                    DragMode::Move => {
                        c.start = (quantize(c.start as f64 / rate as f64 + delta).max(0.)
                            * rate as f64)
                            .round() as u64
                    }
                    DragMode::Left => {
                        let wanted = (quantize(c.start as f64 / rate as f64 + delta) * rate as f64)
                            .round() as i64;
                        let diff = (wanted - c.start as i64).clamp(
                            -(c.source_offset as i64).min(c.start as i64),
                            c.frames.saturating_sub(1) as i64,
                        );
                        c.start = (c.start as i64 + diff) as u64;
                        c.source_offset = (c.source_offset as i64 + diff) as usize;
                        c.frames = (c.frames as i64 - diff) as usize;
                    }
                    DragMode::Right => {
                        let wanted =
                            (quantize((c.start + c.frames as u64) as f64 / rate as f64 + delta)
                                * rate as f64)
                                .round() as i64;
                        let length = (wanted - c.start as i64)
                            .clamp(1, (c.samples.len() - c.source_offset) as i64);
                        c.frames = length as usize;
                    }
                }
                self.status.set(format!(
                    "{} region · {:.3}s → {:.3}s",
                    match drag.mode {
                        DragMode::Move => "Move",
                        _ => "Trim",
                    },
                    c.start as f64 / rate as f64,
                    (c.start + c.frames as u64) as f64 / rate as f64
                ));
                m.drag = Some(drag);
                drop(m);
                self.refresh(true);
                true
            }
            Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                ..
            }) => {
                let mut m = self.model.borrow_mut();
                if let Some(d) = m.drag.take() {
                    if d.moved {
                        match Self::sync_audio(&mut m) {
                            Ok(()) => Self::history(
                                &mut m,
                                d.before,
                                if d.mode == DragMode::Move {
                                    "Move region"
                                } else {
                                    "Trim region"
                                },
                            ),
                            Err(error) => {
                                Self::restore(&mut m, d.before);
                                self.status.set(format!("Could not edit region: {error}"));
                            }
                        }
                    }
                }
                drop(m);
                self.refresh(true);
                true
            }
            Event::Mouse(MouseEvent::ButtonCancelled { .. }) => {
                self.cancel_drag();
                true
            }
            Event::Mouse(MouseEvent::Wheel { delta_x, .. }) if *delta_x != 0 => {
                self.suspend_follow();
                self.view_start.set(
                    (self.view_start.get()
                        - *delta_x as f64 * 0.25 / width as f64 * self.view_span.get())
                    .max(0.),
                );
                self.refresh(true);
                true
            }
            _ => false,
        }
    }
    fn cancel_drag(&self) {
        let mut m = self.model.borrow_mut();
        if let Some(d) = m.drag.take() {
            Self::restore(&mut m, d.before);
        }
        drop(m);
        self.refresh(true);
    }
    fn busy(&self) -> bool {
        let m = self.model.borrow();
        m.io.is_some() || m.picker.is_some()
    }
    fn open_dialog(&self, action: FileAction) {
        if self.busy() {
            return;
        }

        self.finish_mix();
        self.dialog_error.set(String::new());
        self.file_selected.set(None);
        let current = self.model.borrow().current_path.clone();
        let (folder, name) = if matches!(action, FileAction::Save) {
            current
                .as_ref()
                .map(|p| {
                    (
                        p.parent().unwrap_or(Path::new(".")).to_path_buf(),
                        p.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                    )
                })
                .unwrap_or_else(|| {
                    (
                        std::env::current_dir().unwrap_or_default(),
                        "session.resonara.json".into(),
                    )
                })
        } else {
            (
                std::env::current_dir().unwrap_or_default(),
                if action == FileAction::Export {
                    "mix.wav".into()
                } else {
                    String::new()
                },
            )
        };
        self.path.set(folder.to_string_lossy().into());
        self.filename.set(name.clone());
        if let Some(owner) = self.window_id.get() {
            let mut options =
                FileDialog::new(if matches!(action, FileAction::Save | FileAction::Export) {
                    FileDialogMode::Save
                } else {
                    FileDialogMode::Open
                });
            options.title = match action {
                FileAction::Open => "Open Resonara project",
                FileAction::Save => "Save Resonara project",
                FileAction::Import => "Import audio",
                FileAction::Export => "Export stereo WAV",
            }
            .into();
            options.initial_directory = folder.is_absolute().then(|| folder.into());
            options.default_name = (!name.is_empty()).then_some(name);
            options.filters = vec![FileDialogFilter {
                name: if action == FileAction::Import {
                    "Audio files"
                } else if action == FileAction::Export {
                    "WAV audio"
                } else {
                    "Resonara project"
                }
                .into(),
                extensions: match action {
                    FileAction::Import => resonara_core::audio::EXTENSIONS
                        .iter()
                        .map(|e| (*e).into())
                        .collect(),
                    FileAction::Export => vec!["wav".into()],
                    _ => vec!["json".into()],
                },
            }];
            self.model.borrow_mut().picker = Some((action, options.show(owner)));
            self.dialog.set(Dialog::Native(action));
            self.track_menu.set(None);
            return;
        }
        self.open_fallback(action);
    }
    fn open_fallback(&self, action: FileAction) {
        self.dialog.set(Dialog::File(action));
        self.read_directory();
    }
    fn cancel_picker(&self) {
        if let Some((_, handle)) = &self.model.borrow().picker {
            handle.cancel();
        }
    }
    fn poll_picker(&self) {
        let outcome = {
            let m = self.model.borrow();
            m.picker
                .as_ref()
                .and_then(|(action, handle)| handle.take_result().map(|result| (*action, result)))
        };
        if let Some((action, result)) = outcome {
            self.model.borrow_mut().picker = None;
            self.apply_picker_result(action, result);
        }
    }
    fn apply_picker_result(&self, action: FileAction, result: FileDialogResult) {
        self.dialog.set(Dialog::None);
        self.focus.set(true);
        match result {
            Ok(FileDialogOutcome::Selected(paths)) if paths.len() == 1 => {
                let path: PathBuf = paths.into_iter().next().unwrap().into();
                let (supported, kind) = match action {
                    FileAction::Import => (
                        resonara_core::audio::supported_path(&path),
                        "supported audio",
                    ),
                    FileAction::Export => (
                        path.extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("wav")),
                        ".wav",
                    ),
                    _ => (
                        path.extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("json")),
                        ".json",
                    ),
                };
                if !path.is_absolute() || path.is_dir() || !supported {
                    self.picker_failed(format!("Choose an absolute {kind} file path"));
                } else {
                    self.start_io(action, path);
                }
            }
            Ok(FileDialogOutcome::Cancelled) => {
                self.close_after_save.set(false);
                self.open_after_save.set(false);
                self.status.set("File selection cancelled".into());
            }
            Err(FileDialogError::Unsupported(_)) => self.open_fallback(action),
            Err(error) => self.picker_failed(format!("File selection failed: {error}")),
            _ => self.picker_failed("File picker returned an invalid selection".into()),
        }
    }
    fn picker_failed(&self, message: String) {
        self.close_after_save.set(false);
        self.open_after_save.set(false);
        self.dialog_error.set(message.clone());
        self.status.set(message);
    }
    fn read_directory(&self) {
        let folder = PathBuf::from(self.path.get());
        let action = match self.dialog.get() {
            Dialog::File(a) => a,
            _ => return,
        };
        match std::fs::read_dir(&folder) {
            Ok(entries) => {
                let mut files = entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| {
                        let directory = e.file_type().ok()?.is_dir();
                        let name = e.file_name().to_string_lossy().into_owned();
                        let allowed = directory
                            || match action {
                                FileAction::Import => {
                                    resonara_core::audio::supported_path(&e.path())
                                }
                                FileAction::Open => name.ends_with(".json"),
                                _ => true,
                            };
                        if !allowed || name.starts_with('.') {
                            None
                        } else {
                            Some(FileEntry {
                                name,
                                path: e.path(),
                                directory,
                            })
                        }
                    })
                    .collect::<Vec<_>>();
                files.sort_by(|a, b| {
                    b.directory
                        .cmp(&a.directory)
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
                self.files.set(files);
                self.file_selected.set(None);
                self.dialog_error.set(String::new());
            }
            Err(e) => self
                .dialog_error
                .set(format!("Cannot open this folder: {e}")),
        }
    }
    fn submit_file(&self, overwrite: bool) {
        let action = match self.dialog.get() {
            Dialog::File(a) => a,
            _ => return,
        };
        let name = self.filename.get();
        if name.trim().is_empty() {
            self.dialog_error
                .set("Choose a file or enter a filename".into());
            return;
        }
        let path = PathBuf::from(&name);
        let path = if path.is_absolute() {
            path
        } else {
            PathBuf::from(self.path.get()).join(path)
        };
        if path.is_dir() {
            self.path.set(path.to_string_lossy().into());
            self.filename.set(String::new());
            self.read_directory();
            return;
        }
        if matches!(action, FileAction::Save | FileAction::Export) && path.exists() && !overwrite {
            self.dialog_error.set("This file already exists. Click Replace file to overwrite it, or choose another name.".into());
            return;
        }
        self.start_io(action, path);
    }
    fn start_io(&self, action: FileAction, path: PathBuf) {
        self.poll_native_editors(true);
        self.native_edit_group.set(false);
        if self.busy() {
            return;
        }
        self.finish_mix();
        if action == FileAction::Open {
            self.close_native_editors();
            self.stop_audio(false);
            self.model.borrow_mut().retired_audio = None;
        }
        let mut m = self.model.borrow_mut();
        if m.io.is_some() || m.picker.is_some() {
            return;
        }
        let project = m.project.clone();
        let version = m.version;
        let (tx, rx) = mpsc::channel();
        m.io = Some(rx);
        self.status.set(
            match action {
                FileAction::Import => "Importing audio…",
                FileAction::Open => "Opening project…",
                FileAction::Save => "Saving project…",
                FileAction::Export => "Exporting stereo WAV…",
            }
            .into(),
        );
        self.dialog.set(Dialog::None);
        drop(m);
        self.changed();
        std::thread::spawn(move || {
            let r: Result<Option<Project>> = (|| match action {
                FileAction::Import => {
                    let mut p = project;
                    p.import_audio(&path)?;
                    Ok(Some(p))
                }
                FileAction::Open => Ok(Some(Project::load(&path)?)),
                FileAction::Save => {
                    project.save(&path)?;
                    Ok(None)
                }
                FileAction::Export => {
                    project.export_wav(&path)?;
                    Ok(None)
                }
            })();
            let _ = tx.send(IoResult {
                action,
                path,
                version,
                result: r.map_err(|e| e.to_string()),
            });
        });
    }
    fn poll_io(&self) {
        let outcome = {
            let m = self.model.borrow();
            m.io.as_ref().and_then(|rx| rx.try_recv().ok())
        };
        let Some(out) = outcome else {
            return;
        };
        let mut m = self.model.borrow_mut();
        m.io = None;
        let succeeded = out.result.is_ok();
        match out.result {
            Ok(project) => {
                if let Some(project) = project {
                    let before = Self::snapshot(&m);
                    m.project = project;
                    if let Err(error) = Self::sync_audio(&mut m) {
                        Self::restore(&mut m, before);
                        self.status
                            .set(format!("Could not apply audio import: {error}"));
                        drop(m);
                        self.refresh(true);
                        return;
                    }
                    m.selected_bus = None;
                    m.selected = if out.action == FileAction::Import {
                        m.project.tracks.len().saturating_sub(1)
                    } else {
                        0
                    };
                    m.clip = None;
                    Self::history(
                        &mut m,
                        before,
                        if out.action == FileAction::Import {
                            "Import audio"
                        } else {
                            "Open project"
                        },
                    );
                    if out.action == FileAction::Open {
                        if let Some(history) = m.undo.last_mut() {
                            history.restore_path = true;
                        }
                        m.current_path = Some(out.path.clone());
                        m.saved_version = m.version;
                    }
                }
                if out.action == FileAction::Save {
                    m.current_path = Some(out.path.clone());
                    m.saved_version = out.version;
                }
                self.status.set(format!(
                    "{} · {}",
                    match out.action {
                        FileAction::Import => "Audio imported",
                        FileAction::Open => "Project opened",
                        FileAction::Save => "Project saved",
                        FileAction::Export => "Stereo 32-bit float WAV exported",
                    },
                    out.path.display()
                ));
                if out.action == FileAction::Save && self.close_after_save.replace(false) {
                    scarlet_ui::dismiss_window("resonara");
                }
            }
            Err(e) => {
                self.close_after_save.set(false);
                self.open_after_save.set(false);
                self.status.set(format!("File operation failed: {e}"));
                self.dialog_error.set(e);
            }
        }
        let open_next = out.action == FileAction::Save
            && m.saved_version == out.version
            && self.open_after_save.replace(false);
        drop(m);
        if succeeded && matches!(out.action, FileAction::Import | FileAction::Open) {
            self.fit();
            if out.action == FileAction::Open {
                self.seek(0.);
            }
        } else {
            self.refresh(false);
        }
        if open_next {
            self.open_dialog(FileAction::Open);
        }
    }
    fn save(&self) {
        let path = self.model.borrow().current_path.clone();
        if let Some(path) = path {
            self.start_io(FileAction::Save, path);
        } else {
            self.open_dialog(FileAction::Save);
        }
    }
    fn request_open(&self) {
        if self.busy() {
            return;
        }
        self.finish_mix();
        if self.dirty() {
            self.dialog.set(Dialog::ConfirmOpen);
        } else {
            self.open_dialog(FileAction::Open);
        }
    }
    fn handle_key(&self, e: KeyEvent) -> bool {
        if self.model.borrow().picker.is_some() {
            if matches!(
                e,
                KeyEvent::Pressed {
                    keycode: KeyCode::Escape,
                    ..
                }
            ) {
                self.cancel_picker();
            }
            return true;
        }

        if self.busy() {
            return true;
        }
        let KeyEvent::Pressed { keycode, modifiers } = e else {
            return false;
        };
        if self.dialog.get() != Dialog::None {
            if self.handle_send_popup_key(keycode) {
                return true;
            }
            if self.handle_insert_popup_key(keycode) {
                return true;
            }
            if keycode == KeyCode::Escape {
                self.dialog.set(Dialog::None);
                self.close_after_save.set(false);
                self.open_after_save.set(false);
                return true;
            }
            return false;
        }
        if keycode == KeyCode::F(9) && self.profiler.borrow().enabled() {
            if self.profiler.borrow().active() {
                self.finish_profile();
            } else {
                self.profiler.borrow_mut().begin();
                self.status
                    .set("Recording native submission timings for 15 seconds…".into());
            }
            return true;
        }
        let primary = modifiers.control || modifiers.super_key;
        if primary {
            match keycode {
                KeyCode::Char('s' | 'S') => {
                    if modifiers.shift {
                        self.open_dialog(FileAction::Save)
                    } else {
                        self.save()
                    }
                }
                KeyCode::Char('o' | 'O') => self.request_open(),
                KeyCode::Char('i' | 'I') => self.open_dialog(FileAction::Import),
                KeyCode::Char('e' | 'E') => self.open_dialog(FileAction::Export),
                KeyCode::Char('z' | 'Z') => self.undo(modifiers.shift),
                KeyCode::Char('y' | 'Y') => self.undo(true),
                KeyCode::Char('d' | 'D') => self.duplicate(),
                _ => return false,
            };
            return true;
        }
        match keycode {
            KeyCode::Space => self.play(),
            KeyCode::Home => self.seek(0.),
            KeyCode::Escape => {
                self.routing_menu.set(None);
                if !self.cancel_ruler_drag() {
                    self.cancel_drag();
                }
            }
            KeyCode::Delete | KeyCode::Backspace => self.delete(false),
            KeyCode::Up => self.select(-1),
            KeyCode::Down => self.select(1),
            KeyCode::Char('s' | 'S') => self.split(),
            KeyCode::Char('c' | 'C') => self.toggle_metronome(),
            KeyCode::Char('m' | 'M') => {
                let selected_bus = self.model.borrow().selected_bus;
                if let Some(id) = selected_bus {
                    self.bus_mix(id, None, None, true);
                } else {
                    let i = self.model.borrow().selected;
                    self.mix(i, None, None, Some(false));
                }
            }
            KeyCode::Char('x' | 'X') => self.mixer_visible.set(!self.mixer_visible.get()),
            KeyCode::Char('i' | 'I') => self.inspector.set(!self.inspector.get()),
            KeyCode::Char('1') => self.tool.set(0),
            KeyCode::Char('2') => self.tool.set(1),
            KeyCode::Char('+' | '=') => self.zoom(0.5),
            KeyCode::Char('-') => self.zoom(2.),
            KeyCode::Char('f' | 'F') => self.fit(),
            KeyCode::Left => self.seek((self.playhead.get() - 0.1).max(0.)),
            KeyCode::Right => self.seek(self.playhead.get() + 0.1),
            _ => return false,
        };
        true
    }
    fn button(&self, text: &str, help: &'static str, action: impl Fn(Self) + 'static) -> AnyView {
        let s = self.clone();
        let h = self.clone();
        AnyView::new(
            ui::button(text)
                .on_click(move || action(s.clone()))
                .on_hover(move || h.status.set(help.into())),
        )
    }
    fn icon(
        &self,
        icon: Icon,
        help: &'static str,
        active: bool,
        action: impl Fn(Self) + 'static,
    ) -> AnyView {
        let s = self.clone();
        let h = self.clone();
        AnyView::new(
            Button::icon_only(icon)
                .icon_size(IconSize::Small)
                .padding(7.)
                .background_color(if active { ACCENT } else { RAISED })
                .icon_color(if active { BG } else { TEXT })
                .border_color(LINE)
                .on_click(move || action(s.clone()))
                .on_hover(move || h.status.set(help.into())),
        )
    }
    fn track_row(&self, index: usize) -> AnyView {
        let m = self.model.borrow();
        let Some(t) = m.project.tracks.get(index) else {
            return AnyView::new(label(""));
        };
        let c = &m.channels[index];
        let selected = index == m.selected;
        let width = (self.arrangement_size.get().width - HEADER).max(200.);
        let select = self.clone();
        let gain = self.clone();
        let mute = self.clone();
        let solo = self.clone();
        let event = self.clone();
        let mut overlays: Vec<Box<dyn View>> = vec![
            Box::new(SgfxCanvas::from_state(
                c.canvas,
                width,
                ROW,
                c.frame.clone(),
            )),
            Box::new(animation::Playhead::new(
                self.playhead.clone(),
                self.view_start.get(),
                self.view_span.get(),
                Size::new(width, ROW),
            )),
        ];
        for (ci, clip) in t.clips.iter().enumerate() {
            let Some(projection) = wave::project_clip(
                clip,
                width,
                self.view_start.get(),
                self.view_span.get(),
                m.project.sample_rate,
            ) else {
                continue;
            };
            let visible = (projection.right - projection.left - 12.).max(0.);
            if visible > 40. {
                overlays.push(Box::new(
                    label(t.name.clone())
                        .font_size(11.)
                        .color(if selected && m.clip == Some(ci) {
                            BG
                        } else {
                            TEXT
                        })
                        .frame(visible, 18.)
                        .clip()
                        .alignment(Alignment::TopLeading)
                        .padding_insets(EdgeInsets::new(projection.left + 6., 7., 0., 0.)),
                ));
            }
            if clip.source_channels == 2 && visible > 24. {
                for lane in wave::lanes(clip.source_channels, ROW) {
                    overlays.push(Box::new(
                        Text::new(if lane.channel == 0 { "L" } else { "R" })
                            .font_size(8.)
                            .color(TEXT)
                            .alignment(Alignment::Center)
                            .frame(8., 10.)
                            .padding_insets(EdgeInsets::new(
                                projection.left + 4.,
                                lane.top,
                                0.,
                                0.,
                            )),
                    ));
                }
            }
        }
        let source_kind = if t.clips.is_empty() {
            "AUDIO"
        } else if t.clips.iter().all(|c| c.source_channels == 1) {
            "MONO"
        } else if t.clips.iter().all(|c| c.source_channels == 2) {
            "STEREO"
        } else {
            "MIXED"
        };
        AnyView::new(row!{
            TrackArea(AnyView::new(row!{Rectangle::new().fill(ui::color(index)).frame(3.,ROW),vstack!{
                row!{caption(format!("{:02}",index+1)),ui::name_label(&t.name,22,12.,self.status.clone()).frame_width(135.)}.spacing(6.),
                row!{ui::button("M").background_color(if t.mute{GOLD}else{RAISED}).text_color(if t.mute{BG}else{TEXT}).on_click(move||mute.mix(index,None,None,Some(false))).frame(27.,24.),ui::button("S").background_color(if t.solo{GOLD}else{RAISED}).text_color(if t.solo{BG}else{TEXT}).on_click(move||solo.mix(index,None,None,Some(true))).frame(27.,24.),Slider::new(c.gain_normalized.clone()).min(0.).max(1.).dragging_state(c.dragging_gain.clone()).on_change(move|v|gain.mix_normalized(index,v)).frame_width(98.)}.spacing(5.),
                row!{caption(ui::db(t.gain)),Spacer::new(),caption(source_kind)}
            }.spacing(4.).padding(8.).frame(HEADER-3.,ROW)}.spacing(0.).background(if selected{RAISED}else{PANEL}).on_click(move||select.choose(index,None))),Some(index)),
            ZStack::new(Children(overlays)).alignment(Alignment::TopLeading).frame(width,ROW).clip().on_event(move|e|event.timeline_event(index,e))
        }.spacing(0.).frame(HEADER+width,ROW))
    }
    fn ruler(&self) -> AnyView {
        let width = (self.arrangement_size.get().width - HEADER).max(200.);
        let start = self.view_start.get();
        let span = self.view_span.get();
        let m = self.model.borrow();
        let format = self.time_format.get();
        let grid = timeline::ruler_grid(
            format,
            start,
            span,
            width,
            m.project.tempo,
            m.project.sample_rate,
            m.project.time_signature,
        );
        let step = grid.label_step;
        let first = (start / step).floor() as i64;
        let mut labels: Vec<Box<dyn View>> = vec![Box::new(ruler::Marks {
            ticks: grid.ticks,
            start,
            span,
            size: Size::new(width, 30.),
        })];
        for i in first..=first + 12 {
            let seconds = i as f64 * step;
            let x = ((seconds - start) / span) as f32 * width;
            if x >= 0. && x < width - 85. {
                labels.push(Box::new(
                    caption(format.tick(
                        seconds,
                        m.project.tempo,
                        m.project.sample_rate,
                        m.project.time_signature,
                    ))
                    .font_size(10.)
                    .frame(90., 16.)
                    .padding_insets(EdgeInsets::new(x + 5., 0., 0., 0.)),
                ));
            }
        }
        let s = self.clone();
        AnyView::new(row!{row!{caption("TRACKS"),Spacer::new(),caption(format!("{}",self.model.borrow().project.tracks.len())),self.button("+","Add an empty audio track",|s|{let after=(!s.model.borrow().project.tracks.is_empty()).then_some(s.model.borrow().selected);s.add_track(after);}).frame(24.,24.)}.spacing(5.).padding_insets(EdgeInsets::new(10.,3.,6.,3.)).frame(HEADER,30.).background(PANEL),ZStack::new(Children(labels)).alignment(Alignment::TopLeading).frame(width,30.).background(RAISED).on_event(move|e|s.ruler_event(e,start,span,width))}.spacing(0.))
    }
    fn arrangement(&self) -> AnyView {
        let size = self.arrangement_size.get();
        let count = self.model.borrow().project.tracks.len();
        let rows = self.clone();
        let resize = self.clone();
        let content = if count == 0 {
            AnyView::new(vstack!{Text::new("Your arrangement starts here").font_size(20.).color(TEXT),caption("Import an audio file, then arrange it on the timeline."),self.button("Import audio…","Import an audio file · Ctrl/Cmd+I",|s|s.open_dialog(FileAction::Import))}.spacing(12.).frame(size.width,(size.height-58.).max(150.)))
        } else {
            AnyView::new(
                ScrollView::new(LazyVStack::new(count, ROW, move |i| rows.track_row(i)))
                    .vertical()
                    .content_size(size.width, count as f32 * ROW)
                    .frame(size.width, (size.height - 60.).max(100.)),
            )
        };
        let layout=AnyView::new(vstack!{self.ruler(),TrackArea(content,None),row!{self.icon(Icon::ChevronLeft,"Scroll timeline left",false,|s|{s.scroll_timeline(-s.view_span.get()*0.5);}),caption(format!("{:.2}s — {:.2}s",self.view_start.get(),self.view_start.get()+self.view_span.get())),Spacer::new(),caption("Drag region • edge = trim • S = split"),self.icon(Icon::ChevronRight,"Scroll timeline right",false,|s|{s.scroll_timeline(s.view_span.get()*0.5);})}.spacing(6.).padding_insets(EdgeInsets::new(8.,0.,8.,0.)).frame_height(30.).background(PANEL)}.spacing(0.).background(BG).on_geometry_change(|g|g.size(),move|size|{let old=resize.arrangement_size.get();if(old.width-size.width).abs()>1.||(old.height-size.height).abs()>1.{resize.arrangement_size.set(size);if resize.mixer_visible.get(){let full=(resize.size.get().height-44.-66.-36.-28.-4.).max(1.);resize.mixer_fraction.set((size.height/full).clamp(0.1,0.9));}resize.refresh(true);}}));
        let wheel = self.clone();
        AnyView::new(HorizontalWheel(
            layout,
            Rc::new(move |dx| {
                let width = (wheel.arrangement_size.get().width - HEADER).max(200.);
                wheel.scroll_timeline(-dx as f64 * 0.25 / width as f64 * wheel.view_span.get());
            }),
        ))
    }
    fn inspector_panel(&self) -> AnyView {
        self.channel_inspector_panel()
    }
    fn mixer_fader_height(&self) -> f32 {
        let available = (self.size.get().height - 174.).max(320.) - 4.;
        let max_first = (available - MIXER_MIN_HEIGHT).max(0.);
        let min_first = (available - MIXER_MAX_HEIGHT).max(230.).min(max_first);
        let first = (available * self.mixer_fraction.get()).clamp(min_first, max_first);
        (available - first - MIXER_HEADER_HEIGHT - MIXER_STRIP_OVERHEAD)
            .clamp(112., MIXER_FADER_MAX_HEIGHT)
    }
    fn mixer(&self) -> AnyView {
        let fader_height = self.mixer_fader_height();
        let strip_height = MIXER_STRIP_OVERHEAD + fader_height;
        let m = self.model.borrow();
        let mut channels: Vec<Box<dyn View>> = vec![];
        for (i, t) in m.project.tracks.iter().enumerate() {
            let c = &m.channels[i];
            let select = self.clone();
            let status = self.status.clone();
            let full_name = t.name.clone();
            channels.push(Box::new(vstack!{
                Rectangle::new().fill(ui::color(i)).frame(90.,3.),
                ui::button(format!("{:02} {}",i+1,ui::elide(&t.name,11))).font_size(10.).on_click(move||{select.choose(i,None);select.inspector.set(true);}).frame(92.,24.).on_hover(move||status.set(full_name.clone())),
                self.channel_strip_controls(RoutingTarget::Track(i),c,t.gain,t.pan,t.mute,Some(t.solo),fader_height,false)
            }.spacing(2.).padding(4.).frame(100.,strip_height-2.).background(if i==m.selected&&m.selected_bus.is_none(){RAISED}else{PANEL}).border(LINE,1.)));
        }
        for (index, bus) in m.project.buses.iter().enumerate() {
            channels.push(Box::new(self.bus_strip(
                bus,
                &m.bus_channels[index],
                fader_height,
                strip_height,
                m.selected_bus == Some(bus.id),
            )));
        }
        let master = self.clone();
        let master_status = self.status.clone();
        let master_peak = self.master_peak.clone();
        channels.push(Box::new(vstack!{
            Rectangle::new().fill(TEXT).frame(90.,3.),label("STEREO OUT").font_size(11.).alignment(Alignment::Center).frame(92.,24.),caption("MASTER").alignment(Alignment::Center).frame(90.,24.),caption("Post gain · pre clip").font_size(9.).alignment(Alignment::Center).frame(90.,44.),
            fader::Fader::new(self.master.clone(),self.master_peak.clone(),self.master_dragging.clone(),self.master_focus.clone(),move|v|master.master_change(v)).frame(90.,fader_height),
            label(format!("Gain {}",ui::db(m.project.master))).font_size(10.).alignment(Alignment::Center).frame(90.,14.),animation::Readout::new(self.master_meter.clone(),9.,ACCENT,Size::new(90.,12.)).on_hover(move||master_status.set(master_peak.get().detail(true)))
        }.spacing(2.).padding(4.).frame(100.,strip_height - 2.).background(RAISED).border(LINE,1.)));
        let width = channels.len() as f32 * 100.;
        AnyView::new(vstack!{row!{caption("MIXER"),caption(format!("{} audio · {} aux",m.project.tracks.len(),m.project.buses.len())),self.header_button("+ Aux", "Create an aux channel and its input bus", |s|s.add_bus()),Spacer::new(),self.icon(Icon::X,"Hide mixer · X",false,|s|s.mixer_visible.set(false))}.spacing(12.).padding_insets(EdgeInsets::new(12.,0.,8.,0.)).frame_height(MIXER_HEADER_HEIGHT).background(PANEL),ScrollView::new(HStack::new(Children(channels)).spacing(0.).alignment(Alignment::TopLeading)).both_axes().content_size(width,strip_height).frame_height(strip_height)}.spacing(0.).background(BG))
    }
    fn toolbar(&self) -> AnyView {
        let m = self.model.borrow();
        let name = m
            .current_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled session".into());
        let dirty = m.version != m.saved_version;
        let busy = m.io.is_some() || m.picker.is_some();
        AnyView::new(row!{Text::new("resonara").font_size(19.).color(TEXT),caption("AUDIO WORKSTATION"),Spacer::new(),label(format!("{}{}",ui::elide(&name,29),if dirty{"  •"}else{""})),Spacer::new(),self.header_button("Open…","Open a Resonara project · Ctrl/Cmd+O",|s|s.request_open()),self.header_icon(Icon::DeviceFloppy,"Save project · Ctrl/Cmd+S",false,|s|s.save()),self.header_button("Import audio…","Import mono or stereo audio · Ctrl/Cmd+I",|s|s.open_dialog(FileAction::Import)),self.header_button("Export…","Export stereo WAV · Ctrl/Cmd+E",|s|s.open_dialog(FileAction::Export)),self.header_icon(Icon::HelpCircle,"Keyboard shortcuts and editing help",false,|s|s.dialog.set(Dialog::Help)),caption(if busy{"Working…"}else{""})}.spacing(10.).padding_insets(EdgeInsets::new(14.,6.,12.,6.)).frame_height(44.).background(PANEL))
    }
    fn transport(&self) -> AnyView {
        let m = self.model.borrow();
        let playing = m.audio.is_some();
        let duration = m.project.duration() as f64 / m.project.sample_rate as f64;
        let tempo = self.clone();
        let signature = self.clone();
        let format = self.time_format.get();
        AnyView::new(row!{
            ui::transport_group("TRANSPORT",row!{
                self.header_icon(Icon::ChevronLeft,"Return to start · Home",false,|s|s.seek(0.)),
                self.header_icon(if playing{Icon::PlayerPause}else{Icon::PlayerPlay},"Play / stop · Space",playing,|s|s.play()),
                self.header_button("Stop","Stop playback",|s|s.stop_audio(true)),
                self.header_icon(Icon::Music,"Metronome · C · accented bar starts",self.metronome.get(),|s|s.toggle_metronome())
            }.spacing(6.),150.),
            Surface::section(row!{
                ui::lcd_group(format.caption(),counter::Counter::new(self.clock.clone()),COUNTER_WIDTH),
                ui::lcd_group("BPM",ui::lcd_field(self.tempo_input.clone()).on_submit(move||tempo.submit_tempo()).blur_on_submit(true).input_guard(),56.),
                ui::lcd_group("METER",ui::lcd_field(self.signature_input.clone()).on_submit(move||signature.submit_signature()).blur_on_submit(true).input_guard(),52.)
            }.spacing(CONTROL_GAP).padding_insets(EdgeInsets::new(CONTROL_GAP,4.,CONTROL_GAP,4.))).fill(BG).border_color(LINE).corner_radius(8.),
            ui::transport_group("DISPLAY",self.header_button(format.name(),"Switch musical, elapsed-time and sample-position displays",|s|s.cycle_time_format()),88.),
            ui::transport_group("PROJECT LENGTH",label(format.duration(duration,m.project.tempo,m.project.sample_rate,m.project.time_signature)).font_size(14.),110.),
            Spacer::new(),ui::transport_group("OUTPUT",caption(format!("{} Hz / STEREO",m.project.sample_rate)).font_size(CONTROL_FONT),116.)
        }.spacing(CONTROL_GAP).padding_insets(EdgeInsets::new(14.,9.,14.,9.)).frame_height(66.).background(RAISED))
    }
    fn editbar(&self) -> AnyView {
        let snap = if self.snap.get() {
            format!(
                "Snap: {}",
                self.snap_grid_for(&self.model.borrow().project).caption
            )
        } else {
            "Snap: off".into()
        };
        AnyView::new(row!{
            self.header_icon(Icon::ArrowBackUp,"Undo · Ctrl/Cmd+Z",false,|s|s.undo(false)),self.header_icon(Icon::ArrowForwardUp,"Redo · Ctrl/Cmd+Shift+Z",false,|s|s.undo(true)),Rectangle::new().fill(LINE).frame(1.,20.),
            self.header_button(if self.tool.get()==0{"• Pointer  1"}else{"Pointer  1"},"Pointer: select, move, trim edges · 1",|s|s.tool.set(0)),self.header_button(if self.tool.get()==1{"• Split  2"}else{"Split  2"},"Scissors: click a region to split · 2",|s|s.tool.set(1)),
            self.header_button(&snap,"Toggle snapping to the displayed musical/time/sample grid",|s|s.snap.set(!s.snap.get())),Spacer::new(),
            self.header_button(if self.follow_playhead.get(){"Follow"}else{"Fixed"},"Toggle playhead-follow scrolling",|s|s.toggle_follow()),self.header_icon(Icon::ZoomOut,"Zoom out · −",false,|s|s.zoom(2.)),self.header_icon(Icon::ZoomIn,"Zoom in · +",false,|s|s.zoom(0.5)),self.header_button("Fit","Fit project to timeline · F",|s|s.fit()),Rectangle::new().fill(LINE).frame(1.,20.),
            self.header_icon(Icon::List,"Toggle inspector · I",self.inspector.get(),|s|s.inspector.set(!s.inspector.get())),self.header_icon(Icon::Adjustments,"Toggle mixer · X",self.mixer_visible.get(),|s|s.mixer_visible.set(!s.mixer_visible.get()))
        }.spacing(6.).padding_insets(EdgeInsets::new(10.,3.,10.,3.)).frame_height(36.).background(PANEL))
    }
    fn workspace(&self) -> AnyView {
        let size = self.size.get();
        let height = (size.height - 44. - 66. - 36. - 28.).max(320.);
        // The inspector owns the full work-area height. Only the right pane
        // splits vertically, so mixer channels never extend under the inspector.
        let right = if self.mixer_visible.get() {
            AnyView::new(
                SplitView::new(self.arrangement(), self.mixer())
                    .axis(SplitAxis::Vertical)
                    .fraction(self.mixer_fraction.get())
                    .min_first((height - 4. - MIXER_MAX_HEIGHT).max(230.))
                    .min_second(MIXER_MIN_HEIGHT)
                    .divider_thickness(4.)
                    .divider_colors(LINE, ACCENT)
                    .frame_height(height),
            )
        } else {
            AnyView::new(self.arrangement().frame_height(height))
        };
        let content = if self.inspector.get() {
            AnyView::new(
                SplitView::new(self.inspector_panel(), right)
                    .axis(SplitAxis::Horizontal)
                    .fraction(self.inspector_fraction.get())
                    .min_first(224.)
                    .min_second(560.)
                    .divider_thickness(4.)
                    .divider_colors(LINE, ACCENT)
                    .frame(size.width, height),
            )
        } else {
            AnyView::new(right.frame(size.width, height))
        };
        // Both split and mixer-hidden modes own the workspace height; child
        // geometry supplies the waveform viewport after either transition.
        AnyView::new(vstack!{self.toolbar(),self.transport(),self.editbar(),content,Text::from_state(self.status.clone()).font_size(11.).color(MUTED).padding_insets(EdgeInsets::new(12.,5.,12.,5.)).frame(size.width,28.).alignment(Alignment::TopLeading).background(PANEL)}.spacing(0.).frame(size.width,size.height).background(BG))
    }
    fn dialog_view(&self) -> AnyView {
        let kind = self.dialog.get();
        let size = self.size.get();
        let mut rows: Vec<Box<dyn View>> = vec![];
        match kind {
            Dialog::Native(_) => {
                rows.push(Box::new(
                    Text::new("Choose a file in the system dialog")
                        .font_size(22.)
                        .color(TEXT),
                ));
                rows.push(Box::new(self.button(
                    "Cancel",
                    "Cancel file selection",
                    |s| s.cancel_picker(),
                )));
            }
            Dialog::File(action) => {
                let (title, submit, description) = match action {
                    FileAction::Import => (
                        "Import audio",
                        "Import audio",
                        "WAV / MP3 / FLAC / AIFF / OGG / M4A / AAC / CAF · mono/stereo",
                    ),
                    FileAction::Open => (
                        "Open project",
                        "Open project",
                        "Resonara project files (.json)",
                    ),
                    FileAction::Save => (
                        "Save project",
                        "Save project",
                        "Embeds source audio · large sessions can create large files",
                    ),
                    FileAction::Export => (
                        "Export mixdown",
                        "Export WAV",
                        "Stereo 32-bit float WAV · uses project sample rate, mute, solo and master",
                    ),
                };
                rows.push(Box::new(Text::new(title).font_size(22.).color(TEXT)));
                rows.push(Box::new(caption(description)));
                let folder = self.clone();
                rows.push(Box::new(row!{self.icon(Icon::ArrowUp,"Open parent folder",false,|s|{let path=PathBuf::from(s.path.get());if let Some(parent)=path.parent(){s.path.set(parent.to_string_lossy().into());s.read_directory();}}),ui::field(self.path.clone()).on_submit(move||folder.read_directory()).frame_width(545.).input_guard(),self.icon(Icon::Refresh,"Reload folder",false,|s|s.read_directory())}.spacing(8.)));
                let s = self.clone();
                rows.push(Box::new(
                    ListView::new(
                        self.files.clone(),
                        self.file_selected.clone(),
                        30.,
                        move |index, entry, selected| {
                            let s = s.clone();
                            ui::button(format!(
                                "{}  {}",
                                if entry.directory { "▸" } else { "·" },
                                entry.name
                            ))
                            .background_color(if selected == Some(index) { RAISED } else { BG })
                            .on_click(move || {
                                if entry.directory {
                                    s.path.set(entry.path.to_string_lossy().into());
                                    s.filename.set(String::new());
                                    s.read_directory();
                                } else {
                                    s.file_selected.set(Some(index));
                                    s.filename.set(entry.name.clone());
                                }
                            })
                            .frame(625., 30.)
                        },
                    )
                    .frame(640., 250.)
                    .background(BG)
                    .border(LINE, 1.),
                ));
                let submitter = self.clone();
                let cancel = self.clone();
                rows.push(Box::new(row!{caption("FILENAME"),ui::field(self.filename.clone()).placeholder("Name or absolute path").on_submit(move||submitter.submit_file(false)).on_cancel(move||{cancel.dialog.set(Dialog::None);cancel.close_after_save.set(false);cancel.open_after_save.set(false);}).autofocus(true).frame_width(555.).input_guard()}.spacing(10.)));
                rows.push(Box::new(
                    Text::from_state(self.dialog_error.clone())
                        .font_size(12.)
                        .color(GOLD)
                        .frame(640., 38.),
                ));
                let replace = self
                    .dialog_error
                    .get()
                    .starts_with("This file already exists");
                rows.push(Box::new(row!{Spacer::new(),self.button("Cancel","Close without changing the project · Escape",|s|{s.dialog.set(Dialog::None);s.close_after_save.set(false);s.open_after_save.set(false);}),self.button(if replace{"Replace file"}else{submit},"Confirm file operation",move|s|s.submit_file(replace))}.spacing(10.)));
            }
            Dialog::Help => {
                rows.push(Box::new(
                    Text::new("Make yourself at home")
                        .font_size(24.)
                        .color(TEXT),
                ));
                for text in [
                    "Space   Play / stop       Home   Return to start",
                    "Click ruler   Set playhead       ← / →   Nudge 100 ms",
                    "1   Pointer       2   Scissors       S   Split at playhead",
                    "Drag a region to move it. Drag either edge to trim it.",
                    "Snap follows the display: musical notes/bars, 100 ms, or one sample. C toggles the metronome.",
                    "+ / −   Zoom       F   Fit project       I   Inspector       X   Mixer",
                    "↑ / ↓   Select track       M   Mute       Delete   Delete region",
                    "Ctrl/Cmd + Z   Undo       Shift + Ctrl/Cmd + Z   Redo",
                    "Ctrl/Cmd + D   Duplicate track       Ctrl/Cmd + I   Import audio",
                    "Ctrl/Cmd + O   Open       Ctrl/Cmd + S   Save       Ctrl/Cmd + E   Export",
                    "Rename a track in the inspector and press Enter.",
                    "Track edits and mixer changes are undoable. Original audio stays intact.",
                ] {
                    rows.push(Box::new(label(text)));
                }
                rows.push(Box::new(caption("Audio arrangement and mixing. Built-in inserts and native stereo CLAP effects are supported. Recording and MIDI are not implemented.")));
                rows.push(Box::new(self.button(
                    "Back to session",
                    "Close keyboard reference · Escape",
                    |s| s.dialog.set(Dialog::None),
                )));
            }
            Dialog::ConfirmOpen | Dialog::ConfirmClose => {
                let close = kind == Dialog::ConfirmClose;
                rows.push(Box::new(
                    Text::new("Save your changes?").font_size(24.).color(TEXT),
                ));
                rows.push(Box::new(label(
                    "This session has unsaved edits. Save them before continuing.",
                )));
                rows.push(Box::new(row!{self.button("Cancel","Keep working",|s|s.dialog.set(Dialog::None)),self.button("Discard changes","Continue without saving this session",move|s|{if close{scarlet_ui::dismiss_window("resonara");}else{s.open_dialog(FileAction::Open);}}),self.button("Save project…","Choose where to save this session",move|s|{s.close_after_save.set(close);s.open_after_save.set(!close);s.save();})}.spacing(10.)));
            }
            Dialog::SendPicker(target, slot) => return self.send_picker_dialog(target, slot),
            Dialog::SendActions(target, slot) => return self.send_actions_dialog(target, slot),
            Dialog::SendLevel(target, slot) => return self.send_level_dialog(target, slot),
            Dialog::InsertPicker(target) => return self.insert_picker_dialog(target),
            Dialog::InsertActions(target, slot) => return self.insert_actions_dialog(target, slot),
            Dialog::ClapEditor(target, slot) => return self.clap_editor_dialog(target, slot),
            Dialog::ClapPicker(target) => return self.clap_picker_dialog(target),
            Dialog::InsertValue(target, slot) => {
                return self.insert_value_dialog(target, slot);
            }
            Dialog::None => {}
        }
        AnyView::new(
            Surface::overlay(
                VStack::new(Children(rows))
                    .spacing(12.)
                    .alignment(Alignment::TopLeading)
                    .padding(24.),
            )
            .fill(PANEL)
            .border_color(LINE)
            .frame_width(690.)
            .alignment(Alignment::Center)
            .frame(size.width, size.height)
            .background(BG),
        )
    }
    fn sync_content_size(&self, window: Size) {
        let size = Size::new(window.width, (window.height - 32.).max(600.));
        if self.size.get() != size {
            self.size.set(size);
        }
    }
    fn finish_profile(&self) {
        let size = self.size.get();
        let result = self.profiler.borrow_mut().finish(
            "paint-only-playhead-30hz-meters-30hz",
            self.model.borrow().project.tracks.len(),
            size.width,
            size.height,
        );
        if let Some(result) = result {
            self.status.set(match result {
                Ok(path) => format!("Native timing saved: {}", path.display()),
                Err(e) => format!("Timing write failed: {e}"),
            });
        }
    }
    fn body(&self) -> AnyView {
        if self.profiler.borrow().active() {
            self.profiler.borrow_mut().body_builds += 1;
        }
        let key = self.clone();
        let content = if self.model.borrow().io.is_some() {
            AnyView::new(vstack!{Text::new("Working on your audio…").font_size(24.).color(TEXT),Text::from_state(self.status.clone()).font_size(14.).color(MUTED),caption("The session will be available as soon as the file operation finishes.")}.spacing(12.).frame(self.size.get().width,self.size.get().height).background(BG))
        } else if self.dialog.get() == Dialog::None {
            self.workspace()
        } else if matches!(
            self.dialog.get(),
            Dialog::InsertValue(..)
                | Dialog::InsertPicker(..)
                | Dialog::InsertActions(..)
                | Dialog::ClapEditor(..)
                | Dialog::ClapPicker(..)
                | Dialog::SendPicker(..)
                | Dialog::SendActions(..)
                | Dialog::SendLevel(..)
        ) {
            AnyView::new(
                ZStack::new(Children(vec![
                    Box::new(self.workspace()),
                    Box::new(self.dialog_view()),
                ]))
                .frame(self.size.get().width, self.size.get().height),
            )
        } else {
            self.dialog_view()
        };
        let inner = if self.dialog.get() == Dialog::None {
            AnyView::new(
                content
                    .on_key(move |e| key.handle_key(e))
                    .focusable(self.focus.clone()),
            )
        } else {
            AnyView::new(content.on_key(move |e| key.handle_key(e)))
        };
        let content = if let Some(menu) = self.track_menu.get() {
            AnyView::new(
                ZStack::new(Children(vec![
                    Box::new(inner),
                    Box::new(
                        Rectangle::new()
                            .fill(Color::rgba(0., 0., 0., 0.))
                            .frame(self.size.get().width, self.size.get().height),
                    ),
                    Box::new(self.track_menu_view(menu)),
                ]))
                .alignment(Alignment::TopLeading)
                .frame(self.size.get().width, self.size.get().height),
            )
        } else {
            inner
        };
        let context = self.clone();
        AnyView::new(InputBoundary(
            AnyView::new(ShortcutBoundary(content)),
            Rc::new(move |root, e| {
                if matches!(
                    context.dialog.get(),
                    Dialog::InsertPicker(..)
                        | Dialog::ClapPicker(..)
                        | Dialog::InsertActions(..)
                        | Dialog::SendPicker(..)
                        | Dialog::SendActions(..)
                ) {
                    if let Event::Keyboard(KeyEvent::Pressed { keycode, .. }) = e {
                        return context.handle_insert_popup_key(*keycode)
                            || context.handle_send_popup_key(*keycode);
                    }
                    if matches!(e, Event::Keyboard(_)) {
                        return true;
                    }
                }
                context.track_context_event(
                    root,
                    e,
                    matches!(
                        e,
                        Event::Mouse(MouseEvent::ButtonPressed {
                            button: MouseButton::Left,
                            ..
                        })
                    ) && ui::mac_control_down(),
                )
            }),
        ))
    }
}
impl View for Daw {
    fn create_element(&self) -> Box<dyn scarlet_ui::Element> {
        Box::new(scarlet_ui::ComponentElement::new_with_builder(
            self.clone(),
            |s| Box::new(s.body()),
        ))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![
            &self.revision,
            &self.size,
            &self.arrangement_size,
            &self.dialog,
            &self.inspector,
            &self.inspector_details,
            &self.inspector_fraction,
            &self.mixer_fraction,
            &self.mixer_visible,
            &self.snap,
            &self.tool,
            &self.view_start,
            &self.view_span,
            &self.dialog_error,
            &self.time_format,
            &self.track_menu,
            &self.routing_menu,
            &self.menu_choice,
            &self.follow_playhead,
            &self.signature_input,
        ]
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl Application for Daw {
    fn scenes(&self) -> impl Scene {
        WindowGroup::new(
            "resonara",
            Window::new("Resonara", self.clone())
                .size(Size::new(1280., 860.))
                .min_size(Size::new(1000., 720.))
                .background_color(BG),
        )
    }
    // The window descriptor is fixed. Content dependencies are subscribed by
    // the inner Daw component; subscribing here would rebuild it twice.
    fn scene_listenables(&self, _: &scarlet_ui::SceneWindowKey) -> Option<Vec<&dyn Listenable>> {
        Some(vec![])
    }
    fn on_window_created(
        &mut self,
        context: &WindowContext,
        _: &mut dyn scarlet_ui::PlatformWindow,
    ) {
        self.window_id.set(Some(context.window_id));
    }
    fn on_window_resize(&mut self, _: &WindowContext, _: u32, _: u32) {
        // Native resize events may be queued echoes. Publish the final actual
        // window size once in on_window_sync, after the event batch is drained.
    }
    fn on_window_close_requested(&mut self, _: &WindowContext) -> bool {
        if self.busy() {
            return false;
        }
        self.close_native_editors();
        self.finish_mix();
        self.track_menu.set(None);
        if self.dirty() {
            self.dialog.set(Dialog::ConfirmClose);
            false
        } else {
            true
        }
    }
    fn on_window_sync(&mut self, _: &WindowContext, window: &mut dyn scarlet_ui::PlatformWindow) {
        self.sync_content_size(window.size());
        self.profiler.borrow_mut().sync();
    }
    fn on_frame_presented(&mut self, _: &WindowContext) {
        self.profiler.borrow_mut().presented();
    }
    fn on_render_error(&mut self, _: &WindowContext, failure: &scarlet_ui::RenderFailure) {
        self.profiler.borrow_mut().failure();
        if failure.kind != scarlet_ui::RenderFailureKind::Busy {
            eprintln!("[ScarletUI] {failure}");
        }
    }
    fn on_idle(&mut self) {
        self.poll_native_editors(false);
        {
            let mut m = self.model.borrow_mut();
            let m = &mut *m;
            if let Some(audio) = m.audio.as_mut().or(m.retired_audio.as_mut()) {
                audio.collect_retired();
            }
        }
        if self.profiler.borrow().expired() {
            self.finish_profile();
        }
        self.poll_picker();
        self.poll_io();
        let dragging = {
            let m = self.model.borrow();
            self.master_dragging.get()
                || m.channels
                    .iter()
                    .chain(&m.bus_channels)
                    .any(|c| c.dragging_gain.get() || c.dragging_pan.get())
                || self
                    .send_controls
                    .borrow()
                    .values()
                    .any(|c| c.dragging.get())
        };
        if !dragging && self.model.borrow().mixer_before.is_some() {
            self.finish_mix();
        }
        let now = Instant::now();
        let meter_elapsed = now.duration_since(self.last_meter.get());
        let meter_due = meter_elapsed >= animation::METER_INTERVAL;
        let playhead_elapsed = now.duration_since(self.last_playhead.get());
        let playhead_due = playhead_elapsed >= animation::PLAYHEAD_INTERVAL;
        if !meter_due && !playhead_due {
            return;
        }
        if meter_due {
            self.last_meter.set(now);
        }
        if playhead_due {
            self.last_playhead.set(now);
        }
        let mut ended = false;
        let mut failure = false;
        let mut next_position = None;
        {
            let m = self.model.borrow();
            if let Some(a) = &m.audio {
                if playhead_due {
                    let pos = a.controls.position.load(Ordering::Relaxed) as f64
                        / m.project.sample_rate as f64;
                    next_position = Some(pos);
                }
                if meter_due {
                    self.update_meters(Some(&a.controls), meter_elapsed.as_secs_f32());
                }
                ended = a.is_finished();
                failure = a.controls.error.load(Ordering::Relaxed);
            } else if meter_due {
                self.update_meters(None, meter_elapsed.as_secs_f32());
            }
        }
        if meter_due {
            self.release_meters(meter_elapsed.as_secs_f32());
        }
        if let Some(pos) = next_position.filter(|_| self.ruler_drag.borrow().is_none()) {
            self.animate_playhead(pos);
        }
        if (ended && self.ruler_drag.borrow().is_none()) || failure {
            self.finish_audio(false, !failure);
            self.status.set(
                if failure {
                    "Audio processing or device error. Check inserts and the output device, then press Play to retry."
                } else {
                    "Playback complete"
                }
                .into(),
            );
        }
    }
}
fn main() -> Result<()> {
    profiling::install_adapter_log();
    if std::env::var_os("RESONARA_INPUT_DEBUG").is_some() {
        scarlet_ui::debug::set_enabled(true);
    }
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--smoke") {
        return smoke();
    }
    let path = args
        .iter()
        .position(|a| a == "--project")
        .map(|i| args.get(i + 1).ok_or("Missing project path"))
        .transpose()?;
    let project = if std::env::var_os("RESONARA_UI_PROFILE").is_some()
        && std::env::var_os("RESONARA_PROFILE_WAV").is_some()
    {
        let mut p = Project::default();
        p.import_audio(Path::new(
            &std::env::var_os("RESONARA_PROFILE_WAV").unwrap(),
        ))?;
        p.tracks[0].gain = 10f32.powf(-18. / 20.);
        for i in 1..8 {
            let mut track = p.tracks[0].clone();
            track.name = format!("Profile track {}", i + 1);
            p.tracks.push(track);
        }
        p
    } else if let Some(p) = path {
        Project::load(Path::new(p))?
    } else {
        Project::default()
    };
    let mut app = Daw::new(project);
    if let Some(p) = path {
        app.model.borrow_mut().current_path = Some(PathBuf::from(p));
    }
    app.run().map_err(|e| format!("UI: {e:?}").into())
}
fn smoke() -> Result<()> {
    let path = std::env::var("RESONARA_ARTIFACTS").unwrap_or_else(|_| "artifacts".into());
    let dir = std::path::Path::new(&path);
    std::fs::create_dir_all(dir)?;
    let mut p = Project::demo();
    let live_edits = std::env::var_os("RESONARA_SMOKE_LIVE_EDITS").is_some();
    if live_edits {
        p.tracks[0].routing.inserts.push(resonara_core::Insert {
            kind: resonara_core::InsertKind::Gain { gain: 0.5 },
            bypass: false,
        });
    }
    p.split(0, 48000)?;
    p.trim(1, 24000, 96000)?;
    if std::env::var_os("RESONARA_SMOKE_ROUTING").is_some() {
        use resonara_core::{Destination, Insert, InsertKind, Send};
        let aux = p.add_bus("Smoke delay", BusKind::Aux);
        let group = p.add_bus("Smoke group", BusKind::Group);
        p.tracks[0].routing.inserts.push(Insert {
            kind: InsertKind::OnePole { coefficient: 0.5 },
            bypass: false,
        });
        p.tracks[0].routing.sends.push(Send {
            target: aux,
            gain: 0.25,
            pre_fader: true,
            enabled: true,
        });
        p.tracks[0].routing.output = Destination::Bus(group);
        p.bus_mut(aux).unwrap().routing.inserts.push(Insert {
            kind: InsertKind::Delay { frames: 4800 },
            bypass: false,
        });
        p.bus_mut(aux).unwrap().routing.output = Destination::Bus(group);
        p.bus_mut(group).unwrap().routing.inserts.push(Insert {
            kind: InsertKind::Gain { gain: 0.8 },
            bypass: false,
        });
    }
    let clap_id = std::env::var("RESONARA_SMOKE_CLAP_ID").ok();
    if std::env::var_os("RESONARA_SMOKE_CLAP").is_some() || clap_id.is_some() {
        let plugin = if let Some(id) = clap_id {
            let catalog = resonara_core::plugins::scan_installed();
            let choice = catalog
                .effects
                .iter()
                .find(|c| c.plugin_id == id)
                .ok_or_else(|| format!("Smoke CLAP ID not installed: {id}"))?;
            resonara_core::plugins::load_installed(choice)?
        } else {
            let plugin = resonara_core::plugins::load_bundled_gain()?;
            resonara_core::plugins::set_parameter(&plugin, 0, 0.5)?
        };
        println!("Smoke CLAP: {} ({})", plugin.name, plugin.plugin_id);
        p.tracks[0].routing.inserts.push(resonara_core::Insert {
            kind: resonara_core::InsertKind::Clap { plugin },
            bypass: false,
        });
    }
    p.save(&dir.join("smoke.resonara.json"))?;
    let mut p = Project::load(&dir.join("smoke.resonara.json"))?;
    p.export_wav(&dir.join("smoke.wav"))?;
    let mut a = Audio::start(&p, 0)?;
    let transport = a.controls.position.clone();
    let mut edits = 0;
    let start = Instant::now();
    while start.elapsed().as_secs_f32() < 2. {
        a.collect_retired();
        if live_edits && edits < 7 && start.elapsed().as_millis() >= (edits + 1) * 100 {
            match edits {
                0 => p.tracks[0].name = "Live rename".into(),
                1 | 2 => p.tracks[0].routing.inserts[0].bypass = edits == 1,
                3 | 4 => {
                    for insert in &mut p.tracks[0].routing.inserts {
                        if matches!(insert.kind, resonara_core::InsertKind::Clap { .. }) {
                            insert.bypass = edits == 3;
                        }
                    }
                }
                5 => {
                    let bus = p.add_bus("Live routing", BusKind::Aux);
                    p.tracks[0].routing.output = resonara_core::Destination::Bus(bus);
                }
                _ => p.tracks[0].routing.inserts.push(resonara_core::Insert {
                    kind: resonara_core::InsertKind::Gain { gain: 0.8 },
                    bypass: false,
                }),
            }
            let position = transport.load(Ordering::Relaxed);
            a.update(&p)?;
            if !Arc::ptr_eq(&transport, &a.controls.position)
                || transport.load(Ordering::Relaxed) < position
                || !a.controls.playing.load(Ordering::Relaxed)
            {
                return Err("Live edit interrupted or rewound transport".into());
            }
            edits += 1;
        }
        if a.controls.position.load(Ordering::Relaxed) >= p.sample_rate as u64 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if a.controls.error.load(Ordering::Relaxed)
        || a.controls.position.load(Ordering::Relaxed) < p.sample_rate as u64
        || (live_edits && edits != 7)
    {
        return Err("Audio callback did not advance successfully".into());
    }
    println!(
        "Smoke passed: {} tracks, {} buses, save/load/export, {} live edits, audio device {}, callback position {}",
        p.tracks.len(),
        p.buses.len(),
        edits,
        a.device,
        a.controls.position.load(Ordering::Relaxed)
    );
    Ok(())
}
