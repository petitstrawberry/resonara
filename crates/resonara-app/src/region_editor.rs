//! Selected-region workspace. Selection and zoom are independent of arrangement
//! navigation; source peaks and the waveform mesh survive transport animation.
use super::*;
use scarlet_ui::{
    element::{Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult},
    renderer::PaintContext,
};
use std::any::Any;

macro_rules! row { ($($v:expr),* $(,)?) => { HStack::new(Children(vec![$(Box::new($v) as Box<dyn View>),*])) }; }
macro_rules! column { ($($v:expr),* $(,)?) => { VStack::new(Children(vec![$(Box::new($v) as Box<dyn View>),*])) }; }

const SIDEBAR: f32 = 166.;
const TOOLBAR: f32 = CONTROL_HEIGHT + 8.;
const RULER: f32 = 24.;
const FOOTER: f32 = CONTROL_HEIGHT + 4.;

fn local_state<T: 'static>(value: T) -> State<T> {
    State::new(scarlet_ui::state::generate_state_id(), value)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Selection {
    pub anchor: usize,
    pub head: usize,
}
impl Selection {
    pub fn range(self) -> std::ops::Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    track: usize,
    clip: usize,
    source: usize,
    offset: usize,
    frames: usize,
}

struct FadeDrag {
    identity: Identity,
    version: u64,
    original: Clip,
    fade_in: bool,
}

#[derive(Clone, Copy, PartialEq)]
struct WaveformKey {
    identity: Option<Identity>,
    edit: resonara_core::ClipEdit,
    channels: u16,
    size: Size,
    start: f64,
    span: f64,
    amplitude: f32,
}
impl Identity {
    fn new(track: usize, clip: usize, value: &Clip) -> Self {
        Self {
            track,
            clip,
            source: Arc::as_ptr(&value.samples) as usize,
            offset: value.source_offset,
            frames: value.frames,
        }
    }
}

pub(super) struct RegionEditor {
    identity: Cell<Option<Identity>>,
    observed: Cell<Option<(f32, usize, usize, u32)>>,
    pub selection: State<Selection>,
    pub(super) clipboard: RefCell<Option<region_audio_edits::AudioClipboard>>,
    pub(super) menu: State<Option<Point>>,
    pub(super) menu_choice: Cell<usize>,
    drag: Cell<Option<Selection>>,
    select_all: Cell<bool>,
    start: Cell<f64>,
    span: Cell<f64>,
    amplitude: Cell<f32>,
    zero_cross: State<bool>,
    sample_units: State<bool>,
    range_start: State<String>,
    range_end: State<String>,
    observed_selection: Cell<Option<(Selection, bool, u32)>>,
    pub(super) modifiers: Cell<scarlet_ui::event::KeyModifiers>,
    pub(super) ruler_bounds: Cell<Option<(f64, f64)>>,
    fade_drag: RefCell<Option<FadeDrag>>,
    fade_preview: State<Option<(usize, usize)>>,
    scroll_drag: Cell<Option<(f32, f64)>>,
    size: State<Size>,
    gain: State<String>,
    fade_in: State<String>,
    fade_out: State<String>,
    pub(super) normalize: State<String>,
    #[cfg(test)]
    pub(super) transpose: State<String>,
    focused: State<bool>,
    canvas: SgfxCanvasHandle,
    mesh: SgfxMeshHandle,
    mesh_revision: Cell<u64>,
    cache: RefCell<Option<(WaveformKey, Arc<SgfxCanvasFrame>)>>,
    frame: State<Arc<SgfxCanvasFrame>>,
}
impl RegionEditor {
    pub fn new() -> Self {
        Self {
            identity: Cell::new(None),
            observed: Cell::new(None),
            selection: local_state(Selection::default()),
            clipboard: RefCell::new(None),
            menu: local_state(None),
            menu_choice: Cell::new(0),
            drag: Cell::new(None),
            select_all: Cell::new(false),
            start: Cell::new(0.),
            span: Cell::new(1.),
            amplitude: Cell::new(1.),
            zero_cross: local_state(false),
            sample_units: local_state(false),
            range_start: local_state("0.000000".into()),
            range_end: local_state("0.000000".into()),
            observed_selection: Cell::new(None),
            modifiers: Cell::new(Default::default()),
            ruler_bounds: Cell::new(None),
            fade_drag: RefCell::new(None),
            fade_preview: local_state(None),
            scroll_drag: Cell::new(None),
            size: local_state(Size::new(800., 300.)),
            gain: local_state("0.0".into()),
            fade_in: local_state("0.0".into()),
            fade_out: local_state("0.0".into()),
            normalize: local_state("-1.0".into()),
            #[cfg(test)]
            transpose: local_state("0.00".into()),
            focused: local_state(false),
            canvas: SgfxCanvasHandle::new(),
            mesh: SgfxMeshHandle::new(),
            mesh_revision: Cell::new(0),
            cache: RefCell::new(None),
            frame: local_state(Arc::new(SgfxCanvasFrame::new(0, BG))),
        }
    }
    fn waveform_size(&self) -> Size {
        let size = self.size.get();
        Size::new(
            (size.width - SIDEBAR - 1.).max(200.),
            (size.height - TOOLBAR - RULER - FOOTER).max(80.),
        )
    }
    pub(super) fn frame_at(&self, x: f32, frames: usize) -> usize {
        (self.start.get() + (x / self.waveform_size().width).clamp(0., 1.) as f64 * self.span.get())
            .round()
            .clamp(0., frames as f64) as usize
    }
    fn set_view(&self, start: f64, span: f64, frames: usize) {
        let length = frames.max(1) as f64;
        let span = span.clamp(1., length);
        self.span.set(span);
        self.start.set(start.clamp(0., (length - span).max(0.)));
    }
}

impl Daw {
    pub(super) fn editor_gesture_active(&self) -> bool {
        self.region_editor.drag.get().is_some()
            || self.region_editor.fade_drag.borrow().is_some()
            || self.region_editor.scroll_drag.get().is_some()
    }
    pub(super) fn editor_selected(&self) -> Option<(usize, usize, Clip, u32, String)> {
        let m = self.model.borrow();
        if m.selected_bus.is_some() {
            return None;
        }
        let index = m.clip?;
        let track = m.project.tracks.get(m.selected)?;
        Some((
            m.selected,
            index,
            track.clips.get(index)?.clone(),
            m.project.sample_rate,
            track.name.clone(),
        ))
    }

