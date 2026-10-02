//! Guest-runtime checks for native CLAP, SAS, and ScarletUI's Files interface.
//! Run `scarlet_native_smoke clap|audio [output-directory]` or `picker|save-picker`.
use resonara_core::{Clip, Controls, Engine, Insert, InsertKind, Project, Result, Track, plugins};
use scarlet_ui::{file_dialog::*, prelude::*};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

fn render(project: &Project) -> Result<(Vec<f32>, f64)> {
    let controls = Arc::new(Controls::new(project));
    let mut engine = Engine::try_new(project, controls.clone(), 48000, 0)?;
    if engine.graph_info().unavailable_plugins != 0 {
        return Err("Native CLAP missing from graph".into());
    }
    let mut samples = Vec::with_capacity(48000 * 4);
    let mut block = [0.; 512];
    let mut tail_energy = 0.0;
    for index in 0..375 {
        engine.render(&mut block, 2);
        if controls.error.load(Ordering::Relaxed) || block.iter().any(|v| !v.is_finite()) {
            return Err("CLAP processing failed".into());
        }
        if index >= 10 {
            tail_energy += block.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
        }
        samples.extend_from_slice(&block);
    }
    Ok((samples, tail_energy))
}
fn clap_smoke(directory: &Path, play: bool) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let catalog = plugins::scan_installed();
    let choice = catalog
        .effects
        .iter()
        .find(|c| c.plugin_id == "org.scarlet.freeverb")
        .ok_or_else(|| format!("Scarlet Freeverb not discovered: {:?}", catalog.warnings))?;
    let plugin = plugins::load_installed(choice)?;
    println!(
        "[native-smoke] loaded {}: {} params, {} state bytes",
        plugin.name,
        plugin.parameters.len(),
        plugin.state.len()
    );
    let wet = plugins::set_parameters(&plugin, &[(0, 1.0), (1, 0.0), (2, 0.8)])?;
    let mut samples = vec![[0.; 2]; 96000];
    samples[0] = [0.5; 2];
    let mut project = Project {
        sample_rate: 48000,
        tracks: vec![Track {
            name: "Scarlet native Freeverb".into(),
            gain: 1.,
            pan: 0.,
            mute: false,
            solo: false,
            clips: vec![Clip {
                source_channels: 2,
                start: 0,
                source_offset: 0,
                frames: 96000,
                samples: Arc::new(samples),
            }],
            routing: Default::default(),
        }],
        ..Default::default()
    };
    project.tracks[0].routing.inserts.push(Insert {
        kind: InsertKind::Clap { plugin: wet },
        bypass: false,
    });
    let (wet_output, tail_energy) = render(&project)?;
    if tail_energy < 0.00001 {
        return Err(format!("No wet tail: {tail_energy}").into());
    }
    let saved = directory.join("freeverb.resonara.json");
    project.save(&saved)?;
    let reopened = Project::load(&saved)?;
    if render(&reopened)?.0 != wet_output {
        return Err("State save/reopen changed DSP".into());
    }
    reopened.export_wav(&directory.join("freeverb-wet.wav"))?;
    project.tracks[0].routing.inserts[0].bypass = true;
    let bypass = render(&project)?.0;
    project.tracks[0].routing.inserts.clear();
    if render(&project)?.0 != bypass {
        return Err("Bypass changed dry samples".into());
    }
    println!(
        "[native-smoke] CLAP PASS: tail_energy={tail_energy}; parameters/state/reopen/export/exact bypass"
    );
    if play {
        // A tone burst is audible; the remainder of the clip keeps the wet tail alive.
        let mut audible = reopened;
        let samples = Arc::make_mut(&mut audible.tracks[0].clips[0].samples);
        for (i, frame) in samples.iter_mut().enumerate().take(12000) {
            let value = (std::f32::consts::TAU * 440. * i as f32 / 48000.).sin() * 0.1;
            *frame = [value; 2];
        }
        let audio = resonara_platform::Audio::start(&audible, 0)?;
        let started = Instant::now();
        while !audio.is_finished() {
            if audio.controls.error.load(Ordering::Relaxed)
                || started.elapsed() > Duration::from_secs(10)
            {
                return Err("Native audio playback did not complete".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if audio.controls.error.load(Ordering::Relaxed) {
            return Err("Native audio reported a processing failure".into());
        }
        std::thread::sleep(Duration::from_millis(300));
        println!("[native-smoke] SAS PASS: native CLAP playback completed");
    }
    Ok(())
}
#[derive(Clone)]
struct PickerSmoke {
    mode: FileDialogMode,
    handle: Option<FileDialogHandle>,
    started: Instant,
}
impl View for PickerSmoke {
    fn create_element(&self) -> Box<dyn Element> {
        Text::new("Waiting for native Files selection").create_element()
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}
impl Application for PickerSmoke {
    fn scenes(&self) -> impl Scene {
        WindowGroup::new(
            "picker-smoke",
            Window::new(
                "ScarletUI Files smoke",
                Text::new("Waiting for native Files selection"),
            )
            .size(Size::new(480., 200.)),
        )
    }
    fn on_window_created(
        &mut self,
        context: &WindowContext,
        _: &mut dyn scarlet_ui::PlatformWindow,
    ) {
        let mut options = FileDialog::new(self.mode);
        options.title = "ScarletUI native WAV picker".into();
        options.initial_directory = Some("/share/resonara-validation".into());
        options.filters = vec![FileDialogFilter {
            name: "WAV".into(),
            extensions: vec!["wav".into()],
        }];
        if self.mode == FileDialogMode::Save {
            options.default_name = Some("picker-saved.wav".into());
        }
        self.handle = Some(options.show(context.window_id));
    }
    fn on_idle(&mut self) {
        if let Some(result) = self.handle.as_ref().and_then(FileDialogHandle::take_result) {
            match result {
                Ok(FileDialogOutcome::Selected(paths)) if paths.len() == 1 => {
                    let path: PathBuf = paths[0].clone().into();
                    if !path.is_absolute()
                        || !path
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
                    {
                        eprintln!("[native-smoke] Files invalid path: {path:?}");
                        std::process::exit(1);
                    }
                    println!("[native-smoke] Files PASS: {:?} {path:?}", self.mode);
                    std::process::exit(0);
                }
                Ok(FileDialogOutcome::Cancelled) => {
                    println!("[native-smoke] Files CANCEL PASS");
                    std::process::exit(0);
                }
                other => {
                    eprintln!("[native-smoke] Files FAIL: {other:?}");
                    std::process::exit(1);
                }
            }
        }
        if self.started.elapsed() > Duration::from_secs(120) {
            eprintln!("Files smoke timeout");
            std::process::exit(1);
        }
    }
}
fn main() -> Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "clap".into());
    if matches!(mode.as_str(), "picker" | "save-picker") {
        return PickerSmoke {
            mode: if mode == "picker" {
                FileDialogMode::Open
            } else {
                FileDialogMode::Save
            },
            handle: None,
            started: Instant::now(),
        }
        .run()
        .map_err(|error| format!("UI: {error:?}").into());
    }
    let directory = std::env::args_os()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| "artifacts/freeverb-smoke".into());
    clap_smoke(&directory, mode == "audio")
}
