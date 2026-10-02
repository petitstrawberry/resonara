use super::*;
use resonara_core::{ClipEdit, Track};

fn fixture() -> Daw {
    let source = Arc::new(
        (0..4096)
            .map(|i| [(i as f32 / 4096.) * 0.3, -0.1])
            .collect::<Vec<_>>(),
    );
    let mut clip = Clip {
        source_channels: 2,
        start: 128,
        source_offset: 64,
        frames: 2048,
        samples: source,
        edit: ClipEdit::default(),
    };
    clip.set_gain_db(-3.).unwrap();
    clip.set_fades(100, 120).unwrap();
    clip.set_reversed(true);
    let other = Clip {
        start: 2304,
        edit: ClipEdit::default(),
        ..clip.clone()
    };
    let app = Daw::new(Project {
        sample_rate: 8000,
        tracks: [clip, other]
            .into_iter()
            .enumerate()
            .map(|(i, clip)| Track {
                name: format!("Source {i}"),
                clips: vec![clip],
                gain: 1.,
                pan: 0.,
                mute: false,
                solo: false,
                routing: Default::default(),
            })
            .collect(),
        ..Project::default()
    });
    app.choose(0, Some(0));
    app
}

fn pending_job(app: &Daw) -> (mpsc::Sender<WorkerResult>, Arc<AtomicBool>) {
    let (tx, result) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let m = app.model.borrow();
    *app.region_job.borrow_mut() = Some(RegionJob {
        track: 0,
        clip: 0,
        version: m.version,
        rate: m.project.sample_rate,
        original: m.project.tracks[0].clips[0].clone(),
        operation: RegionOperation::Transpose(7.),
        cancel: cancel.clone(),
        result,
    });
    (tx, cancel)
}

fn processed_audio() -> Vec<[f32; 2]> {
    (0..2048)
        .map(|i| [0.2, (i as f32 / 2048.) * -0.25])
        .collect()
}

fn sample_result(samples: Vec<[f32; 2]>) -> WorkerResult {
    ProcessedAudio::new(samples, 2048, &AtomicBool::new(false)).map(RegionOutput::Audio)
}

fn ready_job(app: &Daw) -> Arc<AtomicBool> {
    let (tx, cancel) = pending_job(app);
    tx.send(sample_result(processed_audio())).unwrap();
    cancel
}

fn advance_playback(app: &Daw) {
    let m = app.model.borrow();
    let audio = m.audio.as_ref().expect("region edit stopped playback");
    let before = audio.controls.position.load(Ordering::Relaxed);
    audio.render(32);
    assert_eq!(audio.controls.position.load(Ordering::Relaxed), before + 32);
    assert!(audio.controls.playing.load(Ordering::Relaxed));
}

#[test]
fn completed_pitch_job_edits_captured_region_once_and_keeps_playback_and_history_live() {
    let app = fixture();
    let original = app.model.borrow().project.tracks[0].clips[0].clone();
    app.play();
    advance_playback(&app);
    let cancel = ready_job(&app);
    // Navigating to another region during processing must not retarget the edit.
    app.choose(1, Some(0));
    app.poll_region_processing();
    assert!(!app.region_processing_active());
    assert!(cancel.load(Ordering::Acquire));
    let processed = {
        let m = app.model.borrow();
        let clip = &m.project.tracks[0].clips[0];
        assert_eq!(clip.samples.as_ref(), &processed_audio());
        assert_eq!(
            (clip.start, clip.frames, clip.source_offset),
            (128, 2048, 0)
        );
        assert_eq!(clip.source_channels, original.source_channels);
        assert_eq!(clip.edit, original.edit);
        assert!(!Arc::ptr_eq(&clip.samples, &original.samples));
        assert!(Arc::ptr_eq(
            &m.project.tracks[1].clips[0].samples,
            &original.samples
        ));
        assert_eq!(m.project.tracks[1].clips[0].source_offset, 64);
        assert_eq!(m.selected, 1);
        assert_eq!(m.undo.len(), 1);
        assert_eq!(m.undo[0].label, "Transpose region");
        clip.samples.clone()
    };
    advance_playback(&app);
    // A second idle poll cannot apply the same result twice.
    app.poll_region_processing();
    assert_eq!(app.model.borrow().undo.len(), 1);
    app.undo(false);
    {
        let m = app.model.borrow();
        let clip = &m.project.tracks[0].clips[0];
        assert!(Arc::ptr_eq(&clip.samples, &original.samples));
        assert_eq!(clip.source_offset, 64);
        assert_eq!(clip.edit, original.edit);
    }
    advance_playback(&app);
    app.undo(true);
    assert!(Arc::ptr_eq(
        &app.model.borrow().project.tracks[0].clips[0].samples,
        &processed
    ));
    advance_playback(&app);
}