    pub(super) fn refresh_region_editor(&self) {
        let editor = &self.region_editor;
        let Some((track, index, clip, rate, _)) = self.editor_selected() else {
            if editor.ruler_bounds.get().is_some() {
                self.cancel_ruler_drag();
            }
            editor.menu.set(None);
            editor.fade_drag.borrow_mut().take();
            editor.fade_preview.set(None);
            editor.identity.set(None);
            editor.observed.set(None);
            editor.observed_selection.set(None);
            editor.scroll_drag.set(None);
            editor.drag.set(None);
            if editor.selection.get() != Selection::default() {
                editor.selection.set(Selection::default());
            }
            editor.cache.borrow_mut().take();
            return;
        };
        let identity = Identity::new(track, index, &clip);
        if editor.identity.get() != Some(identity) {
            if editor.ruler_bounds.get().is_some() {
                self.cancel_ruler_drag();
            }
            editor.menu.set(None);
            editor.identity.set(Some(identity));
            editor.observed.set(None);
            editor.observed_selection.set(None);
            editor.scroll_drag.set(None);
            editor.selection.set(Selection::default());
            editor.drag.set(None);
            editor.fade_drag.borrow_mut().take();
            editor.fade_preview.set(None);
            editor.set_view(0., clip.frames.max(1) as f64, clip.frames);
        }
        self.editor_refresh_range_fields(rate);
        // Arrangement-follow and other view refreshes can arrive while a field
        // is being typed. Only actual parameter changes replace its draft.
        let previous = editor.observed.get();
        if previous.is_none_or(|v| v.0 != clip.edit.gain_db) {
            editor.gain.set(format!("{:.1}", clip.edit.gain_db));
        }
        if previous.is_none_or(|v| v.1 != clip.edit.fade_in || v.3 != rate) {
            editor.fade_in.set(format!(
                "{:.1}",
                clip.edit.fade_in.min(clip.frames) as f64 * 1000. / rate as f64
            ));
        }
        if previous.is_none_or(|v| v.2 != clip.edit.fade_out || v.3 != rate) {
            editor.fade_out.set(format!(
                "{:.1}",
                clip.edit.fade_out.min(clip.frames) as f64 * 1000. / rate as f64
            ));
        }
        editor.observed.set(Some((
            clip.edit.gain_db,
            clip.edit.fade_in,
            clip.edit.fade_out,
            rate,
        )));
        if self.panel_visible.get() && self.panel_mode() == lower_panel::PanelTab::Editor {
            self.refresh_editor_waveform(&clip, track);
        }
    }

    fn refresh_editor_waveform(&self, clip: &Clip, track: usize) {
        let e = &self.region_editor;
        let mut preview = clip.clone();
        if let Some((fade_in, fade_out)) = e.fade_preview.get() {
            let _ = preview.set_fades(fade_in, fade_out);
        }
        let clip = &preview;
        let size = e.waveform_size();
        let key = WaveformKey {
            identity: e.identity.get(),
            edit: clip.edit,
            channels: clip.source_channels,
            size,
            start: e.start.get(),
            span: e.span.get(),
            amplitude: e.amplitude.get(),
        };
        if e.cache
            .borrow()
            .as_ref()
            .is_some_and(|(previous, _)| *previous == key)
        {
            return;
        }
        let mut vertices = Vec::new();
        let color = ui::color(track);
        let lanes = editor_lanes(clip.source_channels, size.height);
        let mut m = self.model.borrow_mut();
        let peaks = &mut m.peaks;
        for lane in &lanes {
            wave::rectangle(
                &mut vertices,
                0.,
                lane.center,
                size.width,
                1.,
                LINE,
                size.width,
                size.height,
            );
            // At sample zoom draw the actual signed samples and their connecting
            // line. Peak bars are an overview, not a reconstruction of audio.
            if e.span.get() <= size.width as f64 / 2. {
                let first = e.start.get().floor().max(0.) as usize;
                let last = (e.start.get() + e.span.get())
                    .ceil()
                    .min(clip.frames.saturating_sub(1) as f64) as usize;
                let mut previous = None;
                for at in first..=last {
                    let x = ((at as f64 - e.start.get()) / e.span.get()) as f32 * size.width;
                    let y = lane.center
                        - (clip.sample_at(at as f64)[lane.channel] * e.amplitude.get())
                            .clamp(-1., 1.)
                            * lane.amplitude;
                    if let Some((px, py)) = previous {
                        waveform_line(&mut vertices, (px, py), (x, y), 1.3, color, size);
                    }
                    if e.span.get() <= size.width as f64 / 5. {
                        wave::rectangle(
                            &mut vertices,
                            x - 2.,
                            y - 2.,
                            4.,
                            4.,
                            color,
                            size.width,
                            size.height,
                        );
                    }
                    previous = Some((x, y));
                }
                continue;
            }
            let columns = size.width.ceil() as usize;
            for column in 0..columns {
                let from = (e.start.get() + column as f64 / size.width as f64 * e.span.get())
                    .floor() as usize;
                let to = (e.start.get() + (column + 1) as f64 / size.width as f64 * e.span.get())
                    .ceil() as usize;
                let envelope =
                    peaks.clip_envelope(clip, from.min(clip.frames), to.min(clip.frames));
                let top = lane.center
                    - (envelope.max[lane.channel] * e.amplitude.get()).clamp(-1., 1.)
                        * lane.amplitude;
                let bottom = lane.center
                    - (envelope.min[lane.channel] * e.amplitude.get()).clamp(-1., 1.)
                        * lane.amplitude;
                wave::rectangle(
                    &mut vertices,
                    column as f32,
                    top,
                    1.,
                    (bottom - top).max(0.8),
                    color,
                    size.width,
                    size.height,
                );
            }
        }
        if lanes.len() == 2 {
            wave::rectangle(
                &mut vertices,
                0.,
                size.height / 2.,
                size.width,
                1.,
                LINE,
                size.width,
                size.height,
            );
        }
        let revision = e.mesh_revision.get().wrapping_add(1);
        e.mesh_revision.set(revision);
        let frame = Arc::new(
            SgfxCanvasFrame::new(revision, BG)
                .reference_aspect(size.width / size.height)
                .draw(SgfxCanvasDraw::new(
                    SgfxMesh::with_handle(e.mesh, revision, vertices),
                    wave::ID,
                )),
        );
        e.frame.set(frame.clone());
        *e.cache.borrow_mut() = Some((key, frame));
    }

    fn editor_redraw(&self) {
        if let Some((track, _, clip, _, _)) = self.editor_selected() {
            self.refresh_editor_waveform(&clip, track);
        }
        self.changed();
    }

