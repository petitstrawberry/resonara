use resonara_core::{Project, Result};
use resonara_platform::Audio;
use scarlet_ui::{hstack, prelude::*, vstack};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Arc, atomic::Ordering},
    time::Instant,
};
#[derive(Clone)]
struct Daw {
    model: Rc<RefCell<Model>>,
    path: State<String>,
    project_path: State<String>,
    export_path: State<String>,
    status: State<String>,
    timeline: State<String>,
    selected: State<String>,
    gain: State<f32>,
    pan: State<f32>,
    master: State<f32>,
    cursor: State<String>,
    end: State<String>,
    meters: State<String>,
    canvas: SgfxCanvasHandle,
    frame: State<Arc<SgfxCanvasFrame>>,
    revision: Rc<Cell<u64>>,
    last_meter: Rc<Cell<Instant>>,
}
struct Model {
    project: Project,
    audio: Option<Audio>,
    selected: usize,
    undo: Vec<Project>,
    redo: Vec<Project>,
}
impl Daw {
    fn new(project: Project) -> Self {
        let s = Self {
            model: Rc::new(RefCell::new(Model {
                project,
                audio: None,
                selected: 0,
                undo: vec![],
                redo: vec![],
            })),
            path: State::new(StateId::new(1), String::new()),
            project_path: State::new(StateId::new(2), "session.resonara.json".into()),
            export_path: State::new(StateId::new(3), "mix.wav".into()),
            status: State::new(
                StateId::new(4),
                "Ready · import mono/stereo WAV or play the demo".into(),
            ),
            timeline: State::new(StateId::new(5), String::new()),
            selected: State::new(StateId::new(6), String::new()),
            gain: State::new(StateId::new(7), 1.),
            pan: State::new(StateId::new(8), 0.),
            master: State::new(StateId::new(9), 0.8),
            cursor: State::new(StateId::new(10), "0.0".into()),
            end: State::new(StateId::new(11), "1.0".into()),
            meters: State::new(StateId::new(12), String::new()),
            canvas: SgfxCanvasHandle::new(),
            frame: State::new(
                StateId::new(13),
                Arc::new(SgfxCanvasFrame::new(0, Color::rgb(20u8, 25u8, 38u8))),
            ),
            revision: Rc::new(Cell::new(0)),
            last_meter: Rc::new(Cell::new(Instant::now())),
        };
        s.refresh();
        s
    }
    fn refresh(&self) {
        let m = self.model.borrow();
        let p = &m.project;
        let mut text = format!(
            "TIMELINE   0s                                      {:.2}s\n",
            p.duration() as f64 / p.sample_rate as f64
        );
        for (i, t) in p.tracks.iter().enumerate() {
            text.push_str(&format!(
                "{} {:02} {}  {}{}\n",
                if i == m.selected { ">" } else { " " },
                i + 1,
                t.name,
                if t.mute { "M " } else { "" },
                if t.solo { "S" } else { "" }
            ));
            for c in &t.clips {
                text.push_str(&format!(
                    "    clip {:.2}s → {:.2}s\n",
                    c.start as f64 / p.sample_rate as f64,
                    (c.start + c.frames as u64) as f64 / p.sample_rate as f64
                ));
            }
        }
        self.revision.set(self.revision.get().wrapping_add(1));
        self.frame.set(waveform(p, m.selected, self.revision.get()));
        self.timeline.set(text);
        self.master.set(p.master);
        if let Some(t) = p.tracks.get(m.selected) {
            self.selected
                .set(format!("TRACK {} · {}", m.selected + 1, t.name));
            self.gain.set(t.gain);
            self.pan.set(t.pan);
        } else {
            self.selected.set("No track selected".into());
        }
    }
    fn report(&self, r: Result<()>, success: &str) {
        self.status.set(match r {
            Ok(()) => success.into(),
            Err(e) => format!("Error: {e}"),
        });
        self.refresh();
    }
    fn edit(&self, f: impl FnOnce(&mut Model) -> Result<()>, message: &str) {
        let r = {
            let mut m = self.model.borrow_mut();
            m.audio = None;
            let before = m.project.clone();
            let r = f(&mut m);
            if r.is_ok() {
                m.undo.push(before);
                if m.undo.len() > 32 {
                    m.undo.remove(0);
                }
                m.redo.clear();
            } else {
                m.project = before;
            }
            r
        };
        self.report(r, message);
    }
    fn seconds(&self, s: &str) -> Result<u64> {
        let v: f64 = s.parse()?;
        if !v.is_finite() || !(0.0..=86400.).contains(&v) {
            return Err("Time must be between 0 and 86400 seconds".into());
        }
        Ok((v * self.model.borrow().project.sample_rate as f64).round() as u64)
    }
    fn play(&self) {
        let start = self.seconds(&self.cursor.get());
        let r = start.and_then(|start| {
            let mut m = self.model.borrow_mut();
            m.audio = None;
            let audio = Audio::start(&m.project, start)?;
            self.status.set(format!("Playing · {}", audio.device));
            m.audio = Some(audio);
            Ok(())
        });
        if let Err(e) = r {
            self.status.set(format!("Audio: {e}"));
        }
    }
    fn stop(&self) {
        let mut m = self.model.borrow_mut();
        if let Some(a) = m.audio.take() {
            self.cursor.set(format!(
                "{:.3}",
                a.controls.position.load(Ordering::Relaxed) as f64 / m.project.sample_rate as f64
            ));
        }
        self.status.set("Stopped".into());
    }
    fn select(&self, delta: isize) {
        let mut m = self.model.borrow_mut();
        let len = m.project.tracks.len();
        if len > 0 {
            m.selected = (m.selected as isize + delta).rem_euclid(len as isize) as usize;
        }
        drop(m);
        self.refresh();
    }
    fn mixer(&self, gain: Option<f32>, pan: Option<f32>, toggle: Option<bool>) {
        let mut m = self.model.borrow_mut();
        let i = m.selected;
        if let Some(t) = m.project.tracks.get_mut(i) {
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
        }
        if let Some(a) = &m.audio {
            if let Some(t) = m.project.tracks.get(i) {
                a.controls.tracks[i].set(t);
            }
        }
        drop(m);
        self.refresh();
    }
    fn button(&self, label: &str, action: impl Fn(Self) + 'static) -> Button {
        let s = self.clone();
        Button::new(label).on_click(move || action(s.clone()))
    }
    fn body(&self) -> impl View + Clone + use<> {
        let gain = self.clone();
        let pan = self.clone();
        let master = self.clone();
        vstack! {
            Text::new("RESONARA  /  Native audio workstation").font_size(24.0),
            hstack! {self.button("Play",|s|s.play()), self.button("Stop",|s|s.stop()), self.button("Rewind",|s|{s.stop();s.cursor.set("0".into());}),
                self.button("Undo",|s|{let mut m=s.model.borrow_mut();m.audio=None;if let Some(p)=m.undo.pop(){let old=std::mem::replace(&mut m.project,p);m.redo.push(old);}drop(m);s.refresh();}),
                self.button("Redo",|s|{let mut m=s.model.borrow_mut();m.audio=None;if let Some(p)=m.redo.pop(){let old=std::mem::replace(&mut m.project,p);m.undo.push(old);}drop(m);s.refresh();}),
            }.spacing(8.0),
            vstack! {
            hstack! {Text::new("WAV path"),TextField::new(self.path.clone()).placeholder("/path/to/audio.wav").frame_width(540.0),self.button("Import WAV",|s|{let path=s.path.get();s.edit(|m|{m.project.import_wav(std::path::Path::new(&path))?;m.selected=m.project.tracks.len()-1;Ok(())},"WAV imported");})}.spacing(8.0),
            SgfxCanvas::from_state(self.canvas,940.0,160.0,self.frame.clone()),
            ScrollView::new(Text::from_state(self.timeline.clone()).font_size(13.0)).vertical().content_size(900.0,2000.0).frame(940.0,100.0),
            hstack! {self.button("Previous track",|s|s.select(-1)),Text::from_state(self.selected.clone()).frame_width(400.0),self.button("Next track",|s|s.select(1))}.spacing(8.0),
            hstack! {Text::new("Gain"),Slider::new(self.gain.clone()).min(0.).max(2.).on_change(move |v|gain.mixer(Some(v),None,None)).frame_width(180.0),Text::new("Pan"),Slider::new(self.pan.clone()).min(-1.).max(1.).on_change(move |v|pan.mixer(None,Some(v),None)).frame_width(180.0),self.button("Mute",|s|s.mixer(None,None,Some(false))),self.button("Solo",|s|s.mixer(None,None,Some(true))),Text::new("Master"),Slider::new(self.master.clone()).min(0.).max(1.).on_change(move |v|{let mut m=master.model.borrow_mut();m.project.master=v;if let Some(a)=&m.audio {a.controls.master.store(v.to_bits(),Ordering::Relaxed);}}).frame_width(130.0)}.spacing(8.0),
            Text::from_state(self.meters.clone()).font_size(13.0),
            }.spacing(10.0),
            vstack! {
            hstack! {Text::new("Cursor/start (s)"),TextField::new(self.cursor.clone()).frame_width(90.0),Text::new("End (s)"),TextField::new(self.end.clone()).frame_width(90.0),
                self.button("Split",|s|{match s.seconds(&s.cursor.get()){Ok(at)=>s.edit(|m|m.project.split(m.selected,at),"Clip split"),Err(e)=>s.report(Err(e),"")}}),
                self.button("Trim to range",|s|{let range=s.seconds(&s.cursor.get()).and_then(|a|s.seconds(&s.end.get()).map(|b|(a,b)));match range {Ok((a,b))=>s.edit(|m|m.project.trim(m.selected,a,b),"Track trimmed"),Err(e)=>s.report(Err(e),"")}}),
                self.button("Move to cursor",|s|{match s.seconds(&s.cursor.get()){Ok(at)=>s.edit(|m|{let t=m.project.tracks.get_mut(m.selected).ok_or("Select a track")?;let first=t.clips.iter().map(|c|c.start).min().ok_or("Track has no clips")?;for c in &mut t.clips {c.start=at.checked_add(c.start-first).ok_or("Time overflow")?;}Ok(())},"Track moved"),Err(e)=>s.report(Err(e),"")}}),
            }.spacing(8.0),
            hstack! {self.button("Duplicate track",|s|s.edit(|m|{let mut t=m.project.tracks.get(m.selected).ok_or("Select a track")?.clone();t.name.push_str(" copy");m.project.tracks.push(t);m.selected=m.project.tracks.len()-1;Ok(())},"Track duplicated")),self.button("Delete track",|s|s.edit(|m|{if m.selected>=m.project.tracks.len(){return Err("Select a track".into());}m.project.tracks.remove(m.selected);m.selected=m.selected.min(m.project.tracks.len().saturating_sub(1));Ok(())},"Track deleted"))}.spacing(8.0),
            hstack! {TextField::new(self.project_path.clone()).frame_width(540.0),self.button("Save project",|s|{let r=s.model.borrow().project.save(std::path::Path::new(&s.project_path.get()));s.report(r,"Project saved (embedded audio)");}),self.button("Open project",|s|{let path=s.project_path.get();s.edit(|m|{m.project=Project::load(std::path::Path::new(&path))?;m.selected=0;Ok(())},"Project opened");})}.spacing(8.0),
            hstack! {TextField::new(self.export_path.clone()).frame_width(540.0),self.button("Export WAV",|s|{let r=s.model.borrow().project.export_wav(std::path::Path::new(&s.export_path.get()));s.report(r,"Stereo float WAV exported");})}.spacing(8.0),
            }.spacing(10.0),
            Text::from_state(self.status.clone()).font_size(14.0),
        }.spacing(10.0).padding(18.0)
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
            &self.path,
            &self.project_path,
            &self.export_path,
            &self.status,
            &self.timeline,
            &self.selected,
            &self.gain,
            &self.pan,
            &self.master,
            &self.cursor,
            &self.end,
            &self.meters,
            &self.frame,
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
            Window::new("Resonara", self.body()).size(Size::new(1000., 840.)),
        )
    }
    fn on_idle(&mut self) {
        if self.last_meter.get().elapsed().as_millis() < 50 {
            return;
        }
        self.last_meter.set(Instant::now());
        let m = self.model.borrow();
        if let Some(a) = &m.audio {
            let pos =
                a.controls.position.load(Ordering::Relaxed) as f64 / m.project.sample_rate as f64;
            let meters = a
                .controls
                .tracks
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    format!(
                        "T{} {:.2}",
                        i + 1,
                        f32::from_bits(t.peak.swap(0, Ordering::Relaxed))
                    )
                })
                .collect::<Vec<_>>()
                .join("   ");
            self.meters.set(format!("{pos:.2}s   {meters}"));
            if a.controls.error.load(Ordering::Relaxed) {
                self.status
                    .set("Audio device error · stop and restart playback".into());
            } else if !a.controls.playing.load(Ordering::Relaxed) {
                self.status.set("Playback complete".into());
            }
        }
    }
}
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--smoke") {
        return smoke();
    }
    let project = if let Some(i) = args.iter().position(|a| a == "--project") {
        Project::load(std::path::Path::new(
            args.get(i + 1).ok_or("Missing project path")?,
        ))?
    } else if args.iter().any(|a| a == "--empty") {
        Project::default()
    } else {
        Project::demo()
    };
    Daw::new(project)
        .run()
        .map_err(|e| format!("UI: {e:?}").into())
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

