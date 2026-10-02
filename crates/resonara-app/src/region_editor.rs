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

#[derive(Clone, Copy, PartialEq)]
struct WaveformKey {
    identity: Option<Identity>,
    edit: resonara_core::ClipEdit,
    channels: u16,
    size: Size,
    start: f64,
    span: f64,
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
    drag: Cell<Option<Selection>>,
    select_all: Cell<bool>,
    start: Cell<f64>,
    span: Cell<f64>,
    size: State<Size>,
    gain: State<String>,
    fade_in: State<String>,
    fade_out: State<String>,
    pub(super) normalize: State<String>,
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
            drag: Cell::new(None),
            select_all: Cell::new(false),
            start: Cell::new(0.),
            span: Cell::new(1.),
            size: local_state(Size::new(800., 300.)),
            gain: local_state("0.0".into()),
            fade_in: local_state("0.0".into()),
            fade_out: local_state("0.0".into()),
            normalize: local_state("-1.0".into()),
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
    fn frame_at(&self, x: f32, frames: usize) -> usize {
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
    fn editor_selected(&self) -> Option<(usize, usize, Clip, u32, String)> {
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
            editor.identity.set(None);
            editor.observed.set(None);
            editor.drag.set(None);
            if editor.selection.get() != Selection::default() {
                editor.selection.set(Selection::default());
            }
            editor.cache.borrow_mut().take();
            return;
        };
        let identity = Identity::new(track, index, &clip);
        if editor.identity.get() != Some(identity) {
            editor.identity.set(Some(identity));
            editor.observed.set(None);
            editor.selection.set(Selection::default());
            editor.drag.set(None);
            editor.set_view(0., clip.frames.max(1) as f64, clip.frames);
        }
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
        let size = e.waveform_size();
        let key = WaveformKey {
            identity: e.identity.get(),
            edit: clip.edit,
            channels: clip.source_channels,
            size,
            start: e.start.get(),
            span: e.span.get(),
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
            let columns = size.width.ceil() as usize;
            for column in 0..columns {
                let from = (e.start.get() + column as f64 / size.width as f64 * e.span.get())
                    .floor() as usize;
                let to = (e.start.get() + (column + 1) as f64 / size.width as f64 * e.span.get())
                    .ceil() as usize;
                let envelope =
                    peaks.clip_envelope(clip, from.min(clip.frames), to.min(clip.frames));
                let top = lane.center - envelope.max[lane.channel].clamp(-1., 1.) * lane.amplitude;
                let bottom =
                    lane.center - envelope.min[lane.channel].clamp(-1., 1.) * lane.amplitude;
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
    fn editor_zoom_selection(&self) {
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
        let Some((_, _, clip, rate, _)) = self.editor_selected() else {
            return false;
        };
        let e = &self.region_editor;
        match event {
            Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                click_count,
                ..
            }) => {
                e.drag.set(Some(e.selection.get()));
                e.select_all.set(*click_count >= 2);
                let frame = e.frame_at(*x as f32, clip.frames);
                e.selection.set(if *click_count >= 2 {
                    Selection {
                        anchor: 0,
                        head: clip.frames,
                    }
                } else {
                    Selection {
                        anchor: frame,
                        head: frame,
                    }
                });
                true
            }
            Event::Mouse(MouseEvent::Moved { x, .. }) if e.drag.get().is_some() => {
                if e.select_all.get() {
                    return true;
                }
                let mut selection = e.selection.get();
                selection.head = e.frame_at(*x as f32, clip.frames);
                e.selection.set(selection);
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
                    selection.head = e.frame_at(*x as f32, clip.frames);
                    e.selection.set(selection);
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
            }) if e.drag.get().is_some() => {
                e.selection.set(e.drag.take().unwrap());
                self.changed();
                true
            }
            Event::Mouse(MouseEvent::Wheel {
                delta_x, delta_y, ..
            }) => {
                let delta = if *delta_x != 0 { *delta_x } else { *delta_y };
                self.editor_scroll(-delta as f64 / 480.);
                true
            }
            _ => false,
        }
    }