#[test]
fn processed_region_and_existing_edit_metadata_survive_save_load_and_export() {
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "resonara-pitch-result-{}-{stamp}",
        std::process::id()
    )));
    std::fs::create_dir(&scratch.0).unwrap();
    let app = fixture();
    ready_job(&app);
    app.poll_region_processing();
    let project = app.model.borrow().project.clone();
    let path = scratch.0.join("edited.resonara.json");
    project.save(&path).unwrap();
    let loaded = Project::load(&path).unwrap();
    let clip = &loaded.tracks[0].clips[0];
    assert_eq!(clip.samples.as_ref(), &processed_audio());
    assert_eq!(clip.edit, project.tracks[0].clips[0].edit);
    assert_eq!(
        (clip.start, clip.frames, clip.source_offset),
        (128, 2048, 0)
    );
    let wav = scratch.0.join("edited.wav");
    loaded.export_wav(&wav).unwrap();
    let mut exported = Project {
        sample_rate: loaded.sample_rate,
        ..Project::default()
    };
    exported.import_wav(&wav).unwrap();
    let output = &exported.tracks[0].clips[0];
    assert_eq!(output.frames as u64, loaded.duration());
    let controls = Arc::new(resonara_core::Controls::new(&loaded));
    let mut engine =
        resonara_core::Engine::try_new(&loaded, controls, loaded.sample_rate, 0).unwrap();
    let mut expected = vec![0f32; output.frames * 2];
    engine.render(&mut expected, 2);
    for (actual, expected) in output.samples.iter().flatten().zip(expected) {
        assert!((actual - expected).abs() < 1e-6);
    }
}

#[test]
fn changed_project_or_replaced_region_discards_pitch_result_without_history() {
    for scenario in ["version", "removed", "source", "rate", "metadata"] {
        let app = fixture();
        ready_job(&app);
        {
            let mut m = app.model.borrow_mut();
            match scenario {
                "version" => m.version += 1,
                "removed" => m.project.tracks[0].clips.clear(),
                "source" => {
                    let clip = &mut m.project.tracks[0].clips[0];
                    clip.samples = Arc::new(clip.samples.as_ref().clone());
                }
                "rate" => m.project.sample_rate = 16000,
                "metadata" => m.project.tracks[0].clips[0].set_reversed(false),
                _ => unreachable!(),
            }
        }
        app.poll_region_processing();
        assert!(!app.region_processing_active(), "{scenario}");
        assert!(app.model.borrow().undo.is_empty(), "{scenario}");
        assert!(app.status.get().contains("discarded"), "{scenario}");
        if let Some(clip) = app.model.borrow().project.tracks[0].clips.first() {
            assert_eq!(clip.source_offset, 64, "{scenario}");
        }
    }
}

#[test]
fn cancelling_ready_pitch_job_drops_result_and_signals_worker_without_an_edit() {
    let app = fixture();
    let original = app.model.borrow().project.tracks[0].clips[0]
        .samples
        .clone();
    let cancel = ready_job(&app);
    app.cancel_region_processing();
    app.poll_region_processing();
    assert!(cancel.load(Ordering::Acquire));
    assert!(!app.region_processing_active());
    assert!(app.status.get().contains("cancelled"));
    let m = app.model.borrow();
    assert!(m.undo.is_empty());
    assert!(Arc::ptr_eq(
        &m.project.tracks[0].clips[0].samples,
        &original
    ));
}