fn waveform(p: &Project, selected: usize, revision: u64) -> Arc<SgfxCanvasFrame> {
    let mut vertices = Vec::new();
    let duration = p.duration().max(1) as f32;
    let rows = p.tracks.len().max(1) as f32;
    let mut rect = |x0: f32, y0: f32, x1: f32, y1: f32, color: [f32; 4]| {
        let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        for i in [0, 1, 2, 0, 2, 3] {
            vertices.push(SgfxCanvasVertex::new(
                [corners[i][0], corners[i][1], 0., 1.],
                color,
            ));
        }
    };
    for tick in 0..=8 {
        let x = -1. + tick as f32 / 4.;
        rect(x, -1., x + 0.002, 1., [0.16, 0.19, 0.25, 1.]);
    }
    for (i, t) in p.tracks.iter().enumerate() {
        let top = 1. - i as f32 * 2. / rows;
        let bottom = top - 2. / rows;
        let mid = (top + bottom) * 0.5;
        let half = 0.8 / rows;
        for c in &t.clips {
            let x0 = -1. + 2. * c.start as f32 / duration;
            let x1 = -1. + 2. * (c.start + c.frames as u64) as f32 / duration;
            rect(
                x0,
                bottom + 0.025 / rows,
                x1,
                top - 0.025 / rows,
                if i == selected {
                    [0.15, 0.29, 0.37, 1.]
                } else {
                    [0.09, 0.19, 0.25, 1.]
                },
            );
            for bin in 0..400 {
                let a = c.frames * bin / 400;
                let b = c.frames * (bin + 1) / 400;
                if a == b {
                    continue;
                }
                let peak = c.samples[c.source_offset + a..c.source_offset + b]
                    .iter()
                    .map(|s| s[0].abs().max(s[1].abs()))
                    .fold(0f32, f32::max)
                    .min(1.)
                    .max(0.008);
                let x = x0 + (x1 - x0) * bin as f32 / 400.;
                rect(
                    x,
                    mid - peak * half,
                    x + (x1 - x0) / 500.,
                    mid + peak * half,
                    if t.mute {
                        [0.38, 0.41, 0.43, 1.]
                    } else {
                        [0.27, 0.81, 0.76, 1.]
                    },
                );
            }
        }
    }
    let identity = [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ];
    Arc::new(
        SgfxCanvasFrame::new(revision, Color::rgb(20u8, 25u8, 38u8))
            .reference_aspect(940. / 160.)
            .draw(SgfxCanvasDraw::new(SgfxMesh::new(vertices), identity)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_edit_preserves_project_and_history() {
        let s = Daw::new(Project::demo());
        let length = s.model.borrow().project.duration();
        s.edit(
            |m| {
                m.project.tracks.clear();
                Err("Failed edit".into())
            },
            "unused",
        );
        assert_eq!(s.model.borrow().project.duration(), length);
        assert!(s.model.borrow().undo.is_empty());
        assert!(s.status.get().contains("Failed edit"));
    }
    #[test]
    fn selection_wraps_and_mixer_updates_model() {
        let s = Daw::new(Project::demo());
        s.select(-1);
        assert_eq!(s.model.borrow().selected, 2);
        s.mixer(Some(0.5), Some(-0.4), Some(true));
        let m = s.model.borrow();
        assert_eq!(m.project.tracks[2].gain, 0.5);
        assert_eq!(m.project.tracks[2].pan, -0.4);
        assert!(m.project.tracks[2].solo);
    }
    #[test]
    fn rejects_invalid_edit_times() {
        let s = Daw::new(Project::default());
        for bad in ["NaN", "inf", "-1", "86401", "oops"] {
            assert!(s.seconds(bad).is_err());
        }
        assert_eq!(s.seconds("0.5").unwrap(), 24000);
        s.select(1);
        s.mixer(None, None, Some(false));
    }
}
