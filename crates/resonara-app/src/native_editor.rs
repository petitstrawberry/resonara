//! Native CLAP editors are serviced on the application main thread, beside SGFX.
use super::*;
use resonara_core::InsertKind;

impl Daw {
    fn flattened_insert(&self, target: RoutingTarget, slot: usize) -> Option<usize> {
        let m = self.model.borrow();
        let channel = match target {
            RoutingTarget::Track(index) if index < m.project.tracks.len() => index,
            RoutingTarget::Bus(id) => {
                m.project.tracks.len() + m.project.buses.iter().position(|b| b.id == id)?
            }
            _ => return None,
        };
        let routing = target.get(&m.project)?;
        if slot >= routing.inserts.len() {
            return None;
        }
        Some(
            m.project
                .tracks
                .iter()
                .map(|t| &t.routing)
                .chain(m.project.buses.iter().map(|b| &b.routing))
                .take(channel)
                .map(|r| r.inserts.len())
                .sum::<usize>()
                + slot,
        )
    }
    pub(super) fn open_native_editor(&self, target: RoutingTarget, slot: usize) -> bool {
        #[cfg(not(any(target_os = "macos", target_os = "scarlet")))]
        {
            let _ = (target, slot);
            return false;
        }
        #[cfg(any(target_os = "macos", target_os = "scarlet"))]
        {
            self.close_inactive_editor();
            let Some(index) = self.flattened_insert(target, slot) else {
                return false;
            };
            let insert = target
                .get(&self.model.borrow().project)
                .and_then(|r| r.inserts.get(slot))
                .cloned();
            if !matches!(
                insert,
                Some(resonara_core::Insert {
                    kind: InsertKind::Clap { .. },
                    ..
                })
            ) {
                return false;
            }
            let mut created_session = false;
            let result = {
                let mut m = self.model.borrow_mut();
                if m.audio.is_none() && m.retired_audio.is_none() {
                    match Audio::start_paused(&m.project) {
                        Ok(audio) => {
                            m.retired_audio = Some(audio);
                            created_session = true;
                        }
                        Err(error) => {
                            self.status.set(format!("Plug-in GUI unavailable: {error}"));
                            return false;
                        }
                    }
                }
                m.audio
                    .as_ref()
                    .or(m.retired_audio.as_ref())
                    .unwrap()
                    .open_editor(index)
            };
            if created_session && !matches!(result, Ok(true)) {
                self.model.borrow_mut().retired_audio = None;
            }
            match result {
                Ok(opened) => opened,
                Err(error) => {
                    self.status.set(format!("Plug-in GUI unavailable: {error}"));
                    false
                }
            }
        }
    }
    pub(super) fn poll_native_editors(&self, force: bool) {
        if self.native_edit_group.get() && self.native_edit_time.get().elapsed().as_millis() >= 300
        {
            self.native_edit_group.set(false);
        }
        let mut changes = Vec::new();
        let live = {
            let mut m = self.model.borrow_mut();
            let m = &mut *m;
            m.audio
                .as_mut()
                .or(m.retired_audio.as_mut())
                .map(|audio| audio.poll_plugins(force))
                .transpose()
        };
        match live {
            Ok(Some(updates)) => changes.extend(updates),
            Err(error) => self
                .status
                .set(format!("Plug-in editor update failed: {error}")),
            _ => {}
        }
        let mut stopped = None;
        let mut closed = false;
        {
            let mut editor = self.native_editor.borrow_mut();
            if let Some((target, slot, session)) = editor.as_mut() {
                let result = session.poll().and_then(|open| {
                    closed = !open;
                    session.snapshot(force || closed)
                });
                match result {
                    Ok(Some(plugin)) => stopped = Some((*target, *slot, plugin)),
                    Err(error) => self
                        .status
                        .set(format!("Plug-in editor update failed: {error}")),
                    _ => {}
                }
            }
        }
        if let Some((target, slot, plugin)) = stopped
            && let Some(index) = self.flattened_insert(target, slot)
        {
            changes.push((index, plugin));
        }
        if closed {
            self.native_editor.borrow_mut().take();
        }
        if changes.is_empty() {
            return;
        }
        let mut m = self.model.borrow_mut();
        let changed = changes.iter().any(|(index, plugin)| {
            m.project
                .tracks
                .iter()
                .map(|t| &t.routing)
                .chain(m.project.buses.iter().map(|b| &b.routing))
                .flat_map(|r| &r.inserts)
                .nth(*index)
                .is_some_and(|i| {
                    i.kind
                        != (InsertKind::Clap {
                            plugin: plugin.clone(),
                        })
                })
        });
        if !changed {
            return;
        }
        if !self.native_edit_group.replace(true) {
            let before = Self::snapshot(&m);
            Self::history(&mut m, before, "Plug-in GUI change");
        }
        self.native_edit_time.set(Instant::now());
        let project = &mut m.project;
        for (index, plugin) in changes {
            if let Some(insert) = project
                .tracks
                .iter_mut()
                .map(|t| &mut t.routing)
                .chain(project.buses.iter_mut().map(|b| &mut b.routing))
                .flat_map(|r| &mut r.inserts)
                .nth(index)
            {
                insert.kind = InsertKind::Clap { plugin };
            }
        }
        drop(m);
        self.changed();
    }
    pub(super) fn close_inactive_editor(&self) {
        if let Some((_, _, editor)) = self.native_editor.borrow().as_ref()
            && let Err(error) = editor.close()
        {
            self.status
                .set(format!("Plug-in GUI close failed: {error}"));
        }
        self.poll_native_editors(true);
        self.native_editor.borrow_mut().take();
    }
    pub(super) fn close_native_editors(&self) {
        if let Some(audio) = {
            let m = self.model.borrow();
            m.audio
                .as_ref()
                .or(m.retired_audio.as_ref())
                .map(|audio| audio.close_editors())
        } && let Err(error) = audio
        {
            self.status
                .set(format!("Plug-in GUI close failed: {error}"));
        }
        self.close_inactive_editor();
    }
}