    fn editor_refresh_range_fields(&self, rate: u32) {
        let e = &self.region_editor;
        let selection = e.selection.get();
        let units = e.sample_units.get();
        if e.observed_selection.get() == Some((selection, units, rate)) {
            return;
        }
        let range = selection.range();
        let format = |at: usize| {
            if units {
                at.to_string()
            } else {
                format!("{:.6}", at as f64 / rate as f64)
            }
        };
        e.range_start.set(format(range.start));
        e.range_end.set(format(range.end));
        e.observed_selection.set(Some((selection, units, rate)));
    }
    pub(super) fn editor_set_selection(&self, mut selection: Selection) {
        if let Some((_, _, clip, rate, _)) = self.editor_selected() {
            selection.anchor = selection.anchor.min(clip.frames);
            selection.head = selection.head.min(clip.frames);
            self.region_editor.selection.set(selection);
            self.editor_refresh_range_fields(rate);
            self.changed();
        }
    }
    fn editor_submit_range(&self) {
        let Some((_, _, clip, rate, _)) = self.editor_selected() else {
            return;
        };
        let e = &self.region_editor;
        let parse = |text: String| -> Option<usize> {
            if e.sample_units.get() {
                text.trim()
                    .parse::<usize>()
                    .ok()
                    .filter(|&at| at <= clip.frames)
            } else {
                text.trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|at| {
                        at.is_finite() && *at >= 0. && *at <= clip.frames as f64 / rate as f64
                    })
                    .map(|at| (at * rate as f64).round() as usize)
            }
        };
        match (parse(e.range_start.get()), parse(e.range_end.get())) {
            (Some(start), Some(end)) if start <= end => self.editor_set_selection(Selection {
                anchor: start,
                head: end,
            }),
            _ => {
                self.status
                    .set("Selection needs ordered start/end positions inside this region".into());
            }
        }
    }
    fn editor_zoom_at(&self, factor: f64, x: f32) {
        let Some((_, _, clip, _, _)) = self.editor_selected() else {
            return;
        };
        let e = &self.region_editor;
        let ratio = (x / e.waveform_size().width).clamp(0., 1.) as f64;
        let at = e.start.get() + e.span.get() * ratio;
        let span = (e.span.get() * factor).clamp(1., clip.frames.max(1) as f64);
        e.set_view(at - span * ratio, span, clip.frames);
        self.editor_redraw();
    }
    fn editor_amplitude(&self, factor: f32) {
        let e = &self.region_editor;
        e.amplitude
            .set((e.amplitude.get() * factor).clamp(0.25, 16.));
        self.editor_redraw();
    }
    fn editor_pointer_frame(&self, x: f32, clip: &Clip) -> usize {
        let e = &self.region_editor;
        let at = e.frame_at(x, clip.frames);
        if !e.zero_cross.get() || at == 0 || at == clip.frames {
            return at;
        }
        // Use the source waveform, not gain/fades: a zero fade is not an audio
        // crossing. Search at most 1024 samples, never the complete source.
        let radius =
            ((e.span.get() / e.waveform_size().width as f64 * 8.).ceil() as usize).clamp(8, 512);
        let at_value = clip.samples[clip.source_offset
            + if clip.edit.reversed {
                clip.frames - 1 - at.min(clip.frames - 1)
            } else {
                at.min(clip.frames - 1)
            }];
        let channel = if at_value[0].abs() >= at_value[1].abs() {
            0
        } else {
            1
        };
        let sample = |n: usize| {
            let index = if clip.edit.reversed {
                clip.frames - 1 - n
            } else {
                n
            } + clip.source_offset;
            clip.samples[index][channel]
        };
        (at.saturating_sub(radius).max(1)..at.saturating_add(radius).min(clip.frames))
            .filter(|&n| {
                let (a, b) = (sample(n - 1), sample(n));
                a == 0. || b == 0. || a.is_sign_negative() != b.is_sign_negative()
            })
            .min_by_key(|&n| n.abs_diff(at))
            .unwrap_or(at)
    }
    fn editor_cancel_fade(&self) -> bool {
        if self.region_editor.fade_drag.borrow_mut().take().is_none() {
            return false;
        }
        self.region_editor.fade_preview.set(None);
        self.editor_redraw();
        true
    }
    fn editor_fade_motion(&self, x: f32, commit: bool) {
        let e = &self.region_editor;
        let (fade_in, fade_out) = {
            let drag = e.fade_drag.borrow();
            let Some(drag) = drag.as_ref() else {
                return;
            };
            let at = e.frame_at(x, drag.original.frames);
            let mut fades = (
                drag.original.edit.fade_in.min(drag.original.frames),
                drag.original.edit.fade_out.min(drag.original.frames),
            );
            if drag.fade_in {
                fades.0 = at;
            } else {
                fades.1 = drag.original.frames - at;
            }
            fades
        };
        e.fade_preview.set(Some((fade_in, fade_out)));
        if commit {
            let drag = e.fade_drag.borrow_mut().take().unwrap();
            e.fade_preview.set(None);
            let m = self.model.borrow();
            let valid = m.version == drag.version && e.identity.get() == Some(drag.identity);
            drop(m);
            if valid
                && (fade_in != drag.original.edit.fade_in
                    || fade_out != drag.original.edit.fade_out
                    || drag.original.edit.has_inherited_fades())
            {
                self.edit("Set region fades", |m| {
                    m.project.tracks[drag.identity.track].clips[drag.identity.clip]
                        .set_fades(fade_in, fade_out)
                });
            } else {
                self.editor_redraw();
            }
        } else {
            self.editor_redraw();
        }
    }

    pub(super) fn editor_fit(&self) {
        if let Some((_, _, clip, _, _)) = self.editor_selected() {
            self.region_editor
                .set_view(0., clip.frames.max(1) as f64, clip.frames);
            self.editor_redraw();
        }
    }
    pub(super) fn editor_zoom(&self, factor: f64) {
        if let Some((_, _, clip, _, _)) = self.editor_selected() {
            let e = &self.region_editor;
            let selection = e.selection.get();
            let center = if (selection.head as f64) >= e.start.get()
                && (selection.head as f64) <= e.start.get() + e.span.get()
            {
                selection.head as f64
            } else {
                e.start.get() + e.span.get() * 0.5
            };
            let span = (e.span.get() * factor).clamp(1., clip.frames.max(1) as f64);
            e.set_view(center - span * 0.5, span, clip.frames);
            self.editor_redraw();
        }
    }
    pub(super) fn editor_zoom_selection(&self) {
        if let Some((_, _, clip, _, _)) = self.editor_selected() {
            let range = self.region_editor.selection.get().range();
            if range.is_empty() {
                return;
            }
            self.region_editor
                .set_view(range.start as f64, range.len() as f64, clip.frames);
            self.editor_redraw();
        }
    }
    fn editor_scroll(&self, direction: f64) {
        if let Some((_, _, clip, _, _)) = self.editor_selected() {
            let e = &self.region_editor;
            e.set_view(
                e.start.get() + e.span.get() * direction,
                e.span.get(),
                clip.frames,
            );
            self.editor_redraw();
        }
    }

    /// A selection is local UI state. Cancel never writes a document edit or
    /// leaves a partial history entry, including when the pointer exits the pane.
    fn editor_event(&self, event: &Event) -> bool {
        let Some((track, index, clip, rate, _)) = self.editor_selected() else {
            return false;
        };
        let e = &self.region_editor;
        match event {
            Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                y,
                click_count,
            }) => {
                if self.busy() || self.dialog.get() != Dialog::None {
                    return true;
                }
                let map = |frame: usize| {
                    ((frame as f64 - e.start.get()) / e.span.get()) as f32 * e.waveform_size().width
                };
                let fade_in_x =
                    map(clip.edit.fade_in.min(clip.frames)).clamp(5., e.waveform_size().width - 5.);
                let fade_out_x = map(clip.frames - clip.edit.fade_out.min(clip.frames))
                    .clamp(5., e.waveform_size().width - 5.);
                if *y <= 14
                    && !self.region_processing_active()
                    && ((*x as f32 - fade_in_x).abs() <= 8. || (*x as f32 - fade_out_x).abs() <= 8.)
                {
                    *e.fade_drag.borrow_mut() = Some(FadeDrag {
                        identity: Identity::new(track, index, &clip),
                        version: self.model.borrow().version,
                        original: clip,
                        fade_in: (*x as f32 - fade_in_x).abs() <= (*x as f32 - fade_out_x).abs(),
                    });
                    self.status
                        .set("Drag fade handle · Escape cancels · one Undo on release".into());
                    return true;
                }
                let previous = e.selection.get();
                e.drag.set(Some(previous));
                e.select_all.set(*click_count >= 2);
                let frame = self.editor_pointer_frame(*x as f32, &clip);
                let range = previous.range();
                let selection = if *click_count >= 2 {
                    Selection {
                        anchor: 0,
                        head: clip.frames,
                    }
                } else if e.modifiers.get().shift {
                    Selection {
                        anchor: previous.anchor,
                        head: frame,
                    }
                } else if !range.is_empty() && (*x as f32 - map(range.start)).abs() <= 5. {
                    Selection {
                        anchor: range.end,
                        head: frame,
                    }
                } else if !range.is_empty() && (*x as f32 - map(range.end)).abs() <= 5. {
                    Selection {
                        anchor: range.start,
                        head: frame,
                    }
                } else {
                    Selection {
                        anchor: frame,
                        head: frame,
                    }
                };
                self.editor_set_selection(selection);
                true
            }
            Event::Mouse(MouseEvent::Moved { x, .. }) if e.fade_drag.borrow().is_some() => {
                self.editor_fade_motion(*x as f32, false);
                true
            }
            Event::Mouse(MouseEvent::Moved { x, .. }) if e.drag.get().is_some() => {
                if e.select_all.get() {
                    return true;
                }
                let width = e.waveform_size().width;
                if *x < 0 || *x as f32 > width {
                    let direction = if *x < 0 { -1. } else { 1. };
                    e.set_view(
                        e.start.get() + e.span.get() * direction * 0.04,
                        e.span.get(),
                        clip.frames,
                    );
                    self.editor_redraw();
                }
                let mut selection = e.selection.get();
                selection.head = self.editor_pointer_frame(*x as f32, &clip);
                self.editor_set_selection(selection);
                true
            }
            Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x,
                ..
            }) if e.fade_drag.borrow().is_some() => {
                self.editor_fade_motion(*x as f32, true);
                true
            }
            Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x,
                ..
            }) if e.drag.get().is_some() => {
                e.drag.set(None);
                if !e.select_all.get() {
                    let mut selection = e.selection.get();
                    selection.head = self.editor_pointer_frame(*x as f32, &clip);
                    self.editor_set_selection(selection);
                }
                let selection = e.selection.get();
                if selection.anchor == selection.head {
                    self.seek((clip.start as f64 + selection.head as f64) / rate as f64);
                }
                self.changed();
                true
            }
            Event::Mouse(MouseEvent::ButtonCancelled {
                button: MouseButton::Left,
                ..
            }) => {
                if self.editor_cancel_fade() {
                    return true;
                }
                if let Some(previous) = e.drag.take() {
                    self.editor_set_selection(previous);
                    return true;
                }
                false
            }
            Event::Mouse(MouseEvent::Wheel {
                delta_x,
                delta_y,
                x,
                ..
            }) => {
                if e.modifiers.get().control || e.modifiers.get().super_key {
                    self.editor_zoom_at(2f64.powf(*delta_y as f64 / 240.), *x as f32);
                } else if e.modifiers.get().alt {
                    self.editor_amplitude(2f32.powf(*delta_y as f32 / 240.));
                } else {
                    let delta = if *delta_x != 0 { *delta_x } else { *delta_y };
                    // Native deltas are already normalized to the user's scroll direction.
                    self.editor_scroll(delta as f64 / 480.);
                }
                true
            }
            _ => false,
        }
    }

    fn editor_key(&self, event: KeyEvent) -> bool {
        if let KeyEvent::Pressed { modifiers, .. } | KeyEvent::Released { modifiers, .. } = event {
            self.region_editor.modifiers.set(modifiers);
        }
        let KeyEvent::Pressed { keycode, modifiers } = event else {
            return false;
        };
        if self.busy() || self.dialog.get() != Dialog::None {
            return false;
        }
        let Some((_, _, clip, rate, _)) = self.editor_selected() else {
            return false;
        };
        let e = &self.region_editor;
        let primary = modifiers.control || modifiers.super_key;
        if primary {
            match keycode {
                KeyCode::Char('a' | 'A') => self.editor_set_selection(Selection {
                    anchor: 0,
                    head: clip.frames,
                }),
                KeyCode::Char('c' | 'C') => self.editor_copy(),
                KeyCode::Char('x' | 'X') => self.editor_cut(),
                KeyCode::Char('v' | 'V') => self.editor_paste(),
                KeyCode::Char('t' | 'T') => self.editor_trim(),
                KeyCode::Char('z' | 'Z') => self.undo(modifiers.shift),
                KeyCode::Char('y' | 'Y') => self.undo(true),
                _ => return false,
            }
            return true;
        }
        match keycode {
            KeyCode::Escape => {
                if !self.editor_cancel_fade() {
                    self.editor_set_selection(e.drag.take().unwrap_or_default());
                }
            }
            KeyCode::Delete | KeyCode::Backspace => self.editor_delete_range(),
            KeyCode::Char('s' | 'S') => self.editor_split(),
            KeyCode::Char('f' | 'F') => self.editor_fit(),
            KeyCode::Char('+' | '=') => self.editor_zoom(0.5),
            KeyCode::Char('-') => self.editor_zoom(2.),
            KeyCode::Space => {
                if self.model.borrow().audio.is_none() {
                    self.seek(
                        (clip.start + e.selection.get().range().start as u64) as f64 / rate as f64,
                    );
                }
                self.play();
            }
            KeyCode::Home | KeyCode::End => {
                let mut selection = e.selection.get();
                selection.head = if keycode == KeyCode::Home {
                    0
                } else {
                    clip.frames
                };
                if !modifiers.shift {
                    selection.anchor = selection.head;
                }
                self.editor_set_selection(selection);
                self.editor_reveal_cursor(clip.frames);
            }
            KeyCode::Left | KeyCode::Right => {
                let mut selection = e.selection.get();
                let range = selection.range();
                if !modifiers.shift && !range.is_empty() {
                    selection.head = if keycode == KeyCode::Left {
                        range.start
                    } else {
                        range.end
                    };
                } else {
                    // Fine zoom and Option always move one sample. An overview
                    // moves roughly one screen pixel rather than invisible steps.
                    let step = if modifiers.alt {
                        1
                    } else {
                        (e.span.get() / e.waveform_size().width as f64)
                            .round()
                            .max(1.) as usize
                    };
                    selection.head = if keycode == KeyCode::Left {
                        selection.head.saturating_sub(step)
                    } else {
                        selection.head.saturating_add(step).min(clip.frames)
                    };
                }
                if !modifiers.shift {
                    selection.anchor = selection.head;
                }
                self.editor_set_selection(selection);
                self.editor_reveal_cursor(clip.frames);
            }
            _ => return false,
        }
        true
    }
    fn editor_reveal_cursor(&self, frames: usize) {
        let e = &self.region_editor;
        let cursor = e.selection.get().head as f64;
        if cursor < e.start.get() || cursor > e.start.get() + e.span.get() {
            e.set_view(cursor - e.span.get() * 0.5, e.span.get(), frames);
            self.editor_redraw();
        }
    }
    fn editor_ruler_event(&self, event: &Event) -> bool {
        let Some((_, _, clip, rate, _)) = self.editor_selected() else {
            return false;
        };
        let e = &self.region_editor;
        if matches!(
            event,
            Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                ..
            })
        ) {
            e.ruler_bounds.set(Some((
                clip.start as f64 / rate as f64,
                (clip.start + clip.frames as u64) as f64 / rate as f64,
            )));
        }
        let consumed = self.ruler_event(
            event,
            (clip.start as f64 + e.start.get()) / rate as f64,
            e.span.get() / rate as f64,
            e.waveform_size().width,
        );
        if matches!(
            event,
            Event::Mouse(MouseEvent::ButtonReleased { .. } | MouseEvent::ButtonCancelled { .. })
        ) {
            e.ruler_bounds.set(None);
        }
        consumed
    }

    pub(super) fn editor_trim(&self) {
        let Some((track, index, clip, _, _)) = self.editor_selected() else {
            return;
        };
        let range = self.region_editor.selection.get().range();
        if range.is_empty() {
            self.status
                .set("Drag a range in the audio editor to trim".into());
            return;
        }
        if range.start == 0 && range.end == clip.frames {
            return;
        }
        self.edit("Trim region to selection", |m| {
            m.project
                .tracks
                .get_mut(track)
                .and_then(|t| t.clips.get_mut(index))
                .ok_or("Region no longer exists")?
                .trim_relative(range.start, range.end)
        });
    }
    pub(super) fn editor_split(&self) {
        let Some((track, index, clip, _, _)) = self.editor_selected() else {
            return;
        };
        let selection = self.region_editor.selection.get();
        let range = selection.range();
        let mut cuts = if range.is_empty() {
            vec![selection.head]
        } else {
            vec![range.start, range.end]
        };
        cuts.retain(|&cut| cut > 0 && cut < clip.frames);
        cuts.dedup();
        if cuts.is_empty() {
            self.status
                .set("Choose a cursor or range inside the region to split".into());
            return;
        }
        self.edit("Split region at selection", |m| {
            let clips = &mut m
                .project
                .tracks
                .get_mut(track)
                .ok_or("Track no longer exists")?
                .clips;
            if index >= clips.len() {
                return Err("Region no longer exists".into());
            }
            for &cut in cuts.iter().rev() {
                let right = clips[index].split_relative(cut)?;
                clips.insert(index + 1, right);
            }
            m.clip = Some(if range.start > 0 { index + 1 } else { index });
            Ok(())
        });
    }
    pub(super) fn editor_gain(&self) {
        let Some((track, index, clip, _, _)) = self.editor_selected() else {
            return;
        };
        let text = self.region_editor.gain.get();
        if text.trim() == format!("{:.1}", clip.edit.gain_db) {
            return;
        }
        let Ok(value) = text.trim().parse::<f32>() else {
            self.status.set("Region gain must be a number in dB".into());
            return;
        };
        if !value.is_finite() || !(-60.0..=24.0).contains(&value) {
            self.status
                .set("Region gain must be between −60 and +24 dB".into());
            return;
        }
        if value == clip.edit.gain_db {
            return;
        }
        self.edit("Set region gain", |m| {
            m.project.tracks[track].clips[index].set_gain_db(value)
        });
    }
    pub(super) fn editor_fades(&self) {
        let Some((track, index, clip, rate, _)) = self.editor_selected() else {
            return;
        };
        let parse = |text: String, current: usize| {
            // Display precision is not DSP precision: an untouched 1-sample
            // fade may read 0.0 ms, and must survive editing the other edge.
            if text.trim() == format!("{:.1}", current as f64 * 1000. / rate as f64) {
                return Some(current);
            }
            text.trim()
                .parse::<f64>()
                .ok()
                .filter(|v| {
                    v.is_finite() && *v >= 0. && *v <= clip.frames as f64 * 1000. / rate as f64
                })
                .map(|v| (v * rate as f64 / 1000.).round() as usize)
        };
        let (Some(fade_in), Some(fade_out)) = (
            parse(self.region_editor.fade_in.get(), clip.edit.fade_in),
            parse(self.region_editor.fade_out.get(), clip.edit.fade_out),
        ) else {
            self.status
                .set("Fades must be milliseconds between 0 and the region length".into());
            return;
        };
        if fade_in == clip.edit.fade_in && fade_out == clip.edit.fade_out {
            return;
        }
        self.edit("Set region fades", |m| {
            m.project.tracks[track].clips[index].set_fades(fade_in, fade_out)
        });
    }
    pub(super) fn editor_reverse(&self) {
        let Some((track, index, clip, _, _)) = self.editor_selected() else {
            return;
        };
        self.edit("Reverse region", |m| {
            m.project.tracks[track].clips[index].set_reversed(!clip.edit.reversed);
            Ok(())
        });
    }
    pub(super) fn region_editor_view(&self) -> AnyView {
        AnyView::new(EditorWorkspace(self.clone()))
    }

    fn region_editor_content(&self) -> AnyView {
        let e = &self.region_editor;
        let Some((_, index, clip, rate, name)) = self.editor_selected() else {
            let geometry = self.clone();
            return AnyView::new(
                vstack! {
                    label("Select an audio region").font_size(16.),
                    caption("Edit its waveform, gain, fades and direction here."),
                    caption("Double-click a region in the arrangement to open the editor."),
                    self.editor_button("Paste at playhead", Some(Icon::Clipboard), "Paste the copied region on the selected track · Cmd/Ctrl+V", e.clipboard.borrow().is_some() && self.model.borrow().selected_bus.is_none(), |s| s.editor_paste()).frame(150., CONTROL_HEIGHT)
                }
                .spacing(10.)
                .frame(f32::INFINITY, f32::INFINITY)
                .background(BG)
                .on_geometry_change(|g| g.size(), move |size| geometry.editor_resized(size)),
            );
        };
        let selection = e.selection.get();
        let range = selection.range();
        let size = e.waveform_size();
        let gain = self.clone();
        let fade_in = self.clone();
        let fade_out = self.clone();
        let normalize = self.clone();
        let start_field = self.clone();
        let end_field = self.clone();
        let event = self.clone();
        let key = self.clone();
        let geometry = self.clone();
        let ruler = self.clone();
        self.editor_refresh_range_fields(rate);
        let fades = AnyView::new(vstack! {
            row! {caption("Fade in · ms").frame_width(75.), ui::compact_field(e.fade_in.clone()).on_submit(move || fade_in.editor_fades()).blur_on_submit(true).frame(65., CONTROL_HEIGHT).input_guard()}.spacing(2.),
            row! {caption("Fade out · ms").frame_width(75.), ui::compact_field(e.fade_out.clone()).on_submit(move || fade_out.editor_fades()).blur_on_submit(true).frame(65., CONTROL_HEIGHT).input_guard()}.spacing(2.),
            caption(if clip.edit.has_inherited_fades() { "Original envelope retained" } else { "Drag handles on the waveform" }).font_size(9.)
        }.spacing(7.).alignment(Alignment::TopLeading));
        let sidebar = ScrollView::new(column! {
            caption("AUDIO REGION").font_size(9.),
            ui::name_label(&name, 20, 12., self.status.clone()).frame_width(SIDEBAR - 24.),
            caption(format!("Region {} · {}", index + 1, if clip.source_channels == 1 { "Mono" } else { "Stereo" })).font_size(10.),
            caption(format!("{} Hz · {:.3} s", rate, clip.frames as f64 / rate as f64)).font_size(10.),
            Rectangle::new().fill(LINE).frame(SIDEBAR - 24., 1.),
            caption("REGION LEVEL / FADES").font_size(9.),
            row! {caption("Gain · dB").frame_width(75.), ui::compact_field(e.gain.clone()).on_submit(move || gain.editor_gain()).blur_on_submit(true).frame(65., CONTROL_HEIGHT).input_guard()}.spacing(2.),
            fades,
            row! {self.header_button("Normalize", "Normalize the whole region to the target dBFS", |s| s.editor_normalize()).frame(75., CONTROL_HEIGHT), ui::compact_field(e.normalize.clone()).on_submit(move || normalize.editor_normalize()).blur_on_submit(true).frame(65., CONTROL_HEIGHT).input_guard()}.spacing(2.),
            caption("Peak target · dBFS").font_size(9.),
            self.header_button(if range.is_empty() { "Reverse region" } else { "Reverse selection" }, "Reverse the selected audio; with no range, reverse the region", |s| s.editor_reverse_range()).frame(SIDEBAR - 24., CONTROL_HEIGHT),
            Rectangle::new().fill(LINE).frame(SIDEBAR - 24., 1.),
            caption("SELECTION").font_size(9.),
            row! {caption("Start").frame_width(40.), ui::compact_field(e.range_start.clone()).on_submit(move || start_field.editor_submit_range()).blur_on_submit(true).frame(100., CONTROL_HEIGHT).input_guard()}.spacing(2.),
            row! {caption("End").frame_width(40.), ui::compact_field(e.range_end.clone()).on_submit(move || end_field.editor_submit_range()).blur_on_submit(true).frame(100., CONTROL_HEIGHT).input_guard()}.spacing(2.),
            self.header_button(if e.sample_units.get() { "Samples" } else { "Seconds" }, "Switch selection positions between seconds and samples", |s| {
                s.region_editor.sample_units.set(!s.region_editor.sample_units.get());
                if let Some((_, _, _, rate, _)) = s.editor_selected() { s.editor_refresh_range_fields(rate); }
                s.changed();
            }).frame(SIDEBAR - 24., CONTROL_HEIGHT),
            self.header_button(if e.zero_cross.get() { "Zero crossing · On" } else { "Zero crossing · Off" }, "Snap range boundaries to nearby source zero crossings", |s| { s.region_editor.zero_cross.set(!s.region_editor.zero_cross.get()); s.changed(); }).frame(SIDEBAR - 24., CONTROL_HEIGHT),
            Rectangle::new().fill(LINE).frame(SIDEBAR - 24., 1.),
            self.header_button(&format!("Waveform {:.2}×", e.amplitude.get()), "Display magnification only · Option + wheel for continuous adjustment", |s| {
                let next = if s.region_editor.amplitude.get() >= 8. { 0.5 } else { s.region_editor.amplitude.get() * 2. };
                s.region_editor.amplitude.set(next); s.editor_redraw();
            }).frame(SIDEBAR - 24., CONTROL_HEIGHT),
            self.editor_button("Cancel processing", None, "Cancel the pending audio edit", self.region_processing_active(), |s| s.cancel_region_processing()).frame(SIDEBAR - 24., CONTROL_HEIGHT)
        }.spacing(7.).alignment(Alignment::TopLeading).padding(12.)).frame(SIDEBAR, e.size.get().height).background(PANEL);
        let available = !self.region_processing_active() && !self.busy();
        let selected = !range.is_empty();
        let toolbar = row! {
            self.editor_action(Icon::Copy, "Copy selected region segment · Cmd/Ctrl+C", selected, |s| s.editor_copy()),
            self.editor_action(Icon::Clipboard, "Paste a region at the playhead without shifting later audio · Cmd/Ctrl+V", available && e.clipboard.borrow().is_some(), |s| s.editor_paste()),
            self.editor_action(Icon::Trash, "Delete selected audio without copying or shifting later audio", available && selected, |s| s.editor_delete_range()),
            self.editor_action(Icon::VolumeOff, "Silence selection without changing its length", available && selected, |s| s.editor_silence()),
            Rectangle::new().fill(LINE).frame(1., 18.),
            self.editor_action(Icon::Cut, "Split at cursor / range boundaries · S", available && selection.head > 0 && range.start < clip.frames, |s| s.editor_split()),
            Spacer::new(),
            self.header_icon(Icon::ZoomOut, "Zoom out · −", false, |s| s.editor_zoom(2.)),
            self.header_icon(Icon::ZoomIn, "Zoom in · +", false, |s| s.editor_zoom(0.5)),
            self.editor_action(Icon::ArrowsMaximize, "Fit entire region · F", true, |s| s.editor_fit()),
            self.editor_action(Icon::Search, "Zoom to selection", selected, |s| s.editor_zoom_selection())
        }.spacing(4.).padding_insets(EdgeInsets::new(6., 4., 6., 4.)).frame(size.width, TOOLBAR).background(PANEL);
        let marks = EditorRuler {
            start: e.start.get(),
            span: e.span.get(),
            rate,
            width: size.width,
            position: animation::PaintState(self.playhead.clone()),
            region_start: clip.start as f64,
        }
        .frame(size.width, RULER)
        .alignment(Alignment::TopLeading)
        .on_event(move |event| ruler.editor_ruler_event(event));
        let mut layers: Vec<Box<dyn View>> = vec![
            Box::new(SgfxCanvas::from_state(
                e.canvas,
                size.width,
                size.height,
                e.frame.clone(),
            )),
            Box::new(SelectionOverlay {
                selection: animation::PaintState(e.selection.clone()),
                start: e.start.get(),
                span: e.span.get(),
                size,
            }),
            Box::new(FadeEnvelope {
                preview: animation::PaintState(e.fade_preview.clone()),
                clip: clip.clone(),
                start: e.start.get(),
                span: e.span.get(),
                size,
            }),
            Box::new(animation::Playhead::new(
                self.playhead.clone(),
                (clip.start as f64 + e.start.get()) / rate as f64,
                e.span.get() / rate as f64,
                size,
            )),
        ];
        for lane in editor_lanes(clip.source_channels, size.height) {
            layers.push(Box::new(
                caption(if clip.source_channels == 1 {
                    "M"
                } else if lane.channel == 0 {
                    "L"
                } else {
                    "R"
                })
                .font_size(9.)
                .padding_insets(EdgeInsets::new(5., lane.top + 3., 0., 0.)),
            ));
        }
        let waveform = ZStack::new(Children(layers))
            .alignment(Alignment::TopLeading)
            .frame(size.width, size.height)
            .clip()
            .on_event(move |event_| event.editor_event(event_))
            .focusable(e.focused.clone())
            .on_key(move |event| key.editor_key(event));
        let readout = if e.sample_units.get() {
            if range.is_empty() {
                format!("{} smp", selection.head)
            } else {
                format!("{} smp selected", range.len())
            }
        } else if range.is_empty() {
            format!("{:.3} s", selection.head as f64 / rate as f64)
        } else {
            format!("{:.3} s selected", range.len() as f64 / rate as f64)
        };
        let scroll_width = (size.width - 178.).max(80.);
        let scrolling = self.clone();
        let footer = row! {
            caption(readout).font_size(10.).frame_width(104.),
            self.header_icon(Icon::ChevronLeft, "Scroll the editor left", false, |s| s.editor_scroll(-0.25)),
            EditorScrollbar { daw: self.clone(), frames: clip.frames, width: scroll_width }.on_event(move |event| scrolling.editor_scrollbar_event(event, scroll_width)),
            self.header_icon(Icon::ChevronRight, "Scroll the editor right", false, |s| s.editor_scroll(0.25))
        }.spacing(4.).padding_insets(EdgeInsets::new(3., 0., 3., 0.)).frame(size.width, FOOTER).background(PANEL);
        AnyView::new(row! {
            sidebar,
            Rectangle::new().fill(LINE).frame(1., e.size.get().height),
            vstack! {toolbar, marks, waveform, footer}.spacing(0.).alignment(Alignment::TopLeading).frame(size.width, e.size.get().height)
        }.spacing(0.).frame(f32::INFINITY, f32::INFINITY).background(BG).on_geometry_change(|g| g.size(), move |size| {
            geometry.editor_resized(size);
        }))
    }
    fn editor_resized(&self, size: Size) {
        let old = self.region_editor.size.get();
        if (old.width - size.width).abs() > 1. || (old.height - size.height).abs() > 1. {
            self.region_editor.size.set(size);
            if let Some((track, _, clip, _, _)) = self.editor_selected() {
                self.refresh_editor_waveform(&clip, track);
            }
        }
    }
    fn editor_button(
        &self,
        text: &str,
        icon: Option<Icon>,
        help: &'static str,
        enabled: bool,
        action: impl Fn(Self) + 'static,
    ) -> AnyView {
        let clicked = self.clone();
        let hovered = self.clone();
        let color = if enabled { TEXT } else { MUTED };
        let mut button = ui::compact_button(text).text_color(color);
        if let Some(icon) = icon {
            button = button
                .icon(icon)
                .icon_size(IconSize::Small)
                .icon_color(color);
        }
        AnyView::new(
            button
                .on_click(move || {
                    if enabled {
                        action(clicked.clone());
                    }
                })
                .on_hover(move || hovered.status.set(help.into()))
                .frame_height(CONTROL_HEIGHT),
        )
    }
    fn editor_action(
        &self,
        icon: Icon,
        help: &'static str,
        enabled: bool,
        action: impl Fn(Self) + 'static,
    ) -> AnyView {
        let clicked = self.clone();
        let hovered = self.clone();
        AnyView::new(
            Button::icon_only(icon)
                .icon_size(IconSize::Small)
                .padding(4.)
                .background_color(if enabled { RAISED } else { PANEL })
                .icon_color(if enabled { TEXT } else { MUTED })
                .border_color(if enabled { LINE } else { PANEL })
                .on_click(move || {
                    if enabled {
                        action(clicked.clone());
                    }
                })
                .on_hover(move || hovered.status.set(help.into()))
                .frame(CONTROL_HEIGHT, CONTROL_HEIGHT),
        )
    }
    fn editor_scrollbar_event(&self, event: &Event, width: f32) -> bool {
        let Some((_, _, clip, _, _)) = self.editor_selected() else {
            return false;
        };
        let e = &self.region_editor;
        let thumb = (e.span.get() / clip.frames.max(1) as f64 * width as f64)
            .clamp(18., width as f64) as f32;
        let travel = (width - thumb).max(1.);
        let max_start = (clip.frames as f64 - e.span.get()).max(0.);
        let left = if max_start > 0. {
            e.start.get() as f32 / max_start as f32 * travel
        } else {
            0.
        };
        match event {
            Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                ..
            }) => {
                if *x as f32 >= left && (*x as f32) <= left + thumb {
                    e.scroll_drag.set(Some((*x as f32, e.start.get())));
                } else {
                    let start =
                        ((*x as f32 - thumb * 0.5) / travel).clamp(0., 1.) as f64 * max_start;
                    e.set_view(start, e.span.get(), clip.frames);
                    e.scroll_drag.set(Some((*x as f32, e.start.get())));
                    self.editor_redraw();
                }
                true
            }
            Event::Mouse(
                MouseEvent::Moved { x, .. }
                | MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x,
                    ..
                },
            ) if e.scroll_drag.get().is_some() => {
                let (pointer, initial) = e.scroll_drag.get().unwrap();
                e.set_view(
                    initial + (*x as f32 - pointer) as f64 / travel as f64 * max_start,
                    e.span.get(),
                    clip.frames,
                );
                if matches!(event, Event::Mouse(MouseEvent::ButtonReleased { .. })) {
                    e.scroll_drag.set(None);
                }
                self.editor_redraw();
                true
            }
            Event::Mouse(MouseEvent::ButtonCancelled { .. }) => {
                if let Some((_, initial)) = e.scroll_drag.take() {
                    e.set_view(initial, e.span.get(), clip.frames);
                    self.editor_redraw();
                    return true;
                }
                false
            }
            _ => false,
        }
    }
}