#[test]
fn invalid_audio_and_worker_errors_leave_region_and_history_unchanged() {
    let mut nonfinite = processed_audio();
    nonfinite[100][1] = f32::NAN;
    let mut infinite = processed_audio();
    infinite[2047][0] = f32::INFINITY;
    for result in [
        Ok(Vec::new()),
        Ok(vec![[0., 0.]; 2047]),
        Ok(nonfinite),
        Ok(infinite),
        Err("processor failed".into()),
    ] {
        let app = fixture();
        let original = app.model.borrow().project.tracks[0].clips[0]
            .samples
            .clone();
        let (tx, _) = pending_job(&app);
        tx.send(result.and_then(sample_result)).unwrap();
        app.poll_region_processing();
        assert!(!app.region_processing_active());
        let m = app.model.borrow();
        assert!(m.undo.is_empty());
        assert!(Arc::ptr_eq(
            &m.project.tracks[0].clips[0].samples,
            &original
        ));
        assert!(
            app.status.get().contains("invalid audio")
                || app.status.get().contains("processor failed")
        );
    }
}

#[test]
fn pending_worker_keeps_job_and_disconnected_worker_reports_failure() {
    let app = fixture();
    let (tx, cancel) = pending_job(&app);
    app.poll_region_processing();
    assert!(app.region_processing_active());
    assert!(!cancel.load(Ordering::Acquire));
    assert!(app.model.borrow().undo.is_empty());
    drop(tx);
    app.poll_region_processing();
    assert!(!app.region_processing_active());
    assert!(cancel.load(Ordering::Acquire));
    assert!(app.status.get().contains("without a result"));
    assert!(app.model.borrow().undo.is_empty());
}

#[test]
fn ready_result_waits_for_drag_mixer_gesture_modal_and_file_operation_to_finish() {
    for scenario in ["drag", "mixer", "modal", "io"] {
        let app = fixture();
        ready_job(&app);
        {
            let mut m = app.model.borrow_mut();
            match scenario {
                "drag" => {
                    m.drag = Some(Drag {
                        track: 0,
                        clip: 0,
                        x: 0.,
                        original: m.project.tracks[0].clips[0].clone(),
                        before: Daw::snapshot(&m),
                        mode: DragMode::Move,
                        moved: false,
                    })
                }
                "mixer" => m.mixer_before = Some(Daw::snapshot(&m)),
                "modal" => app.dialog.set(Dialog::Help),
                "io" => m.io = Some(mpsc::channel().1),
                _ => unreachable!(),
            }
        }
        app.poll_region_processing();
        assert!(app.region_processing_active(), "{scenario}");
        assert!(app.model.borrow().undo.is_empty(), "{scenario}");
        {
            let mut m = app.model.borrow_mut();
            m.drag = None;
            m.mixer_before = None;
            m.io = None;
        }
        app.dialog.set(Dialog::None);
        app.poll_region_processing();
        assert!(!app.region_processing_active(), "{scenario}");
        assert_eq!(app.model.borrow().undo.len(), 1, "{scenario}");
        assert_eq!(
            app.model.borrow().project.tracks[0].clips[0].source_offset,
            0
        );
    }
}

#[test]
fn invalid_and_zero_transpose_input_never_starts_a_job() {
    let app = fixture();
    for input in ["", "abc", "NaN", "inf", "12.01", "-12.01", "0", "0.00"] {
        app.region_editor.transpose.set(input.into());
        app.editor_transpose();
        assert!(!app.region_processing_active(), "{input}");
        assert!(app.model.borrow().undo.is_empty(), "{input}");
    }
}

#[test]
fn normalization_worker_matches_core_with_reversed_trimmed_fades_and_prior_gain() {
    let app = fixture();
    let mut original = app.model.borrow().project.tracks[0].clips[0].clone();
    original.trim_relative(50, 2000).unwrap();
    let mut expected = original.clone();
    expected.normalize(-1.).unwrap();
    let gain = normalize_gain(&original, -1., &AtomicBool::new(false)).unwrap();
    assert!((gain - expected.edit.gain_db).abs() < 1e-6);
    assert_eq!(original.edit.gain_db, -3.);
    assert!(original.edit.reversed);
    assert!(Arc::ptr_eq(&original.samples, &expected.samples));
}

