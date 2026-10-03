//! Timeline range edits preserve time, and sample processing runs on a worker.
use super::*;
use resonara_core::audio_edit::AudioEdit;

#[derive(Clone)]
pub(super) struct AudioClipboard {
    pub audio: Clip,
    pub rate: u32,
}

impl Daw {
    fn editor_edit_range(&self) -> Option<(Clip, std::ops::Range<usize>, u32)> {
        let (_, _, clip, rate, _) = self.editor_selected()?;
        let range = self.region_editor.selection.get().range();
        (range.start <= range.end && range.end <= clip.frames).then_some((clip, range, rate))
    }
    pub(super) fn editor_copy(&self) {
        let Some((mut clip, range, rate)) = self.editor_edit_range() else {
            return;
        };
        if range.is_empty() {
            self.status.set("Select audio to copy".into());
            return;
        }
        if let Err(error) = clip.trim_relative(range.start, range.end) {
            self.status.set(error.to_string());
            return;
        }
        *self.region_editor.clipboard.borrow_mut() = Some(AudioClipboard { audio: clip, rate });
        self.status.set(format!("Copied {} samples", range.len()));
        self.changed();
    }
    pub(super) fn editor_cut(&self) {
        if self.busy() || self.region_processing_active() {
            return;
        }
        self.editor_copy();
        self.editor_remove_range("Cut selected audio");
    }
    pub(super) fn editor_delete_range(&self) {
        self.editor_remove_range("Delete selected audio");
    }
    fn editor_remove_range(&self, label: &str) {
        if self.busy() || self.region_processing_active() {
            return;
        }
        let Some((track, index, clip, _, _)) = self.editor_selected() else {
            return;
        };
        let range = self.region_editor.selection.get().range();
        if range.is_empty() {
            self.status.set("Select audio to delete".into());
            return;
        }
        self.edit(label, |m| {
            let remaining = resonara_core::audio_edit::remove_region_range(&clip, range.clone())?;
            let count = remaining.len();
            m.project.tracks[track]
                .clips
                .splice(index..index + 1, remaining);
            // Keep the left segment selected, so its end remains the cut point.
            m.clip = (count > 0).then_some(index);
            Ok(())
        });
        if range.start > 0 {
            self.editor_set_selection(region_editor::Selection {
                anchor: range.start,
                head: range.start,
            });
        }
    }
    pub(super) fn editor_silence(&self) {
        let Some((_, range, _)) = self.editor_edit_range() else {
            return;
        };
        if range.is_empty() {
            return;
        }
        self.start_audio_edit(AudioEdit::Silence(range.clone()), range);
    }
    pub(super) fn editor_reverse_range(&self) {
        let Some((_, range, _)) = self.editor_edit_range() else {
            return;
        };
        if range.is_empty() {
            self.editor_reverse();
        } else {
            self.start_audio_edit(AudioEdit::Reverse(range.clone()), range);
        }
    }
    pub(super) fn editor_paste(&self) {
        if self.busy() || self.region_processing_active() {
            return;
        }
        let Some(clipboard) = self.region_editor.clipboard.borrow().clone() else {
            self.status.set("Copy audio before pasting".into());
            return;
        };
        let m = self.model.borrow();
        if m.selected_bus.is_some() || m.selected >= m.project.tracks.len() {
            return;
        }
        let (track, rate) = (m.selected, m.project.sample_rate);
        drop(m);
        if clipboard.rate != rate {
            self.status
                .set("Clipboard audio uses a different project sample rate".into());
            return;
        }
        let mut pasted = clipboard.audio;
        pasted.start = (self.playhead.get().max(0.) * rate as f64).round() as u64;
        if pasted.start.checked_add(pasted.frames as u64).is_none() {
            return;
        }
        self.edit("Paste audio at playhead", |m| {
            let clips = &mut m.project.tracks[track].clips;
            let at = clips
                .iter()
                .position(|c| c.start > pasted.start)
                .unwrap_or(clips.len());
            clips.insert(at, pasted);
            m.clip = Some(at);
            Ok(())
        });
    }
    fn start_audio_edit(&self, edit: AudioEdit, selection: std::ops::Range<usize>) {
        self.start_region_processing(region_processing::RegionOperation::Edit { edit, selection });
    }
}