/// Geometry must invalidate this local layout boundary, including a measurement
/// delivered during the root's first layout. A Cell plus a whole-window
/// revision can leave retained children at their initial preferred size.
#[derive(Clone)]
struct EditorWorkspace(Daw);
impl View for EditorWorkspace {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |s| EditorWorkspaceRender(s.0.region_editor.size.get()),
            |r, s| {
                r.0 = s.0.region_editor.size.get();
                UpdateResult::Updated
            },
            |s| vec![Box::new(s.0.region_editor_content())],
        ))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.0.region_editor.size]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
struct EditorWorkspaceRender(Size);
impl ElementRenderObject for EditorWorkspaceRender {
    fn update_needs_layout(&self) -> bool {
        true
    }
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        let available = Size::new(
            if constraints.max_width.is_finite() {
                constraints.max_width
            } else {
                self.0.width
            },
            if constraints.max_height.is_finite() {
                constraints.max_height
            } else {
                self.0.height
            },
        );
        self.0 = available.constrain(
            Size::new(constraints.min_width, constraints.min_height),
            Size::new(constraints.max_width, constraints.max_height),
        );
        self.0
    }
    fn size(&self) -> Size {
        self.0
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn render(&mut self) {}
}

fn editor_lanes(channels: u16, height: f32) -> Vec<wave::Lane> {
    let count = if channels == 1 { 1 } else { 2 };
    (0..count)
        .map(|channel| {
            let top = channel as f32 * height / count as f32;
            let bottom = (channel + 1) as f32 * height / count as f32;
            wave::Lane {
                channel,
                top,
                bottom,
                center: (top + bottom) * 0.5,
                amplitude: ((bottom - top) * 0.5 - 12.).max(1.),
            }
        })
        .collect()
}