#[test]
fn normalization_worker_rejects_silence_nonfinite_audio_invalid_targets_and_cancellation() {
    let app = fixture();
    let original = app.model.borrow().project.tracks[0].clips[0].clone();
    for target in [-60.1, 0.1, f32::NAN, f32::INFINITY] {
        assert!(normalize_gain(&original, target, &AtomicBool::new(false)).is_err());
    }
    assert!(
        normalize_gain(&original, -1., &AtomicBool::new(true))
            .unwrap_err()
            .contains("cancelled")
    );
    let mut silent = original.clone();
    silent.samples = Arc::new(vec![[0., 0.]; 4096]);
    assert!(
        normalize_gain(&silent, -1., &AtomicBool::new(false))
            .unwrap_err()
            .contains("silent")
    );
    let mut invalid = original;
    let mut samples = invalid.samples.as_ref().clone();
    samples[1000][1] = f32::NAN;
    invalid.samples = Arc::new(samples);
    assert!(
        normalize_gain(&invalid, -1., &AtomicBool::new(false))
            .unwrap_err()
            .contains("non-finite")
    );
}

#[test]
fn normalization_starts_asynchronous_job_and_applies_only_gain_in_one_undo_step() {
    let app = fixture();
    let original = app.model.borrow().project.tracks[0].clips[0].clone();
    let expected_gain = normalize_gain(&original, -1., &AtomicBool::new(false)).unwrap();
    app.play();
    app.region_editor.normalize.set("-1.0".into());
    app.editor_normalize();
    assert!(app.region_processing_active());
    // Even an already-completed worker cannot mutate the model before polling.
    assert!(app.model.borrow().undo.is_empty());
    assert_eq!(
        app.model.borrow().project.tracks[0].clips[0].edit.gain_db,
        -3.
    );
    app.choose(1, Some(0));
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    while app.region_processing_active() && Instant::now() < deadline {
        app.poll_region_processing();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        !app.region_processing_active(),
        "normalization worker did not finish"
    );
    {
        let m = app.model.borrow();
        let clip = &m.project.tracks[0].clips[0];
        assert_eq!(clip.edit.gain_db, expected_gain);
        assert!(Arc::ptr_eq(&clip.samples, &original.samples));
        assert_eq!(
            (clip.start, clip.source_offset, clip.frames),
            (128, 64, 2048)
        );
        let mut expected_edit = original.edit;
        expected_edit.gain_db = expected_gain;
        assert_eq!(clip.edit, expected_edit);
        assert_eq!(m.project.tracks[1].clips[0].edit.gain_db, 0.);
        assert_eq!(m.undo.len(), 1);
        assert_eq!(m.undo[0].label, "Normalize region");
        assert_eq!(m.selected, 1);
    }
    advance_playback(&app);
    app.undo(false);
    assert_eq!(
        app.model.borrow().project.tracks[0].clips[0].edit.gain_db,
        -3.
    );
    app.undo(true);
    assert_eq!(
        app.model.borrow().project.tracks[0].clips[0].edit.gain_db,
        expected_gain
    );
    advance_playback(&app);
}

#[test]
fn invalid_gain_or_mismatched_output_is_rejected_before_a_normalization_edit() {
    for output in [
        RegionOutput::Gain(f32::NAN),
        RegionOutput::Gain(24.1),
        RegionOutput::Gain(-60.1),
        sample_result(processed_audio()).unwrap(),
    ] {
        let app = fixture();
        let (tx, _) = pending_job(&app);
        app.region_job.borrow_mut().as_mut().unwrap().operation = RegionOperation::Normalize(-1.);
        tx.send(Ok(output)).unwrap();
        app.poll_region_processing();
        assert!(!app.region_processing_active());
        assert!(app.model.borrow().undo.is_empty());
        assert_eq!(
            app.model.borrow().project.tracks[0].clips[0].edit.gain_db,
            -3.
        );
        assert!(app.status.get().contains("invalid audio or gain"));
    }
}

#[test]
fn invalid_normalization_target_never_starts_a_job() {
    let app = fixture();
    for input in ["", "abc", "NaN", "inf", "0.01", "-60.01"] {
        app.region_editor.normalize.set(input.into());
        app.editor_normalize();
        assert!(!app.region_processing_active(), "{input}");
        assert!(app.model.borrow().undo.is_empty(), "{input}");
    }
}
