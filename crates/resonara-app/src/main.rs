mod animation;
mod fader;
mod knob;
mod meter;
mod profiling;
#[cfg(test)]
mod tests;
mod timeline;
mod ui;
mod wave;

use resonara_core::{Clip, Project, Result};
use resonara_platform::Audio;
use scarlet_ui::{
    WindowContext,
    event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent},
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
use ui::*;
macro_rules! row { ($($v:expr),* $(,)?) => { HStack::new(Children(vec![$(Box::new($v) as Box<dyn View>),*])) }; }

#[derive(Clone)]
struct Snapshot {
    project: Project,
    selected: usize,
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
    ConfirmOpen,
    ConfirmClose,
    Help,
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
    selected: usize,
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
    io: Option<mpsc::Receiver<IoResult>>,
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
    inspector_fraction: State<f32>,
    mixer_fraction: State<f32>,
    mixer_visible: State<bool>,
    snap: State<bool>,
    tool: State<usize>,
    view_start: State<f64>,
    view_span: State<f64>,
    focus: State<bool>,
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
        let s = Self {
            model: Rc::new(RefCell::new(Model {
                project,
                audio: None,
                selected: 0,
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
                io: None,
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
            cursor: state(6, "0.000".into()),
            range_end: state(7, "1.000".into()),
            track_name: state(8, String::new()),
            master: state(9, 0.8),
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
            inspector_fraction: state(26, 0.185),
            mixer_fraction: state(27, 0.59),
            mixer_visible: state(18, true),
            snap: state(19, true),
            tool: state(20, 0),
            view_start: state(21, 0.),
            view_span: state(22, span),
            focus: state(23, false),
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
            clip: m.clip,
            version: m.version,
            path: m.current_path.clone(),
        }
    }
    fn restore(m: &mut Model, s: Snapshot) {
        m.project = s.project;
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
    fn cycle_time_format(&self) {
        self.time_format.set(self.time_format.get().next());
        self.refresh(true);
    }
    fn refresh(&self, waveforms: bool) {
        let mut m = self.model.borrow_mut();
        self.tempo_input.set(format!("{}", m.project.tempo));
        self.clock.set(self.time_format.get().position(
            self.playhead.get(),
            m.project.tempo,
            m.project.sample_rate,
        ));
        m.selected = m.selected.min(m.project.tracks.len().saturating_sub(1));
        if let Some(t) = m.project.tracks.get(m.selected) {
            self.track_name.set(t.name.clone());
        } else {
            self.track_name.set(String::new());
        }
        self.master.set(m.project.master);
        let len = m.project.tracks.len();
        while m.channels.len() < len {
            let i = m.channels.len();
            let id = 1000 + i as u64 * 10;
            m.channels.push(Channel {
                gain: state(id, 1.),
                gain_normalized: state(id + 9, fader::gain_to_fraction(1.)),
                pan: state(id + 1, 0.),
                meter: state(id + 2, "Peak −∞ dBFS".into()),
                peak: state(id + 6, meter::StereoMeter::default()),
                focused: state(id + 7, false),
                pan_focused: state(id + 8, false),
                dragging_gain: state(id + 3, false),
                dragging_pan: state(id + 4, false),
                canvas: SgfxCanvasHandle::new(),
                playhead_mesh: SgfxMeshHandle::new(),
                frame: state(id + 5, Arc::new(SgfxCanvasFrame::new(0, BG))),
                mesh: SgfxMesh::new(vec![]),
            });
        }
        m.channels.truncate(len);
        for i in 0..len {
            m.channels[i].gain.set(m.project.tracks[i].gain);
            m.channels[i]
                .gain_normalized
                .set(fader::gain_to_fraction(m.project.tracks[i].gain));
            m.channels[i].pan.set(m.project.tracks[i].pan);
        }
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
                    self.time_format
                        .get()
                        .step(span, project.tempo, project.sample_rate),
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
        let clock =
            self.time_format
                .get()
                .position(position, m.project.tempo, m.project.sample_rate);
        if self.clock.get() != clock {
            self.clock.set(clock);
        }
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
            let changed = before.project.master != m.project.master
                || before
                    .project
                    .tracks
                    .iter()
                    .zip(&m.project.tracks)
                    .any(|(a, b)| {
                        a.gain != b.gain || a.pan != b.pan || a.mute != b.mute || a.solo != b.solo
                    });
            if changed {
                Self::history(&mut m, before, "Mixer change");
            }
        }
        drop(m);
        self.changed();
    }
    fn edit(&self, label: &str, f: impl FnOnce(&mut Model) -> Result<()>) {
        if self.model.borrow().io.is_some() {
            return;
        }
        self.finish_mix();
        self.stop_audio(false);
        let mut m = self.model.borrow_mut();
        let before = Self::snapshot(&m);
        match f(&mut m) {
            Ok(()) => {
                Self::history(&mut m, before, label);
                self.status.set(label.into());
            }
            Err(e) => {
                Self::restore(&mut m, before);
                self.status.set(format!("Could not {label}: {e}"));
            }
        }
        drop(m);
        self.refresh(true);
    }
    fn undo(&self, redo: bool) {
        if self.model.borrow().io.is_some() {
            return;
        }
        self.finish_mix();
        self.stop_audio(false);
        let mut m = self.model.borrow_mut();
        let item = if redo { m.redo.pop() } else { m.undo.pop() };
        if let Some(h) = item {
            let current = Self::snapshot(&m);
            let label = h.label;
            let restore_path = h.restore_path;
            let current_path = m.current_path.clone();
            Self::restore(&mut m, h.state);
            if !restore_path {
                m.current_path = current_path;
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
            self.status
                .set(format!("{}: {label}", if redo { "Redid" } else { "Undid" }));
        }
        drop(m);
        self.refresh(true);
    }
    fn choose(&self, index: usize, clip: Option<usize>) {
        self.finish_mix();
        let mut m = self.model.borrow_mut();
        if index < m.project.tracks.len() {
            m.selected = index;
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
        if m.io.is_some() || index >= m.project.tracks.len() {
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
        if m.io.is_some() {
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
        self.finish_mix();
        self.reset_meters();
        let result = self.seconds(&self.cursor.get()).and_then(|start| {
            let mut m = self.model.borrow_mut();
            if m.project.duration() == 0 {
                return Err("Import an audio file first".into());
            }
            let start = if start >= m.project.duration() {
                0
            } else {
                start
            };
            let a = Audio::start(&m.project, start)?;
            self.status.set(format!("Playing · {}", a.device));
            m.audio = Some(a);
            Ok(())
        });
        if let Err(e) = result {
            self.status.set(format!("Playback unavailable: {e}"));
        }
        self.changed();
    }
    fn stop_audio(&self, message: bool) {
        let mut m = self.model.borrow_mut();
        if let Some(a) = m.audio.take() {
            let pos =
                a.controls.position.load(Ordering::Relaxed) as f64 / m.project.sample_rate as f64;
            self.playhead.set(pos);
            self.cursor.set(format!("{pos:.3}"));
            self.clock.set(self.time_format.get().position(
                pos,
                m.project.tempo,
                m.project.sample_rate,
            ));
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
        for channel in &self.model.borrow().channels {
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
        self.stop_audio(false);
        let seconds = seconds.max(0.);
        self.playhead.set(seconds);
        self.cursor.set(format!("{seconds:.3}"));
        self.animate_playhead(seconds);
        self.update_frames();
        if was_playing {
            self.play();
        }
    }
    fn split(&self) {
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
                        return Err("Select a region, or use Delete track in the inspector".into());
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
        if self.model.borrow().io.is_some() {
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
                self.stop_audio(false);
                self.focus.set(true);
                let at = seconds(*x as f32).max(0.);
                let mut m = self.model.borrow_mut();
                if index >= m.project.tracks.len() {
                    return false;
                }
                let rate = m.project.sample_rate;
                let frame = (at * rate as f64) as u64;
                let hit = m.project.tracks[index]
                    .clips
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, c)| frame >= c.start && frame <= c.start + c.frames as u64)
                    .map(|(i, _)| i);
                m.selected = index;
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
                self.seek(at);
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
                let grid = if self.snap.get() {
                    0.1
                } else {
                    1. / rate as f64
                };
                let quantize = |s: f64| (s / grid).round() * grid;
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
                        Self::history(
                            &mut m,
                            d.before,
                            if d.mode == DragMode::Move {
                                "Move region"
                            } else {
                                "Trim region"
                            },
                        );
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
    fn open_dialog(&self, action: FileAction) {
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
        self.filename.set(name);
        self.dialog.set(Dialog::File(action));
        self.read_directory();
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
                                FileAction::Import => name.to_lowercase().ends_with(".wav"),
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
        self.finish_mix();
        self.stop_audio(false);
        let mut m = self.model.borrow_mut();
        if m.io.is_some() {
            return;
        }
        let project = m.project.clone();
        let version = m.version;
        let (tx, rx) = mpsc::channel();
        m.io = Some(rx);
        self.status.set(
            match action {
                FileAction::Import => "Importing WAV…",
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
                    p.import_wav(&path)?;
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
                            "Import WAV"
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
                        FileAction::Import => "WAV imported",
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
        self.finish_mix();
        if self.dirty() {
            self.dialog.set(Dialog::ConfirmOpen);
        } else {
            self.open_dialog(FileAction::Open);
        }
    }
    fn handle_key(&self, e: KeyEvent) -> bool {
        if self.model.borrow().io.is_some() {
            return true;
        }
        let KeyEvent::Pressed { keycode, modifiers } = e else {
            return false;
        };
        if self.dialog.get() != Dialog::None {
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
            KeyCode::Escape => self.cancel_drag(),
            KeyCode::Delete | KeyCode::Backspace => self.delete(false),
            KeyCode::Up => self.select(-1),
            KeyCode::Down => self.select(1),
            KeyCode::Char('s' | 'S') => self.split(),
            KeyCode::Char('m' | 'M') => {
                let i = self.model.borrow().selected;
                self.mix(i, None, None, Some(false));
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
        let source_kind = if t.clips.iter().all(|c| c.source_channels == 1) {
            "MONO"
        } else if t.clips.iter().all(|c| c.source_channels == 2) {
            "STEREO"
        } else {
            "MIXED"
        };
        AnyView::new(row!{
            row!{Rectangle::new().fill(ui::color(index)).frame(3.,ROW),vstack!{
                row!{caption(format!("{:02}",index+1)),ui::name_label(&t.name,22,12.,self.status.clone()).frame_width(135.)}.spacing(6.),
                row!{ui::button("M").background_color(if t.mute{GOLD}else{RAISED}).text_color(if t.mute{BG}else{TEXT}).on_click(move||mute.mix(index,None,None,Some(false))).frame(27.,24.),ui::button("S").background_color(if t.solo{GOLD}else{RAISED}).text_color(if t.solo{BG}else{TEXT}).on_click(move||solo.mix(index,None,None,Some(true))).frame(27.,24.),Slider::new(c.gain_normalized.clone()).min(0.).max(1.).dragging_state(c.dragging_gain.clone()).on_change(move|v|gain.mix_normalized(index,v)).frame_width(98.)}.spacing(5.),
                row!{caption(ui::db(t.gain)),Spacer::new(),caption(source_kind)}
            }.spacing(4.).padding(8.).frame(HEADER-3.,ROW)}.spacing(0.).background(if selected{RAISED}else{PANEL}).on_click(move||select.choose(index,None)),
            ZStack::new(Children(overlays)).alignment(Alignment::TopLeading).frame(width,ROW).clip().on_event(move|e|event.timeline_event(index,e))
        }.spacing(0.).frame(HEADER+width,ROW))
    }
    fn ruler(&self) -> AnyView {
        let width = (self.arrangement_size.get().width - HEADER).max(200.);
        let start = self.view_start.get();
        let span = self.view_span.get();
        let m = self.model.borrow();
        let format = self.time_format.get();
        let step = format.step(span, m.project.tempo, m.project.sample_rate);
        let first = (start / step).floor() as i64;
        let mut labels: Vec<Box<dyn View>> = vec![];
        for i in first..=first + 12 {
            let seconds = i as f64 * step;
            let x = ((seconds - start) / span) as f32 * width;
            if x >= 0. && x < width - 85. {
                labels.push(Box::new(
                    caption(format.tick(seconds, m.project.tempo, m.project.sample_rate))
                        .font_size(10.)
                        .frame(90., 26.)
                        .padding_insets(EdgeInsets::new(x + 5., 0., 0., 0.)),
                ));
            }
        }
        let s = self.clone();
        AnyView::new(row!{row!{caption("TRACKS"),Spacer::new(),caption(format!("{}",self.model.borrow().project.tracks.len()))}.padding(10.).frame(HEADER,30.).background(PANEL),ZStack::new(Children(labels)).alignment(Alignment::TopLeading).frame(width,30.).background(RAISED).on_event(move|e|{if let Event::Mouse(MouseEvent::ButtonPressed{button:MouseButton::Left,x,..})=e{s.seek((start+*x as f64/width as f64*span).max(0.));true}else{false}})}.spacing(0.))
    }
    fn arrangement(&self) -> AnyView {
        let size = self.arrangement_size.get();
        let count = self.model.borrow().project.tracks.len();
        let rows = self.clone();
        let resize = self.clone();
        let content = if count == 0 {
            AnyView::new(vstack!{Text::new("Your arrangement starts here").font_size(20.).color(TEXT),caption("Import a mono or stereo WAV, then arrange it on the timeline."),self.button("Import WAV…","Import an audio file · Ctrl/Cmd+I",|s|s.open_dialog(FileAction::Import))}.spacing(12.).frame(size.width,(size.height-58.).max(150.)))
        } else {
            AnyView::new(
                ScrollView::new(LazyVStack::new(count, ROW, move |i| rows.track_row(i)))
                    .vertical()
                    .content_size(size.width, count as f32 * ROW)
                    .frame(size.width, (size.height - 60.).max(100.)),
            )
        };
        let layout=AnyView::new(vstack!{self.ruler(),content,row!{self.icon(Icon::ChevronLeft,"Scroll timeline left",false,|s|{s.view_start.set((s.view_start.get()-s.view_span.get()*0.5).max(0.));s.refresh(true);}),caption(format!("{:.2}s — {:.2}s",self.view_start.get(),self.view_start.get()+self.view_span.get())),Spacer::new(),caption("Drag region • edge = trim • S = split"),self.icon(Icon::ChevronRight,"Scroll timeline right",false,|s|{s.view_start.set(s.view_start.get()+s.view_span.get()*0.5);s.refresh(true);})}.spacing(6.).padding_insets(EdgeInsets::new(8.,0.,8.,0.)).frame_height(30.).background(PANEL)}.spacing(0.).background(BG).on_geometry_change(|g|g.size(),move|size|{let old=resize.arrangement_size.get();if(old.width-size.width).abs()>1.||(old.height-size.height).abs()>1.{resize.arrangement_size.set(size);if resize.mixer_visible.get(){let full=(resize.size.get().height-44.-66.-36.-28.-4.).max(1.);resize.mixer_fraction.set((size.height/full).clamp(0.1,0.9));}resize.refresh(true);}}));
        let wheel = self.clone();
        AnyView::new(HorizontalWheel(
            layout,
            Rc::new(move |dx| {
                let width = (wheel.arrangement_size.get().width - HEADER).max(200.);
                wheel.view_start.set(
                    (wheel.view_start.get()
                        - dx as f64 * 0.25 / width as f64 * wheel.view_span.get())
                    .max(0.),
                );
                wheel.refresh(true);
            }),
        ))
    }
    fn inspector_panel(&self) -> AnyView {
        let m = self.model.borrow();
        let i = m.selected;
        let panel_resize = self.clone();
        let title = m
            .project
            .tracks
            .get(i)
            .map(|t| t.name.as_str())
            .unwrap_or("No track selected");
        let rename = self.clone();
        let gain = self.clone();
        let pan = self.clone();
        let mut rows:Vec<Box<dyn View>>=vec![Box::new(row!{caption("INSPECTOR"),Spacer::new(),self.icon(Icon::X,"Hide inspector · I",false,|s|s.inspector.set(false))}.frame_height(30.)),Box::new(ui::name_label(title,22,18.,self.status.clone())),Box::new(caption("AUDIO TRACK"))];
        if let Some(t) = m.project.tracks.get(i) {
            let c = &m.channels[i];
            rows.push(Box::new(
                ui::field(self.track_name.clone())
                    .on_submit(move || {
                        let name = rename.track_name.get();
                        rename.edit("Rename track", |m| {
                            if name.trim().is_empty() {
                                return Err("Track name cannot be empty".into());
                            }
                            m.project.tracks[m.selected].name = name.trim().into();
                            Ok(())
                        });
                    })
                    .blur_on_submit(true)
                    .frame_width(190.)
                    .input_guard(),
            ));
            rows.push(Box::new(caption("LEVEL")));
            rows.push(Box::new(row!{Slider::new(c.gain_normalized.clone()).min(0.).max(1.).dragging_state(c.dragging_gain.clone()).on_change(move|v|gain.mix_normalized(i,v)).frame_width(115.),label(ui::db(t.gain)).frame_width(70.)}.spacing(5.)));
            rows.push(Box::new(caption("PAN")));
            rows.push(Box::new(row!{Slider::new(c.pan.clone()).min(-1.).max(1.).dragging_state(c.dragging_pan.clone()).on_change(move|v|pan.mix(i,None,Some(v),None)).frame_width(115.),label(ui::pan(t.pan)).frame_width(70.)}.spacing(5.)));
            rows.push(Box::new(row!{self.button(if t.mute{"Unmute"}else{"Mute"},"Toggle track mute · M",move|s|s.mix(i,None,None,Some(false))),self.button(if t.solo{"Unsolo"}else{"Solo"},"Listen to this track in isolation",move|s|s.mix(i,None,None,Some(true)))}.spacing(6.)));
            rows.push(Box::new(Rectangle::new().fill(LINE).frame(190., 1.)));
            rows.push(Box::new(caption("SELECTED REGION")));
            if let Some(c) = m.clip.and_then(|ci| t.clips.get(ci)) {
                rows.push(Box::new(label(format!(
                    "Start  {:.3}s",
                    c.start as f64 / m.project.sample_rate as f64
                ))));
                rows.push(Box::new(label(format!(
                    "Length  {:.3}s",
                    c.frames as f64 / m.project.sample_rate as f64
                ))));
                rows.push(Box::new(caption("Drag edges for non-destructive trim")));
            } else {
                rows.push(Box::new(caption("Click a waveform to select a region")));
            }
            rows.push(Box::new(self.button(
                "Split at playhead",
                "Split region at current position · S",
                |s| s.split(),
            )));
            rows.push(Box::new(row!{self.button("Duplicate track","Duplicate selected track · Ctrl/Cmd+D",|s|s.duplicate()),self.icon(Icon::Trash,"Delete selected track (undoable)",false,|s|s.delete(true))}.spacing(6.)));
            rows.push(Box::new(Rectangle::new().fill(LINE).frame(190., 1.)));
            rows.push(Box::new(caption("PRECISE RANGE · SECONDS")));
            rows.push(Box::new(row!{ui::field(self.cursor.clone()).frame_width(90.).input_guard(),ui::field(self.range_end.clone()).frame_width(90.).input_guard()}.spacing(8.)));
            rows.push(Box::new(self.button(
                "Keep range on track",
                "Keep only audio between range start and end (undoable)",
                |s| s.trim_range(),
            )));
        }
        rows.push(Box::new(Spacer::new()));
        rows.push(Box::new(caption("Stereo output")));
        rows.push(Box::new(caption(format!(
            "{} Hz • 32-bit float export",
            m.project.sample_rate
        ))));
        AnyView::new(
            ScrollView::new(
                VStack::new(Children(rows))
                    .alignment(Alignment::TopLeading)
                    .spacing(10.)
                    .padding(14.),
            )
            .vertical()
            .content_size(220., 630.)
            .background(PANEL)
            .on_geometry_change(
                |g| g.size().width,
                move |width| {
                    let f = width / (panel_resize.size.get().width - 4.);
                    if (f - panel_resize.inspector_fraction.get()).abs() > 0.001 {
                        panel_resize.inspector_fraction.set(f);
                    }
                },
            ),
        )
    }
    fn mixer(&self) -> AnyView {
        let m = self.model.borrow();
        let mut channels: Vec<Box<dyn View>> = vec![];
        for (i, t) in m.project.tracks.iter().enumerate() {
            let c = &m.channels[i];
            let gain = self.clone();
            let pan = self.clone();
            let select = self.clone();
            let mute = self.clone();
            let solo = self.clone();
            let status = self.status.clone();
            let full_name = t.name.clone();
            let meter_status = self.status.clone();
            let meter_peak = c.peak.clone();
            channels.push(Box::new(vstack!{
                Rectangle::new().fill(ui::color(i)).frame(90.,3.),
                ui::button(format!("{:02} {}",i+1,ui::elide(&t.name,11))).font_size(10.).on_click(move||select.choose(i,None)).frame(92.,24.).on_hover(move||status.set(full_name.clone())),
                row!{ui::button("M").background_color(if t.mute{GOLD}else{RAISED}).text_color(if t.mute{BG}else{TEXT}).on_click(move||mute.mix(i,None,None,Some(false))),ui::button("S").background_color(if t.solo{GOLD}else{RAISED}).text_color(if t.solo{BG}else{TEXT}).on_click(move||solo.mix(i,None,None,Some(true)))}.spacing(6.).frame_height(24.),
                vstack!{knob::PanKnob::new(c.pan.clone(),c.dragging_pan.clone(),c.pan_focused.clone(),move|v|pan.mix(i,None,Some(v),None)).frame(32.,32.),caption(ui::pan(t.pan)).font_size(10.)}.spacing(0.).frame(90.,44.),
                fader::Fader::new(c.gain.clone(),c.peak.clone(),c.dragging_gain.clone(),c.focused.clone(),move|v|gain.mix(i,Some(v),None,None)).frame(90.,112.),
                label(format!("Gain {}",ui::db(t.gain))).font_size(10.).alignment(Alignment::Center).frame(90.,14.),
                animation::Readout::new(c.meter.clone(),9.,ACCENT,Size::new(90.,12.)).on_hover(move||meter_status.set(meter_peak.get().detail(false))),
            }.spacing(2.).padding(4.).frame(100.,254.).background(if i==m.selected{RAISED}else{PANEL}).border(LINE,1.)));
        }
        let master = self.clone();
        let master_status = self.status.clone();
        let master_peak = self.master_peak.clone();
        channels.push(Box::new(vstack!{
            Rectangle::new().fill(TEXT).frame(90.,3.),label("STEREO OUT").font_size(11.).alignment(Alignment::Center).frame(92.,24.),caption("MASTER").alignment(Alignment::Center).frame(90.,24.),caption("Post gain · pre clip").font_size(9.).alignment(Alignment::Center).frame(90.,44.),
            fader::Fader::new(self.master.clone(),self.master_peak.clone(),self.master_dragging.clone(),self.master_focus.clone(),move|v|master.master_change(v)).frame(90.,112.),
            label(format!("Gain {}",ui::db(m.project.master))).font_size(10.).alignment(Alignment::Center).frame(90.,14.),animation::Readout::new(self.master_meter.clone(),9.,ACCENT,Size::new(90.,12.)).on_hover(move||master_status.set(master_peak.get().detail(true)))
        }.spacing(2.).padding(4.).frame(100.,254.).background(RAISED).border(LINE,1.)));
        let width = channels.len() as f32 * 100.;
        AnyView::new(vstack!{row!{caption("MIXER"),caption(format!("{} audio channels",m.project.tracks.len())),Spacer::new(),caption("Gain dB · Peak dBFS L/R  |  ↑ / ↓ · Shift = fine · double-click = reset"),self.icon(Icon::X,"Hide mixer · X",false,|s|s.mixer_visible.set(false))}.spacing(12.).padding_insets(EdgeInsets::new(12.,0.,8.,0.)).frame_height(30.).background(PANEL),ScrollView::new(HStack::new(Children(channels)).spacing(0.).alignment(Alignment::TopLeading)).horizontal().content_size(width,256.).frame_height(256.)}.spacing(0.).background(BG))
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
        let busy = m.io.is_some();
        AnyView::new(row!{Text::new("resonara").font_size(19.).color(TEXT),caption("AUDIO WORKSTATION"),Spacer::new(),label(format!("{}{}",name,if dirty{"  •"}else{""})),Spacer::new(),self.button("Open…","Open a Resonara project · Ctrl/Cmd+O",|s|s.request_open()),self.icon(Icon::DeviceFloppy,"Save project · Ctrl/Cmd+S",false,|s|s.save()),self.button("Import WAV…","Import mono or stereo WAV · Ctrl/Cmd+I",|s|s.open_dialog(FileAction::Import)),self.button("Export…","Export stereo WAV · Ctrl/Cmd+E",|s|s.open_dialog(FileAction::Export)),self.icon(Icon::HelpCircle,"Keyboard shortcuts and editing help",false,|s|s.dialog.set(Dialog::Help)),caption(if busy{"Working…"}else{""})}.spacing(10.).padding_insets(EdgeInsets::new(14.,6.,12.,6.)).frame_height(44.).background(PANEL))
    }
    fn transport(&self) -> AnyView {
        let m = self.model.borrow();
        let playing = m.audio.is_some();
        let duration = m.project.duration() as f64 / m.project.sample_rate as f64;
        let seek = self.clone();
        let tempo = self.clone();
        let format = self.time_format.get();
        AnyView::new(row!{
            self.icon(Icon::ChevronLeft,"Return to start · Home",false,|s|s.seek(0.)),self.icon(if playing{Icon::PlayerPause}else{Icon::PlayerPlay},"Play / stop · Space",playing,|s|s.play()),self.button("Stop","Stop playback",|s|s.stop_audio(true)),
            vstack!{caption(format.caption()),animation::Readout::new(self.clock.clone(),25.,ACCENT,Size::new(172.,32.))}.spacing(1.).frame(172.,48.).background(BG).border(LINE,1.),
            vstack!{caption("BPM · 4/4"),ui::field(self.tempo_input.clone()).on_submit(move||tempo.submit_tempo()).blur_on_submit(true).frame(68.,26.).input_guard()}.spacing(2.),
            self.button(format.name(),"Switch musical, elapsed-time and sample-position displays",|s|s.cycle_time_format()).frame_width(92.),
            vstack!{caption("GO TO · SECONDS"),ui::field(self.cursor.clone()).on_submit(move||{if let Ok(at)=seek.seconds(&seek.cursor.get()){let rate=seek.model.borrow().project.sample_rate;seek.seek(at as f64/rate as f64);}}).blur_on_submit(true).frame(88.,26.).input_guard()}.spacing(2.),
            vstack!{caption("PROJECT LENGTH"),label(format.duration(duration,m.project.tempo,m.project.sample_rate)).font_size(16.)}.spacing(5.),Spacer::new(),caption(format!("{} Hz  /  STEREO",m.project.sample_rate))
        }.spacing(10.).padding_insets(EdgeInsets::new(14.,7.,14.,7.)).frame_height(66.).background(RAISED))
    }
    fn editbar(&self) -> AnyView {
        AnyView::new(row!{
            self.icon(Icon::ArrowBackUp,"Undo · Ctrl/Cmd+Z",false,|s|s.undo(false)),self.icon(Icon::ArrowForwardUp,"Redo · Ctrl/Cmd+Shift+Z",false,|s|s.undo(true)),Rectangle::new().fill(LINE).frame(1.,20.),
            self.button(if self.tool.get()==0{"• Pointer  1"}else{"Pointer  1"},"Pointer: select, move, trim edges · 1",|s|s.tool.set(0)),self.button(if self.tool.get()==1{"• Split  2"}else{"Split  2"},"Scissors: click a region to split · 2",|s|s.tool.set(1)),
            self.button(if self.snap.get(){"Snap: 100 ms"}else{"Snap: off"},"Toggle absolute 100 ms grid snapping",|s|s.snap.set(!s.snap.get())),Spacer::new(),
            self.icon(Icon::ZoomOut,"Zoom out · −",false,|s|s.zoom(2.)),self.icon(Icon::ZoomIn,"Zoom in · +",false,|s|s.zoom(0.5)),self.button("Fit","Fit project to timeline · F",|s|s.fit()),Rectangle::new().fill(LINE).frame(1.,20.),
            self.icon(Icon::List,"Toggle inspector · I",self.inspector.get(),|s|s.inspector.set(!s.inspector.get())),self.icon(Icon::Adjustments,"Toggle mixer · X",self.mixer_visible.get(),|s|s.mixer_visible.set(!s.mixer_visible.get()))
        }.spacing(6.).padding_insets(EdgeInsets::new(10.,3.,10.,3.)).frame_height(36.).background(PANEL))
    }
    fn workspace(&self) -> AnyView {
        let size = self.size.get();
        let height = (size.height - 44. - 66. - 36. - 28.).max(320.);
        let arrangement = if self.inspector.get() {
            AnyView::new(
                SplitView::new(self.inspector_panel(), self.arrangement())
                    .axis(SplitAxis::Horizontal)
                    .fraction(self.inspector_fraction.get())
                    .min_first(205.)
                    .min_second(560.)
                    .divider_thickness(4.)
                    .divider_colors(LINE, ACCENT)
                    .frame_width(size.width),
            )
        } else {
            AnyView::new(self.arrangement().frame_width(size.width))
        };
        let content = if self.mixer_visible.get() {
            AnyView::new(
                SplitView::new(arrangement, self.mixer())
                    .axis(SplitAxis::Vertical)
                    .fraction(self.mixer_fraction.get())
                    .min_first(230.)
                    .min_second(286.)
                    .divider_thickness(4.)
                    .divider_colors(LINE, ACCENT)
                    .frame(size.width, height),
            )
        } else {
            AnyView::new(arrangement.frame(size.width, height))
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
            Dialog::File(action) => {
                let (title, submit, description) = match action {
                    FileAction::Import => (
                        "Import audio",
                        "Import WAV",
                        "New track at 0:00 · mono/stereo WAV · audio is embedded when saved",
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
                    "Snap locks edits to 100 ms. Turn it off for finer edits.",
                    "+ / −   Zoom       F   Fit project       I   Inspector       X   Mixer",
                    "↑ / ↓   Select track       M   Mute       Delete   Delete region",
                    "Ctrl/Cmd + Z   Undo       Shift + Ctrl/Cmd + Z   Redo",
                    "Ctrl/Cmd + D   Duplicate track       Ctrl/Cmd + I   Import WAV",
                    "Ctrl/Cmd + O   Open       Ctrl/Cmd + S   Save       Ctrl/Cmd + E   Export",
                    "Rename a track in the inspector and press Enter.",
                    "Track edits and mixer changes are undoable. Original audio stays intact.",
                ] {
                    rows.push(Box::new(label(text)));
                }
                rows.push(Box::new(caption("Audio arrangement and mixing. Recording, MIDI and plug-ins are not implemented.")));
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
            "paint-only-playhead-30hz-meters-20hz",
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
        AnyView::new(ShortcutBoundary(inner))
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
            &self.inspector_fraction,
            &self.mixer_fraction,
            &self.mixer_visible,
            &self.snap,
            &self.tool,
            &self.view_start,
            &self.view_span,
            &self.dialog_error,
            &self.time_format,
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
    fn on_window_resize(&mut self, _: &WindowContext, _: u32, _: u32) {
        // Native resize events may be queued echoes. Publish the final actual
        // window size once in on_window_sync, after the event batch is drained.
    }
    fn on_window_close_requested(&mut self, _: &WindowContext) -> bool {
        if self.model.borrow().io.is_some() {
            return false;
        }
        self.finish_mix();
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
        if self.profiler.borrow().expired() {
            self.finish_profile();
        }
        self.poll_io();
        let dragging = {
            let m = self.model.borrow();
            self.master_dragging.get()
                || m.channels
                    .iter()
                    .any(|c| c.dragging_gain.get() || c.dragging_pan.get())
        };
        if !dragging && self.model.borrow().mixer_before.is_some() {
            self.finish_mix();
        }
        let now = Instant::now();
        let meter_elapsed = now.duration_since(self.last_meter.get());
        let meter_due = meter_elapsed >= animation::METER_INTERVAL;
        let playhead_due =
            now.duration_since(self.last_playhead.get()) >= animation::PLAYHEAD_INTERVAL;
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
        {
            let m = self.model.borrow();
            if let Some(a) = &m.audio {
                if playhead_due {
                    let pos = a.controls.position.load(Ordering::Relaxed) as f64
                        / m.project.sample_rate as f64;
                    self.animate_playhead(pos);
                }
                if meter_due {
                    self.update_meters(Some(&a.controls), meter_elapsed.as_secs_f32());
                }
                ended = !a.controls.playing.load(Ordering::Relaxed);
                failure = a.controls.error.load(Ordering::Relaxed);
            } else if meter_due {
                self.update_meters(None, meter_elapsed.as_secs_f32());
            }
        }
        if ended || failure {
            self.stop_audio(false);
            self.status.set(
                if failure {
                    "Audio device error. Check the output device, then press Play to retry."
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
        p.import_wav(Path::new(
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
    } else if args.iter().any(|a| a == "--empty") {
        Project::default()
    } else {
        Project::demo()
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
    p.split(0, 48000)?;
    p.trim(1, 24000, 96000)?;
    p.save(&dir.join("smoke.resonara.json"))?;
    let p = Project::load(&dir.join("smoke.resonara.json"))?;
    p.export_wav(&dir.join("smoke.wav"))?;
    let a = Audio::start(&p, 0)?;
    let start = Instant::now();
    while start.elapsed().as_secs_f32() < 2. {
        if a.controls.position.load(Ordering::Relaxed) >= p.sample_rate as u64 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if a.controls.error.load(Ordering::Relaxed)
        || a.controls.position.load(Ordering::Relaxed) < p.sample_rate as u64
    {
        return Err("Audio callback did not advance successfully".into());
    }
    println!(
        "Smoke passed: {} tracks, save/load/export, audio device {}, callback position {}",
        p.tracks.len(),
        a.device,
        a.controls.position.load(Ordering::Relaxed)
    );
    Ok(())
}