#[derive(Clone)]
struct SelectionOverlay {
    selection: animation::PaintState<Selection>,
    start: f64,
    span: f64,
    size: Size,
}
impl View for SelectionOverlay {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |v| SelectionRender(v.clone()),
            |r, v| {
                r.0 = v.clone();
                UpdateResult::Updated
            },
            |_| vec![],
        ))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.selection]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
pub(super) struct SelectionRender(SelectionOverlay);
impl ElementRenderObject for SelectionRender {
    fn update_needs_layout(&self) -> bool {
        true
    }
    fn layout(&mut self, c: LayoutConstraints) -> Size {
        self.0.size = self.0.size.constrain(
            Size::new(c.min_width, c.min_height),
            Size::new(c.max_width, c.max_height),
        );
        self.0.size
    }
    fn size(&self) -> Size {
        self.0.size
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn render(&mut self) {}
    fn paint(&self, ctx: &mut PaintContext<'_>, origin: Point) -> bool {
        let range = self.0.selection.0.get().range();
        let size = self.0.size;
        let map = |frame: usize| ((frame as f64 - self.0.start) / self.0.span) as f32 * size.width;
        let left = map(range.start).clamp(0., size.width);
        let right = map(range.end).clamp(0., size.width);
        if right > left {
            let mut color = ACCENT;
            color.a = 0.16;
            ctx.fill_rect(
                Rect::from_xywh(origin.x + left, origin.y, right - left, size.height),
                color,
            );
        }
        for frame in [range.start, range.end] {
            let x = map(frame);
            if x >= 0. && x < size.width {
                ctx.fill_rect(
                    Rect::from_xywh(origin.x + x, origin.y, 1., size.height),
                    ACCENT,
                );
            }
        }
        true
    }
}

#[derive(Clone)]
struct EditorRuler {
    start: f64,
    span: f64,
    rate: u32,
    width: f32,
    position: animation::PaintState<f64>,
    region_start: f64,
}
impl View for EditorRuler {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |v| EditorRulerRender(v.clone()),
            |r, v| {
                r.0 = v.clone();
                UpdateResult::Updated
            },
            |_| vec![],
        ))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.position]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