    fn editor_key(&self, event: KeyEvent) -> bool {
        let KeyEvent::Pressed { keycode, modifiers } = event else {
            return false;
        };
        let Some((_, _, clip, _, _)) = self.editor_selected() else {
            return false;
        };
        let e = &self.region_editor;
        match keycode {
            KeyCode::Escape => {
                e.selection.set(e.drag.take().unwrap_or_default());
                self.changed();
            }
            KeyCode::Char('a' | 'A') if modifiers.control || modifiers.super_key => {
                e.selection.set(Selection {
                    anchor: 0,
                    head: clip.frames,
                });
                self.changed();
            }
            KeyCode::Left | KeyCode::Right if !modifiers.control && !modifiers.super_key => {
                let mut selection = e.selection.get();
                selection.head = if keycode == KeyCode::Left {
                    selection.head.saturating_sub(1)
                } else {
                    selection.head.saturating_add(1).min(clip.frames)
                };
                if !modifiers.shift {
                    selection.anchor = selection.head;
                }
                e.selection.set(selection);
                if (selection.head as f64) < e.start.get()
                    || (selection.head as f64) > e.start.get() + e.span.get()
                {
                    e.set_view(
                        selection.head as f64 - e.span.get() * 0.5,
                        e.span.get(),
                        clip.frames,
                    );
                    self.editor_redraw();
                } else {
                    self.changed();
                }
            }
            _ => return false,
        }
        true
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
        if clip.edit.has_inherited_fades() {
            self.status
                .set("Reset inherited fades before editing new region edges".into());
            return;
        }
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
    fn editor_reset_fades(&self) {
        let Some((track, index, _, _, _)) = self.editor_selected() else {
            return;
        };
        self.edit("Reset inherited region fades", |m| {
            m.project.tracks[track].clips[index].set_fades(0, 0)
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
                    caption("Double-click a region in the arrangement to open the editor.")
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
        let transpose = self.clone();
        let event = self.clone();
        let key = self.clone();
        let geometry = self.clone();
        let fades = if clip.edit.has_inherited_fades() {
            AnyView::new(vstack! {
                caption("Fades follow original edges").font_size(10.),
                self.header_button("Reset fades", "Remove inherited fades to set new fades at this region's edges", |s| s.editor_reset_fades()).frame(SIDEBAR - 24., CONTROL_HEIGHT)
            }.spacing(7.).alignment(Alignment::TopLeading))
        } else {
            AnyView::new(vstack! {
                row! {caption("Fade in · ms").frame_width(75.), ui::compact_field(e.fade_in.clone()).on_submit(move || fade_in.editor_fades()).blur_on_submit(true).frame(65., CONTROL_HEIGHT).input_guard()}.spacing(2.),
                row! {caption("Fade out · ms").frame_width(75.), ui::compact_field(e.fade_out.clone()).on_submit(move || fade_out.editor_fades()).blur_on_submit(true).frame(65., CONTROL_HEIGHT).input_guard()}.spacing(2.)
            }.spacing(7.).alignment(Alignment::TopLeading))
        };
        let sidebar = ScrollView::new(column! {
            caption("REGION").font_size(9.),
            ui::name_label(&name, 20, 12., self.status.clone()).frame_width(SIDEBAR - 24.),
            caption(format!("Region {} · {}", index + 1, if clip.source_channels == 1 { "Mono" } else { "Stereo" })).font_size(10.),
            Rectangle::new().fill(LINE).frame(SIDEBAR - 24., 1.),
            row! {caption("Gain · dB").frame_width(75.), ui::compact_field(e.gain.clone()).on_submit(move || gain.editor_gain()).blur_on_submit(true).frame(65., CONTROL_HEIGHT).input_guard()}.spacing(2.),
            fades,
            self.header_button(if clip.edit.reversed { "Reverse · On" } else { "Reverse" }, "Reverse this region without changing its source file", |s| s.editor_reverse()).frame(SIDEBAR - 24., CONTROL_HEIGHT),
            row! {self.header_button("Normalize", "Set the region peak to the target dBFS", |s| s.editor_normalize()).frame(75., CONTROL_HEIGHT), ui::compact_field(e.normalize.clone()).on_submit(move || normalize.editor_normalize()).blur_on_submit(true).frame(65., CONTROL_HEIGHT).input_guard()}.spacing(2.),
            caption("Target · dBFS").font_size(9.),
            Rectangle::new().fill(LINE).frame(SIDEBAR - 24., 1.),
            row! {caption("Pitch · st").frame_width(75.), ui::compact_field(e.transpose.clone()).on_submit(move || transpose.editor_transpose()).blur_on_submit(true).frame(65., CONTROL_HEIGHT).input_guard()}.spacing(2.),
            self.header_button(if self.region_processing_active() { "Cancel processing" } else { "Transpose" }, "Transpose this region while preserving its duration", |s| { if s.region_processing_active() { s.cancel_region_processing(); } else { s.editor_transpose(); } }).frame(SIDEBAR - 24., CONTROL_HEIGHT),
            caption(format!("{} Hz · {:.3} s", rate, clip.frames as f64 / rate as f64)).font_size(10.)
        }.spacing(7.).alignment(Alignment::TopLeading).padding(12.)).frame(SIDEBAR, e.size.get().height).background(PANEL);
        let toolbar = row! {
            self.editor_button("Trim", Some(Icon::ArrowsMinimize), "Keep only the selected range", !range.is_empty(), |s| s.editor_trim()).frame(62., CONTROL_HEIGHT),
            self.editor_button("Split", Some(Icon::Cut), "Split at the cursor or both selection boundaries", selection.head > 0 && range.start < clip.frames, |s| s.editor_split()).frame(62., CONTROL_HEIGHT),
            Spacer::new(),
            self.header_icon(Icon::ZoomOut, "Zoom out the audio editor", false, |s| s.editor_zoom(2.)),
            self.header_icon(Icon::ZoomIn, "Zoom in to individual samples", false, |s| s.editor_zoom(0.5)),
            self.editor_button("Fit", None, "Fit the full selected region", true, |s| s.editor_fit()).frame(72., CONTROL_HEIGHT),
            self.editor_button("Selection", None, "Zoom to the selected range", !range.is_empty(), |s| s.editor_zoom_selection()).frame(72., CONTROL_HEIGHT)
        }.spacing(4.).padding_insets(EdgeInsets::new(6., 4., 6., 4.)).frame(size.width, TOOLBAR).background(PANEL);
        let marks = EditorRuler {
            start: e.start.get(),
            span: e.span.get(),
            rate,
            width: size.width,
        }
        .frame(size.width, RULER)
        .alignment(Alignment::TopLeading);
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
        let readout = if range.is_empty() {
            format!(
                "Cursor {} · {:.6} s",
                selection.head,
                selection.head as f64 / rate as f64
            )
        } else {
            format!(
                "{}–{} · {} samples · {:.4} s",
                range.start,
                range.end,
                range.len(),
                range.len() as f64 / rate as f64
            )
        };
        let footer = row! {
            self.header_icon(Icon::ChevronLeft, "Scroll the editor left", false, |s| s.editor_scroll(-0.5)),
            caption(readout).font_size(10.), Spacer::new(),
            self.header_icon(Icon::ChevronRight, "Scroll the editor right", false, |s| s.editor_scroll(0.5))
        }
        .spacing(4.)
        .padding_insets(EdgeInsets::new(4., 0., 4., 0.))
        .frame(size.width, FOOTER)
        .background(PANEL);
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
struct SelectionRender(SelectionOverlay);
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
        true
    }
}

#[cfg(test)]
#[path = "region_editor_tests.rs"]
mod tests;