struct EditorRulerRender(EditorRuler);
impl ElementRenderObject for EditorRulerRender {
    fn update_needs_layout(&self) -> bool {
        true
    }
    fn layout(&mut self, c: LayoutConstraints) -> Size {
        self.0.width = self.0.width.clamp(c.min_width, c.max_width);
        Size::new(self.0.width, RULER)
    }
    fn size(&self) -> Size {
        Size::new(self.0.width, RULER)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn render(&mut self) {}
    fn paint(&self, ctx: &mut PaintContext<'_>, origin: Point) -> bool {
        let v = &self.0;
        ctx.fill_rect(Rect::from_xywh(origin.x, origin.y, v.width, RULER), PANEL);
        let sample_mode = v.span <= 128.;
        let units = if sample_mode { 1. } else { v.rate as f64 };
        let span = v.span / units;
        let step = if sample_mode {
            wave::tick_step(span).max(1.)
        } else {
            wave::tick_step(span)
        };
        let start = v.start / units;
        let first = (start / step).floor() as i64;
        for index in first..=first + 12 {
            let tick = index as f64 * step;
            let x = ((tick - start) / span) as f32 * v.width;
            if x < 0. || x >= v.width {
                continue;
            }
            ctx.fill_rect(
                Rect::from_xywh(origin.x + x, origin.y + RULER - 5., 1., 5.),
                MUTED,
            );
            let text = if sample_mode {
                format!("{tick:.0}")
            } else if step < 0.001 {
                format!("{tick:.5}s")
            } else if step < 1. {
                format!("{tick:.3}s")
            } else {
                format!("{tick:.1}s")
            };
            if x + 58. < v.width {
                ctx.draw_text(
                    Point::new(origin.x + x + 3., origin.y + 3.),
                    &text,
                    MUTED,
                    9.,
                );
            }
        }
        let cursor = ((v.position.0.get() * v.rate as f64 - v.region_start - v.start) / v.span)
            as f32
            * v.width;
        if cursor >= 0. && cursor <= v.width {
            ctx.fill_rect(
                Rect::from_xywh(origin.x + cursor - 3., origin.y + RULER - 9., 6., 4.),
                TEXT,
            );
            ctx.fill_rect(
                Rect::from_xywh(origin.x + cursor - 0.75, origin.y + RULER - 5., 1.5, 5.),
                TEXT,
            );
        }
        true
    }
}

// A finite-width line in the retained SGFX waveform mesh.
fn waveform_line(
    vertices: &mut Vec<SgfxCanvasVertex>,
    a: (f32, f32),
    b: (f32, f32),
    thickness: f32,
    color: Color,
    size: Size,
) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx.hypot(dy).max(0.001);
    let (nx, ny) = (
        -dy / length * thickness * 0.5,
        dx / length * thickness * 0.5,
    );
    let points = [
        (a.0 + nx, a.1 + ny),
        (b.0 + nx, b.1 + ny),
        (b.0 - nx, b.1 - ny),
        (a.0 - nx, a.1 - ny),
    ];
    for index in [0, 1, 2, 0, 2, 3] {
        let (x, y) = points[index];
        vertices.push(SgfxCanvasVertex::new(
            [x / size.width * 2. - 1., 1. - y / size.height * 2., 0., 1.],
            [color.r, color.g, color.b, color.a],
        ));
    }
}

#[derive(Clone)]
struct FadeEnvelope {
    preview: animation::PaintState<Option<(usize, usize)>>,
    clip: Clip,
    start: f64,
    span: f64,
    size: Size,
}
impl View for FadeEnvelope {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |v| FadeEnvelopeRender(v.clone()),
            |r, v| {
                r.0 = v.clone();
                UpdateResult::Updated
            },
            |_| vec![],
        ))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.preview]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
struct FadeEnvelopeRender(FadeEnvelope);
impl ElementRenderObject for FadeEnvelopeRender {
    fn update_needs_layout(&self) -> bool {
        true
    }
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.0.size = self.0.size.constrain(
            Size::new(constraints.min_width, constraints.min_height),
            Size::new(constraints.max_width, constraints.max_height),
        );
        self.0.size
    }
    fn size(&self) -> Size {
        self.0.size
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn render(&mut self) {}
    fn paint(&self, context: &mut PaintContext<'_>, origin: Point) -> bool {
        let v = &self.0;
        let mut clip = v.clip.clone();
        if let Some((fade_in, fade_out)) = v.preview.0.get() {
            let _ = clip.set_fades(fade_in, fade_out);
        }
        let map = |at: usize| ((at as f64 - v.start) / v.span) as f32 * v.size.width;
        let mut color = GOLD;
        color.a = 0.75;
        if clip.edit.fade_in > 0 || clip.edit.fade_out > 0 {
            for lane in editor_lanes(clip.source_channels, v.size.height) {
                let height = (lane.bottom - lane.top - 24.).max(1.);
                let mut previous = None;
                for x in (0..v.size.width.ceil() as usize).step_by(3) {
                    let at = v.start + x as f64 / v.size.width as f64 * v.span;
                    let gain = (clip.amplitude_at(at) / clip.edit.gain_linear()).clamp(0., 1.);
                    let y = origin.y + lane.top + 12. + (1. - gain) * height;
                    if let Some((px, py)) = previous {
                        context.draw_line(
                            Point::new(px, py),
                            Point::new(origin.x + x as f32, y),
                            1.,
                            color,
                        );
                    }
                    previous = Some((origin.x + x as f32, y));
                }
            }
        }
        for at in [
            clip.edit.fade_in.min(clip.frames),
            clip.frames - clip.edit.fade_out.min(clip.frames),
        ] {
            let x = map(at);
            if x >= 0. && x <= v.size.width {
                let x = x.clamp(5., v.size.width - 5.);
                context.fill_rect(
                    Rect::from_xywh(origin.x + x - 4., origin.y + 3., 8., 8.),
                    color,
                );
            }
        }
        true
    }
}

#[derive(Clone)]
struct EditorScrollbar {
    daw: Daw,
    frames: usize,
    width: f32,
}
impl View for EditorScrollbar {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |v| EditorScrollbarRender(v.clone()),
            |r, v| {
                r.0 = v.clone();
                UpdateResult::Updated
            },
            |_| vec![],
        ))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
struct EditorScrollbarRender(EditorScrollbar);
impl ElementRenderObject for EditorScrollbarRender {
    fn update_needs_layout(&self) -> bool {
        true
    }
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.0.width = self
            .0
            .width
            .clamp(constraints.min_width, constraints.max_width);
        Size::new(self.0.width, CONTROL_HEIGHT)
    }
    fn size(&self) -> Size {
        Size::new(self.0.width, CONTROL_HEIGHT)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn render(&mut self) {}
    fn paint(&self, context: &mut PaintContext<'_>, origin: Point) -> bool {
        let e = &self.0.daw.region_editor;
        let width = self.0.width;
        let thumb = (e.span.get() / self.0.frames.max(1) as f64 * width as f64)
            .clamp(18., width as f64) as f32;
        let remaining = (self.0.frames as f64 - e.span.get()).max(0.);
        let left = if remaining > 0. {
            e.start.get() as f32 / remaining as f32 * (width - thumb)
        } else {
            0.
        };
        context.fill_rect(Rect::from_xywh(origin.x, origin.y + 11., width, 6.), LINE);
        context.fill_rect(
            Rect::from_xywh(origin.x + left, origin.y + 9., thumb, 10.),
            MUTED,
        );
        true
    }
}

#[cfg(test)]
#[path = "region_editor_tests.rs"]
mod tests;
