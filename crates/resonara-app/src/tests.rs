use super::*;
use resonara_core::Track;
use scarlet_ui::{SgfxCanvasRenderObject, WindowId, event::KeyModifiers, pipeline::PipelineId};
use std::{sync::atomic::AtomicU64, time::Duration};

// Exercise the real render engine and transport state without opening a device.
pub(super) struct TestAudio {
    // Renderer is destroyed before its control-thread plugin owners.
    engine: RefCell<resonara_core::live::PlaybackRenderer>,
    playback: resonara_core::live::Playback,
    pub controls: Arc<resonara_core::Controls>,
    pub device: String,
    editor_open: bool,
}
impl TestAudio {
    pub fn update(&mut self, project: &Project) -> Result<()> {
        self.playback.update(project)?;
        self.controls = self.playback.controls.clone();
        Ok(())
    }
    pub fn open_editor(&self, slot: usize) -> Result<bool> {
        self.playback.open_editor(slot)
    }
    pub fn has_open_editors(&self) -> bool {
        self.editor_open
    }
    pub fn pause(&self) {
        self.controls.playing.store(false, Ordering::Release);
    }
    pub fn seek(&self, start: u64) -> bool {
        resonara_core::Engine::request_seek(&self.controls, start);
        self.controls.position.store(start, Ordering::Relaxed);
        true
    }
    pub fn resume(&self, start: u64, metronome: bool) -> bool {
        self.controls.metronome.store(metronome, Ordering::Relaxed);
        resonara_core::Engine::request_resume(&self.controls, start);
        self.controls.position.store(start, Ordering::Relaxed);
        true
    }
    pub fn start_paused(project: &Project) -> Result<Self> {
        let audio = Self::start_with_metronome(project, 0, false)?;
        audio.pause();
        Ok(audio)
    }
    pub fn close_editors(&self) -> Result<()> {
        self.playback.close_editors()
    }
    pub fn poll_plugins(
        &mut self,
        force: bool,
    ) -> Result<Vec<(usize, resonara_core::plugins::ClapInsert)>> {
        self.playback.poll_plugins(force)
    }
    pub fn collect_retired(&mut self) {
        self.playback.collect_retired();
    }
    pub fn is_finished(&self) -> bool {
        !self.controls.playing.load(Ordering::Relaxed)
    }
    pub fn start(project: &Project, start: u64) -> Result<Self> {
        Self::start_with_metronome(project, start, false)
    }
    pub fn start_with_metronome(project: &Project, start: u64, metronome: bool) -> Result<Self> {
        project.validate()?;
        let (playback, engine) =
            resonara_core::live::Playback::new(project, project.sample_rate, start, metronome)?;
        let controls = playback.controls.clone();
        Ok(Self {
            playback,
            controls,
            device: "Test output".into(),
            editor_open: false,
            engine: RefCell::new(engine),
        })
    }
    fn render(&self, frames: usize) {
        self.engine
            .borrow_mut()
            .render(&mut vec![0f32; frames * 2], 2);
    }
}

fn project() -> Project {
    Project {
        sample_rate: 8000,
        tracks: (0..2)
            .map(|i| Track {
                name: format!("Track {i}"),
                clips: vec![Clip {
                    source_channels: 2,
                    start: 1600,
                    source_offset: 400,
                    frames: 8000,
                    samples: Arc::new(vec![[0.2 + i as f32 * 0.1, -0.2]; 16000]),
                }],
                gain: 1.,
                pan: 0.,
                mute: false,
                solo: false,
                routing: Default::default(),
            })
            .collect(),
        ..Project::default()
    }
}

fn assert_playback_advances(s: &Daw, frames: usize) {
    let m = s.model.borrow();
    let audio = m.audio.as_ref().expect("edit stopped playback");
    let before = audio.controls.position.load(Ordering::Relaxed);
    audio.render(frames);
    assert_eq!(
        audio.controls.position.load(Ordering::Relaxed),
        before + frames as u64
    );
    assert!(audio.controls.playing.load(Ordering::Relaxed));
}

#[test]
fn playing_channel_rename_and_builtin_bypass_keep_engine_and_undo_live() {
    let s = Daw::new(project());
    s.add_insert(
        RoutingTarget::Track(0),
        resonara_core::InsertKind::Gain { gain: 0.5 },
    );
    s.play();
    let controls = s.model.borrow().audio.as_ref().unwrap().controls.clone();
    assert_playback_advances(&s, 1700);
    s.edit("Rename channel", |m| {
        m.project.tracks[0].name = "Live rename".into();
        Ok(())
    });
    assert_playback_advances(&s, 17);
    s.toggle_insert(RoutingTarget::Track(0), 0);
    assert_playback_advances(&s, 17);
    assert!(Arc::ptr_eq(
        &controls,
        &s.model.borrow().audio.as_ref().unwrap().controls
    ));
    s.undo(false);
    assert!(!s.model.borrow().project.tracks[0].routing.inserts[0].bypass);
    assert_playback_advances(&s, 17);
    s.undo(false);
    assert_eq!(s.model.borrow().project.tracks[0].name, "Track 0");
    s.undo(true);
    s.undo(true);
    assert!(s.model.borrow().project.tracks[0].routing.inserts[0].bypass);
    assert_playback_advances(&s, 17);
    assert!(Arc::ptr_eq(
        &controls,
        &s.model.borrow().audio.as_ref().unwrap().controls
    ));
}

#[test]
fn playing_structural_edits_inserts_routes_tracks_and_history_keep_transport() {
    use resonara_core::{Destination, InsertKind};
    let s = Daw::new(project());
    s.play();
    assert_playback_advances(&s, 1700);
    let transport = s
        .model
        .borrow()
        .audio
        .as_ref()
        .unwrap()
        .controls
        .position
        .clone();
    s.add_bus();
    let bus = s.model.borrow().selected_bus.unwrap();
    assert_playback_advances(&s, 31);
    s.set_output(RoutingTarget::Track(0), Destination::Bus(bus));
    s.add_send(RoutingTarget::Track(1), bus);
    s.add_insert(RoutingTarget::Track(0), InsertKind::Gain { gain: 0.5 });
    assert_playback_advances(&s, 31);
    s.add_insert(RoutingTarget::Track(0), InsertKind::Delay { frames: 8 });
    s.move_insert(RoutingTarget::Track(0), 1, -1);
    s.remove_insert(RoutingTarget::Track(0), 0);
    s.delete_bus(bus);
    assert_playback_advances(&s, 31);
    s.choose(0, None);
    s.duplicate();
    assert_playback_advances(&s, 31);
    s.delete(true);
    s.add_track(None);
    assert_playback_advances(&s, 31);
    s.undo(false);
    s.undo(true);
    assert_playback_advances(&s, 31);
    assert!(Arc::ptr_eq(
        &transport,
        &s.model.borrow().audio.as_ref().unwrap().controls.position
    ));
}

#[test]
fn playing_region_drag_commits_or_cancels_without_stopping_or_seeking() {
    for cancel in [false, true] {
        let s = Daw::new(project());
        s.snap.set(false);
        s.play();
        assert_playback_advances(&s, 2000);
        let start = s.model.borrow().project.tracks[0].clips[0].start;
        s.timeline_event(0, &press(100));
        assert_eq!(
            s.model
                .borrow()
                .audio
                .as_ref()
                .unwrap()
                .controls
                .position
                .load(Ordering::Relaxed),
            2000
        );
        s.timeline_event(0, &Event::Mouse(MouseEvent::Moved { x: 125, y: 40 }));
        assert_playback_advances(&s, 31);
        if cancel {
            s.cancel_drag();
        } else {
            s.timeline_event(
                0,
                &Event::Mouse(MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x: 125,
                    y: 40,
                    click_count: 1,
                }),
            );
        }
        assert_playback_advances(&s, 31);
        assert_eq!(s.model.borrow().undo.len(), usize::from(!cancel));
        assert_eq!(
            s.model.borrow().project.tracks[0].clips[0].start == start,
            cancel
        );
    }
}

#[test]
fn playing_save_export_and_import_keep_transport_running_during_io() {
    let temp = Temp::new();
    for action in [FileAction::Save, FileAction::Export, FileAction::Import] {
        let s = Daw::new(project());
        let path = temp.0.join(match action {
            FileAction::Save => "live.json",
            FileAction::Export => "live.wav",
            _ => "import.wav",
        });
        if action == FileAction::Import {
            project().export_wav(&path).unwrap();
        }
        s.play();
        assert_playback_advances(&s, 1700);
        s.start_io(action, path.clone());
        assert_playback_advances(&s, 31);
        finish_io(&s);
        assert_playback_advances(&s, 31);
        assert!(path.is_file());
        if action == FileAction::Import {
            assert_eq!(s.model.borrow().project.tracks.len(), 3);
        }
    }
}

#[test]
fn playing_failed_edits_and_empty_history_leave_audio_untouched() {
    let s = Daw::new(project());
    s.play();
    let controls = s.model.borrow().audio.as_ref().unwrap().controls.clone();
    s.undo(false);
    s.undo(true);
    s.edit("Rejected rename", |_| Err("Empty name".into()));
    s.edit("Rejected graph", |m| {
        m.project.tracks[0]
            .routing
            .inserts
            .push(resonara_core::Insert {
                kind: resonara_core::InsertKind::Delay { frames: 0 },
                bypass: false,
            });
        Ok(())
    });
    assert!(
        s.model.borrow().project.tracks[0]
            .routing
            .inserts
            .is_empty()
    );
    assert!(s.model.borrow().undo.is_empty());
    assert!(Arc::ptr_eq(
        &controls,
        &s.model.borrow().audio.as_ref().unwrap().controls
    ));
    assert_playback_advances(&s, 2000);
}

#[test]
fn playing_missing_plugin_edit_warns_and_bypass_keeps_audio_running() {
    let s = Daw::new(project());
    s.play();
    s.add_insert(
        RoutingTarget::Track(0),
        resonara_core::InsertKind::Clap {
            plugin: resonara_core::ClapInsert {
                library: "missing.clap".into(),
                plugin_id: "org.example.missing".into(),
                name: "Missing effect".into(),
                parameters: vec![],
                state: vec![],
            },
        },
    );
    assert!(
        s.status
            .get()
            .contains("1 unavailable CLAP insert(s) bypassed")
    );
    assert_playback_advances(&s, 1700);
    s.toggle_insert(RoutingTarget::Track(0), 0);
    assert!(!s.status.get().contains("unavailable"));
    assert_playback_advances(&s, 31);
    s.undo(false);
    assert!(
        s.status
            .get()
            .contains("1 unavailable CLAP insert(s) bypassed")
    );
    assert_playback_advances(&s, 31);
}

fn assert_project(actual: &Project, expected: &Project) {
    assert_eq!(actual.version, expected.version);
    assert_eq!(actual.sample_rate, expected.sample_rate);
    assert_eq!(actual.master, expected.master);
    assert_eq!(actual.tempo, expected.tempo);
    assert_eq!(actual.time_signature, expected.time_signature);
    assert_eq!(actual.tracks.len(), expected.tracks.len());
    assert_eq!(actual.buses, expected.buses);
    for (a, b) in actual.tracks.iter().zip(&expected.tracks) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.routing, b.routing);
        assert_eq!(
            (a.gain, a.pan, a.mute, a.solo),
            (b.gain, b.pan, b.mute, b.solo)
        );
        assert_eq!(a.clips.len(), b.clips.len());
        for (a, b) in a.clips.iter().zip(&b.clips) {
            assert_eq!(
                (a.source_channels, a.start, a.source_offset, a.frames),
                (b.source_channels, b.start, b.source_offset, b.frames)
            );
            assert!(
                a.samples.as_ref() == b.samples.as_ref(),
                "Audio samples changed"
            );
        }
    }
}

fn assert_snapshot(actual: &Model, expected: &Snapshot) {
    assert_project(&actual.project, &expected.project);
    assert_eq!(actual.selected, expected.selected);
    assert_eq!(actual.clip, expected.clip);
    assert_eq!(actual.selected_bus, expected.selected_bus);
    assert_eq!(actual.version, expected.version);
    assert_eq!(actual.current_path, expected.path);
}

fn window() -> WindowContext {
    WindowContext {
        window_id: WindowId::generate(),
        scene_key: "resonara".into(),
        pipeline_id: PipelineId::generate(),
        platform_window_id: 0,
        is_primary: true,
    }
}

fn press(x: i32) -> Event {
    Event::Mouse(MouseEvent::ButtonPressed {
        button: MouseButton::Left,
        x,
        y: 40,
        click_count: 1,
    })
}

#[test]
fn undo_redo_restores_mixer_master_and_track_clip_selection() {
    let s = Daw::new(project());
    s.choose(1, Some(0));
    let original = Daw::snapshot(&s.model.borrow());
    s.mix(1, Some(0.6), Some(-0.3), None);
    s.master_change(0.4);
    s.finish_mix();
    let mixed = Daw::snapshot(&s.model.borrow());
    assert_eq!(
        s.model.borrow().undo.len(),
        1,
        "One fader transaction should create one history entry"
    );
    assert!(s.dirty());

    s.duplicate();
    s.choose(2, Some(0));
    let duplicated = Daw::snapshot(&s.model.borrow());
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &mixed);
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &original);
    assert!(!s.dirty());
    s.undo(true);
    assert_snapshot(&s.model.borrow(), &mixed);
    s.undo(true);
    assert_snapshot(&s.model.borrow(), &duplicated);

    s.delete(true);
    assert_eq!(s.model.borrow().selected, 1);
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &duplicated);
}

#[test]
fn snapshot_restore_clamps_missing_track_and_clip_selection() {
    let s = Daw::new(project());
    let mut snapshot = Daw::snapshot(&s.model.borrow());
    snapshot.selected = 99;
    snapshot.clip = Some(99);
    snapshot.path = Some(PathBuf::from("restored.resonara.json"));
    Daw::restore(&mut s.model.borrow_mut(), snapshot);
    assert_eq!(s.model.borrow().selected, 1);
    assert_eq!(s.model.borrow().clip, None);
    assert_eq!(
        s.model.borrow().current_path.as_deref(),
        Some(Path::new("restored.resonara.json"))
    );
}

#[test]
fn cancelled_move_and_edge_trims_restore_the_entire_snapshot() {
    for (mode, x, moved_x) in [
        (DragMode::Move, 100, 140),
        (DragMode::Left, 34, 64),
        (DragMode::Right, 200, 170),
    ] {
        let s = Daw::new(project());
        s.snap.set(false);
        s.choose(0, Some(0));
        let before = Daw::snapshot(&s.model.borrow());
        assert!(s.timeline_event(0, &press(x)));
        assert!(
            s.model
                .borrow()
                .drag
                .as_ref()
                .is_some_and(|d| d.mode == mode)
        );
        assert!(s.timeline_event(0, &Event::Mouse(MouseEvent::Moved { x: moved_x, y: 40 })));
        {
            let m = s.model.borrow();
            let c = &m.project.tracks[0].clips[0];
            let old = &before.project.tracks[0].clips[0];
            assert_ne!(
                (c.start, c.source_offset, c.frames),
                (old.start, old.source_offset, old.frames)
            );
            assert!(m.undo.is_empty());
        }
        assert!(s.timeline_event(
            0,
            &Event::Mouse(MouseEvent::ButtonCancelled {
                button: MouseButton::Left,
                x: moved_x,
                y: 40,
            })
        ));
        assert_snapshot(&s.model.borrow(), &before);
        assert!(s.model.borrow().drag.is_none());
        assert!(s.model.borrow().undo.is_empty());
        assert!(!s.dirty());
    }
}

#[test]
fn released_drag_commits_one_edit_and_can_be_undone() {
    let s = Daw::new(project());
    s.choose(0, Some(0));
    let before = Daw::snapshot(&s.model.borrow());
    s.timeline_event(0, &press(100));
    for x in [125, 150] {
        s.timeline_event(0, &Event::Mouse(MouseEvent::Moved { x, y: 40 }));
    }
    s.timeline_event(
        0,
        &Event::Mouse(MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: 150,
            y: 40,
            click_count: 1,
        }),
    );
    assert_eq!(s.model.borrow().undo.len(), 1);
    assert!(s.model.borrow().drag.is_none());
    assert!(s.dirty());
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &before);
}

#[test]
fn pending_io_blocks_edits_undo_redo_mixer_pointer_keyboard_and_close() {
    let mut s = Daw::new(project());
    s.mix(0, Some(0.5), None, None);
    s.finish_mix();
    s.duplicate();
    s.undo(false);
    let before = Daw::snapshot(&s.model.borrow());
    let history = (s.model.borrow().undo.len(), s.model.borrow().redo.len());
    let (_tx, rx) = mpsc::channel();
    s.model.borrow_mut().io = Some(rx);

    let called = Cell::new(false);
    s.edit("Blocked edit", |m| {
        called.set(true);
        m.project.tracks.clear();
        Ok(())
    });
    s.undo(false);
    s.undo(true);
    s.mix(0, Some(0.2), Some(0.8), Some(false));
    s.master_change(0.1);
    s.duplicate();
    s.delete(true);
    s.split();
    s.trim_range();
    assert!(s.timeline_event(0, &press(100)));
    assert!(s.handle_key(KeyEvent::Pressed {
        keycode: KeyCode::Delete,
        modifiers: KeyModifiers::default(),
    }));
    assert!(!s.on_window_close_requested(&window()));

    assert!(!called.get());
    assert_snapshot(&s.model.borrow(), &before);
    assert_eq!(
        (s.model.borrow().undo.len(), s.model.borrow().redo.len()),
        history
    );
    assert!(s.model.borrow().mixer_before.is_none());
    assert!(s.model.borrow().drag.is_none());
    assert!(s.model.borrow().io.is_some());
    assert!(s.dialog.get() == Dialog::None);
}

#[test]
fn failed_edit_restores_project_selection_path_and_history() {
    let s = Daw::new(project());
    s.choose(1, Some(0));
    s.model.borrow_mut().current_path = Some(PathBuf::from("original.resonara.json"));
    let before = Daw::snapshot(&s.model.borrow());
    s.edit("Rejected edit", |m| {
        m.project.tracks.clear();
        m.selected = 42;
        m.clip = Some(42);
        m.current_path = None;
        Err("Deliberate rejection".into())
    });
    assert_snapshot(&s.model.borrow(), &before);
    assert!(s.model.borrow().undo.is_empty());
    assert!(s.status.get().contains("Deliberate rejection"));
}

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "resonara-app-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn submit(s: &Daw, action: FileAction, path: &Path) {
    s.dialog.set(Dialog::File(action));
    s.filename.set(path.to_string_lossy().into_owned());
    s.submit_file(false);
    assert!(
        s.model.borrow().io.is_some(),
        "File operation did not start"
    );
}

fn finish_io(s: &Daw) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while s.model.borrow().io.is_some() {
        assert!(
            Instant::now() < deadline,
            "File operation did not complete: {}",
            s.status.get()
        );
        s.poll_io();
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn failed_import_and_open_preserve_project_and_clear_continuations() {
    let temp = Temp::new();
    let corrupt = temp.0.join("corrupt.json");
    std::fs::write(&corrupt, "not a project").unwrap();
    for (action, path) in [
        (FileAction::Import, temp.0.join("missing.wav")),
        (FileAction::Open, corrupt),
    ] {
        let s = Daw::new(project());
        s.choose(1, Some(0));
        s.model.borrow_mut().current_path = Some(temp.0.join("original.json"));
        let before = Daw::snapshot(&s.model.borrow());
        s.open_after_save.set(true);
        s.close_after_save.set(true);
        submit(&s, action, &path);
        finish_io(&s);
        assert_snapshot(&s.model.borrow(), &before);
        assert!(s.model.borrow().undo.is_empty());
        assert!(s.status.get().contains("File operation failed"));
        assert!(!s.dialog_error.get().is_empty());
        assert!(!s.open_after_save.get());
        assert!(!s.close_after_save.get());
    }
}

#[test]
fn save_then_open_continuation_waits_for_save_and_loads_requested_project() {
    let temp = Temp::new();
    let saved_path = temp.0.join("saved.json");
    let opened_path = temp.0.join("opened.json");
    let s = Daw::new(project());
    s.mix(0, Some(0.55), Some(0.25), None);
    s.finish_mix();
    let saved = s.model.borrow().project.clone();
    s.request_open();
    assert!(s.dialog.get() == Dialog::ConfirmOpen);
    s.open_after_save.set(true);
    s.save();
    submit(&s, FileAction::Save, &saved_path);
    assert!(s.dialog.get() == Dialog::None);
    assert!(s.dirty());
    finish_io(&s);
    assert_project(&Project::load(&saved_path).unwrap(), &saved);
    assert!(!s.dirty());
    assert!(!s.open_after_save.get());
    assert!(s.dialog.get() == Dialog::File(FileAction::Open));
    assert_eq!(
        s.model.borrow().current_path.as_deref(),
        Some(saved_path.as_path())
    );

    let mut opened = project();
    opened.tracks.remove(0);
    opened.tracks[0].name = "Opened session".into();
    opened.save(&opened_path).unwrap();
    submit(&s, FileAction::Open, &opened_path);
    finish_io(&s);
    assert_project(&s.model.borrow().project, &opened);
    assert!(!s.dirty());
    assert_eq!(s.model.borrow().selected, 0);
    assert_eq!(s.model.borrow().clip, None);
    assert_eq!(
        s.model.borrow().current_path.as_deref(),
        Some(opened_path.as_path())
    );
    s.undo(false);
    assert_project(&s.model.borrow().project, &saved);
    assert_eq!(
        s.model.borrow().current_path.as_deref(),
        Some(saved_path.as_path())
    );
}

#[test]
fn failed_save_keeps_dirty_state_and_does_not_continue_open_or_close() {
    let temp = Temp::new();
    let s = Daw::new(project());
    s.master_change(0.3);
    s.finish_mix();
    let before = Daw::snapshot(&s.model.borrow());
    s.open_after_save.set(true);
    s.close_after_save.set(true);
    submit(
        &s,
        FileAction::Save,
        &temp.0.join("missing-parent/session.json"),
    );
    finish_io(&s);
    assert_snapshot(&s.model.borrow(), &before);
    assert!(s.dirty());
    assert!(!s.open_after_save.get());
    assert!(!s.close_after_save.get());
    assert!(s.dialog.get() == Dialog::None);
}

#[test]
fn open_after_uncommitted_mixer_change_requires_confirmation() {
    let s = Daw::new(project());
    s.mix(0, Some(0.3), None, None);
    assert!(s.model.borrow().mixer_before.is_some());
    s.request_open();
    assert!(s.dirty());
    assert!(s.dialog.get() == Dialog::ConfirmOpen);
}

#[test]
fn close_after_uncommitted_mixer_change_requires_confirmation() {
    let mut s = Daw::new(project());
    assert!(s.on_window_close_requested(&window()));
    s.master_change(0.3);
    assert!(!s.on_window_close_requested(&window()));
    assert!(s.dirty());
    assert!(s.dialog.get() == Dialog::ConfirmClose);
}

#[test]
fn empty_project_actions_are_safe_and_failed_edits_do_not_dirty_it() {
    let mut s = Daw::new(Project::default());
    s.play();
    assert!(s.status.get().contains("Import an audio file first"));
    s.stop_audio(true);
    s.select(1);
    s.select(-1);
    s.choose(99, Some(99));
    s.split();
    s.duplicate();
    s.delete(false);
    s.delete(true);
    s.trim_range();
    s.undo(false);
    s.undo(true);
    s.mix(0, Some(0.5), Some(0.5), Some(false));
    s.cancel_drag();
    s.seek(0.5);
    s.zoom(0.5);
    s.fit();
    let m = s.model.borrow();
    assert!(m.project.tracks.is_empty());
    assert_eq!(m.selected, 0);
    assert_eq!(m.clip, None);
    assert!(m.audio.is_none());
    assert!(m.undo.is_empty());
    assert!(m.redo.is_empty());
    drop(m);
    assert!(!s.dirty());
    assert!(s.on_window_close_requested(&window()));
}

fn dispatched_click(tree: &mut scarlet_ui::ElementTree, x: i32, y: i32) -> String {
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    let path = dispatcher
        .hit_test_with_path(tree, Point::new(x as f32, y as f32))
        .map(|hit| {
            hit.path
                .iter()
                .map(|e| format!("{} {:?}", e.type_name_debug(), e.bounds()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_else(|| "No hit".into());
    dispatcher.dispatch(tree, &Event::Mouse(MouseEvent::Moved { x, y }));
    dispatcher.dispatch(
        tree,
        &Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x,
            y,
            click_count: 1,
        }),
    );
    dispatcher.dispatch(
        tree,
        &Event::Mouse(MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x,
            y,
            click_count: 1,
        }),
    );
    path
}

#[test]
fn element_dispatcher_routes_timeline_and_ruler_clicks() {
    for ruler in [false, true] {
        let s = Daw::new(project());
        let mut tree = scarlet_ui::ElementTree::new();
        let view = if ruler { s.ruler() } else { s.track_row(0) };
        tree.set_root(view.create_element());
        tree.layout(scarlet_ui::LayoutConstraints::tight(
            1044.,
            if ruler { 30. } else { ROW },
        ));
        let path = dispatched_click(&mut tree, 310, if ruler { 15 } else { 40 });
        assert!(
            s.playhead.get() > 0.1,
            "ruler={ruler}; click did not seek; hit path:\n{path}"
        );
        if !ruler {
            assert_eq!(
                s.model.borrow().clip,
                Some(0),
                "Timeline did not select clip; hit path:\n{path}"
            );
        }
    }
}

fn find_canvas(element: &dyn scarlet_ui::Element, parent: Point) -> Option<(Point, Size)> {
    let origin = Point::new(
        parent.x + element.position().x,
        parent.y + element.position().y,
    );
    if element
        .render_object()
        .is_some_and(|r| r.as_any().is::<SgfxCanvasRenderObject>())
    {
        return Some((origin, element.bounds().size));
    }
    element
        .children()
        .iter()
        .find_map(|child| find_canvas(child.as_ref(), origin))
}

#[test]
fn complete_daw_tree_routes_a_timeline_click() {
    let s = Daw::new(project());
    s.choose(1, None);
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1280., 790.));
    let (origin, size) =
        find_canvas(tree.root().unwrap(), Point::ZERO).expect("No waveform canvas in Daw tree");
    let x = (origin.x + size.width * 0.12).round() as i32;
    let y = (origin.y + ROW * 0.5).round() as i32;
    let path = dispatched_click(&mut tree, x, y);
    assert_eq!(
        s.model.borrow().selected,
        0,
        "Click ({x},{y}) did not select first track; hit path:\n{path}"
    );
    assert_eq!(
        s.model.borrow().clip,
        Some(0),
        "Click ({x},{y}) did not select clip; hit path:\n{path}"
    );
    assert!(s.playhead.get() > 0.1);
}

fn arrangement_scroll_offset(element: &dyn scarlet_ui::Element) -> Option<(f32, f32)> {
    element
        .render_object()
        .and_then(|r| {
            r.as_any()
                .downcast_ref::<scarlet_ui::views::ScrollViewRenderObject<LazyVStack>>()
                .map(|r| r.offset())
        })
        .or_else(|| {
            element
                .children()
                .iter()
                .find_map(|child| arrangement_scroll_offset(child.as_ref()))
        })
}

#[test]
fn arrangement_routes_horizontal_wheel_without_blocking_vertical_tracks() {
    use scarlet_ui::event::{ScrollSource, WheelPhase};
    let mut p = project();
    let track = p.tracks[0].clone();
    while p.tracks.len() < 12 {
        p.tracks.push(track.clone());
    }
    let s = Daw::new(p);
    s.view_start.set(1.);
    s.refresh(true);
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.arrangement().create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1044., 420.));
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    let wheel = |delta_x, delta_y, phase| {
        Event::Mouse(MouseEvent::Wheel {
            delta_x,
            delta_y,
            x: 310,
            y: 72,
            phase,
            source: ScrollSource::Trackpad,
        })
    };
    let before_y = arrangement_scroll_offset(tree.root().unwrap()).unwrap().1;
    let width = (s.arrangement_size.get().width - HEADER).max(200.) as f64;
    let expected_step = 120. * 0.25 / width * s.view_span.get();
    // Native ScrollView subtracts the scaled wheel delta: a negative delta
    // scrolls toward later timeline content, and a positive one toward zero.
    assert!(dispatcher.dispatch(&mut tree, &wheel(-120, 0, WheelPhase::Started)));
    assert!(
        (s.view_start.get() - (1. + expected_step)).abs() < 1e-9,
        "Rightward timeline scrolling must match native ScrollView direction and scale"
    );
    assert_eq!(
        arrangement_scroll_offset(tree.root().unwrap()).unwrap().1,
        before_y
    );
    dispatcher.dispatch(&mut tree, &wheel(0, 0, WheelPhase::Ended));
    assert!(dispatcher.dispatch(&mut tree, &wheel(120, 0, WheelPhase::Started)));
    assert!(
        (s.view_start.get() - 1.).abs() < 1e-9,
        "Leftward scrolling should undo an equal rightward gesture"
    );
    dispatcher.dispatch(&mut tree, &wheel(0, 0, WheelPhase::Ended));
    assert!(dispatcher.dispatch(&mut tree, &wheel(10_000, 0, WheelPhase::Started)));
    assert_eq!(
        s.view_start.get(),
        0.,
        "Leftward scrolling must clamp at the timeline origin"
    );
    dispatcher.dispatch(&mut tree, &wheel(0, 0, WheelPhase::Ended));

    // A trackpad burst locks subsequent Moved events to the first handler.
    // Those events arrive at Target rather than Capture and must all apply.
    let burst_step = 32. * 0.25 / width * s.view_span.get();
    for tick in 1..=3 {
        assert!(dispatcher.dispatch(&mut tree, &wheel(-32, 0, WheelPhase::Moved)));
        assert!(
            (s.view_start.get() - tick as f64 * burst_step).abs() < 1e-9,
            "Captured rightward wheel burst dropped or duplicated tick {tick}"
        );
    }
    dispatcher.dispatch(&mut tree, &wheel(0, 0, WheelPhase::Ended));
    for tick in 1..=3 {
        assert!(dispatcher.dispatch(&mut tree, &wheel(32, 0, WheelPhase::Moved)));
        assert!(
            (s.view_start.get() - (3 - tick) as f64 * burst_step).abs() < 1e-9,
            "Captured leftward wheel burst dropped or duplicated tick {tick}"
        );
    }
    dispatcher.dispatch(&mut tree, &wheel(0, 0, WheelPhase::Ended));

    tree.root_mut().unwrap().update(&s.arrangement());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1044., 420.));
    let timeline_start = s.view_start.get();
    assert!(dispatcher.dispatch(&mut tree, &wheel(0, -1000, WheelPhase::Started)));
    dispatcher.dispatch(&mut tree, &wheel(0, 0, WheelPhase::Ended));
    let offset = arrangement_scroll_offset(tree.root().unwrap()).unwrap().1;
    assert!(
        offset > ROW,
        "Vertical track scrolling no longer reaches lower tracks"
    );
    assert_eq!(
        s.view_start.get(),
        timeline_start,
        "Vertical scrolling should not pan the timeline"
    );
    tree.layout(scarlet_ui::LayoutConstraints::tight(1044., 420.));
    let expected_track = ((offset + 42.) / ROW).floor() as usize;
    // The color strip belongs to the row selector; clicking the embedded
    // gain slider would intentionally edit gain instead of selecting a track.
    let path = dispatched_click(&mut tree, 1, 72);
    assert_eq!(
        s.model.borrow().selected,
        expected_track,
        "Scrolled track-header click selected the wrong track; hit path:\n{path}"
    );
    assert!(!s.dirty());
}

#[test]
fn offscreen_playhead_omits_empty_sgfx_draw() {
    let s = Daw::new(project());
    s.view_start.set(0.5);
    s.view_span.set(0.25);
    s.refresh(true);
    // The clipped canvas is half-open: a line beginning exactly at its right
    // edge has no visible area and must not create an empty SGFX draw.
    for (position, expected_draws) in [
        (0., 1),
        (0.5, 2),
        (0.625, 2),
        (0.7499, 2),
        (0.75, 1),
        (1_000_000., 1),
    ] {
        s.playhead.set(position);
        s.update_frames();
        for channel in &s.model.borrow().channels {
            let frame = wave::frame(
                channel.mesh.clone(),
                (s.arrangement_size.get().width - HEADER).max(200.),
                ROW,
                s.view_start.get(),
                s.view_span.get(),
                position,
                false,
                42,
                channel.playhead_mesh,
            );
            assert_eq!(
                frame.draw_count(),
                expected_draws,
                "Offscreen playhead at {position} must not submit an empty SGFX mesh"
            );
            assert_eq!(
                channel.frame.get().draw_count(),
                1,
                "Playback overlay is separate from the static waveform frame"
            );
        }
    }
}

#[test]
fn waveform_refresh_reuses_mesh_handles_and_advances_content_revisions() {
    let s = Daw::new(project());
    let meshes = s
        .model
        .borrow()
        .channels
        .iter()
        .map(|c| c.mesh.clone())
        .collect::<Vec<_>>();
    let playhead_handles = s
        .model
        .borrow()
        .channels
        .iter()
        .map(|c| c.playhead_mesh)
        .collect::<Vec<_>>();
    s.playhead.set(0.6);
    s.refresh(false);
    for (index, channel) in s.model.borrow().channels.iter().enumerate() {
        assert!(
            Arc::ptr_eq(&channel.mesh, &meshes[index]),
            "Meter/playhead refresh should retain waveform geometry"
        );
        assert_eq!(channel.playhead_mesh, playhead_handles[index]);
    }
    let mut revisions = meshes
        .iter()
        .map(|mesh| mesh.revision())
        .collect::<Vec<_>>();
    for iteration in 0..4 {
        s.view_start.set(iteration as f64 * 0.1);
        s.view_span.set(0.5 + iteration as f64 * 0.25);
        s.refresh(true);
        for (index, channel) in s.model.borrow().channels.iter().enumerate() {
            assert_eq!(
                channel.mesh.handle(),
                meshes[index].handle(),
                "Waveform refresh allocated a new retained GPU mesh identity"
            );
            assert_eq!(channel.playhead_mesh, playhead_handles[index]);
            assert!(
                channel.mesh.revision() > revisions[index],
                "Changed waveform content needs a new revision"
            );
            revisions[index] = channel.mesh.revision();
        }
    }
    assert!(!s.dirty());
}

#[test]
fn text_input_guard_preserves_typing_without_triggering_global_shortcuts() {
    let value = state(9000, String::new());
    let shortcuts = Rc::new(Cell::new(0));
    let seen = shortcuts.clone();
    let view = ui::field(value.clone())
        .input_guard()
        .on_key(move |_| {
            seen.set(seen.get() + 1);
            false
        })
        .frame(180., 30.);
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(view.create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(180., 30.));
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    dispatcher.dispatch(
        &mut tree,
        &Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 20,
            y: 15,
            click_count: 1,
        }),
    );
    dispatcher.dispatch(
        &mut tree,
        &Event::Mouse(MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: 20,
            y: 15,
            click_count: 1,
        }),
    );
    for c in "s m2".chars() {
        let keycode = if c == ' ' {
            KeyCode::Space
        } else {
            KeyCode::Char(c)
        };
        assert!(dispatcher.dispatch(
            &mut tree,
            &Event::Keyboard(KeyEvent::Pressed {
                keycode,
                modifiers: KeyModifiers::default(),
            })
        ));
        assert!(dispatcher.dispatch(&mut tree, &Event::Keyboard(KeyEvent::Char { c })));
        dispatcher.dispatch(
            &mut tree,
            &Event::Keyboard(KeyEvent::Released {
                keycode,
                modifiers: KeyModifiers::default(),
            }),
        );
    }
    assert_eq!(value.get(), "s m2");
    assert_eq!(
        shortcuts.get(),
        0,
        "Typing reached the application's shortcut handler"
    );
}

fn keyboard_input_fixture(
    initial: &str,
) -> (
    State<String>,
    scarlet_ui::ElementTree,
    scarlet_ui::EventDispatcher,
) {
    let value = state(9200, initial.to_owned());
    let view = ui::ShortcutBoundary(ui::AnyView::new(
        ui::field(value.clone()).input_guard().frame(240., 30.),
    ));
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(view.create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(240., 30.));
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    for event in [
        MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 20,
            y: 15,
            click_count: 1,
        },
        MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: 20,
            y: 15,
            click_count: 1,
        },
    ] {
        dispatcher.dispatch(&mut tree, &Event::Mouse(event));
    }
    (value, tree, dispatcher)
}

fn physical_key_and_text(
    dispatcher: &mut scarlet_ui::EventDispatcher,
    tree: &mut scarlet_ui::ElementTree,
    c: char,
    modifiers: KeyModifiers,
) {
    dispatcher.dispatch(
        tree,
        &Event::Keyboard(KeyEvent::Pressed {
            keycode: if c == ' ' {
                KeyCode::Space
            } else {
                KeyCode::Char(c)
            },
            modifiers,
        }),
    );
    dispatcher.dispatch(tree, &Event::Keyboard(KeyEvent::Char { c }));
}

#[test]
fn modified_select_all_does_not_insert_shortcut_character() {
    let initial = "/tmp/session.resonara.json";
    let (value, mut tree, mut dispatcher) = keyboard_input_fixture(initial);
    physical_key_and_text(
        &mut dispatcher,
        &mut tree,
        'a',
        KeyModifiers {
            control: true,
            ..KeyModifiers::default()
        },
    );
    assert_eq!(
        value.get(),
        initial,
        "Native Ctrl+A Char('a') must not replace selected text"
    );
    // Selection must still work, and a subsequent unmodified key must not
    // inherit suppression from the preceding shortcut.
    physical_key_and_text(&mut dispatcher, &mut tree, 'n', KeyModifiers::default());
    assert_eq!(value.get(), "n");
    for c in "ew.wav".chars() {
        physical_key_and_text(&mut dispatcher, &mut tree, c, KeyModifiers::default());
    }
    assert_eq!(value.get(), "new.wav");
}

#[test]
fn modified_save_does_not_insert_shortcut_character() {
    let initial = "session.resonara.json";
    let (value, mut tree, mut dispatcher) = keyboard_input_fixture(initial);
    dispatcher.dispatch(
        &mut tree,
        &Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::End,
            modifiers: KeyModifiers::default(),
        }),
    );
    physical_key_and_text(
        &mut dispatcher,
        &mut tree,
        's',
        KeyModifiers {
            control: true,
            ..KeyModifiers::default()
        },
    );
    assert_eq!(
        value.get(),
        initial,
        "Native Ctrl+S Char('s') must not enter a text field"
    );
    physical_key_and_text(
        &mut dispatcher,
        &mut tree,
        '!',
        KeyModifiers {
            shift: true,
            ..KeyModifiers::default()
        },
    );
    assert_eq!(
        value.get(),
        format!("{initial}!"),
        "Shift-modified ordinary text must remain usable"
    );
    physical_key_and_text(
        &mut dispatcher,
        &mut tree,
        '€',
        KeyModifiers {
            control: true,
            alt: true,
            ..KeyModifiers::default()
        },
    );
    assert_eq!(
        value.get(),
        format!("{initial}!€"),
        "AltGr text must remain usable"
    );
}

#[test]
fn shortcut_text_is_suppressed_across_save_dialog_rebuild() {
    let s = Daw::new(project());
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1280., 790.));
    let (origin, size) = find_canvas(tree.root().unwrap(), Point::ZERO).unwrap();
    let x = (origin.x + size.width * 0.12).round() as i32;
    let y = (origin.y + ROW * 0.5).round() as i32;
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    for event in [
        MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x,
            y,
            click_count: 1,
        },
        MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x,
            y,
            click_count: 1,
        },
    ] {
        dispatcher.dispatch(&mut tree, &Event::Mouse(event));
    }
    assert!(dispatcher.dispatch(
        &mut tree,
        &Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Char('s'),
            modifiers: KeyModifiers {
                control: true,
                ..KeyModifiers::default()
            },
        })
    ));
    assert!(s.dialog.get() == Dialog::File(FileAction::Save));
    let filename = s.filename.get();
    tree.root_mut().unwrap().rebuild();
    tree.layout(scarlet_ui::LayoutConstraints::tight(1280., 790.));
    assert!(dispatcher.dispatch(&mut tree, &Event::Keyboard(KeyEvent::Char { c: 's' })));
    assert_eq!(
        s.filename.get(),
        filename,
        "Save shortcut text leaked into the newly focused filename field"
    );
    physical_key_and_text(
        &mut dispatcher,
        &mut tree,
        'a',
        KeyModifiers {
            control: true,
            ..KeyModifiers::default()
        },
    );
    assert_eq!(s.filename.get(), filename);
    physical_key_and_text(&mut dispatcher, &mut tree, 'n', KeyModifiers::default());
    assert_eq!(
        s.filename.get(),
        "n",
        "Plain typing must reach the filename field after the shortcut"
    );
}

#[test]
fn fader_dispatcher_handles_mouse_keyboard_reset_and_cancel() {
    let gain = state(9100, 1.);
    let dragging = state(9101, false);
    let focused = state(9102, false);
    let changed = gain.clone();
    let fader = crate::fader::Fader::new(
        gain.clone(),
        state(9103, meter::StereoMeter::default()),
        dragging.clone(),
        focused.clone(),
        move |value| changed.set(value),
    );
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(fader.create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(96., 108.));
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    let mouse = |dispatcher: &mut scarlet_ui::EventDispatcher,
                 tree: &mut scarlet_ui::ElementTree,
                 event: MouseEvent| {
        assert!(dispatcher.dispatch(tree, &Event::Mouse(event)));
    };
    mouse(
        &mut dispatcher,
        &mut tree,
        MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 46,
            y: 54,
            click_count: 1,
        },
    );
    assert!(dragging.get() && focused.get());
    let midpoint_db = -6. + (0.5 - 0.485) / (0.612 - 0.485) * 3.;
    assert!((gain.get() - 10f32.powf(midpoint_db / 20.)).abs() < 0.00001);
    mouse(
        &mut dispatcher,
        &mut tree,
        MouseEvent::Moved { x: 46, y: 6 },
    );
    assert!((gain.get() - 10f32.powf(6. / 20.)).abs() < 0.00001);
    mouse(
        &mut dispatcher,
        &mut tree,
        MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: 46,
            y: 6,
            click_count: 1,
        },
    );
    assert!(!dragging.get());

    for (keycode, shift, expected) in [
        (KeyCode::Down, false, 10f32.powf(5. / 20.)),
        (KeyCode::Down, true, 10f32.powf(4.9 / 20.)),
        (KeyCode::End, false, 0.),
        (KeyCode::Up, false, 10f32.powf(-99. / 20.)),
        (KeyCode::Down, false, 0.),
        (KeyCode::Home, false, 1.),
        (KeyCode::Up, false, 10f32.powf(1. / 20.)),
    ] {
        assert!(dispatcher.dispatch(
            &mut tree,
            &Event::Keyboard(KeyEvent::Pressed {
                keycode,
                modifiers: KeyModifiers {
                    shift,
                    ..KeyModifiers::default()
                },
            })
        ));
        assert!(
            (gain.get() - expected).abs() < 0.00001,
            "Incorrect fader keyboard gain"
        );
    }

    mouse(
        &mut dispatcher,
        &mut tree,
        MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 46,
            y: 90,
            click_count: 2,
        },
    );
    assert_eq!(gain.get(), 1.);
    mouse(
        &mut dispatcher,
        &mut tree,
        MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: 46,
            y: 90,
            click_count: 2,
        },
    );
    assert_eq!(
        gain.get(),
        1.,
        "Double-click release must preserve unity reset"
    );
    mouse(
        &mut dispatcher,
        &mut tree,
        MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 46,
            y: 90,
            click_count: 1,
        },
    );
    assert!(gain.get() < 0.1);
    mouse(
        &mut dispatcher,
        &mut tree,
        MouseEvent::ButtonCancelled {
            button: MouseButton::Left,
            x: 46,
            y: 90,
        },
    );
    assert_eq!(
        gain.get(),
        1.,
        "Cancelled fader gesture must restore its starting gain"
    );
    assert!(!dragging.get());
}

#[test]
fn spectrum_gain_and_peak_scales_match_independent_reference_anchors() {
    let gain = [
        (-100., 0.),
        (-40., 0.082),
        (-30., 0.123),
        (-20., 0.209),
        (-15., 0.291),
        (-10., 0.399),
        (-6., 0.485),
        (-3., 0.612),
        (0., 0.743),
        (3., 0.869),
        (6., 1.),
    ];
    let peak = [
        (-60., 0.),
        (-50., 0.082),
        (-45., 0.157),
        (-40., 0.231),
        (-35., 0.302),
        (-30., 0.377),
        (-24., 0.466),
        (-21., 0.534),
        (-18., 0.601),
        (-15., 0.668),
        (-12., 0.735),
        (-9., 0.799),
        (-6., 0.866),
        (-3., 0.933),
        (0., 1.),
    ];
    assert_eq!(crate::fader::GAIN_SCALE, gain);
    assert_eq!(crate::fader::PEAK_SCALE, peak);
    for (curve, anchors) in [
        (
            crate::fader::gain_fraction as fn(f32) -> f32,
            gain.as_slice(),
        ),
        (
            crate::fader::peak_fraction as fn(f32) -> f32,
            peak.as_slice(),
        ),
    ] {
        for &(db, fraction) in anchors {
            assert!(
                (curve(db) - fraction).abs() < 1e-6,
                "Incorrect Spectrum knot at {db} dB"
            );
        }
        for pair in anchors.windows(2) {
            let midpoint_db = (pair[0].0 + pair[1].0) * 0.5;
            let midpoint_fraction = (pair[0].1 + pair[1].1) * 0.5;
            assert!(
                (curve(midpoint_db) - midpoint_fraction).abs() < 1e-6,
                "Curve must interpolate linearly between neighboring knots"
            );
        }
        assert_eq!(curve(f32::NEG_INFINITY), 0.);
        assert_eq!(curve(f32::INFINITY), 1.);
        let mut previous = 0.;
        for step in 0..=600 {
            let value = curve(-120. + step as f32 * 0.25);
            assert!(value.is_finite() && (0. ..=1.).contains(&value));
            assert!(value >= previous, "Scale is not monotonic");
            previous = value;
        }
    }
    assert!((crate::fader::gain_fraction(0.) - 0.743).abs() < 1e-6);
    assert_eq!(crate::fader::peak_fraction(0.), 1.);
    assert!((crate::fader::gain_fraction(-6.) - 0.485).abs() < 1e-6);
    assert!((crate::fader::peak_fraction(-6.) - 0.866).abs() < 1e-6);
}

#[test]
fn spectrum_fader_roundtrip_and_endpoints_share_one_axis() {
    for size in [
        Size::new(90., 112.),
        Size::new(124., 224.),
        Size::new(160., 351.),
    ] {
        let g = crate::fader::Geometry::new(size);
        assert_eq!(g.axis, size.width / 2.);
        assert_eq!(g.y(0.), g.bottom);
        assert_eq!(g.y(1.), g.top);
        assert_eq!(g.y(-2.), g.bottom);
        assert_eq!(g.y(2.), g.top);
        assert_eq!(crate::fader::gain_at(g.bottom, size.height), 0.);
        assert_eq!(crate::fader::gain_at(g.bottom + 100., size.height), 0.);
        let maximum = 10f32.powf(6. / 20.);
        assert!((crate::fader::gain_at(g.top - 100., size.height) - maximum).abs() < 1e-5);
        for db in [
            -99.5, -80., -60., -48., -40., -30., -24., -20., -18., -15., -12., -10., -9., -6., -3.,
            0., 3., 6.,
        ] {
            let y = g.y(crate::fader::gain_fraction(db));
            let roundtrip = 20. * crate::fader::gain_at(y, size.height).log10();
            assert!(
                (roundtrip - db).abs() < 0.001,
                "dB/y roundtrip failed at {db}: {roundtrip}"
            );
        }
    }
}

fn find_control_render<'a>(
    element: &'a dyn scarlet_ui::Element,
    name: &str,
) -> Option<&'a dyn scarlet_ui::ElementRenderObject> {
    if element.children().is_empty() && element.type_name_debug().contains(name) {
        return element.render_object();
    }
    element
        .children()
        .iter()
        .find_map(|child| find_control_render(child.as_ref(), name))
}

fn visible_ink_bounds(text: &str, font: f32) -> (f32, f32, f32, f32) {
    let mut points = Vec::new();
    for glyph in scarlet_ui::graphics::rasterize_text(text, font, 1000) {
        for (index, &alpha) in glyph.mask.iter().enumerate() {
            if alpha > 0 {
                points.push((
                    glyph.x + (index as u32 % glyph.width) as i32,
                    glyph.y + (index as u32 / glyph.width) as i32,
                ));
            }
        }
    }
    assert!(
        !points.is_empty(),
        "Scale label has no visible glyphs: {text}"
    );
    (
        points.iter().map(|p| p.0).min().unwrap() as f32,
        points.iter().map(|p| p.1).min().unwrap() as f32,
        (points.iter().map(|p| p.0).max().unwrap() + 1) as f32,
        (points.iter().map(|p| p.1).max().unwrap() + 1) as f32,
    )
}

fn wait_for_test_font() {
    // ScarletUI marks discovery as started before publishing its font stack.
    // Parallel tests may reach rasterization during that brief interval.
    let deadline = Instant::now() + Duration::from_secs(5);
    while scarlet_ui::graphics::default_font_stack().is_none() {
        assert!(
            Instant::now() < deadline,
            "A discovered UI font is required for glyph-alignment regression tests"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn retained_fader_paint_aligns_thumb_ticks_and_visible_label_ink() {
    use scarlet_ui::renderer::{PaintCommand, PaintContext};
    wait_for_test_font();
    for size in [Size::new(90., 112.), Size::new(124., 224.)] {
        let origin = Point::new(13., 21.);
        let g = crate::fader::Geometry::new(size);
        for db in [6., 0., -6., -18., -48., -100.] {
            let gain = if db <= -100. {
                0.
            } else {
                10f32.powf(db / 20.)
            };
            let fader = crate::fader::Fader::new(
                state(9400, gain),
                state(
                    9401,
                    meter::StereoMeter {
                        peak: [gain; 2],
                        display: [gain; 2],
                        ..Default::default()
                    },
                ),
                state(9402, false),
                state(9403, false),
                |_| {},
            );
            let mut element = fader.create_element();
            element.layout(scarlet_ui::LayoutConstraints::tight(
                size.width,
                size.height,
            ));
            let mut ctx = PaintContext::new();
            assert!(
                find_control_render(element.as_ref(), "::fader::FaderRender")
                    .unwrap()
                    .paint(&mut ctx, origin)
            );
            let thumb = ctx
                .commands()
                .iter()
                .find_map(|c| {
                    if let PaintCommand::FillRoundedRect { rect, .. } = c {
                        Some(rect)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert!(
                (thumb.origin.x + thumb.size.width * 0.5 - (origin.x + size.width * 0.5)).abs()
                    < 1e-4
            );
            assert!(
                (thumb.origin.y + thumb.size.height * 0.5
                    - (origin.y + g.y(crate::fader::gain_fraction(db))))
                .abs()
                    < 1e-4
            );
            let mut gain_ticks = Vec::new();
            let mut meter_ticks = Vec::new();
            let mut meter_rects = [Vec::new(), Vec::new()];
            let mut stereo_labels = Vec::new();
            let mut labels = 0;
            for command in ctx.commands() {
                if let PaintCommand::FillPath { path, .. } = command {
                    for (channel, rects) in meter_rects.iter_mut().enumerate() {
                        if path.len() == 4
                            && (path[0].x - (origin.x + g.meter_left + channel as f32 * 5.)).abs()
                                < 1e-4
                            && (path[1].x - path[0].x - 4.).abs() < 1e-4
                            && path[0].y >= origin.y + g.top
                        {
                            rects.push(path);
                        }
                    }
                }
                if let PaintCommand::StrokePath { path, .. } = command {
                    if path.len() == 2 && (path[0].x - (origin.x + g.axis - 18.)).abs() < 1e-4 {
                        gain_ticks.push(path[0].y);
                    }
                    if path.len() == 2 && (path[0].x - (origin.x + g.meter_left - 2.)).abs() < 1e-4
                    {
                        meter_ticks.push(path[0].y);
                    }
                }
                if let PaintCommand::DrawText {
                    position,
                    text,
                    font_size_px,
                    ..
                } = command
                {
                    let (l, t, r, b) = visible_ink_bounds(text, *font_size_px);
                    let center = Point::new(position.x + (l + r) * 0.5, position.y + (t + b) * 0.5);
                    if matches!(text.as_str(), "L" | "R") {
                        let channel = usize::from(text == "R");
                        assert!(
                            (center.x - (origin.x + g.meter_left + channel as f32 * 5. + 2.)).abs()
                                < 1e-4
                        );
                        assert!((center.y - (origin.y + g.bottom + 3.)).abs() < 1e-4);
                        assert!(position.y + b <= origin.y + size.height);
                        stereo_labels.push(text.as_str());
                        continue;
                    }
                    labels += 1;
                    let label_db = match text.as_str() {
                        "+6" => 6.,
                        "0" => 0.,
                        "−6" => -6.,
                        "−18" => -18.,
                        "−48" => -48.,
                        "−∞" => -100.,
                        "−60" => -60.,
                        other => panic!("Unexpected scale label {other}"),
                    };
                    let meter = center.x > origin.x + g.axis;
                    let expected_x = origin.x + if meter { g.peak_label } else { g.gain_label };
                    let expected_y = origin.y
                        + g.y(if meter {
                            crate::fader::peak_fraction(label_db)
                        } else {
                            crate::fader::gain_fraction(label_db)
                        });
                    assert!(
                        (center.x - expected_x).abs() < 1e-4
                            && (center.y - expected_y).abs() < 1e-4,
                        "Visible glyph ink for {text} is not centered on its scale tick"
                    );
                    assert!(
                        position.x + l >= origin.x && position.x + r <= origin.x + size.width,
                        "Label {text} escapes the horizontal fader bounds"
                    );
                    assert!(
                        position.y + t >= origin.y && position.y + b <= origin.y + size.height,
                        "Label {text} escapes the vertical fader bounds"
                    );
                }
            }
            assert_eq!(gain_ticks.len(), 6);
            assert_eq!(meter_ticks.len(), 5);
            assert_eq!(labels, 11);
            assert_eq!(stereo_labels, ["L", "R"]);
            let peak_amount = crate::fader::peak_fraction(db);
            for rects in &meter_rects {
                assert_eq!(rects.len(), if peak_amount > 0. { 2 } else { 1 });
                if peak_amount > 0. {
                    let filled = rects[1];
                    let top = filled.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
                    let bottom = filled.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
                    assert!(
                        (top - (origin.y + g.y(peak_amount))).abs() < 1e-4,
                        "Meter fill must use the peak curve, independently of the fader taper"
                    );
                    assert!((bottom - (origin.y + g.bottom)).abs() < 1e-4);
                }
            }
            for (y, db) in gain_ticks.iter().zip([6., 0., -6., -18., -48., -100.]) {
                assert!((*y - (origin.y + g.y(crate::fader::gain_fraction(db)))).abs() < 1e-4);
            }
            for (y, db) in meter_ticks.iter().zip([0., -6., -18., -48., -60.]) {
                assert!((*y - (origin.y + g.y(crate::fader::peak_fraction(db)))).abs() < 1e-4);
            }
        }
    }
}

fn control_bounds(
    element: &dyn scarlet_ui::Element,
    parent: Point,
    name: &str,
    out: &mut Vec<(Point, Size)>,
) {
    let origin = Point::new(
        parent.x + element.position().x,
        parent.y + element.position().y,
    );
    if element.children().is_empty() && element.type_name_debug().contains(name) {
        out.push((origin, element.bounds().size));
    }
    for child in element.children() {
        control_bounds(child.as_ref(), origin, name, out);
    }
}

#[test]
fn mixer_knob_and_fader_axes_align_and_pan_gestures_are_undoable() {
    let s = Daw::new(project());
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.mixer().create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(600., 286.));
    let mut knobs = Vec::new();
    let mut faders = Vec::new();
    control_bounds(
        tree.root().unwrap(),
        Point::ZERO,
        "::knob::KnobRender",
        &mut knobs,
    );
    control_bounds(
        tree.root().unwrap(),
        Point::ZERO,
        "::fader::FaderRender",
        &mut faders,
    );
    assert_eq!(knobs.len(), 2);
    assert_eq!(faders.len(), 3);
    for (knob, fader) in knobs.iter().zip(&faders) {
        assert!(
            (knob.0.x + knob.1.width * 0.5 - (fader.0.x + fader.1.width * 0.5)).abs() < 0.01,
            "Pan knob and gain fader do not share their strip's center axis"
        );
    }
    assert!(
        faders.iter().all(|f| (f.0.y - faders[0].0.y).abs() < 0.01),
        "Channel and master faders must align vertically"
    );
    let x = (knobs[0].0.x + knobs[0].1.width * 0.5).round() as i32;
    let y = (knobs[0].0.y + knobs[0].1.height * 0.5).round() as i32;
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    for event in [
        MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x,
            y,
            click_count: 1,
        },
        MouseEvent::Moved { x, y: y - 12 },
        MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x,
            y: y - 12,
            click_count: 1,
        },
    ] {
        assert!(dispatcher.dispatch(&mut tree, &Event::Mouse(event)));
    }
    assert!((s.model.borrow().project.tracks[0].pan - 0.2).abs() < 1e-5);
    assert_eq!(s.model.borrow().project.tracks[0].gain, 1.);
    assert!(!s.model.borrow().channels[0].dragging_pan.get());
    s.finish_mix();
    assert_eq!(s.model.borrow().undo.len(), 1);
    s.undo(false);
    assert_eq!(s.model.borrow().project.tracks[0].pan, 0.);
    s.undo(true);
    assert!((s.model.borrow().project.tracks[0].pan - 0.2).abs() < 1e-5);
    for event in [
        MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x,
            y,
            click_count: 1,
        },
        MouseEvent::Moved { x, y: y - 24 },
        MouseEvent::ButtonCancelled {
            button: MouseButton::Left,
            x,
            y: y - 24,
        },
    ] {
        assert!(dispatcher.dispatch(&mut tree, &Event::Mouse(event)));
    }
    s.finish_mix();
    assert_eq!(
        s.model.borrow().undo.len(),
        1,
        "Cancelled pan gesture must not create a history item"
    );
    assert!((s.model.borrow().project.tracks[0].pan - 0.2).abs() < 1e-5);
    assert!(dispatcher.dispatch(
        &mut tree,
        &Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Home,
            modifiers: KeyModifiers::default()
        })
    ));
    assert_eq!(s.model.borrow().project.tracks[0].pan, 0.);
}

fn text_layouts(
    element: &dyn scarlet_ui::Element,
    parent: Point,
    out: &mut Vec<(String, Point, Size)>,
) {
    let origin = Point::new(
        parent.x + element.position().x,
        parent.y + element.position().y,
    );
    if let Some(counter) = element
        .render_object()
        .and_then(|r| r.as_any().downcast_ref::<counter::CounterRender>())
    {
        out.push((counter.text(), origin, element.bounds().size));
    }
    if element.children().is_empty()
        && (element
            .type_name_debug()
            .contains("::views::text::TextRenderObject")
            || element
                .type_name_debug()
                .contains("::animation::ReadoutRender"))
    {
        let mut ctx = scarlet_ui::renderer::PaintContext::new();
        element.render_object().unwrap().paint(&mut ctx, origin);
        for command in ctx.commands() {
            if let scarlet_ui::renderer::PaintCommand::DrawText { text, .. } = command {
                out.push((text.clone(), origin, element.bounds().size));
            }
        }
    }
    for child in element.children() {
        text_layouts(child.as_ref(), origin, out);
    }
}

#[test]
fn complete_mixer_centers_numeric_readouts_and_master_titles_on_fader_axes() {
    wait_for_test_font();
    let s = Daw::new(project());
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.mixer().create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(600., 286.));
    let mut faders = Vec::new();
    control_bounds(
        tree.root().unwrap(),
        Point::ZERO,
        "::fader::FaderRender",
        &mut faders,
    );
    let axes = faders
        .iter()
        .map(|(origin, size)| origin.x + size.width / 2.)
        .collect::<Vec<_>>();
    assert_eq!(axes.len(), 3);
    let mut texts = Vec::new();
    text_layouts(tree.root().unwrap(), Point::ZERO, &mut texts);
    let mut gain_count = 0;
    let mut peak_count = 0;
    let mut title_count = 0;
    for (text, origin, size) in texts {
        let is_gain = text.starts_with("Gain ") && text.ends_with(" dB");
        let is_peak = text.ends_with("dBFS");
        let is_title = matches!(
            text.as_str(),
            "STEREO OUT" | "MASTER" | "Post gain · pre clip"
        );
        if !(is_gain || is_peak || is_title) {
            continue;
        }
        gain_count += usize::from(is_gain);
        peak_count += usize::from(is_peak);
        title_count += usize::from(is_title);
        let center = origin.x + size.width / 2.;
        let distance = axes
            .iter()
            .map(|axis| (center - axis).abs())
            .fold(f32::INFINITY, f32::min);
        assert!(
            distance < 0.01,
            "Mixer text {text:?} has horizontal center {center}, inconsistent with fader axes {axes:?}"
        );
    }
    assert_eq!(gain_count, 3);
    assert_eq!(peak_count, 3);
    assert_eq!(title_count, 3);
}

#[test]
fn pan_display_preserves_fine_adjustments_and_integer_percentages() {
    for (value, expected) in [
        (0., "C"),
        (0.0004, "C"),
        (-0.0004, "C"),
        (0.002, "R 0.2"),
        (-0.002, "L 0.2"),
        (0.005, "R 0.5"),
        (-0.005, "L 0.5"),
        (0.01, "R 1"),
        (-0.12, "L 12"),
        (1., "R 100"),
        (-1., "L 100"),
    ] {
        assert_eq!(
            ui::pan(value),
            expected,
            "Incorrect pan readout for {value}"
        );
    }
}

#[test]
fn amplitude_and_normalized_gain_helpers_share_the_spectrum_scale() {
    use crate::fader::{gain_from_fraction, gain_to_fraction};
    assert_eq!(gain_to_fraction(0.), 0.);
    assert_eq!(gain_from_fraction(0.), 0.);
    assert!((gain_to_fraction(1.) - 0.743).abs() < 1e-6);
    assert!((gain_from_fraction(0.743) - 1.).abs() < 1e-6);
    assert!((gain_from_fraction(0.485) - 10f32.powf(-6. / 20.)).abs() < 1e-6);
    assert!((gain_from_fraction(1.) - 10f32.powf(6. / 20.)).abs() < 1e-6);
    assert_eq!(gain_from_fraction(-1.), 0.);
    assert_eq!(gain_from_fraction(2.), gain_from_fraction(1.));
    for db in [
        -99., -80., -48., -30., -20., -18., -10., -6., -3., 0., 3., 6.,
    ] {
        let amplitude = 10f32.powf(db / 20.);
        let restored = gain_from_fraction(gain_to_fraction(amplitude));
        assert!(
            (amplitude - restored).abs() <= amplitude * 1e-5 + 1e-8,
            "Normalized gain roundtrip failed at {db} dB"
        );
    }
}

struct SliderProbe {
    id: scarlet_ui::ElementId,
    origin: Point,
    size: Size,
    value: f32,
    min: f32,
    max: f32,
}

fn slider_probes(element: &dyn scarlet_ui::Element, parent: Point, out: &mut Vec<SliderProbe>) {
    let origin = Point::new(
        parent.x + element.position().x,
        parent.y + element.position().y,
    );
    if let Some(slider) = element.render_object().and_then(|r| {
        r.as_any()
            .downcast_ref::<scarlet_ui::views::SliderRenderObject>()
    }) {
        out.push(SliderProbe {
            id: element.id(),
            origin,
            size: element.bounds().size,
            value: slider.get_value(),
            min: slider.get_min(),
            max: slider.get_max(),
        });
    }
    for child in element.children() {
        slider_probes(child.as_ref(), origin, out);
    }
}

fn gain_slider(tree: &scarlet_ui::ElementTree) -> SliderProbe {
    let mut sliders = Vec::new();
    slider_probes(tree.root().unwrap(), Point::ZERO, &mut sliders);
    sliders
        .into_iter()
        .find(|s| s.min == 0. && s.max == 1.)
        .expect("Gain slider must use normalized 0…1 Spectrum position")
}

fn assert_gain_controls_coherent(s: &Daw) {
    let expected = {
        let m = s.model.borrow();
        assert_eq!(m.channels.len(), m.project.tracks.len());
        m.project
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let normalized = crate::fader::gain_to_fraction(t.gain);
                assert!(
                    (m.channels[i].gain.get() - t.gain).abs() < 1e-6,
                    "Mixer gain state is stale on track {i}"
                );
                assert!(
                    (m.channels[i].gain_normalized.get() - normalized).abs() < 1e-6,
                    "Normalized gain state is stale on track {i}"
                );
                normalized
            })
            .collect::<Vec<_>>()
    };
    for (index, &normalized) in expected.iter().enumerate() {
        let mut tree = scarlet_ui::ElementTree::new();
        tree.set_root(s.track_row(index).create_element());
        tree.layout(scarlet_ui::LayoutConstraints::tight(1044., ROW));
        assert!(
            (gain_slider(&tree).value - normalized).abs() < 1e-6,
            "Track header {index} displays a stale or linear gain position"
        );
    }
    if !expected.is_empty() {
        let mut tree = scarlet_ui::ElementTree::new();
        tree.set_root(s.inspector_panel().create_element());
        tree.layout(scarlet_ui::LayoutConstraints::tight(220., 630.));
        let mut faders = Vec::new();
        control_bounds(
            tree.root().unwrap(),
            Point::ZERO,
            "::fader::FaderRender",
            &mut faders,
        );
        assert_eq!(
            faders.len(),
            1,
            "Inspector has one shared-state selected-channel fader"
        );
    }
}

#[test]
fn all_gain_entry_points_synchronize_and_group_each_gesture_for_undo() {
    for entry in ["track header", "inspector", "mixer"] {
        let s = Daw::new(project());
        s.choose(0, Some(0));
        assert_gain_controls_coherent(&s);
        let mut tree = scarlet_ui::ElementTree::new();
        let (view, size) = match entry {
            "track header" => (s.track_row(0), Size::new(1044., ROW)),
            "inspector" => (s.inspector_panel(), Size::new(220., 630.)),
            _ => (s.mixer(), Size::new(600., 286.)),
        };
        tree.set_root(view.create_element());
        tree.layout(scarlet_ui::LayoutConstraints::tight(
            size.width,
            size.height,
        ));
        let mut dispatcher = scarlet_ui::EventDispatcher::new();
        let mut expected_position = 0.;
        if entry != "track header" {
            let mut bounds = Vec::new();
            control_bounds(
                tree.root().unwrap(),
                Point::ZERO,
                "::fader::FaderRender",
                &mut bounds,
            );
            let (origin, size) = bounds[0];
            let geometry = crate::fader::Geometry::new(size);
            let x = (origin.x + geometry.axis).round() as i32;
            let first_y = (origin.y + geometry.y(0.3)).round() as i32;
            assert!(dispatcher.dispatch(
                &mut tree,
                &Event::Mouse(MouseEvent::ButtonPressed {
                    button: MouseButton::Left,
                    x,
                    y: first_y,
                    click_count: 1
                })
            ));
            for fraction in [0.5, 0.85] {
                let y = (origin.y + geometry.y(fraction)).round() as i32;
                assert!(dispatcher.dispatch(&mut tree, &Event::Mouse(MouseEvent::Moved { x, y })));
                expected_position =
                    (geometry.bottom - (y as f32 - origin.y)) / (geometry.bottom - geometry.top);
                assert!(
                    (crate::fader::gain_to_fraction(s.model.borrow().project.tracks[0].gain)
                        - expected_position)
                        .abs()
                        < 0.015
                );
                assert_gain_controls_coherent(&s);
            }
            let y = (origin.y + geometry.y(0.85)).round() as i32;
            dispatcher.dispatch(
                &mut tree,
                &Event::Mouse(MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x,
                    y,
                    click_count: 1,
                }),
            );
        } else {
            let slider = gain_slider(&tree);
            let y = (slider.origin.y + slider.size.height * 0.5).round() as i32;
            let start_x = (slider.origin.x + slider.size.width * 0.3).round() as i32;
            assert!(dispatcher.dispatch(
                &mut tree,
                &Event::Mouse(MouseEvent::ButtonPressed {
                    button: MouseButton::Left,
                    x: start_x,
                    y,
                    click_count: 1
                })
            ));
            for fraction in [0.5, 0.85] {
                let x = (slider.origin.x + slider.size.width * fraction).round() as i32;
                assert!(dispatcher.dispatch(&mut tree, &Event::Mouse(MouseEvent::Moved { x, y })));
                expected_position = tree
                    .find_element(slider.id)
                    .unwrap()
                    .render_object()
                    .unwrap()
                    .as_any()
                    .downcast_ref::<scarlet_ui::views::SliderRenderObject>()
                    .unwrap()
                    .get_value();
                let expected_gain = crate::fader::gain_from_fraction(expected_position);
                assert!(
                    (s.model.borrow().project.tracks[0].gain - expected_gain).abs() < 1e-5,
                    "{entry} did not convert the normalized slider through the Spectrum taper"
                );
                assert_gain_controls_coherent(&s);
            }
            let x = (slider.origin.x + slider.size.width * 0.85).round() as i32;
            dispatcher.dispatch(
                &mut tree,
                &Event::Mouse(MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x,
                    y,
                    click_count: 1,
                }),
            );
        }
        assert!(
            s.model.borrow().undo.is_empty(),
            "A {entry} drag must remain one pending mixer transaction"
        );
        assert!(!s.model.borrow().channels[0].dragging_gain.get());
        assert!(
            (s.model.borrow().channels[0].gain_normalized.get() - expected_position).abs() < 0.015
        );
        let final_gain = s.model.borrow().project.tracks[0].gain;
        s.finish_mix();
        assert_eq!(s.model.borrow().undo.len(), 1);
        assert_gain_controls_coherent(&s);
        s.undo(false);
        assert_eq!(s.model.borrow().project.tracks[0].gain, 1.);
        assert_gain_controls_coherent(&s);
        s.undo(true);
        assert!((s.model.borrow().project.tracks[0].gain - final_gain).abs() < 1e-6);
        assert_gain_controls_coherent(&s);
    }
}

#[test]
fn gain_selection_save_open_and_history_never_leave_normalized_state_stale() {
    let temp = Temp::new();
    let path = temp.0.join("coherent-gain.json");
    let s = Daw::new(project());
    s.mix_normalized(0, 0.209);
    s.finish_mix();
    s.mix(1, Some(10f32.powf(-6. / 20.)), None, None);
    s.finish_mix();
    for index in [1, 0, 1, 0] {
        s.choose(index, None);
        assert_gain_controls_coherent(&s);
    }
    assert!((s.model.borrow().project.tracks[0].gain - 0.1).abs() < 1e-6);
    let saved = s.model.borrow().project.clone();
    submit(&s, FileAction::Save, &path);
    finish_io(&s);
    assert_gain_controls_coherent(&s);
    assert_project(&Project::load(&path).unwrap(), &saved);
    s.mix_normalized(0, 1.);
    s.finish_mix();
    s.mix(1, Some(0.), None, None);
    s.finish_mix();
    s.choose(1, None);
    assert_eq!(s.model.borrow().channels[1].gain_normalized.get(), 0.);
    let changed = s.model.borrow().project.clone();
    assert_gain_controls_coherent(&s);
    submit(&s, FileAction::Open, &path);
    finish_io(&s);
    assert_project(&s.model.borrow().project, &saved);
    assert_gain_controls_coherent(&s);
    s.choose(1, None);
    assert_gain_controls_coherent(&s);
    s.undo(false);
    assert_project(&s.model.borrow().project, &changed);
    assert_gain_controls_coherent(&s);
    s.undo(true);
    assert_project(&s.model.borrow().project, &saved);
    assert_gain_controls_coherent(&s);
}

#[test]
fn no_op_or_reverted_mixer_gestures_do_not_create_history() {
    let s = Daw::new(project());
    let initial_master = s.master.get();
    s.mix(0, Some(1.), Some(0.), None);
    s.finish_mix();
    assert!(s.model.borrow().undo.is_empty());
    s.mix(0, Some(0.2), Some(0.3), None);
    s.master_change(0.4);
    s.mix(0, Some(1.), Some(0.), None);
    s.master_change(initial_master);
    s.finish_mix();
    assert!(s.model.borrow().undo.is_empty());
    assert!(!s.dirty());
}

#[test]
fn save_uses_current_path_and_commits_pending_mixer_change() {
    let temp = Temp::new();
    let path = temp.0.join("existing.json");
    let s = Daw::new(project());
    s.model.borrow().project.save(&path).unwrap();
    s.model.borrow_mut().current_path = Some(path.clone());
    s.mix(1, Some(0.45), Some(-0.4), None);
    s.save();
    assert!(s.model.borrow().io.is_some());
    assert!(s.dialog.get() == Dialog::None);
    finish_io(&s);
    assert!(!s.dirty());
    let saved = Project::load(&path).unwrap();
    assert_eq!(saved.tracks[1].gain, 0.45);
    assert_eq!(saved.tracks[1].pan, -0.4);
    assert_project(&saved, &s.model.borrow().project);

    s.handle_key(KeyEvent::Pressed {
        keycode: KeyCode::Char('s'),
        modifiers: KeyModifiers {
            control: true,
            shift: true,
            ..KeyModifiers::default()
        },
    });
    assert!(s.dialog.get() == Dialog::File(FileAction::Save));
    assert!(
        s.model.borrow().io.is_none(),
        "Save As must wait for a destination"
    );
}

#[test]
fn save_as_association_survives_ordinary_undo_and_redo() {
    let temp = Temp::new();
    for previously_saved in [false, true] {
        let s = Daw::new(project());
        if previously_saved {
            let old_path = temp.0.join("old-association.json");
            s.model.borrow().project.save(&old_path).unwrap();
            s.model.borrow_mut().current_path = Some(old_path);
        }
        let original = s.model.borrow().project.clone();
        s.duplicate();
        let edited = s.model.borrow().project.clone();
        let path = temp.0.join(format!("save-as-{previously_saved}.json"));
        submit(&s, FileAction::Save, &path);
        finish_io(&s);
        assert!(!s.dirty());
        assert_eq!(
            s.model.borrow().current_path.as_deref(),
            Some(path.as_path())
        );

        s.undo(false);
        assert_project(&s.model.borrow().project, &original);
        assert_eq!(
            s.model.borrow().current_path.as_deref(),
            Some(path.as_path()),
            "Undoing an ordinary edit must retain the latest Save As destination"
        );
        assert!(s.dirty());
        s.undo(true);
        assert_project(&s.model.borrow().project, &edited);
        assert_eq!(
            s.model.borrow().current_path.as_deref(),
            Some(path.as_path())
        );
        assert!(
            !s.dirty(),
            "Redo should return to the saved project version"
        );
    }
}

fn set_detail_view(s: &Daw) -> (f64, f64, f64, String) {
    s.view_start.set(0.275);
    s.view_span.set(0.75);
    s.seek(0.7);
    s.refresh(true);
    (
        s.view_start.get(),
        s.view_span.get(),
        s.playhead.get(),
        s.cursor.get(),
    )
}

fn assert_detail_view(s: &Daw, expected: &(f64, f64, f64, String)) {
    assert_eq!(
        (
            s.view_start.get(),
            s.view_span.get(),
            s.playhead.get(),
            s.cursor.get()
        ),
        *expected,
        "File I/O unexpectedly changed the user's timeline view or cursor"
    );
}

#[test]
fn save_and_export_preserve_timeline_zoom_scroll_and_cursor() {
    let temp = Temp::new();
    let s = Daw::new(project());
    let expected = set_detail_view(&s);
    for (action, name) in [
        (FileAction::Save, "viewport-session.json"),
        (FileAction::Export, "viewport-mix.wav"),
    ] {
        let path = temp.0.join(name);
        submit(&s, action, &path);
        finish_io(&s);
        assert!(path.is_file());
        assert!(!s.status.get().contains("failed"));
        assert_detail_view(&s, &expected);
    }
}

#[test]
fn failed_file_operations_preserve_timeline_zoom_scroll_and_cursor() {
    let temp = Temp::new();
    let invalid = temp.0.join("invalid-project.json");
    std::fs::write(&invalid, "invalid project").unwrap();
    for (action, path) in [
        (FileAction::Import, temp.0.join("missing.wav")),
        (FileAction::Open, invalid),
        (FileAction::Save, temp.0.join("missing-parent/session.json")),
        (FileAction::Export, temp.0.join("missing-parent/mix.wav")),
    ] {
        let s = Daw::new(project());
        let expected = set_detail_view(&s);
        let before = Daw::snapshot(&s.model.borrow());
        submit(&s, action, &path);
        finish_io(&s);
        assert!(s.status.get().contains("File operation failed"));
        assert_detail_view(&s, &expected);
        assert_snapshot(&s.model.borrow(), &before);
    }
}

#[test]
fn cancelling_save_dialog_clears_open_and_close_continuations() {
    let s = Daw::new(project());
    for open in [false, true] {
        s.open_after_save.set(open);
        s.close_after_save.set(!open);
        s.save();
        assert!(s.dialog.get() == Dialog::File(FileAction::Save));
        assert!(s.handle_key(KeyEvent::Pressed {
            keycode: KeyCode::Escape,
            modifiers: KeyModifiers::default(),
        }));
        assert!(s.dialog.get() == Dialog::None);
        assert!(!s.open_after_save.get());
        assert!(!s.close_after_save.get());
    }
}

/// Opt-in stress check with real audio; deliberately excluded from normal tests.
/// This measures model/CPU layout work, not GPU presentation or audio deadlines.
#[test]
#[ignore = "requires RESONARA_LONG_AUDIO_WAV; run in release mode with --ignored --nocapture"]
fn long_audio_eight_track_ui_model_stress() {
    let path = PathBuf::from(
        std::env::var_os("RESONARA_LONG_AUDIO_WAV")
            .expect("Set RESONARA_LONG_AUDIO_WAV to a real long-form WAV fixture"),
    );
    let import_start = Instant::now();
    let mut p = Project::default();
    p.import_wav(&path).expect("Long-form WAV import failed");
    let import_ms = import_start.elapsed().as_secs_f64() * 1000.;
    let source = p.tracks[0].clone();
    let duration = p.duration() as f64 / p.sample_rate as f64;
    assert!(duration >= 60., "Use at least one minute of real audio");
    for index in 1..8 {
        let mut track = source.clone();
        track.name = format!("Long audio track {}", index + 1);
        p.tracks.push(track);
    }
    let expected_frames = source.clips[0].frames;
    let setup = Instant::now();
    let s = Daw::new(p);
    let setup_ms = setup.elapsed().as_secs_f64() * 1000.;
    let mut update_ms = Vec::with_capacity(48);
    for iteration in 0..48 {
        let start = Instant::now();
        s.choose(iteration % 8, Some(0));
        s.zoom(if iteration % 2 == 0 { 0.5 } else { 2. });
        let scroll_range = (duration - s.view_span.get()).max(0.);
        s.view_start.set(scroll_range * (iteration % 5) as f64 / 4.);
        s.refresh(true);
        update_ms.push(start.elapsed().as_secs_f64() * 1000.);
        assert_eq!(s.model.borrow().selected, iteration % 8);
        assert_eq!(s.model.borrow().clip, Some(0));
        assert!(s.view_start.get() >= 0.);
        assert!(s.view_span.get().is_finite() && s.view_span.get() > 0.);
    }

    let layout_start = Instant::now();
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1280., 790.));
    let first_layout_ms = layout_start.elapsed().as_secs_f64() * 1000.;
    let mut layout_ms = Vec::with_capacity(8);
    for index in 0..8 {
        let start = Instant::now();
        s.choose(index, Some(0));
        tree.root_mut().unwrap().rebuild();
        tree.layout(scarlet_ui::LayoutConstraints::tight(1280., 790.));
        layout_ms.push(start.elapsed().as_secs_f64() * 1000.);
    }

    let m = s.model.borrow();
    assert_eq!(m.project.tracks.len(), 8);
    assert_eq!(m.channels.len(), 8);
    assert!(m.undo.is_empty() && m.redo.is_empty());
    assert!(m.drag.is_none() && m.mixer_before.is_none());
    for track in &m.project.tracks {
        assert_eq!(track.clips.len(), 1);
        assert_eq!(track.clips[0].frames, expected_frames);
        assert!(Arc::ptr_eq(
            &track.clips[0].samples,
            &source.clips[0].samples
        ));
    }
    m.project.validate().unwrap();
    drop(m);
    assert!(!s.dirty(), "View navigation must not change the session");
    update_ms.sort_by(f64::total_cmp);
    layout_ms.sort_by(f64::total_cmp);
    eprintln!(
        "LONG_AUDIO_STRESS: duration_s={duration:.3}, tracks=8, shared_source_frames={expected_frames}, import_ms={import_ms:.3}, peak_cache_and_model_setup_ms={setup_ms:.3}"
    );
    eprintln!(
        "LONG_AUDIO_STRESS: 48 select+zoom+scroll model cycles: median_ms={:.3}, p95_ms={:.3}, max_ms={:.3}",
        update_ms[24], update_ms[45], update_ms[47]
    );
    eprintln!(
        "LONG_AUDIO_STRESS: first_cpu_tree_layout_ms={first_layout_ms:.3}; 8 select+rebuild+CPU-layout cycles: median_ms={:.3}, max_ms={:.3}",
        layout_ms[4], layout_ms[7]
    );
    eprintln!(
        "LONG_AUDIO_STRESS: model/CPU-layout-only measurements; no GPU FPS, physical audio latency, or audio callback deadline measurement"
    );
}

fn stereo_meter_project() -> Project {
    let mut p = project();
    p.master = 2.;
    for (track, sample) in p.tracks.iter_mut().zip([[0.75, -0.25], [0.25, 0.125]]) {
        track.clips = vec![Clip {
            source_channels: 2,
            start: 0,
            source_offset: 0,
            frames: 256,
            samples: Arc::new(vec![sample; 256]),
        }];
    }
    p
}

#[test]
fn stereo_meter_poll_uses_one_snapshot_for_bars_text_and_master_overrange() {
    use resonara_core::{Controls, Engine};
    let p = stereo_meter_project();
    let controls = Arc::new(Controls::new(&p));
    let mut engine = Engine::new(&p, controls.clone(), p.sample_rate, 0);
    let s = Daw::new(p);
    let mut output = [0.; 16];
    engine.render(&mut output, 2);
    assert_eq!(output, [1., -0.25].repeat(8).as_slice());
    // on_idle also holds an immutable model borrow while polling.
    let model = s.model.borrow();
    s.update_meters(Some(&controls), 0.05);
    for (index, expected) in [[0.75, 0.25], [0.25, 0.125]].into_iter().enumerate() {
        let reading = model.channels[index].peak.get();
        assert_eq!(reading.peak, expected);
        assert_eq!(reading.held, expected);
        assert_eq!(model.channels[index].meter.get(), reading.summary());
        assert_eq!(controls.tracks[index].peak.load(Ordering::Relaxed), 0);
        assert_eq!(controls.tracks[index].peak_left.load(Ordering::Relaxed), 0);
        assert_eq!(controls.tracks[index].peak_right.load(Ordering::Relaxed), 0);
    }
    assert_eq!(model.channels[0].meter.get(), "Peak -2.5 dBFS");
    assert_eq!(
        model.channels[0].peak.get().detail(false),
        "L -2.5 dBFS · R -12.0 dBFS · post-pan, post-fader · peak/clip hold 1 s"
    );
    assert_eq!(s.master_peak.get().peak, [2., 0.25]);
    assert_eq!(s.master_meter.get(), "Peak +6.0 dBFS");
    assert_eq!(
        s.master_peak.get().detail(true),
        "L +6.0 dBFS · R -12.0 dBFS · post-master gain, pre-output clip · peak/clip hold 1 s"
    );
    assert_eq!(s.master_peak.get().clip_seconds, [1., 0.]);
    assert_eq!(controls.master_peak_left.load(Ordering::Relaxed), 0);
    assert_eq!(controls.master_peak_right.load(Ordering::Relaxed), 0);
    drop(model);

    // Raw measurements become silent; displayed bars decay and peak numbers hold.
    s.update_meters(Some(&controls), 0.05);
    assert_eq!(s.master_peak.get().peak, [0.; 2]);
    assert_eq!(s.master_peak.get().held, [2., 0.25]);
    assert_eq!(s.master_meter.get(), "Peak +6.0 dBFS");
    for channel in &s.model.borrow().channels {
        assert_eq!(channel.peak.get().peak, [0.; 2]);
        assert_eq!(channel.meter.get(), channel.peak.get().summary());
    }

    controls.tracks[0]
        .pan
        .store(1f32.to_bits(), Ordering::Relaxed);
    controls.tracks[1].mute.store(true, Ordering::Relaxed);
    engine.render(&mut output, 2);
    s.update_meters(Some(&controls), 0.05);
    assert_eq!(s.model.borrow().channels[0].peak.get().peak, [0., 0.25]);
    assert_eq!(s.model.borrow().channels[1].peak.get().peak, [0.; 2]);
    assert_eq!(s.master_peak.get().peak, [0., 0.5]);
    controls.master.store(0.5f32.to_bits(), Ordering::Relaxed);
    engine.render(&mut output, 2);
    s.update_meters(Some(&controls), 0.05);
    assert_eq!(s.model.borrow().channels[0].peak.get().peak, [0., 0.25]);
    assert_eq!(s.master_peak.get().peak, [0., 0.125]);
    controls.tracks[1].solo.store(true, Ordering::Relaxed);
    engine.render(&mut output, 2);
    s.update_meters(Some(&controls), 0.05);
    assert_eq!(s.master_peak.get().peak, [0.; 2]);
    assert!(
        s.model
            .borrow()
            .channels
            .iter()
            .all(|c| c.peak.get().peak == [0.; 2])
    );
}

fn fader_paints(
    element: &dyn scarlet_ui::Element,
    parent: Point,
    out: &mut Vec<(Point, Size, Vec<scarlet_ui::renderer::PaintCommand>)>,
) {
    let origin = Point::new(
        parent.x + element.position().x,
        parent.y + element.position().y,
    );
    if element.children().is_empty() && element.type_name_debug().contains("::fader::FaderRender") {
        let mut context = scarlet_ui::renderer::PaintContext::new();
        assert!(element.render_object().unwrap().paint(&mut context, origin));
        out.push((origin, element.bounds().size, context.commands().to_vec()));
    }
    for child in element.children() {
        fader_paints(child.as_ref(), origin, out);
    }
}

#[test]
fn complete_mixer_paints_unequal_stereo_track_and_master_bars_and_hover_values() {
    use resonara_core::{Controls, Engine};
    use scarlet_ui::renderer::PaintCommand;
    wait_for_test_font();
    let p = stereo_meter_project();
    let controls = Arc::new(Controls::new(&p));
    let mut engine = Engine::new(&p, controls.clone(), p.sample_rate, 0);
    engine.render(&mut [0f32; 16], 2);
    let s = Daw::new(p);
    s.update_meters(Some(&controls), 0.05);
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.mixer().create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(600., 286.));
    let mut paints = Vec::new();
    fader_paints(tree.root().unwrap(), Point::ZERO, &mut paints);
    assert_eq!(
        paints.len(),
        3,
        "Two track faders and the master must all have live meters"
    );
    let expected = [[0.75f32, 0.25], [0.25, 0.125], [2., 0.25]];
    for ((origin, size, commands), peaks) in paints.iter().zip(expected) {
        assert_eq!(*size, Size::new(90., s.mixer_fader_height()));
        let g = crate::fader::Geometry::new(*size);
        let mut tops = [f32::NAN; 2];
        let mut labels = Vec::new();
        for command in commands {
            if let PaintCommand::DrawText { text, .. } = command {
                if matches!(text.as_str(), "L" | "R") {
                    labels.push(text.as_str());
                }
            }
            let PaintCommand::FillPath { path, color } = command else {
                continue;
            };
            if path.len() != 4 || *color == ui::BG {
                continue;
            }
            let width = path[1].x - path[0].x;
            let top = path.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
            let bottom = path.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
            if (width - 4.).abs() > 1e-4
                || (bottom - (origin.y + g.bottom)).abs() > 1e-4
                || bottom - top <= 1.
            {
                continue;
            }
            let channel = if (path[0].x - (origin.x + g.meter_left)).abs() < 1e-4 {
                0
            } else if (path[0].x - (origin.x + g.meter_left + 5.)).abs() < 1e-4 {
                1
            } else {
                continue;
            };
            assert!(tops[channel].is_nan(), "Meter channel was painted twice");
            tops[channel] = top;
            let expected_color = if peaks[channel] >= 1. {
                Color::rgb(0.98, 0.4, 0.4)
            } else if peaks[channel] > 0.7 {
                ui::GOLD
            } else {
                ui::ACCENT
            };
            assert_eq!(*color, expected_color);
        }
        assert_eq!(labels, ["L", "R"]);
        for channel in 0..2 {
            let expected_top =
                origin.y + g.y(crate::fader::peak_fraction(20. * peaks[channel].log10()));
            assert!(
                (tops[channel] - expected_top).abs() < 1e-4,
                "Channel {channel} fill does not match its own signal"
            );
        }
        assert!(
            tops[0] < tops[1],
            "Unequal L/R levels must not be duplicated into identical bars"
        );
    }
    let mut texts = Vec::new();
    text_layouts(tree.root().unwrap(), Point::ZERO, &mut texts);
    let mut readouts: Vec<_> = texts
        .into_iter()
        .filter(|(text, _, _)| text.starts_with("Peak "))
        .collect();
    readouts.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
    assert_eq!(
        readouts.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        ["Peak -2.5 dBFS", "Peak -12.0 dBFS", "Peak +6.0 dBFS"]
    );
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    for (index, (_, origin, size)) in readouts.iter().enumerate() {
        s.status.set(String::new());
        dispatcher.dispatch(
            &mut tree,
            &Event::Mouse(MouseEvent::Moved {
                x: (origin.x + size.width * 0.5) as i32,
                y: (origin.y + size.height * 0.5) as i32,
            }),
        );
        let expected_detail = if index < 2 {
            s.model.borrow().channels[index].peak.get().detail(false)
        } else {
            s.master_peak.get().detail(true)
        };
        assert_eq!(
            s.status.get(),
            expected_detail,
            "Readout hover must reveal the same L/R snapshot"
        );
    }
}

#[test]
fn stereo_meter_peak_and_clip_holds_expire_on_stop_and_reset_for_new_playback() {
    let mut reading = meter::StereoMeter::default();
    reading.advance([1.25, 0.25], 0.05);
    assert_eq!(reading.peak, [1.25, 0.25]);
    assert_eq!(reading.held, [1.25, 0.25]);
    assert_eq!(reading.hold_seconds, [1.; 2]);
    assert_eq!(reading.clip_seconds, [1., 0.]);
    reading.advance([0.125, 0.0625], 0.4);
    assert_eq!(reading.peak, [0.125, 0.0625]);
    assert_eq!(reading.held, [1.25, 0.25]);
    assert!((reading.clip_seconds[0] - 0.6).abs() < 1e-6);
    reading.advance([0.; 2], 0.59);
    assert_eq!(reading.held, [1.25, 0.25]);
    assert!(reading.clip_seconds[0] > 0.);
    reading.advance([0.; 2], 0.02);
    reading.release(3.);
    assert_eq!(reading, meter::StereoMeter::default());

    let s = Daw::new(stereo_meter_project());
    let controls = resonara_core::Controls::new(&s.model.borrow().project);
    controls.tracks[0]
        .peak_left
        .store(1.25f32.to_bits(), Ordering::Relaxed);
    controls.tracks[0]
        .peak_right
        .store(0.25f32.to_bits(), Ordering::Relaxed);
    controls
        .master_peak_left
        .store(2f32.to_bits(), Ordering::Relaxed);
    controls
        .master_peak_right
        .store(0.5f32.to_bits(), Ordering::Relaxed);
    s.update_meters(Some(&controls), 0.05);
    s.stop_audio(false);
    assert_eq!(s.master_peak.get().peak, [0.; 2]);
    assert_eq!(s.master_peak.get().held, [2., 0.5]);
    assert_eq!(s.master_meter.get(), "Peak +6.0 dBFS");
    s.update_meters(None, 1.01);
    s.release_meters(3.);
    assert_eq!(s.master_peak.get(), meter::StereoMeter::default());
    assert!(
        s.model
            .borrow()
            .channels
            .iter()
            .all(|c| c.peak.get() == meter::StereoMeter::default())
    );
    controls
        .master_peak_left
        .store(2f32.to_bits(), Ordering::Relaxed);
    s.update_meters(Some(&controls), 0.05);
    assert!(s.master_peak.get().clip_seconds[0] > 0.);
    s.reset_meters();
    assert_eq!(s.master_peak.get(), meter::StereoMeter::default());
    assert_eq!(s.master_meter.get(), "Peak −∞ dBFS");
}

#[test]
fn waveform_peak_cache_keeps_signed_channels_and_exact_trimmed_bin_edges() {
    let mut raw: Vec<[f32; 2]> = (0..512)
        .map(|i| {
            [
                ((i * 17 % 31) as f32 - 15.) / 32.,
                ((i * 7 % 13) as f32 - 6.) / 16.,
            ]
        })
        .collect();
    raw[128] = [9., -8.];
    raw[255] = [-7., 6.];
    let samples = Arc::new(raw);
    let mut peaks = wave::Peaks::default();
    for (from, to) in [
        (0, 0),
        (0, 512),
        (1, 511),
        (127, 129),
        (129, 255),
        (128, 256),
        (256, 257),
        (511, 700),
        (700, 800),
    ] {
        let start = from.min(samples.len());
        let end = to.min(samples.len()).max(start);
        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        for sample in &samples[start..end] {
            for ch in 0..2 {
                min[ch] = min[ch].min(sample[ch]);
                max[ch] = max[ch].max(sample[ch]);
            }
        }
        if start == end {
            min = [0.; 2];
            max = [0.; 2];
        }
        assert_eq!(
            peaks.envelope(&samples, from, to),
            wave::Envelope { min, max },
            "range {from}..{to}"
        );
    }
    let trimmed = peaks.envelope(&samples, 129, 255);
    assert!(trimmed.min.iter().all(|v| *v > -1.));
    assert!(trimmed.max.iter().all(|v| *v < 1.));
    assert_ne!(trimmed.min[0], trimmed.min[1]);
    assert_ne!(trimmed.max[0], trimmed.max[1]);
    assert_eq!(
        peaks.envelope(&Arc::new(vec![]), 0, 999),
        wave::Envelope {
            min: [0.; 2],
            max: [0.; 2]
        }
    );
}

fn expected_wave_rect(
    x: f32,
    top: f32,
    width: f32,
    height: f32,
    color: Color,
    canvas_width: f32,
    canvas_height: f32,
) -> Vec<SgfxCanvasVertex> {
    let x1 = 2. * x / canvas_width - 1.;
    let x2 = 2. * (x + width) / canvas_width - 1.;
    let y1 = 1. - 2. * top / canvas_height;
    let y2 = 1. - 2. * (top + height) / canvas_height;
    let p = [[x1, y1], [x2, y1], [x2, y2], [x1, y2]];
    [0, 1, 2, 0, 2, 3]
        .into_iter()
        .map(|i| {
            SgfxCanvasVertex::new(
                [p[i][0], p[i][1], 0., 1.],
                [color.r, color.g, color.b, color.a],
            )
        })
        .collect()
}

#[test]
fn waveform_vertices_draw_true_stereo_lanes_and_one_mono_lane_at_each_height() {
    let mut track = project().tracks.remove(0);
    track.clips = vec![Clip {
        source_channels: 2,
        start: 0,
        source_offset: 0,
        frames: 64,
        samples: Arc::new(
            (0..64)
                .map(|i| if i % 2 == 0 { [-0.8, -0.2] } else { [0.8, 0.2] })
                .collect(),
        ),
    }];
    let stereo_audio = track.clips[0].samples.clone();
    for height in [56., 84., 128.] {
        let mut triangle_counts = [0; 2];
        for channels in [1, 2] {
            let mut clip_track = track.clone();
            clip_track.clips[0].source_channels = channels;
            if channels == 1 {
                clip_track.clips[0].samples = Arc::new(
                    (0..64)
                        .map(|i| [if i % 2 == 0 { -0.8 } else { 0.8 }; 2])
                        .collect(),
                );
            }
            let layout = wave::lanes(channels, height);
            assert_eq!(layout.len(), channels as usize);
            let top = 27f32.min(height * 0.45);
            let bottom = height - 10.;
            assert!((layout[0].top - top).abs() < 1e-5);
            assert!((layout.last().unwrap().bottom - bottom).abs() < 1e-5);
            if channels == 2 {
                assert!((layout[1].top - layout[0].bottom - 3.).abs() < 1e-5);
            }
            let mut peaks = wave::Peaks::default();
            let vertices = wave::vertices(
                &clip_track,
                0,
                None,
                64.,
                height,
                0.,
                64. / 8000.,
                wave::tick_step(64. / 8000.),
                8000,
                &mut peaks,
            );
            for (channel, lane) in layout.iter().enumerate() {
                assert_eq!(lane.channel, channel);
                assert!(lane.top < lane.center && lane.center < lane.bottom);
                let level = if channel == 0 { 0.8 } else { 0.2 };
                let top = lane.center - level * lane.amplitude;
                let bottom = lane.center + level * lane.amplitude;
                assert!(top >= lane.top && bottom <= lane.bottom);
                for x in (0..64).step_by(2) {
                    let expected = expected_wave_rect(
                        x as f32,
                        top,
                        1.3,
                        bottom - top,
                        ui::color(0),
                        64.,
                        height,
                    );
                    assert_eq!(
                        vertices
                            .chunks_exact(6)
                            .filter(|chunk| *chunk == expected.as_slice())
                            .count(),
                        1,
                        "missing or duplicated channel {channel} waveform at x={x}, height={height}"
                    );
                }
            }
            let handle = SgfxMeshHandle::new();
            let mesh = wave::base(
                &clip_track,
                0,
                None,
                64.,
                height,
                0.,
                64. / 8000.,
                wave::tick_step(64. / 8000.),
                8000,
                &mut peaks,
                handle,
                42,
            );
            assert_eq!(mesh.handle(), handle);
            assert_eq!(mesh.revision(), 42);
            assert_eq!(mesh.triangle_count(), vertices.len() / 3);
            triangle_counts[channels as usize - 1] = mesh.triangle_count();
        }
        // Stereo adds 32 signal columns, one center line and one separator.
        assert_eq!(triangle_counts[1] - triangle_counts[0], (32 + 2) * 2);
    }
    assert!(Arc::ptr_eq(&track.clips[0].samples, &stereo_audio));
    assert_eq!(track.clips[0].samples[0], [-0.8, -0.2]);
    assert_eq!(track.clips[0].samples[1], [0.8, 0.2]);
}

#[test]
fn waveform_projection_clamps_viewport_and_source_trim_without_exposing_neighbor_samples() {
    let clip = Clip {
        source_channels: 2,
        start: 64,
        source_offset: 30,
        frames: 256,
        samples: Arc::new(vec![[0.; 2]; 512]),
    };
    let projection = wave::project_clip(&clip, 200., 128. / 8192., 64. / 8192., 8192).unwrap();
    assert_eq!((projection.left, projection.right), (0., 200.));
    assert_eq!(projection.source_range(&clip, 0., 200.), 94..158);
    assert_eq!(projection.source_range(&clip, -1e6, 1e6), 30..286);
    for (offset, span) in [(0., 64. / 8192.), (320. / 8192., 100. / 8192.)] {
        assert!(wave::project_clip(&clip, 200., offset, span, 8192).is_none());
    }
    for (width, span, rate) in [(0., 1., 8192), (200., 0., 8192), (200., 1., 0)] {
        assert!(wave::project_clip(&clip, width, 0., span, rate).is_none());
    }
    let mut trimmed = clip.clone();
    trimmed.source_offset = 129;
    trimmed.frames = 126;
    let data = Arc::make_mut(&mut trimmed.samples);
    data[128] = [10., -10.];
    data[255] = [-10., 10.];
    for frame in &mut data[129..255] {
        *frame = [0.25, -0.125];
    }
    let mut peaks = wave::Peaks::default();
    let full = wave::project_clip(&trimmed, 126., 64. / 8192., 126. / 8192., 8192).unwrap();
    assert_eq!(full.source_range(&trimmed, full.left, full.right), 129..255);
    let envelope = peaks.envelope(&trimmed.samples, 129, 255);
    assert_eq!(envelope.min, [0.25, -0.125]);
    assert_eq!(envelope.max, [0.25, -0.125]);
}

#[test]
fn waveform_cache_keys_source_identity_and_metadata_rebuilds_retained_mesh_revision() {
    let mut peaks = wave::Peaks::default();
    let samples = Arc::new(vec![[0.25, -0.5]; 512]);
    let count = Arc::strong_count(&samples);
    peaks.envelope(&samples, 0, 512);
    assert_eq!(Arc::strong_count(&samples), count + 1);
    peaks.envelope(&samples, 127, 385);
    assert_eq!(
        Arc::strong_count(&samples),
        count + 1,
        "one source must retain one cache entry"
    );
    let replacement = Arc::new(vec![[0.75, -0.125]; 512]);
    assert_eq!(peaks.envelope(&replacement, 0, 512).max, [0.75, -0.125]);
    assert_eq!(peaks.envelope(&samples, 0, 512).max, [0.25, -0.5]);
    peaks.retain(&[]);
    assert_eq!(Arc::strong_count(&samples), count);
    assert_eq!(Arc::strong_count(&replacement), 1);

    let mut p = project();
    p.tracks.truncate(1);
    let clip = &mut p.tracks[0].clips[0];
    clip.source_channels = 1;
    clip.samples = Arc::new(vec![[0.25; 2]; 16000]);
    let s = Daw::new(p);
    let mono = s.model.borrow().channels[0].mesh.clone();
    let source = s.model.borrow().project.tracks[0].clips[0].samples.clone();
    s.edit("Mark dual-mono source as stereo", |m| {
        m.project.tracks[0].clips[0].source_channels = 2;
        Ok(())
    });
    let stereo = s.model.borrow().channels[0].mesh.clone();
    assert_eq!(stereo.handle(), mono.handle());
    assert!(stereo.revision() > mono.revision());
    assert!(stereo.triangle_count() > mono.triangle_count());
    assert!(Arc::ptr_eq(
        &source,
        &s.model.borrow().project.tracks[0].clips[0].samples
    ));
    s.refresh(false);
    assert!(Arc::ptr_eq(&stereo, &s.model.borrow().channels[0].mesh));
    s.undo(false);
    assert_eq!(
        s.model.borrow().project.tracks[0].clips[0].source_channels,
        1
    );
    assert_eq!(s.model.borrow().channels[0].mesh.handle(), mono.handle());
    assert_eq!(
        s.model.borrow().channels[0].mesh.triangle_count(),
        mono.triangle_count()
    );
    s.undo(true);
    assert_eq!(
        s.model.borrow().project.tracks[0].clips[0].source_channels,
        2
    );
    assert_eq!(
        s.model.borrow().channels[0].mesh.triangle_count(),
        stereo.triangle_count()
    );
}

fn write_source_channel_wav(path: &Path, channels: u16, samples: &[[f32; 2]]) {
    // Small standard IEEE-float RIFF fixture without an extra app dependency.
    let data_bytes = samples.len() as u32 * u32::from(channels) * 4;
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend((36 + data_bytes).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(3u16.to_le_bytes());
    bytes.extend(channels.to_le_bytes());
    bytes.extend(48000u32.to_le_bytes());
    bytes.extend((48000 * u32::from(channels) * 4).to_le_bytes());
    bytes.extend((channels * 4).to_le_bytes());
    bytes.extend(32u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(data_bytes.to_le_bytes());
    for frame in samples {
        for sample in &frame[..channels as usize] {
            bytes.extend(sample.to_le_bytes());
        }
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn source_channel_import_clone_split_trim_save_open_and_legacy_loading_preserve_audio() {
    use resonara_core::{Controls, Engine};
    let temp = Temp::new();
    let mono = [
        [0.25; 2],
        [-0.5; 2],
        [0.125; 2],
        [0.375; 2],
        [-0.25; 2],
        [0.5; 2],
        [0.0625; 2],
        [-0.125; 2],
    ];
    let stereo = [
        [0.25, -0.5],
        [-0.5, 0.125],
        [0.125, -0.25],
        [0.375, 0.25],
        [-0.25, 0.5],
        [0.5, -0.125],
        [0.0625, -0.375],
        [-0.125, 0.25],
    ];
    let s = Daw::new(Project::default());
    for (channels, samples, filename) in [(1, &mono, "mono.wav"), (2, &stereo, "stereo.wav")] {
        let path = temp.0.join(filename);
        write_source_channel_wav(&path, channels, samples);
        submit(&s, FileAction::Import, &path);
        finish_io(&s);
        let m = s.model.borrow();
        let clip = &m.project.tracks.last().unwrap().clips[0];
        assert_eq!(clip.source_channels, channels);
        assert_eq!(clip.samples.as_slice(), samples);
    }
    let source = s.model.borrow().project.tracks[1].clips[0].samples.clone();
    s.choose(1, Some(0));
    s.duplicate();
    assert_eq!(
        s.model.borrow().project.tracks[2].clips[0].source_channels,
        2
    );
    assert!(Arc::ptr_eq(
        &source,
        &s.model.borrow().project.tracks[2].clips[0].samples
    ));
    s.edit("Split and trim stereo copy", |m| {
        m.project.split(2, 3)?;
        m.project.trim(2, 1, 7)
    });
    let expected = s.model.borrow().project.clone();
    let clips = &expected.tracks[2].clips;
    assert_eq!(clips.len(), 2);
    assert_eq!(
        (clips[0].source_offset, clips[0].start, clips[0].frames),
        (1, 1, 2)
    );
    assert_eq!(
        (clips[1].source_offset, clips[1].start, clips[1].frames),
        (3, 3, 4)
    );
    assert!(
        clips
            .iter()
            .all(|c| c.source_channels == 2 && Arc::ptr_eq(&c.samples, &source))
    );
    assert_eq!(source.as_slice(), stereo);
    assert_eq!(expected.tracks[0].clips[0].source_channels, 1);
    let isolated = Project {
        tracks: vec![expected.tracks[1].clone()],
        master: 1.,
        ..Project::default()
    };
    let mut rendered = [0.; 16];
    Engine::new(&isolated, Arc::new(Controls::new(&isolated)), 48000, 0).render(&mut rendered, 2);
    assert_eq!(
        rendered,
        stereo.into_iter().flatten().collect::<Vec<_>>().as_slice()
    );

    let path = temp.0.join("source-channel-session.json");
    submit(&s, FileAction::Save, &path);
    finish_io(&s);
    s.edit("Remove imported tracks", |m| {
        m.project.tracks.clear();
        Ok(())
    });
    submit(&s, FileAction::Open, &path);
    finish_io(&s);
    assert_project(&s.model.borrow().project, &expected);
    let json = std::fs::read_to_string(&path).unwrap();
    assert!(json.contains("\"source_channels\":1"));
    assert!(json.contains("\"source_channels\":2"));
    let legacy = json
        .replace("\"source_channels\":1,", "")
        .replace("\"source_channels\":2,", "");
    assert!(!legacy.contains("source_channels"));
    let legacy_path = temp.0.join("legacy-channel-session.json");
    std::fs::write(&legacy_path, legacy).unwrap();
    let loaded = Project::load(&legacy_path).unwrap();
    assert!(
        loaded
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .all(|c| c.source_channels == 2)
    );
    for (old, new) in expected.tracks.iter().zip(&loaded.tracks) {
        for (a, b) in old.clips.iter().zip(&new.clips) {
            assert_eq!(a.samples, b.samples);
        }
    }
    let mut old_audio = [0.; 32];
    let mut legacy_audio = [0.; 32];
    Engine::new(&expected, Arc::new(Controls::new(&expected)), 48000, 0).render(&mut old_audio, 2);
    Engine::new(&loaded, Arc::new(Controls::new(&loaded)), 48000, 0).render(&mut legacy_audio, 2);
    assert_eq!(legacy_audio, old_audio);
}

#[test]
fn region_native_labels_follow_source_lanes_and_waveform_clicks_preserve_selection() {
    wait_for_test_font();
    let mut p = project();
    p.tracks[0].name = "Stereo region title should stay within its visible clip".into();
    p.tracks[1].clips[0].source_channels = 1;
    p.tracks[1].clips[0].samples = Arc::new(vec![[0.2; 2]; 16000]);
    let s = Daw::new(p);
    s.view_start.set(0.2);
    s.view_span.set(1.);
    s.refresh(true);
    for (index, kind, labels) in [(0, "STEREO", vec!["L", "R"]), (1, "MONO", vec![])] {
        let mut tree = scarlet_ui::ElementTree::new();
        tree.set_root(s.track_row(index).create_element());
        tree.layout(scarlet_ui::LayoutConstraints::tight(1044., ROW));
        let mut texts = Vec::new();
        text_layouts(tree.root().unwrap(), Point::ZERO, &mut texts);
        assert!(
            texts.iter().any(|(text, _, _)| text.contains(kind)),
            "missing source badge {kind}"
        );
        let lane_texts: Vec<_> = texts
            .iter()
            .filter(|(text, _, _)| matches!(text.as_str(), "L" | "R"))
            .collect();
        assert_eq!(
            lane_texts.iter().map(|v| v.0.as_str()).collect::<Vec<_>>(),
            labels
        );
        if index == 0 {
            let layout = wave::lanes(2, ROW);
            for ((_, origin, size), lane) in lane_texts.iter().zip(&layout) {
                assert!(origin.x >= HEADER && origin.x + size.width <= 1044.);
                assert!(
                    origin.y >= lane.top - 0.01 && origin.y + size.height <= lane.bottom + 0.01
                );
            }
        }
        let before = s.model.borrow().project.clone();
        let path = dispatched_click(&mut tree, (HEADER + 120.) as i32, (ROW - 17.) as i32);
        assert_eq!(
            s.model.borrow().selected,
            index,
            "wrong selected track: {path}"
        );
        assert_eq!(s.model.borrow().clip, Some(0));
        assert_project(&s.model.borrow().project, &before);
    }
}

#[test]
fn selected_waveform_draws_trim_handles_only_at_real_visible_clip_endpoints() {
    let mut track = project().tracks.remove(0);
    track.clips = vec![Clip {
        source_channels: 2,
        start: 8000,
        source_offset: 0,
        frames: 16000,
        samples: Arc::new(vec![[0.25; 2]; 16000]),
    }];
    let source = track.clips[0].samples.clone();
    let width = 200.;
    let height = 84.;
    for channels in [1, 2] {
        track.clips[0].source_channels = channels;
        let lanes = wave::lanes(channels, height);
        let middle = (lanes[0].top + lanes.last().unwrap().bottom) / 2.;
        for (offset, span, visible) in [
            (0., 4., [true, true]),
            (1.5, 2., [false, true]),
            (0.5, 2., [true, false]),
            (1.5, 1., [false, false]),
            // A real endpoint exactly on the viewport boundary remains an edge.
            (1., 2., [true, true]),
        ] {
            let projection =
                wave::project_clip(&track.clips[0], width, offset, span, 8000).unwrap();
            assert_eq!(
                [projection.left_edge_visible, projection.right_edge_visible],
                visible
            );
            let a = projection.left;
            let b = projection.right;
            let mut peaks = wave::Peaks::default();
            for selected in [None, Some(0)] {
                let vertices = wave::vertices(
                    &track,
                    0,
                    selected,
                    width,
                    height,
                    offset,
                    span,
                    wave::tick_step(span),
                    8000,
                    &mut peaks,
                );
                let contains = |expected: &[SgfxCanvasVertex]| {
                    vertices
                        .chunks_exact(6)
                        .filter(|chunk| *chunk == expected)
                        .count()
                };
                for (side, x) in [a + 3., b - 5.].into_iter().enumerate() {
                    let handle =
                        expected_wave_rect(x, middle - 8., 2., 16., ui::TEXT, width, height);
                    assert_eq!(
                        contains(&handle),
                        usize::from(selected.is_some() && visible[side]),
                        "false or missing trim handle: channels={channels}, offset={offset}, span={span}, side={side}, selected={selected:?}"
                    );
                    let edge_x = if side == 0 { a } else { b - 2. };
                    let edge = expected_wave_rect(
                        edge_x,
                        7.,
                        2.,
                        height - 14.,
                        ui::color(0),
                        width,
                        height,
                    );
                    assert_eq!(
                        contains(&edge),
                        usize::from(selected.is_some() && visible[side]),
                        "viewport clipping must not invent a selected vertical clip edge"
                    );
                }
                let horizontal =
                    expected_wave_rect(a, height - 9., b - a, 2., ui::color(0), width, height);
                assert_eq!(
                    contains(&horizontal),
                    usize::from(selected.is_some()),
                    "selected region highlight must remain visible when both true endpoints are offscreen"
                );
            }
        }
    }
    assert!(Arc::ptr_eq(&source, &track.clips[0].samples));
    assert_eq!(
        (
            track.clips[0].start,
            track.clips[0].source_offset,
            track.clips[0].frames
        ),
        (8000, 0, 16000)
    );
}

#[test]
fn playback_only_updates_do_not_notify_root_dependencies_after_layout_settles() {
    use scarlet_ui::pipeline::PipelineOwner;
    let s = Daw::new(project());
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1280., 790.));
    let mut owner = PipelineOwner::with_pipeline_id(tree.pipeline_id());
    for _ in 0..8 {
        owner.flush_with_legacy_paint(&mut tree, Size::new(1280., 790.), false);
    }
    let root_counts = Arc::new(
        (0..s.listenables().len())
            .map(|_| std::sync::atomic::AtomicUsize::new(0))
            .collect::<Vec<_>>(),
    );
    let subscriptions: Vec<_> = s
        .listenables()
        .iter()
        .enumerate()
        .map(|(index, listenable)| {
            let counts = root_counts.clone();
            listenable.subscribe_any(Arc::new(move || {
                counts[index].fetch_add(1, Ordering::Relaxed);
            }))
        })
        .collect();
    let controls = resonara_core::Controls::new(&s.model.borrow().project);
    for tick in 0..4 {
        s.playhead.set(0.25 + tick as f64 * 0.05);
        s.clock.set(ui::time(s.playhead.get()));
        s.update_frames();
        controls.tracks[0]
            .peak_left
            .store((0.25 + tick as f32 * 0.01).to_bits(), Ordering::Relaxed);
        s.update_meters(Some(&controls), 0.05);
        owner.flush_with_legacy_paint(&mut tree, Size::new(1280., 790.), false);
    }
    for (listenable, subscription) in s.listenables().iter().zip(subscriptions) {
        listenable.unsubscribe(subscription);
    }
    let counts = root_counts
        .iter()
        .map(|count| count.load(Ordering::Relaxed))
        .collect::<Vec<_>>();
    assert!(
        counts.iter().all(|count| *count == 0),
        "playback notified root dependencies [revision,size,arrangement,dialog,inspector,inspector_fraction,mixer_fraction,mixer_visible,snap,tool,view_start,view_span,dialog_error]: {counts:?}"
    );
    tree.clear_root();
}

#[test]
fn mounted_rendering_pipeline_playback_ticks_preserve_body_and_static_waveforms() {
    wait_for_test_font();
    let s = Daw::new(project());
    let temp = Temp::new();
    s.profiler.borrow_mut().path = Some(temp.0.join("mounted-profile.json"));
    let mut pipeline = scarlet_ui::RenderingPipeline::new();
    pipeline.set_root(
        Window::new("Playback isolation", s.clone())
            .size(Size::new(1280., 822.))
            .create_element(),
    );
    pipeline.layout_initial();
    for _ in 0..8 {
        let _ = pipeline.render();
    }
    s.profiler.borrow_mut().begin_at(Instant::now());
    let root_revision = s.revision.get();
    let frame_revision = s.last_frame.get();
    let static_content = s
        .model
        .borrow()
        .channels
        .iter()
        .map(|c| (c.mesh.clone(), c.frame.get(), c.canvas, c.playhead_mesh))
        .collect::<Vec<_>>();
    let controls = resonara_core::Controls::new(&s.model.borrow().project);
    for tick in 0..12 {
        s.animate_playhead(0.25 + tick as f64 / 30.);
        if tick % 2 == 0 {
            controls.tracks[0]
                .peak_left
                .store((0.25 + tick as f32 * 0.01).to_bits(), Ordering::Relaxed);
            controls.tracks[0]
                .peak_right
                .store((0.125 + tick as f32 * 0.001).to_bits(), Ordering::Relaxed);
            s.update_meters(Some(&controls), 0.05);
        }
        let _ = pipeline.render();
        assert_eq!(
            s.profiler.borrow().body_builds,
            0,
            "dynamic child updates rebuilt the whole DAW body"
        );
        assert_eq!(s.revision.get(), root_revision);
        assert_eq!(s.last_frame.get(), frame_revision);
        for (channel, (mesh, frame, canvas, playhead)) in
            s.model.borrow().channels.iter().zip(&static_content)
        {
            assert!(Arc::ptr_eq(&channel.mesh, mesh));
            assert!(Arc::ptr_eq(&channel.frame.get(), frame));
            assert_eq!(channel.canvas, *canvas);
            assert_eq!(channel.playhead_mesh, *playhead);
        }
    }
    assert_eq!(s.profiler.borrow().waveform_refreshes, 0);
    assert_eq!(s.profiler.borrow().playhead_updates, 12);
    assert_eq!(s.profiler.borrow().meter_updates, 6);
    pipeline.teardown();
}

#[test]
fn paint_only_animation_leaves_use_fixed_geometry_and_correct_playhead_bounds() {
    use scarlet_ui::{
        renderer::{PaintCommand, PaintContext},
        state::InvalidationKind,
    };
    wait_for_test_font();
    let position = state(9700, 0.);
    let view = animation::Playhead::new(position.clone(), 0.5, 0.25, Size::new(200., 84.));
    assert!(
        view.listenables()
            .iter()
            .all(|s| s.invalidation_kind() == InvalidationKind::Paint)
    );
    let mut element = view.create_element();
    element.layout(scarlet_ui::LayoutConstraints::tight(200., 84.));
    for (at, expected) in [
        (0., None),
        (0.5, Some((0., 1.5))),
        (0.625, Some((100., 1.5))),
        (0.749, Some((199.2, 0.8))),
        (0.75, None),
    ] {
        position.set(at);
        let mut context = PaintContext::new();
        element
            .render_object()
            .unwrap()
            .paint(&mut context, Point::ZERO);
        if let Some((left, width)) = expected {
            let [PaintCommand::FillPath { path, .. }] = context.commands() else {
                panic!("expected one playhead rectangle")
            };
            assert!((path[0].x - left).abs() < 1e-3);
            assert!((path[1].x - path[0].x - width).abs() < 1e-3);
        } else {
            assert!(context.commands().is_empty());
        }
    }
    let text = state(9701, String::from("Peak −∞ dBFS"));
    let readout = animation::Readout::new(text.clone(), 9., ACCENT, Size::new(90., 12.));
    assert!(
        readout
            .listenables()
            .iter()
            .all(|s| s.invalidation_kind() == InvalidationKind::Paint)
    );
    let mut element = readout.create_element();
    element.layout(scarlet_ui::LayoutConstraints::tight(90., 12.));
    for value in ["Peak -48.0 dBFS", "Peak +6.0 dBFS", "Peak −∞ dBFS"] {
        text.set(value.into());
        let mut context = PaintContext::new();
        element
            .render_object()
            .unwrap()
            .paint(&mut context, Point::ZERO);
        let [
            PaintCommand::DrawText {
                text: painted,
                position,
                font_size_px,
                ..
            },
        ] = context.commands()
        else {
            panic!("expected one readout")
        };
        assert_eq!(painted, value);
        let (l, t, r, b) = visible_ink_bounds(painted, *font_size_px);
        assert!((position.x + (l + r) / 2. - 45.).abs() < 1e-4);
        assert!((position.y + (t + b) / 2. - 6.).abs() < 1e-4);
        assert_eq!(element.bounds().size, Size::new(90., 12.));
    }
}

#[test]
fn frame_profile_is_default_off_and_records_bounded_distinct_millisecond_metrics() {
    let start = Instant::now();
    let mut disabled = profiling::Profiler::default();
    disabled.begin_at(start);
    disabled.sync_at(start);
    disabled.presented_at(start);
    disabled.failure();
    assert!(
        !disabled.enabled()
            && !disabled.active()
            && !disabled.expired_at(start + profiling::WINDOW)
    );
    assert_eq!(disabled.frames, 0);
    assert_eq!(disabled.failures, 0);
    assert_eq!(disabled.intervals.capacity(), 0);
    assert_eq!(disabled.submissions.capacity(), 0);
    assert!(disabled.finish("disabled", 0, 100., 100.).is_none());

    let temp = Temp::new();
    let mut p = profiling::Profiler::default();
    p.path = Some(temp.0.join("profile.json"));
    p.begin_at(start);
    p.sync_at(start + Duration::from_millis(2));
    p.presented_at(start + Duration::from_millis(5));
    p.failure();
    p.sync_at(start + Duration::from_millis(15));
    p.presented_at(start + Duration::from_millis(25));
    assert_eq!(p.frames, 2);
    assert_eq!(p.failures, 1);
    assert_eq!(p.intervals, [20.]);
    assert_eq!(p.submissions, [3., 10.]);
    assert!(!p.expired_at(start + profiling::WINDOW - Duration::from_nanos(1)));
    assert!(p.expired_at(start + profiling::WINDOW));
    for i in 0..profiling::CAP + 20 {
        let at = start + Duration::from_millis(30 + i as u64 * 2);
        p.sync_at(at);
        p.presented_at(at + Duration::from_millis(1));
    }
    assert!(p.expired_at(start + Duration::from_secs(10)));
    assert_eq!(p.intervals.len(), profiling::CAP);
    assert_eq!(p.submissions.len(), profiling::CAP);
    assert_eq!(p.intervals.capacity(), profiling::CAP);
    assert_eq!(p.submissions.capacity(), profiling::CAP);
    let path = p.finish("test-30hz", 2, 1280., 790.).unwrap().unwrap();
    let report = std::fs::read_to_string(path).unwrap();
    assert!(report.contains("not GPU completion or scanout"));
    assert!(report.contains("input dispatch and on_idle before on_window_sync"));
    assert!(report.contains("\"render_failures\":1"));
    assert!(report.contains("\"median_ms\":") && report.contains("\"over_33_333_ms\":"));
    assert!(!p.active());
    assert!(
        !p.expired_at(start + profiling::WINDOW),
        "a completed capped run must no longer expire on every idle tick"
    );
    assert!(p.finish("again", 2, 1280., 790.).is_none());
    p.begin_at(start);
    assert_eq!(
        (p.frames, p.failures, p.intervals.len(), p.submissions.len()),
        (0, 0, 0, 0)
    );
}

#[test]
fn frame_profile_quantiles_and_separate_animation_budgets_are_explicit() {
    let empty = profiling::stats(&[]);
    assert_eq!(
        (
            empty.count,
            empty.median,
            empty.p95,
            empty.p99,
            empty.max,
            empty.over_budget,
            empty.over_33_ms,
            empty.over_50_ms
        ),
        (0, 0., 0., 0., 0., 0, 0, 0)
    );
    // The collector uses the higher observed-sample quantile at ceil((n-1)*q).
    let mut values: Vec<_> = (0..101).map(|v| v as f64).collect();
    values.reverse();
    let stats = profiling::stats(&values);
    assert_eq!(
        (stats.count, stats.median, stats.p95, stats.p99, stats.max),
        (101, 50., 95., 99., 100.)
    );
    assert_eq!(
        (stats.over_budget, stats.over_33_ms, stats.over_50_ms),
        (84, 67, 50)
    );
    let boundary = profiling::stats(&[1000. / 60., 1000. / 30., 50.]);
    assert_eq!(
        (
            boundary.over_budget,
            boundary.over_33_ms,
            boundary.over_50_ms
        ),
        (2, 1, 0)
    );
    assert_eq!(
        animation::PLAYHEAD_INTERVAL,
        Duration::from_nanos(33_333_333)
    );
    assert_eq!(animation::METER_INTERVAL, Duration::from_nanos(33_333_333));
    assert_eq!(animation::PLAYHEAD_INTERVAL, animation::METER_INTERVAL);
}

#[test]
fn bpm_field_next_to_counter_accepts_enter_without_global_shortcuts() {
    wait_for_test_font();
    let s = Daw::new(project());
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1000., 790.));
    let mut fields = Vec::new();
    control_bounds(
        tree.root().unwrap(),
        Point::ZERO,
        "::views::text_field::TextFieldRenderObject",
        &mut fields,
    );
    let (origin, size) = fields[0];
    let mut labels = Vec::new();
    text_layouts(tree.root().unwrap(), Point::ZERO, &mut labels);
    let (_, counter, counter_size) = labels
        .iter()
        .find(|(text, _, _)| text == "001.01.000")
        .unwrap();
    assert!(origin.x >= counter.x + counter_size.width);
    assert!(origin.x - (counter.x + counter_size.width) < 24.);
    assert!(origin.x + size.width <= 1000.);
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    // Use the same dispatcher for focus ownership and subsequent key delivery.
    for event in [
        MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: (origin.x + 20.) as i32,
            y: (origin.y + 10.) as i32,
            click_count: 1,
        },
        MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: (origin.x + 20.) as i32,
            y: (origin.y + 10.) as i32,
            click_count: 1,
        },
    ] {
        dispatcher.dispatch(&mut tree, &Event::Mouse(event));
    }
    physical_key_and_text(
        &mut dispatcher,
        &mut tree,
        'a',
        KeyModifiers {
            super_key: true,
            ..KeyModifiers::default()
        },
    );
    for c in "96.5".chars() {
        physical_key_and_text(&mut dispatcher, &mut tree, c, KeyModifiers::default());
    }
    assert_eq!(s.tempo_input.get(), "96.5");
    assert!(dispatcher.dispatch(
        &mut tree,
        &Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Enter,
            modifiers: KeyModifiers::default()
        })
    ));
    assert_eq!(s.model.borrow().project.tempo, 96.5);
    assert_eq!(s.model.borrow().project.tracks.len(), 2);
    assert!(s.dialog.get() == Dialog::None);
}

#[test]
fn master_pointer_drag_updates_thumb_state_until_release_and_groups_history() {
    let s = Daw::new(project());
    let initial_master = s.master.get();
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.mixer().create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(600., 286.));
    let mut faders = Vec::new();
    control_bounds(
        tree.root().unwrap(),
        Point::ZERO,
        "::fader::FaderRender",
        &mut faders,
    );
    let (origin, size) = *faders.last().unwrap();
    let geometry = fader::Geometry::new(size);
    let x = (origin.x + geometry.axis).round() as i32;
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    let y = (origin.y + geometry.y(0.25)).round() as i32;
    assert!(dispatcher.dispatch(
        &mut tree,
        &Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x,
            y,
            click_count: 1
        })
    ));
    for fraction in [0.35, 0.7, 0.5] {
        let y = (origin.y + geometry.y(fraction)).round() as i32;
        assert!(dispatcher.dispatch(&mut tree, &Event::Mouse(MouseEvent::Moved { x, y })));
        assert!(s.master_dragging.get());
        let expected = fader::gain_at(y as f32 - origin.y, size.height);
        assert!(
            (s.master.get() - expected).abs() < 1e-6,
            "master thumb stayed at its previous value while dragging"
        );
        assert_eq!(s.master.get(), s.model.borrow().project.master);
        assert!(s.model.borrow().undo.is_empty());
    }
    assert!(dispatcher.dispatch(
        &mut tree,
        &Event::Mouse(MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x,
            y,
            click_count: 1
        })
    ));
    assert!(!s.master_dragging.get());
    let final_gain = s.master.get();
    s.finish_mix();
    assert_eq!(s.model.borrow().undo.len(), 1);
    s.undo(false);
    assert_eq!(s.master.get(), initial_master);
    s.undo(true);
    assert_eq!(s.master.get(), final_gain);
}

#[test]
fn tempo_entry_is_undoable_and_display_switch_keeps_sample_positions() {
    let s = Daw::new(project());
    let original = s.model.borrow().project.clone();
    assert_eq!(s.time_format.get(), timeline::Format::Bars);
    s.seek(2.);
    assert_eq!(s.clock.get(), "002.01.000");
    for invalid in ["abc", "NaN", "inf", "0", "500"] {
        s.tempo_input.set(invalid.into());
        s.submit_tempo();
        assert_eq!(s.model.borrow().project.tempo, 120.);
        assert!(s.model.borrow().undo.is_empty());
    }
    s.tempo_input.set("60".into());
    s.submit_tempo();
    assert_eq!(s.clock.get(), "001.03.000");
    assert_eq!(
        s.model.borrow().project.tracks[0].clips[0].start,
        original.tracks[0].clips[0].start
    );
    assert!(Arc::ptr_eq(
        &s.model.borrow().project.tracks[0].clips[0].samples,
        &original.tracks[0].clips[0].samples
    ));
    s.cycle_time_format();
    assert_eq!(s.clock.get(), "00:02.000");
    s.cycle_time_format();
    assert_eq!(s.clock.get(), "16000");
    assert_eq!(s.playhead.get(), 2.);
    assert_eq!(s.cursor.get(), "2.000");
    s.cycle_time_format();
    s.undo(false);
    assert_eq!(s.tempo_input.get(), "120");
    assert_eq!(s.clock.get(), "002.01.000");
    s.undo(true);
    assert_eq!(s.tempo_input.get(), "60");
}

#[test]
fn hiding_mixer_fills_arrangement_and_restores_split_after_resize() {
    wait_for_test_font();
    for inspector in [true, false] {
        for empty in [true, false] {
            let s = Daw::new(if empty { Project::default() } else { project() });
            s.inspector.set(inspector);
            s.view_start.set(0.25);
            let mut pipeline = scarlet_ui::RenderingPipeline::new();
            pipeline.set_root(
                Window::new("Mixer visibility", s.clone())
                    .size(Size::new(1280., 822.))
                    .create_element(),
            );
            pipeline.layout_initial();
            for _ in 0..8 {
                let _ = pipeline.render();
            }
            let split = s.mixer_fraction.get();
            let before = s.arrangement_size.get().height;
            s.mixer_visible.set(false);
            for _ in 0..8 {
                let _ = pipeline.render();
            }
            let full = s.size.get().height - 174.;
            assert!(
                (s.arrangement_size.get().height - full).abs() <= 1.,
                "hidden mixer left unused space: inspector={inspector}, empty={empty}, arrangement={:?}, expected height={full}",
                s.arrangement_size.get()
            );
            assert!(s.arrangement_size.get().height > before + 280.);
            assert_eq!(s.mixer_fraction.get(), split);

            s.sync_content_size(Size::new(1280., 1032.));
            pipeline.resize(Size::new(1280., 1032.));
            for _ in 0..8 {
                let _ = pipeline.render();
            }
            assert!((s.arrangement_size.get().height - 826.).abs() <= 1.);
            assert_eq!(s.mixer_fraction.get(), split);
            s.mixer_visible.set(true);
            for _ in 0..8 {
                let _ = pipeline.render();
            }
            let available = s.size.get().height - 174. - 4.;
            let mixer_height = available - s.arrangement_size.get().height;
            assert!(mixer_height <= MIXER_MAX_HEIGHT + 1.);
            assert!(mixer_height >= MIXER_MIN_HEIGHT - 1.);
            assert!(
                (s.mixer_fraction.get() - s.arrangement_size.get().height / available).abs()
                    < 0.002
            );
            assert_eq!(s.view_start.get(), 0.25);
            pipeline.teardown();
        }
    }
}

#[test]
fn native_resize_notifications_coalesce_to_final_size_without_duplicate_scene_dependencies() {
    wait_for_test_font();
    let mut s = Daw::new(project());
    let temp = Temp::new();
    s.profiler.borrow_mut().path = Some(temp.0.join("resize-profile.json"));
    assert!(
        s.scene_listenables(&"resonara".into()).unwrap().is_empty(),
        "the fixed scene/window descriptor must not duplicate Daw content subscriptions"
    );
    let mut pipeline = scarlet_ui::RenderingPipeline::new();
    pipeline.set_root(
        Window::new("Resize isolation", s.clone())
            .size(Size::new(1280., 822.))
            .create_element(),
    );
    pipeline.layout_initial();
    for _ in 0..8 {
        let _ = pipeline.render();
    }
    let notifications = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let received = notifications.clone();
    let subscription = s.size.subscribe_any(Arc::new(move || {
        received.fetch_add(1, Ordering::Relaxed);
    }));
    s.profiler.borrow_mut().begin_at(Instant::now());
    let original = s.size.get();
    let ctx = window();
    for _ in 0..12 {
        // Queued platform resize echoes may disagree. Only the authoritative
        // size sampled after event draining should drive the content tree.
        s.on_window_resize(&ctx, 1188, 848);
        s.on_window_resize(&ctx, 1280, 860);
        s.on_window_resize(&ctx, 1188, 816);
        assert_eq!(s.size.get(), original);
        s.sync_content_size(Size::new(original.width, original.height + 32.));
        s.animate_playhead(0.3);
        let _ = pipeline.render();
    }
    assert_eq!(notifications.load(Ordering::Relaxed), 0);
    assert_eq!(s.profiler.borrow().body_builds, 0);
    s.sync_content_size(Size::new(1300., 842.));
    assert_eq!(s.size.get(), Size::new(1300., 810.));
    assert_eq!(notifications.load(Ordering::Relaxed), 1);
    s.sync_content_size(Size::new(1300., 842.));
    assert_eq!(
        notifications.load(Ordering::Relaxed),
        1,
        "equal sync size must not notify"
    );
    s.size.unsubscribe(subscription);
    pipeline.teardown();
}

#[test]
fn gain_scale_clicks_set_exact_track_and_master_values_without_touching_neighbors() {
    for target in [0, 2] {
        for db in [6., 0., -6., -18., -48., -100.] {
            let mut p = stereo_meter_project();
            p.tracks[0].gain = 0.7;
            p.master = 0.8;
            let original = p.clone();
            let s = Daw::new(p);
            s.choose(1, None);
            let mut tree = scarlet_ui::ElementTree::new();
            tree.set_root(s.mixer().create_element());
            tree.layout(scarlet_ui::LayoutConstraints::tight(600., 286.));
            let mut bounds = Vec::new();
            control_bounds(
                tree.root().unwrap(),
                Point::ZERO,
                "::fader::FaderRender",
                &mut bounds,
            );
            let (origin, size) = bounds[target];
            let g = fader::Geometry::new(size);
            let x = (origin.x + g.gain_label - 5.).round() as i32;
            let y = (origin.y + g.y(fader::gain_fraction(db))).round() as i32;
            let expected = if db == -100. {
                0.
            } else {
                10f32.powf(db / 20.)
            };
            let mut dispatcher = scarlet_ui::EventDispatcher::new();
            for event in [
                MouseEvent::ButtonPressed {
                    button: MouseButton::Left,
                    x,
                    y,
                    click_count: 1,
                },
                MouseEvent::Moved { x: x + 2, y: y - 2 },
                MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x: x + 2,
                    y: y - 2,
                    click_count: 1,
                },
            ] {
                assert!(dispatcher.dispatch(&mut tree, &Event::Mouse(event)));
            }
            s.finish_mix();
            let mut wanted = original.clone();
            if target == 0 {
                wanted.tracks[0].gain = expected;
            } else {
                wanted.master = expected;
            }
            assert_project(&s.model.borrow().project, &wanted);
            assert_eq!(s.master.get(), wanted.master);
            assert_gain_controls_coherent(&s);
            assert_eq!(s.model.borrow().selected, 1);
            assert_eq!(s.model.borrow().undo.len(), 1);
            assert!(s.dirty());
            let actual = s.model.borrow().project.clone();
            let controls = Arc::new(resonara_core::Controls::new(&actual));
            let mut output = [0.; 16];
            resonara_core::Engine::new(&actual, controls, actual.sample_rate, 0)
                .render(&mut output, 2);
            let mut reference = [0.; 16];
            resonara_core::Engine::new(
                &wanted,
                Arc::new(resonara_core::Controls::new(&wanted)),
                wanted.sample_rate,
                0,
            )
            .render(&mut reference, 2);
            assert_eq!(output, reference);
            s.undo(false);
            assert_project(&s.model.borrow().project, &original);
            assert!(!s.dirty());
            s.undo(true);
            assert_project(&s.model.borrow().project, &wanted);
            assert_gain_controls_coherent(&s);
        }
    }
}

#[test]
fn gain_scale_targets_ignore_meter_and_gaps_and_cancel_restores_gain() {
    let gain = state(9400, 0.8);
    let dragging = state(9401, false);
    let changed = gain.clone();
    let fader = fader::Fader::new(
        gain.clone(),
        state(9402, meter::StereoMeter::default()),
        dragging.clone(),
        state(9403, false),
        move |v| changed.set(v),
    );
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(fader.create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(90., 112.));
    let g = fader::Geometry::new(Size::new(90., 112.));
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    for (x, y) in [
        (g.meter_left, 30.),
        (g.peak_label, g.y(fader::peak_fraction(-6.))),
        (g.gain_label, 18.),
        (g.axis + 15., 60.),
    ] {
        for e in [
            MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: x as i32,
                y: y as i32,
                click_count: 1,
            },
            MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: x as i32,
                y: y as i32,
                click_count: 1,
            },
        ] {
            dispatcher.dispatch(&mut tree, &Event::Mouse(e));
        }
        assert_eq!(gain.get(), 0.8);
        assert!(!dragging.get());
    }
    let x = g.gain_label as i32;
    let y = g.y(fader::gain_fraction(-6.)).round() as i32;
    assert!(dispatcher.dispatch(
        &mut tree,
        &Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x,
            y,
            click_count: 1
        })
    ));
    assert_eq!(gain.get(), 10f32.powf(-6. / 20.));
    assert!(dispatcher.dispatch(
        &mut tree,
        &Event::Mouse(MouseEvent::ButtonCancelled {
            button: MouseButton::Left,
            x,
            y
        })
    ));
    assert_eq!(gain.get(), 0.8);
    assert!(!dragging.get());
}

fn context_tree(s: &Daw) -> scarlet_ui::ElementTree {
    s.size.set(Size::new(1000., 790.));
    s.inspector.set(false);
    s.mixer_visible.set(false);
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1000., 790.));
    tree
}
fn context_point(tree: &scarlet_ui::ElementTree, target: Option<usize>) -> Point {
    (0..790)
        .step_by(4)
        .find_map(|y| {
            let p = Point::new(100., y as f32);
            (ui::track_at(tree.root().unwrap(), Point::ZERO, p) == Some(target)).then_some(p)
        })
        .expect("track header or blank column must have a secondary-click target")
}
fn dispatch_mouse(
    dispatcher: &mut scarlet_ui::EventDispatcher,
    tree: &mut scarlet_ui::ElementTree,
    button: MouseButton,
    p: Point,
    pressed: bool,
) {
    let e = if pressed {
        MouseEvent::ButtonPressed {
            button,
            x: p.x as i32,
            y: p.y as i32,
            click_count: 1,
        }
    } else {
        MouseEvent::ButtonReleased {
            button,
            x: p.x as i32,
            y: p.y as i32,
            click_count: 1,
        }
    };
    dispatcher.dispatch(tree, &Event::Mouse(e));
}
fn rebuild_context(tree: &mut scarlet_ui::ElementTree) {
    tree.root_mut().unwrap().rebuild();
    tree.layout(scarlet_ui::LayoutConstraints::tight(1000., 790.));
}
#[test]
fn track_context_menu_dispatches_to_clicked_header_and_undoes_duplicate_and_delete() {
    for (row, action_index) in [(0, 1), (1, 2)] {
        let s = Daw::new(project());
        s.choose(1 - row, Some(0));
        let mut tree = context_tree(&s);
        let mut dispatcher = scarlet_ui::EventDispatcher::new();
        let p = context_point(&tree, Some(row));
        dispatch_mouse(&mut dispatcher, &mut tree, MouseButton::Right, p, true);
        dispatch_mouse(&mut dispatcher, &mut tree, MouseButton::Right, p, false);
        let menu = s.track_menu.get().expect("right-click opens a menu");
        assert_eq!(menu.target, Some(row));
        assert_eq!(s.model.borrow().selected, row);
        assert!(s.model.borrow().undo.is_empty());
        assert!(!s.dirty());
        rebuild_context(&mut tree);
        let action = Point::new(
            menu.anchor.x + 80.,
            menu.anchor.y + 6. + action_index as f32 * 32. + 16.,
        );
        dispatch_mouse(&mut dispatcher, &mut tree, MouseButton::Left, action, true);
        dispatch_mouse(&mut dispatcher, &mut tree, MouseButton::Left, action, false);
        assert!(s.track_menu.get().is_none());
        let edited = s.model.borrow().project.clone();
        if action_index == 1 {
            assert_eq!(edited.tracks.len(), 3);
            assert_eq!(edited.tracks[1].name, "Track 0 copy");
            assert!(Arc::ptr_eq(
                &edited.tracks[0].clips[0].samples,
                &edited.tracks[1].clips[0].samples
            ));
        } else {
            assert_eq!(edited.tracks.len(), 1);
            assert_eq!(edited.tracks[0].name, "Track 0");
        }
        assert_eq!(s.model.borrow().undo.len(), 1);
        assert!(s.dirty());
        s.undo(false);
        assert_project(&s.model.borrow().project, &project());
        assert!(!s.dirty());
        s.undo(true);
        assert_project(&s.model.borrow().project, &edited);
    }
}
#[test]
fn track_menu_escape_outside_cancel_control_click_and_shortcuts_preserve_roles() {
    let s = Daw::new(project());
    let mut tree = context_tree(&s);
    let p = context_point(&tree, Some(1));
    let event = Event::Mouse(MouseEvent::ButtonPressed {
        button: MouseButton::Left,
        x: p.x as i32,
        y: p.y as i32,
        click_count: 1,
    });
    assert!(s.track_context_event(tree.root().unwrap(), &event, true));
    assert_eq!(s.track_menu.get().unwrap().target, Some(1));
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    assert!(dispatcher.dispatch(
        &mut tree,
        &Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Escape,
            modifiers: KeyModifiers::default()
        })
    ));
    assert!(s.track_menu.get().is_none());
    let wave = Event::Mouse(MouseEvent::ButtonPressed {
        button: MouseButton::Left,
        x: 400,
        y: p.y as i32,
        click_count: 1,
    });
    assert!(
        !s.track_context_event(tree.root().unwrap(), &wave, true),
        "Control-click in the audio timeline remains outside track management"
    );
    for cancel in [false, true] {
        s.open_track_menu(Some(0), p);
        rebuild_context(&mut tree);
        let event = if cancel {
            MouseEvent::ButtonCancelled {
                button: MouseButton::Right,
                x: 900,
                y: 50,
            }
        } else {
            MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: 900,
                y: 50,
                click_count: 1,
            }
        };
        dispatcher.dispatch(&mut tree, &Event::Mouse(event));
        assert!(s.track_menu.get().is_none());
        assert_project(&s.model.borrow().project, &project());
        assert!(!s.dirty());
    }
    rebuild_context(&mut tree);
    assert!(s.handle_key(KeyEvent::Pressed {
        keycode: KeyCode::Char('d'),
        modifiers: KeyModifiers {
            super_key: true,
            ..KeyModifiers::default()
        }
    }));
    assert_eq!(s.model.borrow().project.tracks.len(), 3);
    let mut inspector = s.inspector_panel().create_element();
    inspector.layout(scarlet_ui::LayoutConstraints::tight(220., 630.));
    let mut paint = scarlet_ui::renderer::PaintContext::new();
    paint_text_recursive(inspector.as_ref(), &mut paint);
    assert!(!paint.commands().iter().any(|c|matches!(c,scarlet_ui::renderer::PaintCommand::DrawText{text,..} if text=="Duplicate track"||text=="Delete track"||text=="Split at playhead")));
}
fn paint_text_recursive<'a>(
    e: &'a dyn scarlet_ui::Element,
    ctx: &mut scarlet_ui::renderer::PaintContext<'a>,
) {
    if let Some(r) = e.render_object() {
        r.paint(ctx, Point::ZERO);
    }
    for child in e.children() {
        paint_text_recursive(child.as_ref(), ctx);
    }
}
#[test]
fn blank_track_column_and_plus_add_empty_tracks_with_history_and_persistence() {
    for initial in [Project::default(), project()] {
        let s = Daw::new(initial.clone());
        let mut tree = context_tree(&s);
        let mut dispatcher = scarlet_ui::EventDispatcher::new();
        let blank = context_point(&tree, None);
        dispatch_mouse(&mut dispatcher, &mut tree, MouseButton::Right, blank, true);
        dispatch_mouse(&mut dispatcher, &mut tree, MouseButton::Right, blank, false);
        assert_eq!(s.track_menu.get().unwrap().target, None);
        rebuild_context(&mut tree);
        assert!(dispatcher.dispatch(
            &mut tree,
            &Event::Keyboard(KeyEvent::Pressed {
                keycode: KeyCode::Enter,
                modifiers: KeyModifiers::default()
            })
        ));
        let edited = s.model.borrow().project.clone();
        assert_eq!(edited.tracks.len(), initial.tracks.len() + 1);
        assert!(edited.tracks.last().unwrap().clips.is_empty());
        assert_eq!(s.model.borrow().selected, initial.tracks.len());
        assert!(s.dirty());
        let temp = Temp::new();
        edited.save(&temp.0.join("context.json")).unwrap();
        assert_project(
            &Project::load(&temp.0.join("context.json")).unwrap(),
            &edited,
        );
        s.undo(false);
        assert_project(&s.model.borrow().project, &initial);
        assert!(!s.dirty());
        s.undo(true);
        assert_project(&s.model.borrow().project, &edited);
        s.delete(true);
        assert_project(&s.model.borrow().project, &initial);
        // The small + in the track-list header uses the same add operation.
        let mut ruler = scarlet_ui::ElementTree::new();
        ruler.set_root(s.ruler().create_element());
        ruler.layout(scarlet_ui::LayoutConstraints::tight(1000., 30.));
        dispatched_click(&mut ruler, HEADER as i32 - 18, 15);
        assert_eq!(
            s.model.borrow().project.tracks.len(),
            initial.tracks.len() + 1
        );
    }
}
#[test]
fn ruler_capture_follows_drag_without_rebuilding_waveforms_and_clamps_and_cancels() {
    let s = Daw::new(project());
    s.snap.set(false); // This regression checks continuous pointer capture; snapped input is tested separately.
    s.seek(0.4);
    s.view_start.set(0.2);
    s.view_span.set(0.8);
    s.arrangement_size.set(Size::new(1000., 500.));
    s.refresh(true);
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.ruler().create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1000., 30.));
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    let p = Point::new(HEADER + 395., 15.);
    dispatch_mouse(&mut dispatcher, &mut tree, MouseButton::Left, p, true);
    assert!((s.playhead.get() - 0.6).abs() < 1e-6);
    let mesh = s.model.borrow().channels[0].mesh.clone();
    let frame = s.model.borrow().channels[0].frame.get();
    let revision = s.revision.get();
    for (x, expected) in [
        (HEADER as i32 + 100, 0.2 + 100. / 790. * 0.8),
        (-200, 0.2),
        (2000, 1.),
    ] {
        assert!(dispatcher.dispatch(&mut tree, &Event::Mouse(MouseEvent::Moved { x, y: 1000 })));
        assert!((s.playhead.get() - expected).abs() < 1e-6);
        assert!(Arc::ptr_eq(&mesh, &s.model.borrow().channels[0].mesh));
        assert!(Arc::ptr_eq(
            &frame,
            &s.model.borrow().channels[0].frame.get()
        ));
        assert_eq!(s.revision.get(), revision);
        assert!(s.model.borrow().audio.is_none());
    }
    assert!(dispatcher.dispatch(
        &mut tree,
        &Event::Mouse(MouseEvent::ButtonCancelled {
            button: MouseButton::Left,
            x: 2000,
            y: 1000
        })
    ));
    assert!((s.playhead.get() - 0.4).abs() < 1e-6);
    assert!(s.ruler_drag.borrow().is_none());
    dispatch_mouse(&mut dispatcher, &mut tree, MouseButton::Left, p, true);
    dispatch_mouse(
        &mut dispatcher,
        &mut tree,
        MouseButton::Left,
        Point::new(2000., 1000.),
        false,
    );
    assert!((s.playhead.get() - 1.).abs() < 1e-6);
    assert!(s.ruler_drag.borrow().is_none());
    assert!(!s.dirty());
    s.ruler_event(
        &Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 1000,
            y: 15,
            click_count: 1,
        }),
        0.,
        5.,
        790.,
    );
    assert!((s.playhead.get() - 1.2).abs() < 1e-6);
    assert!(s.cancel_ruler_drag());
}

#[test]
fn playing_ruler_drag_keeps_stream_alive_and_seeks_once_on_release() {
    let mut s = Daw::new(project());
    s.seek(0.4);
    s.view_span.set(1.);
    s.play();
    let original = s.model.borrow().audio.as_ref().unwrap().controls.clone();
    let revision = s.revision.get();
    let mesh = s.model.borrow().channels[0].mesh.clone();
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.ruler().create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1044., 30.));
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    dispatch_mouse(
        &mut dispatcher,
        &mut tree,
        MouseButton::Left,
        Point::new(HEADER + 200., 15.),
        true,
    );
    assert_eq!(s.revision.get(), revision);
    assert!(Arc::ptr_eq(
        &original,
        &s.model.borrow().audio.as_ref().unwrap().controls
    ));
    assert!(dispatcher.dispatch(
        &mut tree,
        &Event::Mouse(MouseEvent::Moved {
            x: HEADER as i32 + 600,
            y: 400
        })
    ));
    let preview = s.playhead.get();
    s.model.borrow().audio.as_ref().unwrap().render(800);
    s.last_playhead
        .set(Instant::now() - Duration::from_millis(40));
    s.on_idle();
    assert!((original.position.load(Ordering::Relaxed) as f64 / 8000. - 0.5).abs() < 1e-6);
    assert_eq!(s.playhead.get(), preview);
    assert!(Arc::ptr_eq(&mesh, &s.model.borrow().channels[0].mesh));
    assert_eq!(s.revision.get(), revision);
    dispatch_mouse(
        &mut dispatcher,
        &mut tree,
        MouseButton::Left,
        Point::new(HEADER + 600., 400.),
        false,
    );
    let resumed = s.model.borrow().audio.as_ref().unwrap().controls.clone();
    assert!(Arc::ptr_eq(&original, &resumed));
    assert!(resumed.playing.load(Ordering::Relaxed));
    assert!((resumed.position.load(Ordering::Relaxed) as f64 / 8000. - preview).abs() < 0.002);
    s.model.borrow().audio.as_ref().unwrap().render(80);
    assert!(resumed.position.load(Ordering::Relaxed) > (preview * 8000.) as u64);
    assert!(s.ruler_drag.borrow().is_none());
    assert!(!s.dirty());
}

#[test]
fn cancelling_playing_ruler_drag_preserves_current_audio_position_and_eof_can_seek_again() {
    let mut s = Daw::new(project());
    s.seek(0.4);
    s.play();
    let original = s.model.borrow().audio.as_ref().unwrap().controls.clone();
    let press = Event::Mouse(MouseEvent::ButtonPressed {
        button: MouseButton::Left,
        x: 300,
        y: 15,
        click_count: 1,
    });
    assert!(s.ruler_event(&press, 0., 1., 800.));
    s.model.borrow().audio.as_ref().unwrap().render(800);
    assert!(s.cancel_ruler_drag());
    assert!(Arc::ptr_eq(
        &original,
        &s.model.borrow().audio.as_ref().unwrap().controls
    ));
    assert!((s.playhead.get() - 0.5).abs() < 1e-6);
    assert!(s.ruler_event(&press, 0., 1., 800.));
    s.model.borrow().audio.as_ref().unwrap().render(8000);
    assert!(!original.playing.load(Ordering::Relaxed));
    s.last_playhead
        .set(Instant::now() - Duration::from_millis(40));
    s.on_idle();
    assert!(s.model.borrow().audio.is_some());
    assert!(s.ruler_event(
        &Event::Mouse(MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: 400,
            y: 15,
            click_count: 1
        }),
        0.,
        1.,
        800.
    ));
    let audio = s.model.borrow();
    let audio = audio.audio.as_ref().unwrap();
    assert!(audio.controls.playing.load(Ordering::Relaxed));
    assert_eq!(audio.controls.position.load(Ordering::Relaxed), 4000);
}

#[test]
fn manual_timeline_scroll_suspends_follow_until_transport_restarts() {
    use scarlet_ui::event::{ScrollSource, WheelPhase};
    let s = Daw::new(project());
    s.view_span.set(0.5);
    s.toggle_follow();
    s.play();
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.arrangement().create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1044., 420.));
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    assert!(dispatcher.dispatch(
        &mut tree,
        &Event::Mouse(MouseEvent::Wheel {
            delta_x: -120,
            delta_y: 0,
            x: 310,
            y: 72,
            phase: WheelPhase::Started,
            source: ScrollSource::Trackpad,
        })
    ));
    let start = s.view_start.get();
    assert!(s.follow_playhead.get());
    assert!(s.follow_suspended.get());
    s.animate_playhead(1.);
    assert_eq!(s.view_start.get(), start);
    s.stop_audio(true);
    s.cursor.set("0.900".into());
    s.play();
    assert!(!s.follow_suspended.get());
    assert!((s.view_start.get() - 0.85).abs() < 1e-6);
    s.scroll_timeline(-0.3);
    assert!(s.follow_suspended.get());
    let start = s.view_start.get();
    s.animate_playhead(0.);
    assert_eq!(s.view_start.get(), start);
    s.toggle_follow();
    s.toggle_follow();
    assert!(!s.follow_suspended.get());
    assert_eq!(s.view_start.get(), 0.);
}

#[test]
fn mixer_divider_resizes_meter_and_fader_within_the_content_limits() {
    wait_for_test_font();
    let s = Daw::new(project());
    s.sync_content_size(Size::new(1280., 1032.));
    let mut pipeline = scarlet_ui::RenderingPipeline::new();
    pipeline.set_root(
        Window::new("Resizable meters", s.clone())
            .size(Size::new(1280., 1032.))
            .create_element(),
    );
    pipeline.layout_initial();
    for _ in 0..8 {
        let _ = pipeline.render();
    }
    let available = s.size.get().height - 174. - 4.;
    for requested in [0., 1., 0.55, 0.65] {
        s.mixer_fraction.set(requested);
        for _ in 0..8 {
            let _ = pipeline.render();
        }
        let height = available - s.arrangement_size.get().height;
        assert!(
            height >= MIXER_MIN_HEIGHT - 1. && height <= MIXER_MAX_HEIGHT + 1.,
            "mixer height {height}"
        );
        let mut faders = Vec::new();
        control_bounds(
            pipeline.element_tree().root().unwrap(),
            Point::ZERO,
            "::fader::FaderRender",
            &mut faders,
        );
        assert_eq!(faders.len(), 4);

        for (_, size) in faders {
            assert!((size.height - s.mixer_fader_height()).abs() <= 1.);
            assert!(
                size.height >= MIXER_FADER_MIN_HEIGHT - 1.
                    && size.height <= MIXER_FADER_MAX_HEIGHT + 1.
            );
            let g = fader::Geometry::new(size);
            assert!((fader::gain_at(g.y(fader::gain_fraction(0.)), size.height) - 1.).abs() < 1e-5);
        }
    }
    pipeline.teardown();
}
#[test]
fn follow_pages_keep_playhead_visible_and_fixed_mode_preserves_retained_geometry() {
    wait_for_test_font();
    let s = Daw::new(Project::demo());
    s.view_span.set(1.);
    s.refresh(true);
    let mut pipeline = scarlet_ui::RenderingPipeline::new();
    pipeline.set_root(
        Window::new("Follow mode", s.clone())
            .size(Size::new(1280., 860.))
            .create_element(),
    );
    pipeline.layout_initial();
    for _ in 0..8 {
        let _ = pipeline.render();
    }
    let fixed = s.view_start.get();
    let mesh = s.model.borrow().channels[0].mesh.clone();
    for position in [0.25, 0.5, 1., 2., 3.] {
        s.animate_playhead(position);
        let _ = pipeline.render();
    }
    assert_eq!(s.view_start.get(), fixed);
    assert!(Arc::ptr_eq(&mesh, &s.model.borrow().channels[0].mesh));
    s.animate_playhead(0.);
    s.toggle_follow();
    for _ in 0..3 {
        let _ = pipeline.render();
    }
    let temp = Temp::new();
    s.profiler.borrow_mut().path = Some(temp.0.join("follow.json"));
    s.profiler.borrow_mut().begin_at(Instant::now());
    for position in [0.2, 0.3, 0.8] {
        s.animate_playhead(position);
        let _ = pipeline.render();
    }
    assert_eq!(s.profiler.borrow().body_builds, 0);
    assert_eq!(s.profiler.borrow().waveform_refreshes, 0);
    s.animate_playhead(0.95);
    let _ = pipeline.render();
    assert!((s.view_start.get() - 0.85).abs() < 1e-6);
    let builds = s.profiler.borrow().body_builds;
    let refreshes = s.profiler.borrow().waveform_refreshes;
    assert!(builds > 0 && refreshes > 0);
    for position in [1., 1.2, 1.6] {
        s.animate_playhead(position);
        let _ = pipeline.render();
    }
    assert_eq!(s.profiler.borrow().body_builds, builds);
    assert_eq!(s.profiler.borrow().waveform_refreshes, refreshes);
    s.animate_playhead(0.2);
    let _ = pipeline.render();
    assert!((s.view_start.get() - 0.1).abs() < 1e-6);
    s.toggle_follow();
    let start = s.view_start.get();
    s.animate_playhead(3.);
    assert_eq!(s.view_start.get(), start);
    assert!(!s.dirty());
}
#[test]
fn transport_controls_use_common_heights_and_fit_minimum_window_in_all_formats() {
    wait_for_test_font();
    let s = Daw::new(project());
    for mode in [
        timeline::Format::Bars,
        timeline::Format::Seconds,
        timeline::Format::Samples,
    ] {
        s.time_format.set(mode);
        s.refresh(true);
        let mut tree = scarlet_ui::ElementTree::new();
        tree.set_root(s.transport().create_element());
        tree.layout(scarlet_ui::LayoutConstraints::tight(1000., 66.));
        for control in [
            "::views::text_field::TextFieldRenderObject",
            "::views::button::ButtonRenderObject",
        ] {
            let mut bounds = Vec::new();
            control_bounds(tree.root().unwrap(), Point::ZERO, control, &mut bounds);
            assert!(!bounds.is_empty());
            for (origin, size) in bounds {
                assert_eq!(size.height, CONTROL_HEIGHT);
                assert!(origin.x >= 0. && origin.x + size.width <= 1000.);
                assert!(origin.y >= 0. && origin.y + size.height <= 66.);
            }
        }
        let mut text = Vec::new();
        text_layouts(tree.root().unwrap(), Point::ZERO, &mut text);
        for (_, origin, size) in text {
            assert!(origin.x + size.width <= 1000. && origin.y + size.height <= 66.);
        }
    }
}

#[test]
fn meter_field_enter_updates_counter_ruler_and_history_without_retiming_audio() {
    wait_for_test_font();
    let s = Daw::new(project());
    s.seek(1.5);
    let before = s.model.borrow().project.clone();
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1000., 790.));
    let mut fields = Vec::new();
    control_bounds(
        tree.root().unwrap(),
        Point::ZERO,
        "::views::text_field::TextFieldRenderObject",
        &mut fields,
    );
    let (origin, _) = fields[1];
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    for e in [
        MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: (origin.x + 10.) as i32,
            y: (origin.y + 10.) as i32,
            click_count: 1,
        },
        MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: (origin.x + 10.) as i32,
            y: (origin.y + 10.) as i32,
            click_count: 1,
        },
    ] {
        dispatcher.dispatch(&mut tree, &Event::Mouse(e));
    }
    physical_key_and_text(
        &mut dispatcher,
        &mut tree,
        'a',
        KeyModifiers {
            super_key: true,
            ..KeyModifiers::default()
        },
    );
    for c in "3/4".chars() {
        physical_key_and_text(&mut dispatcher, &mut tree, c, KeyModifiers::default());
    }
    assert_eq!(s.signature_input.get(), "3/4");
    dispatcher.dispatch(
        &mut tree,
        &Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Enter,
            modifiers: KeyModifiers::default(),
        }),
    );
    assert_eq!(s.clock.get(), "002.01.000");
    assert_eq!(s.playhead.get(), 1.5);
    assert_eq!(s.model.borrow().project.duration(), before.duration());
    assert!(s.dirty());
    let mesh = s.model.borrow().channels[0].mesh.revision();
    s.undo(false);
    assert_eq!(s.signature_input.get(), "4/4");
    assert_eq!(s.clock.get(), "001.04.000");
    s.undo(true);
    assert_eq!(s.signature_input.get(), "3/4");
    assert!(s.model.borrow().channels[0].mesh.revision() > mesh);
    for invalid in ["0/4", "3/3", "33/8", "6/0", "wat"] {
        s.signature_input.set(invalid.into());
        s.submit_signature();
        assert_eq!(s.signature_input.get(), "3/4");
        assert_eq!(s.model.borrow().undo.len(), 1);
    }
    let mut labels = Vec::new();
    tree.set_root(s.ruler().create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1000., 30.));
    text_layouts(tree.root().unwrap(), Point::ZERO, &mut labels);
    assert!(labels.iter().any(|(text, _, _)| text.starts_with("002.")));
}

#[test]
fn native_picker_cancel_preserves_dirty_session_and_clears_save_continuations() {
    for action in [
        FileAction::Open,
        FileAction::Save,
        FileAction::Import,
        FileAction::Export,
    ] {
        let s = Daw::new(project());
        s.mix(0, Some(0.6), None, None);
        s.finish_mix();
        let before = Daw::snapshot(&s.model.borrow());
        let view = set_detail_view(&s);
        s.open_after_save.set(true);
        s.close_after_save.set(true);
        s.dialog.set(Dialog::Native(action));
        s.apply_picker_result(action, Ok(FileDialogOutcome::Cancelled));
        assert_snapshot(&s.model.borrow(), &before);
        assert_detail_view(&s, &view);
        assert!(s.dirty());
        assert!(s.dialog.get() == Dialog::None);
        assert!(s.focus.get());
        assert!(!s.open_after_save.get() && !s.close_after_save.get());
        assert!(s.model.borrow().io.is_none());
    }
}

#[test]
fn native_picker_invalid_path_error_and_retry_preserve_association() {
    let temp = Temp::new();
    let s = Daw::new(project());
    s.mix(0, Some(0.6), None, None);
    s.finish_mix();
    s.model.borrow_mut().current_path = Some(temp.0.join("old.json"));
    let before = Daw::snapshot(&s.model.borrow());
    for result in [
        Ok(FileDialogOutcome::Selected(vec![])),
        Ok(FileDialogOutcome::Selected(vec![
            PathBuf::from("relative.json").into(),
        ])),
        Ok(FileDialogOutcome::Selected(vec![
            temp.0.join("wrong.wav").into(),
        ])),
        Ok(FileDialogOutcome::Selected(vec![temp.0.clone().into()])),
        Ok(FileDialogOutcome::Selected(vec![
            temp.0.join("one.json").into(),
            temp.0.join("two.json").into(),
        ])),
        Err(FileDialogError::Platform("IPC disconnected".into())),
        Err(FileDialogError::OwnerClosed),
        Err(FileDialogError::Busy),
    ] {
        s.close_after_save.set(true);
        s.open_after_save.set(true);
        s.apply_picker_result(FileAction::Save, result);
        assert_snapshot(&s.model.borrow(), &before);
        assert!(s.model.borrow().io.is_none());
        assert!(!s.close_after_save.get() && !s.open_after_save.get());
        assert!(!s.dialog_error.get().is_empty());
    }
    let saved = temp.0.join("retry.json");
    s.apply_picker_result(
        FileAction::Save,
        Ok(FileDialogOutcome::Selected(vec![saved.clone().into()])),
    );
    finish_io(&s);
    assert_project(&Project::load(&saved).unwrap(), &before.project);
    assert_eq!(s.model.borrow().current_path.as_ref(), Some(&saved));
    assert!(!s.dirty());
}

#[test]
fn native_selection_routes_all_four_actions_through_transactional_io() {
    let temp = Temp::new();
    let json = temp.0.join("session.json");
    let wav = temp.0.join("audio.wav");
    let s = Daw::new(project());
    let before = s.model.borrow().project.clone();
    for (action, path) in [(FileAction::Save, &json), (FileAction::Export, &wav)] {
        let view = set_detail_view(&s);
        s.apply_picker_result(
            action,
            Ok(FileDialogOutcome::Selected(vec![path.clone().into()])),
        );
        finish_io(&s);
        assert!(path.is_file());
        assert_detail_view(&s, &view);
    }
    s.apply_picker_result(
        FileAction::Import,
        Ok(FileDialogOutcome::Selected(vec![wav.into()])),
    );
    finish_io(&s);
    assert_eq!(
        s.model.borrow().project.tracks.len(),
        before.tracks.len() + 1
    );
    s.apply_picker_result(
        FileAction::Open,
        Ok(FileDialogOutcome::Selected(vec![json.into()])),
    );
    finish_io(&s);
    assert_project(&s.model.borrow().project, &before);
    assert!(!s.dirty());
}

#[test]
fn native_selected_io_failure_keeps_dirty_session_and_allows_retry() {
    let temp = Temp::new();
    let s = Daw::new(project());
    s.mix(0, Some(0.6), None, None);
    s.finish_mix();
    let before = Daw::snapshot(&s.model.borrow());
    s.close_after_save.set(true);
    let bad = temp.0.join("missing/session.json");
    s.apply_picker_result(
        FileAction::Save,
        Ok(FileDialogOutcome::Selected(vec![bad.into()])),
    );
    finish_io(&s);
    assert_snapshot(&s.model.borrow(), &before);
    assert!(s.dirty());
    assert!(!s.close_after_save.get());
    s.apply_picker_result(
        FileAction::Save,
        Ok(FileDialogOutcome::Selected(vec![
            temp.0.join("retry.json").into(),
        ])),
    );
    finish_io(&s);
    assert!(!s.dirty());
}

#[test]
fn unsupported_native_picker_keeps_existing_browser_and_save_continuation() {
    let temp = Temp::new();
    let s = Daw::new(project());
    s.path.set(temp.0.to_string_lossy().into());
    s.filename.set("session.json".into());
    s.close_after_save.set(true);
    s.apply_picker_result(
        FileAction::Save,
        Err(FileDialogError::Unsupported(
            "backend filter capability".into(),
        )),
    );
    assert!(s.dialog.get() == Dialog::File(FileAction::Save));
    assert!(s.close_after_save.get());
    assert!(s.model.borrow().io.is_none());
}

#[test]
fn pending_native_picker_blocks_edits_io_and_close_without_replacing_receipt() {
    let mut s = Daw::new(project());
    s.window_id.set(Some(WindowId::generate()));
    s.open_dialog(FileAction::Save);
    let before = Daw::snapshot(&s.model.borrow());
    let called = Cell::new(false);
    s.edit("Blocked", |m| {
        called.set(true);
        m.project.tracks.clear();
        Ok(())
    });
    s.undo(false);
    s.mix(0, Some(0.2), None, None);
    s.master_change(0.1);
    s.open_dialog(FileAction::Import);
    s.start_io(
        FileAction::Export,
        PathBuf::from("/tmp/should-not-exist.wav"),
    );
    s.request_open();
    assert!(!s.on_window_close_requested(&window()));
    assert!(!called.get());
    assert_snapshot(&s.model.borrow(), &before);
    assert!(s.model.borrow().io.is_none());
    assert!(s.dialog.get() == Dialog::Native(FileAction::Save));
    s.cancel_picker();
    assert!(s.model.borrow().picker.is_some());
}

#[test]
fn ruler_subdivisions_adapt_to_zoom_reset_at_odd_bars_and_paint_distinct_lengths() {
    use scarlet_ui::renderer::{PaintCommand, PaintContext};
    use timeline::{Format, TickKind, ruler_grid};
    let meter = resonara_core::TimeSignature::default();
    let fine = ruler_grid(Format::Bars, 0., 2., 800., 120., 48000, meter);
    assert_eq!(fine.ticks.len(), 16);
    for (i, tick) in fine.ticks.iter().enumerate() {
        assert_eq!(tick.seconds, i as f64 * 0.125);
        assert_eq!(
            tick.kind,
            if i == 0 {
                TickKind::Bar
            } else if i % 4 == 0 {
                TickKind::Quarter
            } else if i % 2 == 0 {
                TickKind::Eighth
            } else {
                TickKind::Sixteenth
            }
        );
    }
    let mut element = crate::ruler::Marks {
        ticks: fine.ticks,
        start: 0.,
        span: 2.,
        size: Size::new(800., 30.),
    }
    .create_element();
    element.layout(scarlet_ui::LayoutConstraints::tight(800., 30.));
    let mut paint = PaintContext::new();
    element
        .render_object()
        .unwrap()
        .paint(&mut paint, Point::new(20., 10.));
    assert_eq!(paint.commands().len(), 16);
    for (i, cmd) in paint.commands().iter().enumerate() {
        let PaintCommand::FillPath { path, .. } = cmd else {
            panic!("missing ruler line")
        };
        let length = if i == 0 {
            14.
        } else if i % 4 == 0 {
            11.
        } else if i % 2 == 0 {
            7.
        } else {
            4.
        };
        assert_eq!(path[0].x, 20. + i as f32 * 50.);
        assert_eq!(path[0].y, 40. - length);
        assert_eq!(path[2].y, 40.);
    }
    let medium = ruler_grid(Format::Bars, 0., 10., 300., 120., 48000, meter);
    assert!(
        medium
            .ticks
            .iter()
            .all(|t| matches!(t.kind, TickKind::Bar | TickKind::Quarter | TickKind::Major))
    );
    let far = ruler_grid(Format::Bars, 0., 200., 200., 120., 48000, meter);
    assert!(far.ticks.len() <= 3);
    assert!(far.ticks.iter().all(|t| t.kind == TickKind::Major));
    let odd = ruler_grid(
        Format::Bars,
        1.6,
        1.,
        400.,
        120.,
        48000,
        resonara_core::TimeSignature {
            numerator: 7,
            denominator: 8,
        },
    );
    assert!(
        odd.ticks
            .iter()
            .any(|t| t.seconds == 1.75 && t.kind == TickKind::Bar)
    );
    assert!(
        odd.ticks
            .iter()
            .any(|t| t.seconds == 2. && t.kind == TickKind::Eighth)
    );
    assert!(
        odd.ticks
            .iter()
            .any(|t| t.seconds == 2.25 && t.kind == TickKind::Quarter)
    );
    assert!(
        odd.ticks
            .iter()
            .all(|t| t.seconds >= 1.6 && t.seconds < 2.6)
    );
    let huge = ruler_grid(Format::Bars, 0., 86400., 100., 400., 48000, meter);
    assert!(huge.ticks.len() < 20);
}

#[test]
fn snap_uses_display_units_zoom_and_bar_boundaries() {
    use timeline::{Format, snap_grid};
    let meter = resonara_core::TimeSignature {
        numerator: 7,
        denominator: 8,
    };
    let fine = snap_grid(Format::Bars, 2., 800., 120., 48000, meter);
    assert_eq!(fine.caption, "1/16");
    assert_eq!(fine.position(0.19), 0.25);
    let quarter = snap_grid(Format::Bars, 6., 200., 120., 48000, meter);
    assert_eq!(quarter.caption, "1/4");
    assert_eq!(quarter.position(1.65), 1.75);
    assert_eq!(quarter.position(2.24), 2.25);
    assert_eq!(quarter.position(-0.2), 0.);
    let bar = snap_grid(Format::Bars, 20., 200., 120., 48000, meter);
    assert_eq!(bar.caption, "bar");
    assert_eq!(bar.position(3.3), 3.5);
    let time = snap_grid(Format::Seconds, 2., 800., 120., 48000, meter);
    assert!((time.position(0.19) - 0.2).abs() < 1e-12);
    let samples = snap_grid(Format::Samples, 2., 800., 120., 48000, meter);
    assert_eq!(samples.caption, "1 sample");
    assert!((samples.position(10.4 / 48000.) * 48000. - 10.).abs() < 1e-12);
}

#[test]
fn musical_snap_applies_to_ruler_split_and_move_and_edge_trims_with_undo() {
    for (x, moved, expected) in [
        (100, 180, (5000, 400, 8000)),
        (34, 75, (4000, 2800, 5600)),
        (200, 150, (1600, 400, 5400)),
    ] {
        let s = Daw::new(project());
        s.view_span.set(1.2);
        s.arrangement_size.set(Size::new(410., 500.));
        assert_eq!(s.snap_grid_for(&s.model.borrow().project).caption, "1/16");
        let before = s.model.borrow().project.clone();
        assert!(s.timeline_event(0, &press(x)));
        assert!(s.timeline_event(0, &Event::Mouse(MouseEvent::Moved { x: moved, y: 40 })));
        assert!(s.timeline_event(
            0,
            &Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: moved,
                y: 40,
                click_count: 1
            })
        ));
        let m = s.model.borrow();
        let c = &m.project.tracks[0].clips[0];
        assert_eq!((c.start, c.source_offset, c.frames), expected);
        drop(m);
        s.undo(false);
        assert_project(&s.model.borrow().project, &before);
    }
    let s = Daw::new(project());
    s.view_span.set(1.);
    s.arrangement_size.set(Size::new(1000., 500.));
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.ruler().create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1000., 30.));
    dispatched_click(&mut tree, (HEADER + 790. * 0.61) as i32, 15);
    assert_eq!(s.playhead.get(), 0.625);
    s.split();
    assert_eq!(s.model.borrow().project.tracks[0].clips[1].start, 5000);
    // Single-sample snapping must survive conversion through the transport input.
    s.time_format.set(timeline::Format::Samples);
    s.view_span.set(0.01);
    assert!(s.ruler_event(&press(1), 0., 0.01, 80.));
    assert_eq!(s.playhead.get(), 1. / 8000.);
    assert_eq!(s.seconds(&s.cursor.get()).unwrap(), 1);
    s.cancel_ruler_drag();
}

#[test]
fn metronome_empty_transport_and_live_toggle_keep_audio_and_project_state() {
    let s = Daw::new(Project::default());
    let before = s.model.borrow().project.clone();
    assert!(s.handle_key(KeyEvent::Pressed {
        keycode: KeyCode::Char('c'),
        modifiers: KeyModifiers::default()
    }));
    assert!(s.metronome.get());
    s.play();
    let controls = s.model.borrow().audio.as_ref().unwrap().controls.clone();
    s.model.borrow().audio.as_ref().unwrap().render(60000);
    assert!(controls.position.load(Ordering::Relaxed) >= 60000);
    assert!(controls.playing.load(Ordering::Relaxed));
    s.toggle_metronome();
    assert!(!controls.metronome.load(Ordering::Relaxed));
    assert!(Arc::ptr_eq(
        &controls,
        &s.model.borrow().audio.as_ref().unwrap().controls
    ));
    s.toggle_metronome();
    assert!(controls.metronome.load(Ordering::Relaxed));
    s.play();
    assert!(s.model.borrow().audio.is_none());
    assert_eq!(s.seconds(&s.cursor.get()).unwrap(), 60000);
    s.play();
    assert_eq!(
        s.model
            .borrow()
            .audio
            .as_ref()
            .unwrap()
            .controls
            .position
            .load(Ordering::Relaxed),
        60000
    );
    assert_project(&s.model.borrow().project, &before);
    assert!(!s.dirty());
    assert!(s.model.borrow().undo.is_empty());
}

#[test]
fn audio_browser_filters_and_async_import_accept_new_formats() {
    let temp = Temp::new();
    for name in [
        "a.MP3",
        "b.flac",
        "c.AIFF",
        "d.m4a",
        "e.ogg",
        "f.caf",
        "notes.txt",
        "unsupported.wma",
    ] {
        std::fs::write(temp.0.join(name), "fixture").unwrap();
    }
    std::fs::create_dir(temp.0.join("folder")).unwrap();
    let s = Daw::new(Project::default());
    s.dialog.set(Dialog::File(FileAction::Import));
    s.path.set(temp.0.to_string_lossy().into_owned());
    s.read_directory();
    assert_eq!(
        s.files
            .get()
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>(),
        [
            "folder", "a.MP3", "b.flac", "c.AIFF", "d.m4a", "e.ogg", "f.caf"
        ]
    );
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../resonara-core/tests/fixtures/audio/stereo.mp3");
    submit(&s, FileAction::Import, &fixture);
    finish_io(&s);
    assert!(s.status.get().starts_with("Audio imported"));
    assert_eq!(s.model.borrow().project.tracks.len(), 1);
    assert_eq!(
        s.model.borrow().project.tracks[0].clips[0].source_channels,
        2
    );
    assert!(s.dirty());
    s.undo(false);
    assert!(s.model.borrow().project.tracks.is_empty());
    assert!(!s.dirty());
}

#[test]
fn native_audio_selection_decodes_all_supported_fixture_formats_and_undoes_import() {
    let temp = Temp::new();
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../resonara-core/tests/fixtures/audio");
    let uppercase_mp3 = temp.0.join("音声.MP3");
    std::fs::copy(fixtures.join("stereo.mp3"), &uppercase_mp3).unwrap();
    let mut paths: Vec<_> = [
        "source.wav",
        "stereo.flac",
        "stereo.aiff",
        "stereo.caf",
        "alac.m4a",
        "aac.m4a",
        "stereo.aac",
        "stereo.ogg",
        "mono.flac",
    ]
    .iter()
    .map(|name| fixtures.join(name))
    .collect();
    paths.push(uppercase_mp3);
    for path in paths {
        let s = Daw::new(Project::default());
        s.dialog.set(Dialog::Native(FileAction::Import));
        s.apply_picker_result(
            FileAction::Import,
            Ok(FileDialogOutcome::Selected(vec![path.clone().into()])),
        );
        assert!(
            s.model.borrow().io.is_some(),
            "Selection was rejected before decoding: {} · {}",
            path.display(),
            s.status.get()
        );
        assert_eq!(s.status.get(), "Importing audio…");
        finish_io(&s);
        assert!(
            s.status.get().starts_with("Audio imported"),
            "{} · {}",
            path.display(),
            s.status.get()
        );
        {
            let m = s.model.borrow();
            assert_eq!(m.project.tracks.len(), 1);
            let clip = &m.project.tracks[0].clips[0];
            assert!(!clip.samples.is_empty());
            assert_eq!(
                clip.source_channels,
                if path.ends_with("mono.flac") { 1 } else { 2 }
            );
        }
        assert!(s.dirty());
        s.undo(false);
        assert!(s.model.borrow().project.tracks.is_empty());
        assert!(!s.dirty());
    }
}

#[test]
fn native_audio_import_filter_does_not_weaken_project_or_export_filters() {
    let temp = Temp::new();
    for (action, path) in [
        (FileAction::Import, PathBuf::from("relative.mp3")),
        (FileAction::Import, temp.0.join("session.json")),
        (FileAction::Import, temp.0.join("unsupported.wma")),
        (FileAction::Export, temp.0.join("mix.mp3")),
        (FileAction::Open, temp.0.join("audio.mp3")),
        (FileAction::Save, temp.0.join("audio.mp3")),
    ] {
        let s = Daw::new(project());
        let before = Daw::snapshot(&s.model.borrow());
        s.apply_picker_result(action, Ok(FileDialogOutcome::Selected(vec![path.into()])));
        assert!(s.model.borrow().io.is_none());
        assert!(!s.dialog_error.get().is_empty());
        assert_snapshot(&s.model.borrow(), &before);
    }
}

#[test]
fn routing_insert_chain_edit_bypass_reorder_remove_and_undo() {
    use resonara_core::InsertKind;
    let s = Daw::new(project());
    let target = RoutingTarget::Track(0);
    let before = Daw::snapshot(&s.model.borrow());
    s.add_insert(target, InsertKind::Gain { gain: 0.5 });
    s.add_insert(target, InsertKind::Delay { frames: 32 });
    s.move_insert(target, 1, -1);
    assert!(matches!(
        s.model.borrow().project.tracks[0].routing.inserts[0].kind,
        InsertKind::Delay { frames: 32 }
    ));
    s.toggle_insert(target, 0);
    assert!(s.model.borrow().project.tracks[0].routing.inserts[0].bypass);
    s.remove_insert(target, 1);
    assert_eq!(s.model.borrow().project.tracks[0].routing.inserts.len(), 1);
    for _ in 0..5 {
        s.undo(false);
    }
    assert_snapshot(&s.model.borrow(), &before);
    for _ in 0..5 {
        s.undo(true);
    }
    assert_eq!(s.model.borrow().project.tracks[0].routing.inserts.len(), 1);
    assert!(s.model.borrow().project.tracks[0].routing.inserts[0].bypass);
}

#[test]
fn routing_bus_reference_repair_and_undo_restore_the_full_session() {
    use resonara_core::Destination;
    let s = Daw::new(project());
    s.add_bus();
    let aux = s.model.borrow().selected_bus.unwrap();
    s.add_bus();
    let group = s.model.borrow().selected_bus.unwrap();
    s.set_output(RoutingTarget::Track(0), Destination::Bus(group));
    s.set_output(RoutingTarget::Bus(aux), Destination::Bus(group));
    s.add_send(RoutingTarget::Track(1), aux);
    s.add_send(RoutingTarget::Bus(group), aux); // This creates a cycle and must be rejected.
    assert!(s.status.get().to_lowercase().contains("cycle"));
    assert!(
        s.model
            .borrow()
            .project
            .bus(group)
            .unwrap()
            .routing
            .sends
            .is_empty()
    );
    s.choose_bus(aux);
    let before = Daw::snapshot(&s.model.borrow());
    s.delete_bus(group);
    let m = s.model.borrow();
    assert_eq!(m.project.tracks[0].routing.output, Destination::Master);
    assert_eq!(
        m.project.bus(aux).unwrap().routing.output,
        Destination::Master
    );
    assert!(m.project.bus(group).is_none());
    m.project.validate().unwrap();
    drop(m);
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &before);
    s.delete_bus(aux);
    assert!(s.model.borrow().project.tracks[1].routing.sends.is_empty());
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &before);
}

#[test]
fn routing_live_bus_controls_keep_engine_and_coalesce_history() {
    let s = Daw::new(project());
    s.add_bus();
    let id = s.model.borrow().selected_bus.unwrap();
    s.add_send(RoutingTarget::Track(0), id);
    s.play();
    let controls = s.model.borrow().audio.as_ref().unwrap().controls.clone();
    let previous = s.model.borrow().undo.len();
    s.bus_mix(id, Some(0.6), Some(-0.25), false);
    s.bus_mix(id, Some(0.4), Some(0.25), false);
    assert!(Arc::ptr_eq(
        &controls,
        &s.model.borrow().audio.as_ref().unwrap().controls
    ));
    assert_eq!(
        f32::from_bits(controls.buses[0].gain.load(Ordering::Relaxed)),
        0.4
    );
    assert_eq!(
        f32::from_bits(controls.buses[0].pan.load(Ordering::Relaxed)),
        0.25
    );
    s.finish_mix();
    assert_eq!(s.model.borrow().undo.len(), previous + 1);
    s.bus_mix(id, None, None, true);
    assert!(controls.buses[0].mute.load(Ordering::Relaxed));
    s.undo(false);
    assert!(!s.model.borrow().project.bus(id).unwrap().mute);
    s.undo(false);
    assert_eq!(s.model.borrow().project.bus(id).unwrap().gain, 1.);
}

#[test]
fn routing_invalid_and_repeated_edits_do_not_stop_playback_or_add_history() {
    use resonara_core::Destination;
    let s = Daw::new(project());
    s.add_bus();
    let id = s.model.borrow().selected_bus.unwrap();
    s.play();
    let controls = s.model.borrow().audio.as_ref().unwrap().controls.clone();
    let before = Daw::snapshot(&s.model.borrow());
    let undo = s.model.borrow().undo.len();
    s.set_output(RoutingTarget::Bus(id), Destination::Bus(id));
    assert!(s.status.get().to_lowercase().contains("cycle"));
    assert_snapshot(&s.model.borrow(), &before);
    s.set_output(RoutingTarget::Track(0), Destination::Master);
    s.move_insert(RoutingTarget::Track(0), 0, -1);
    assert_eq!(s.model.borrow().undo.len(), undo);
    assert!(Arc::ptr_eq(
        &controls,
        &s.model.borrow().audio.as_ref().unwrap().controls
    ));
}

#[test]
fn routing_insert_parameter_rejects_invalid_input_and_cancel_is_clean() {
    use resonara_core::InsertKind;
    let s = Daw::new(project());
    let target = RoutingTarget::Track(0);
    s.add_insert(target, InsertKind::OnePole { coefficient: 0.5 });
    let before = Daw::snapshot(&s.model.borrow());
    s.edit_insert_value(target, 0);
    s.routing_value.set("NaN".into());
    s.submit_insert_value(target, 0);
    assert!(s.dialog.get() == Dialog::InsertValue(target, 0));
    assert!(!s.dialog_error.get().is_empty());
    assert_snapshot(&s.model.borrow(), &before);
    s.handle_key(KeyEvent::Pressed {
        keycode: KeyCode::Escape,
        modifiers: KeyModifiers::default(),
    });
    assert!(s.dialog.get() == Dialog::None);
    assert_snapshot(&s.model.borrow(), &before);
    s.edit_insert_value(target, 0);
    s.routing_value.set("0.75".into());
    s.submit_insert_value(target, 0);
    assert!(s.dialog.get() == Dialog::None);
    assert!(matches!(
        s.model.borrow().project.tracks[0].routing.inserts[0].kind,
        InsertKind::OnePole { coefficient: 0.75 }
    ));
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &before);
}

#[test]
fn routing_save_open_and_export_use_the_same_session_graph() {
    use resonara_core::{Destination, InsertKind};
    let temp = Temp::new();
    let s = Daw::new(project());
    s.add_bus();
    let id = s.model.borrow().selected_bus.unwrap();
    s.set_output(RoutingTarget::Track(0), Destination::Bus(id));
    s.add_insert(RoutingTarget::Bus(id), InsertKind::Gain { gain: 0.4 });
    s.add_send(RoutingTarget::Track(1), id);
    s.edit_send(RoutingTarget::Track(1), 0, routing::SendEdit::PrePost);
    let expected = s.model.borrow().project.clone();
    let saved = temp.0.join("routing.json");
    submit(&s, FileAction::Save, &saved);
    finish_io(&s);
    assert_project(&Project::load(&saved).unwrap(), &expected);
    let output = temp.0.join("routing.wav");
    submit(&s, FileAction::Export, &output);
    finish_io(&s);
    assert!(output.metadata().unwrap().len() > 100);
    s.delete_bus(id);
    submit(&s, FileAction::Open, &saved);
    finish_io(&s);
    assert_project(&s.model.borrow().project, &expected);
    assert!(s.model.borrow().selected_bus.is_none());
}

#[test]
fn routing_bus_selection_never_deletes_or_splits_a_hidden_track() {
    let s = Daw::new(project());
    s.add_bus();
    let id = s.model.borrow().selected_bus.unwrap();
    let before = Daw::snapshot(&s.model.borrow());
    s.delete(false);
    s.delete(true);
    s.duplicate();
    s.split();
    s.trim_range();
    assert_snapshot(&s.model.borrow(), &before);
    s.handle_key(KeyEvent::Pressed {
        keycode: KeyCode::Char('m'),
        modifiers: KeyModifiers::default(),
    });
    assert!(s.model.borrow().project.bus(id).unwrap().mute);
    assert!(!s.model.borrow().project.tracks[0].mute);
}

#[test]
fn routing_busy_state_rejects_bus_and_insert_mutations() {
    use resonara_core::InsertKind;
    let s = Daw::new(project());
    let before = Daw::snapshot(&s.model.borrow());
    let (_tx, rx) = mpsc::channel();
    s.model.borrow_mut().io = Some(rx);
    s.add_bus();
    s.add_insert(RoutingTarget::Track(0), InsertKind::Gain { gain: 0.5 });
    assert_snapshot(&s.model.borrow(), &before);
    assert!(s.model.borrow().undo.is_empty());
}

#[test]
fn routing_native_slots_dispatch_picker_add_and_bypass_without_button_clutter() {
    wait_for_test_font();
    let s = Daw::new(project());
    let target = RoutingTarget::Track(0);
    let build = || {
        let mut tree = scarlet_ui::ElementTree::new();
        tree.set_root(
            s.routing_rack(&s.model.borrow().project, target)
                .create_element(),
        );
        tree.layout(scarlet_ui::LayoutConstraints::tight(190., 700.));
        let mut slots = Vec::new();
        control_bounds(
            tree.root().unwrap(),
            Point::ZERO,
            "::insert_slot::SlotRender",
            &mut slots,
        );
        let mut buttons = Vec::new();
        control_bounds(
            tree.root().unwrap(),
            Point::ZERO,
            "::views::button::ButtonRenderObject",
            &mut buttons,
        );
        assert_eq!(
            buttons.len(),
            1,
            "Only the output selector remains a persistent button"
        );
        (tree, slots)
    };
    let (mut tree, slots) = build();
    assert_eq!(slots.len(), 1);
    let (origin, size) = slots[0];
    dispatched_click(
        &mut tree,
        (origin.x + 50.) as i32,
        (origin.y + size.height / 2.) as i32,
    );
    assert!(s.dialog.get() == Dialog::InsertPicker(target));
    let mut picker = scarlet_ui::ElementTree::new();
    picker.set_root(s.dialog_view().create_element());
    picker.layout(scarlet_ui::LayoutConstraints::tight(1280., 790.));
    let mut buttons = Vec::new();
    control_bounds(
        picker.root().unwrap(),
        Point::ZERO,
        "::views::button::ButtonRenderObject",
        &mut buttons,
    );
    let (origin, size) = buttons[0];
    dispatched_click(
        &mut picker,
        (origin.x + size.width / 2.) as i32,
        (origin.y + size.height / 2.) as i32,
    );
    assert_eq!(s.model.borrow().project.tracks[0].routing.inserts.len(), 1);
    let (mut tree, slots) = build();
    assert_eq!(slots.len(), 2);
    let (origin, size) = slots[0];
    dispatched_click(
        &mut tree,
        (origin.x + size.width - 31.) as i32,
        (origin.y + size.height / 2.) as i32,
    );
    assert!(s.model.borrow().project.tracks[0].routing.inserts[0].bypass);
    for (origin, size) in slots {
        assert!(origin.x >= -0.01 && origin.x + size.width <= 190.01);
    }
}

#[test]
fn routing_repeated_parameter_apply_preserves_exact_value_and_audio() {
    use resonara_core::InsertKind;
    let s = Daw::new(project());
    let target = RoutingTarget::Track(0);
    s.add_insert(target, InsertKind::Gain { gain: 0.25 });
    s.play();
    let controls = s.model.borrow().audio.as_ref().unwrap().controls.clone();
    let before = Daw::snapshot(&s.model.borrow());
    let undo = s.model.borrow().undo.len();
    s.edit_insert_value(target, 0);
    s.submit_insert_value(target, 0);
    s.move_insert(target, 1, -1);
    assert_snapshot(&s.model.borrow(), &before);
    assert_eq!(s.model.borrow().undo.len(), undo);
    assert!(Arc::ptr_eq(
        &controls,
        &s.model.borrow().audio.as_ref().unwrap().controls
    ));
}

#[test]
fn routing_clicking_a_clip_leaves_bus_selection_and_allows_track_edits() {
    let s = Daw::new(project());
    s.add_bus();
    assert!(s.model.borrow().selected_bus.is_some());
    s.timeline_event(0, &press(100));
    assert!(s.model.borrow().selected_bus.is_none());
    assert_eq!(s.model.borrow().clip, Some(0));
    s.cancel_drag();
    s.delete(false);
    assert!(s.model.borrow().project.tracks[0].clips.is_empty());
}

#[test]
fn routing_add_bus_budget_failure_keeps_project_and_history_clean() {
    use resonara_core::{Insert, InsertKind};
    let mut project = project();
    project.tracks[0].routing.inserts = vec![
        Insert {
            kind: InsertKind::Gain { gain: 1. },
            bypass: false
        };
        4090
    ];
    project.validate_routing().unwrap();
    let s = Daw::new(project);
    let before = Daw::snapshot(&s.model.borrow());
    s.add_bus();
    assert_snapshot(&s.model.borrow(), &before);
    assert!(s.model.borrow().undo.is_empty());
    assert!(s.status.get().contains("Could not add aux channel"));
}

#[test]
fn routing_full_native_tree_aux_header_selects_its_inspector() {
    wait_for_test_font();
    let mut project = project();
    project.tracks.push(project.tracks[0].clone());
    let s = Daw::new(project);
    s.add_bus();
    let aux = s.model.borrow().selected_bus.unwrap();
    s.add_bus();
    s.choose(0, None);
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1280., 790.));
    let mut buttons = Vec::new();
    control_bounds(
        tree.root().unwrap(),
        Point::ZERO,
        "::views::button::ButtonRenderObject",
        &mut buttons,
    );
    let (origin, size) = buttons
        .into_iter()
        .find(|(origin, size)| {
            origin.x >= (1280. - 4.) * s.inspector_fraction.get() + 4. + 300.
                && origin.x < (1280. - 4.) * s.inspector_fraction.get() + 4. + 400.
                && origin.y > 350.
                && size.width >= 90.
                && size.height == 24.
        })
        .expect("Aux mixer header");
    eprintln!("Aux header bounds {origin:?} {size:?}");
    let hit = dispatched_click(
        &mut tree,
        (origin.x + size.width / 2.) as i32,
        (origin.y + size.height / 2.) as i32,
    );
    assert_eq!(s.model.borrow().selected_bus, Some(aux), "{hit}");
    assert_eq!(s.track_name.get(), "Aux 1");
    assert!(s.inspector.get());
}

#[test]
fn routing_new_bus_assignments_create_one_aux_and_are_atomic_undo_steps() {
    use resonara_core::Destination;
    let s = Daw::new(project());
    let target = RoutingTarget::Track(0);
    let before = Daw::snapshot(&s.model.borrow());
    s.create_aux_route(Some(RoutingMenu::Output(target)));
    let first = s.model.borrow().project.buses[0].id;
    assert_eq!(
        s.model.borrow().project.tracks[0].routing.output,
        Destination::Bus(first)
    );
    assert!(
        s.model.borrow().selected_bus.is_none(),
        "Keep the source selected after routing"
    );
    assert_eq!(s.model.borrow().undo.len(), 1);
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &before);
    s.undo(true);
    let routed = Daw::snapshot(&s.model.borrow());
    s.create_aux_route(Some(RoutingMenu::Send(target)));
    let m = s.model.borrow();
    assert_eq!(m.project.buses.len(), 2);
    assert_eq!(m.project.tracks[0].routing.output, Destination::Bus(first));
    let send = &m.project.tracks[0].routing.sends[0];
    assert_eq!(send.target, m.project.buses[1].id);
    assert!(!send.pre_fader);
    assert!(send.enabled);
    m.project.validate_routing().unwrap();
    drop(m);
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &routed);
}

#[test]
fn routing_native_new_bus_choice_creates_and_connects_receiver() {
    wait_for_test_font();
    let s = Daw::new(project());
    let target = RoutingTarget::Track(0);
    s.routing_menu.set(Some(RoutingMenu::Output(target)));
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(
        s.routing_rack(&s.model.borrow().project, target)
            .create_element(),
    );
    tree.layout(scarlet_ui::LayoutConstraints::tight(190., 700.));
    let mut buttons = Vec::new();
    control_bounds(
        tree.root().unwrap(),
        Point::ZERO,
        "::views::button::ButtonRenderObject",
        &mut buttons,
    );
    let (origin, size) = buttons[2]; // Output slot, Stereo Out, New Bus → Aux.
    dispatched_click(
        &mut tree,
        (origin.x + size.width / 2.) as i32,
        (origin.y + size.height / 2.) as i32,
    );
    let m = s.model.borrow();
    assert_eq!(m.project.buses.len(), 1);
    assert_eq!(m.project.buses[0].name, "Aux 1");
    assert_eq!(
        m.project.tracks[0].routing.output,
        resonara_core::Destination::Bus(m.project.buses[0].id)
    );
    assert!(s.routing_menu.get().is_none());
}

#[test]
fn natural_audio_completion_retains_output_until_explicit_transport_action() {
    let app = Daw::new(project());
    app.play();
    assert!(app.model.borrow().audio.is_some());
    app.finish_audio(false, true);
    assert!(app.model.borrow().audio.is_none());
    assert!(app.model.borrow().retired_audio.is_some());
    app.play();
    assert!(app.model.borrow().audio.is_some());
    assert!(app.model.borrow().retired_audio.is_none());
    app.finish_audio(false, true);
    app.stop_audio(true);
    assert!(app.model.borrow().audio.is_none());
    assert!(app.model.borrow().retired_audio.is_none());
}

#[test]
fn insert_context_keyboard_moves_removes_and_undoes_without_hidden_controls() {
    use resonara_core::InsertKind;
    let s = Daw::new(project());
    let target = RoutingTarget::Track(0);
    s.add_insert(target, InsertKind::Gain { gain: 0.5 });
    s.add_insert(target, InsertKind::Delay { frames: 12 });
    let before = Daw::snapshot(&s.model.borrow());
    s.open_insert_actions(target, 1);
    s.menu_choice.set(2);
    s.handle_key(KeyEvent::Pressed {
        keycode: KeyCode::Enter,
        modifiers: KeyModifiers::default(),
    });
    assert!(matches!(
        s.model.borrow().project.tracks[0].routing.inserts[0].kind,
        InsertKind::Delay { .. }
    ));
    assert!(s.dialog.get() == Dialog::None);
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &before);
    s.open_insert_actions(target, 1);
    s.insert_action(target, 1, 4);
    assert_eq!(s.model.borrow().project.tracks[0].routing.inserts.len(), 1);
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &before);
    s.open_insert_picker(target);
    s.handle_key(KeyEvent::Pressed {
        keycode: KeyCode::Escape,
        modifiers: KeyModifiers::default(),
    });
    assert!(s.dialog.get() == Dialog::None);
    assert_snapshot(&s.model.borrow(), &before);
}

#[test]
fn insert_parameter_popup_outside_click_cancels_without_editing_session() {
    use resonara_core::InsertKind;
    let s = Daw::new(project());
    let target = RoutingTarget::Track(0);
    s.add_insert(target, InsertKind::Gain { gain: 0.5 });
    let before = Daw::snapshot(&s.model.borrow());
    s.open_insert_editor(target, 0);
    s.routing_value.set("-30".into());
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1280., 790.));
    dispatched_click(&mut tree, 1200, 50);
    assert!(s.dialog.get() == Dialog::None);
    assert_snapshot(&s.model.borrow(), &before);
}

#[test]
#[ignore = "requires the built native CLAP fixture; set RESONARA_CLAP_LIBRARY"]
fn clap_generic_editor_roundtrips_opaque_state_and_undoes_parameter_change() {
    use resonara_core::InsertKind;
    let s = Daw::new(project());
    let target = RoutingTarget::Track(0);
    let plugin = resonara_core::plugins::load_bundled_gain().unwrap();
    s.add_insert(target, InsertKind::Clap { plugin });
    let before = Daw::snapshot(&s.model.borrow());
    s.open_insert_editor(target, 0);
    assert!(s.dialog.get() == Dialog::ClapEditor(target, 0));
    assert_eq!(s.plugin_fields.borrow().len(), 1);
    (s.plugin_knobs.borrow()[0].1.changed)(-0.75);
    assert_eq!(s.plugin_fields.borrow()[0].1.get(), "0.25");
    s.submit_clap_parameters(target, 0);
    assert!(s.dialog.get() == Dialog::None, "{}", s.dialog_error.get());
    if let InsertKind::Clap { plugin } = &s.model.borrow().project.tracks[0].routing.inserts[0].kind
    {
        assert_eq!(plugin.parameters[0].value, 0.25);
        assert_eq!(plugin.state.len(), 16);
    } else {
        panic!("Expected CLAP insert");
    }
    let temp = Temp::new();
    let path = temp.0.join("clap-session.json");
    s.model.borrow().project.save(&path).unwrap();
    assert_project(&Project::load(&path).unwrap(), &s.model.borrow().project);
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &before);
    s.open_insert_editor(target, 0);
    s.plugin_fields.borrow()[0].1.set("NaN".into());
    s.submit_clap_parameters(target, 0);
    assert!(s.dialog.get() == Dialog::ClapEditor(target, 0));
    assert_snapshot(&s.model.borrow(), &before);
}

#[test]
#[ignore = "requires external-gain.clap in CLAP_PATH (native gain fixture)"]
fn installed_clap_picker_edit_bypass_and_undo_keep_playback_running() {
    let s = Daw::new(project());
    let target = RoutingTarget::Track(0);
    s.play();
    assert_playback_advances(&s, 1700);
    s.open_insert_picker(target);
    s.menu_choice.set(5);
    s.handle_insert_popup_key(KeyCode::Enter);
    assert!(s.dialog.get() == Dialog::ClapPicker(target));
    assert_playback_advances(&s, 17);
    let index = s
        .plugin_catalog
        .borrow()
        .effects
        .iter()
        .position(|c| c.library == "external-gain.clap")
        .unwrap();
    s.menu_choice.set(index);
    s.handle_insert_popup_key(KeyCode::Enter);
    assert!(s.dialog.get() == Dialog::None, "{}", s.dialog_error.get());
    assert_playback_advances(&s, 17);
    s.open_insert_editor(target, 0);
    s.plugin_fields.borrow()[0].1.set("0.25".into());
    s.submit_clap_parameters(target, 0);
    assert!(s.dialog.get() == Dialog::None, "{}", s.dialog_error.get());
    assert_playback_advances(&s, 17);
    s.toggle_insert(target, 0);
    assert_playback_advances(&s, 17);
    s.undo(false);
    assert_playback_advances(&s, 17);
    s.undo(false);
    assert_playback_advances(&s, 17);
    let m = s.model.borrow();
    if let resonara_core::InsertKind::Clap { plugin } = &m.project.tracks[0].routing.inserts[0].kind
    {
        assert_eq!(plugin.library, "external-gain.clap");
        assert_eq!(plugin.parameters[0].value, 1.);
    } else {
        panic!("CLAP insert missing");
    }
}

#[test]
fn clap_editor_hides_internal_parameters_and_keeps_read_only_values_visible() {
    wait_for_test_font();
    let s = Daw::new(project());
    let target = RoutingTarget::Track(0);
    let parameters = (0..3)
        .map(|id| resonara_core::ClapParameter {
            id,
            name: ["Editable", "Read only", "Internal"][id as usize].into(),
            min: 0.,
            max: 2.,
            value: 1.,
            stepped: false,
            read_only: id == 1,
            hidden: id == 2,
        })
        .collect();
    s.add_insert(
        target,
        resonara_core::InsertKind::Clap {
            plugin: resonara_core::ClapInsert {
                library: "missing-editor-fixture.clap".into(),
                plugin_id: "org.resonara.test.editor".into(),
                name: "Editor fixture".into(),
                state: vec![],
                parameters,
            },
        },
    );
    s.open_insert_editor(target, 0);
    assert_eq!(
        s.plugin_fields
            .borrow()
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        vec![0]
    );
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.dialog_view().create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1280., 790.));
    let mut text = Vec::new();
    text_layouts(tree.root().unwrap(), Point::ZERO, &mut text);
    assert!(text.iter().any(|(text, _, _)| text == "Read only"));
    assert!(!text.iter().any(|(text, _, _)| text == "Internal"));
    let mut fields = Vec::new();
    control_bounds(
        tree.root().unwrap(),
        Point::ZERO,
        "::views::text_field::TextFieldRenderObject",
        &mut fields,
    );
    assert_eq!(fields.len(), 1);
    let mut knobs = Vec::new();
    control_bounds(
        tree.root().unwrap(),
        Point::ZERO,
        "::knob::KnobRender",
        &mut knobs,
    );
    assert_eq!(knobs.len(), 1, "Only the editable parameter gets a knob");
}

#[test]
fn full_height_inspector_is_outside_right_mixer_and_has_one_editable_name() {
    wait_for_test_font();
    let s = Daw::new(project());
    for mixer in [true, false] {
        s.mixer_visible.set(mixer);
        let mut tree = scarlet_ui::ElementTree::new();
        tree.set_root(s.create_element());
        tree.layout(scarlet_ui::LayoutConstraints::tight(1280., 790.));
        let mut faders = Vec::new();
        control_bounds(
            tree.root().unwrap(),
            Point::ZERO,
            "::fader::FaderRender",
            &mut faders,
        );
        let left: Vec<_> = faders
            .iter()
            .filter(|(origin, _)| origin.x < 224.)
            .collect();
        assert_eq!(left.len(), 1);
        assert!(
            (left[0].0.y + left[0].1.height - 724.).abs() < 1.,
            "Selected fader remains at inspector bottom: {:?}",
            left[0]
        );
        let right: Vec<_> = faders
            .iter()
            .filter(|(origin, _)| origin.x >= 224.)
            .collect();
        assert_eq!(right.len(), if mixer { 3 } else { 0 });
        let mut panel = scarlet_ui::ElementTree::new();
        panel.set_root(s.inspector_panel().create_element());
        panel.layout(scarlet_ui::LayoutConstraints::tight(224., 616.));
        let mut fields = Vec::new();
        control_bounds(
            panel.root().unwrap(),
            Point::ZERO,
            "::views::text_field::TextFieldRenderObject",
            &mut fields,
        );
        assert_eq!(
            fields.len(),
            1,
            "One editable name, no duplicate heading or default range fields"
        );
        let mut text = Vec::new();
        text_layouts(panel.root().unwrap(), Point::ZERO, &mut text);
        assert!(!text.iter().any(|(text, _, _)| text == "Track 0"));
    }
}

#[test]
fn aux_inspector_fader_shares_gain_state_and_one_gesture_undo() {
    let s = Daw::new(project());
    s.add_bus();
    let id = s.model.borrow().selected_bus.unwrap();
    let before = Daw::snapshot(&s.model.borrow());
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.inspector_panel().create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(224., 616.));
    let mut faders = Vec::new();
    control_bounds(
        tree.root().unwrap(),
        Point::ZERO,
        "::fader::FaderRender",
        &mut faders,
    );
    assert_eq!(faders.len(), 1);
    let (origin, size) = faders[0];
    let geom = fader::Geometry::new(size);
    dispatched_click(
        &mut tree,
        (origin.x + geom.axis) as i32,
        (origin.y + geom.y(0.5)) as i32,
    );
    s.finish_mix();
    let m = s.model.borrow();
    assert_ne!(m.project.bus(id).unwrap().gain, 1.);
    assert_eq!(
        m.project.bus(id).unwrap().gain,
        m.bus_channels[0].gain.get()
    );
    drop(m);
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &before);
}

#[test]
fn inspector_and_mixer_faders_have_identical_travel_at_all_layout_sizes() {
    wait_for_test_font();
    for (width, height) in [(1000., 790.), (1188., 790.), (1280., 1000.)] {
        let s = Daw::new(project());
        s.size.set(Size::new(width, height));
        for fraction in [0., 0.5, 0.9] {
            s.mixer_fraction.set(fraction);
            let mut tree = scarlet_ui::ElementTree::new();
            for _ in 0..4 {
                tree.set_root(s.create_element());
                tree.layout(scarlet_ui::LayoutConstraints::tight(width, height));
            }
            let mut faders = Vec::new();
            control_bounds(
                tree.root().unwrap(),
                Point::ZERO,
                "::fader::FaderRender",
                &mut faders,
            );
            assert_eq!(faders.len(), 4);
            let inspector = &faders[0];
            let expected = inspector.1.height;
            for (origin, size) in &faders[1..] {
                assert!(
                    (size.height - expected).abs() < 0.01,
                    "{width}x{height} split{fraction}: inspector {:?}, mixer {:?}",
                    inspector,
                    (origin, size)
                );
                let a = fader::Geometry::new(inspector.1);
                let b = fader::Geometry::new(*size);
                assert!(((a.bottom - a.top) - (b.bottom - b.top)).abs() < 0.01);
            }
            assert!((expected - s.mixer_fader_height()).abs() < 0.01);
        }
    }
}

#[test]
fn insert_slots_are_contiguous_including_the_empty_add_slot() {
    let s = Daw::new(project());
    let target = RoutingTarget::Track(0);
    s.add_insert(target, resonara_core::InsertKind::Gain { gain: 1. });
    s.add_insert(target, resonara_core::InsertKind::Delay { frames: 12 });
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(
        s.routing_rack(&s.model.borrow().project, target)
            .create_element(),
    );
    tree.layout(scarlet_ui::LayoutConstraints::tight(190., 700.));
    let mut slots = Vec::new();
    control_bounds(
        tree.root().unwrap(),
        Point::ZERO,
        "::insert_slot::SlotRender",
        &mut slots,
    );
    assert_eq!(slots.len(), 3);
    for pair in slots.windows(2) {
        assert!((pair[0].0.y + pair[0].1.height - pair[1].0.y).abs() < 0.01);
    }
}

#[test]
fn shared_inspector_controls_match_mixer_pan_and_fader_geometry() {
    let s = Daw::new(project());
    let measure = |view: AnyView, size: Size| {
        let mut tree = scarlet_ui::ElementTree::new();
        tree.set_root(view.create_element());
        tree.layout(scarlet_ui::LayoutConstraints::tight(
            size.width,
            size.height,
        ));
        let mut faders = Vec::new();
        let mut knobs = Vec::new();
        control_bounds(
            tree.root().unwrap(),
            Point::ZERO,
            "::fader::FaderRender",
            &mut faders,
        );
        control_bounds(
            tree.root().unwrap(),
            Point::ZERO,
            "::knob::KnobRender",
            &mut knobs,
        );
        (faders[0], knobs[0])
    };
    let (a, ap) = measure(s.inspector_panel(), Size::new(224., 616.));
    let (b, bp) = measure(s.mixer(), Size::new(900., 382.));
    assert_eq!(a.1, b.1);
    assert_eq!(ap.1, bp.1);
    assert!(((ap.0.x - a.0.x) - (bp.0.x - b.0.x)).abs() < 0.01);
    assert!(((ap.0.y - a.0.y) - (bp.0.y - b.0.y)).abs() < 0.01);
}

#[test]
fn live_send_knob_levels_keep_engine_and_coalesce_undo() {
    let s = Daw::new(project());
    s.add_bus();
    let id = s.model.borrow().selected_bus.unwrap();
    let target = RoutingTarget::Track(0);
    s.add_send(target, id);
    s.choose(0, None);
    let before = Daw::snapshot(&s.model.borrow());
    let count = s.model.borrow().undo.len();
    s.play();
    let controls = s.model.borrow().audio.as_ref().unwrap().controls.clone();
    s.send_gain(target, 0, 0.4);
    s.send_gain(target, 0, 0.7);
    assert!(Arc::ptr_eq(
        &controls,
        &s.model.borrow().audio.as_ref().unwrap().controls
    ));
    assert_eq!(s.model.borrow().undo.len(), count);
    assert_eq!(
        f32::from_bits(controls.send_gains[0].load(Ordering::Relaxed)),
        0.7
    );
    s.finish_mix();
    assert_eq!(s.model.borrow().undo.len(), count + 1);
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &before);
}

#[test]
fn native_send_knob_cancel_restores_value_without_history_or_stopping_audio() {
    let s = Daw::new(project());
    s.add_bus();
    let id = s.model.borrow().selected_bus.unwrap();
    let target = RoutingTarget::Track(0);
    s.add_send(target, id);
    s.choose(0, None);
    s.play();
    let before = Daw::snapshot(&s.model.borrow());
    let controls = s.model.borrow().audio.as_ref().unwrap().controls.clone();
    let count = s.model.borrow().undo.len();
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(
        s.routing_rack(&s.model.borrow().project, target)
            .create_element(),
    );
    tree.layout(scarlet_ui::LayoutConstraints::tight(190., 700.));
    let mut knobs = Vec::new();
    control_bounds(
        tree.root().unwrap(),
        Point::ZERO,
        "::send_knob::SendRender",
        &mut knobs,
    );
    assert_eq!(knobs.len(), 1);
    let (origin, size) = knobs[0];
    let x = (origin.x + size.width / 2.) as i32;
    let y = (origin.y + size.height / 2.) as i32;
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    dispatcher.dispatch(&mut tree, &Event::Mouse(MouseEvent::Moved { x, y }));
    dispatcher.dispatch(
        &mut tree,
        &Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x,
            y,
            click_count: 1,
        }),
    );
    dispatcher.dispatch(&mut tree, &Event::Mouse(MouseEvent::Moved { x, y: y - 30 }));
    assert_ne!(
        s.model.borrow().project.tracks[0].routing.sends[0].gain,
        0.25
    );
    dispatcher.dispatch(
        &mut tree,
        &Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Escape,
            modifiers: KeyModifiers::default(),
        }),
    );
    s.finish_mix();
    assert_snapshot(&s.model.borrow(), &before);
    assert_eq!(s.model.borrow().undo.len(), count);
    assert!(Arc::ptr_eq(
        &controls,
        &s.model.borrow().audio.as_ref().unwrap().controls
    ));
}

#[test]
fn compact_send_rack_has_contiguous_rows_tiny_knob_and_no_add_button() {
    let s = Daw::new(project());
    s.add_bus();
    let id = s.model.borrow().selected_bus.unwrap();
    let target = RoutingTarget::Track(0);
    s.add_send(target, id);
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(
        s.routing_rack(&s.model.borrow().project, target)
            .create_element(),
    );
    tree.layout(scarlet_ui::LayoutConstraints::tight(190., 700.));
    let mut knobs = Vec::new();
    control_bounds(
        tree.root().unwrap(),
        Point::ZERO,
        "::send_knob::SendRender",
        &mut knobs,
    );
    assert_eq!(knobs.len(), 1);
    assert_eq!(knobs[0].1, Size::new(24., 24.));
    let mut buttons = Vec::new();
    control_bounds(
        tree.root().unwrap(),
        Point::ZERO,
        "::views::button::ButtonRenderObject",
        &mut buttons,
    );
    assert_eq!(
        buttons.len(),
        1,
        "Only Output, never add/mode/remove buttons"
    );
    fn slot_bounds(element: &dyn scarlet_ui::Element, parent: Point, out: &mut Vec<(Point, Size)>) {
        let origin = Point::new(
            parent.x + element.position().x,
            parent.y + element.position().y,
        );
        if element
            .type_name_debug()
            .contains("::send_slot::SlotRender")
        {
            out.push((origin, element.bounds().size));
        }
        for child in element.children() {
            slot_bounds(child.as_ref(), origin, out);
        }
    }
    let mut slots = Vec::new();
    slot_bounds(tree.root().unwrap(), Point::ZERO, &mut slots);
    assert_eq!(slots.len(), 2);
    assert_eq!(slots[0].1.height, 26.);
    assert!((slots[0].0.y + 26. - slots[1].0.y).abs() < 0.01);
    dispatched_click(
        &mut tree,
        (slots[1].0.x + 40.) as i32,
        (slots[1].0.y + 13.) as i32,
    );
    assert!(s.dialog.get() == Dialog::SendPicker(target, None));
}

#[test]
fn send_menu_retargets_and_creates_receiver_as_single_undo_step() {
    let s = Daw::new(project());
    s.add_bus();
    let first = s.model.borrow().selected_bus.unwrap();
    s.add_bus();
    let second = s.model.borrow().selected_bus.unwrap();
    let target = RoutingTarget::Track(0);
    s.add_send(target, first);
    s.choose(0, None);
    let before = Daw::snapshot(&s.model.borrow());
    s.open_send_picker(target, Some(0));
    s.choose_send_destination(target, Some(0), 1);
    assert_eq!(
        s.model.borrow().project.tracks[0].routing.sends[0].target,
        second
    );
    assert_eq!(s.model.borrow().project.tracks[0].routing.sends.len(), 1);
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &before);
    s.open_send_picker(target, Some(0));
    s.choose_send_destination(target, Some(0), 2);
    assert_eq!(s.model.borrow().project.buses.len(), 3);
    assert_eq!(s.model.borrow().project.tracks[0].routing.sends.len(), 1);
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &before);
}

#[test]
fn send_level_popup_preserves_exact_noop_and_updates_live_with_undo() {
    let s = Daw::new(project());
    s.add_bus();
    let id = s.model.borrow().selected_bus.unwrap();
    let target = RoutingTarget::Track(0);
    s.add_send(target, id);
    s.choose(0, None);
    s.play();
    let controls = s.model.borrow().audio.as_ref().unwrap().controls.clone();
    let before = Daw::snapshot(&s.model.borrow());
    s.send_action(target, 0, 0);
    s.submit_send_level(target, 0);
    assert_snapshot(&s.model.borrow(), &before);
    s.send_action(target, 0, 0);
    s.routing_value.set("-6".into());
    s.submit_send_level(target, 0);
    assert!(Arc::ptr_eq(
        &controls,
        &s.model.borrow().audio.as_ref().unwrap().controls
    ));
    s.undo(false);
    assert_snapshot(&s.model.borrow(), &before);
}

#[test]
fn send_popup_captures_keys_instead_of_adjusting_the_underlying_knob() {
    let s = Daw::new(project());
    s.add_bus();
    let id = s.model.borrow().selected_bus.unwrap();
    let target = RoutingTarget::Track(0);
    s.add_send(target, id);
    s.choose(0, None);
    let _ = s.routing_rack(&s.model.borrow().project, target);
    s.send_controls
        .borrow()
        .values()
        .next()
        .unwrap()
        .focused
        .set(true);
    s.open_send_actions(target, 0);
    let before = Daw::snapshot(&s.model.borrow());
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.create_element());
    tree.layout(scarlet_ui::LayoutConstraints::tight(1280., 790.));
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    dispatcher.dispatch(
        &mut tree,
        &Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Down,
            modifiers: KeyModifiers::default(),
        }),
    );
    assert_eq!(s.menu_choice.get(), 1);
    assert_snapshot(&s.model.borrow(), &before);
    dispatcher.dispatch(
        &mut tree,
        &Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Escape,
            modifiers: KeyModifiers::default(),
        }),
    );
    assert!(s.dialog.get() == Dialog::None);
    assert_snapshot(&s.model.borrow(), &before);
}

#[test]
fn empty_send_slot_keeps_keyboard_focus_across_retained_rack_updates() {
    let s = Daw::new(project());
    let target = RoutingTarget::Track(0);
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(
        s.routing_rack(&s.model.borrow().project, target)
            .create_element(),
    );
    tree.layout(scarlet_ui::LayoutConstraints::tight(190., 700.));
    s.send_controls
        .borrow()
        .get(&(target, 0))
        .unwrap()
        .focused
        .set(true);
    let next = s.routing_rack(&s.model.borrow().project, target);
    tree.root_mut().unwrap().update(&next);
    tree.layout(scarlet_ui::LayoutConstraints::tight(190., 700.));
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    assert!(dispatcher.dispatch(
        &mut tree,
        &Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Enter,
            modifiers: KeyModifiers::default(),
        })
    ));
    assert!(s.dialog.get() == Dialog::SendPicker(target, None));
    assert!(
        !s.send_controls
            .borrow()
            .get(&(target, 0))
            .unwrap()
            .focused
            .get()
    );
}

#[test]
fn region_metadata_precedes_channel_routing_and_never_interrupts_send_to_fader() {
    wait_for_test_font();
    let s = Daw::new(project());
    let inspect = || {
        let mut tree = scarlet_ui::ElementTree::new();
        tree.set_root(s.inspector_panel().create_element());
        tree.layout(scarlet_ui::LayoutConstraints::tight(224., 616.));
        let mut text = Vec::new();
        text_layouts(tree.root().unwrap(), Point::ZERO, &mut text);
        fn button_labels(
            element: &dyn scarlet_ui::Element,
            parent: Point,
            out: &mut Vec<(String, Point, Size)>,
        ) {
            let origin = Point::new(
                parent.x + element.position().x,
                parent.y + element.position().y,
            );
            if element
                .type_name_debug()
                .contains("::views::button::ButtonRenderObject")
            {
                let mut paint = scarlet_ui::renderer::PaintContext::new();
                element.render_object().unwrap().paint(&mut paint, origin);
                for command in paint.commands() {
                    if let scarlet_ui::renderer::PaintCommand::DrawText { text, .. } = command {
                        out.push((text.clone(), origin, element.bounds().size));
                    }
                }
            }
            for child in element.children() {
                button_labels(child.as_ref(), origin, out);
            }
        }
        button_labels(tree.root().unwrap(), Point::ZERO, &mut text);
        let mut fields = Vec::new();
        control_bounds(
            tree.root().unwrap(),
            Point::ZERO,
            "::views::text_field::TextFieldRenderObject",
            &mut fields,
        );
        let mut faders = Vec::new();
        control_bounds(
            tree.root().unwrap(),
            Point::ZERO,
            "::fader::FaderRender",
            &mut faders,
        );
        (text, fields, faders)
    };
    let (text, fields, initial_faders) = inspect();
    assert!(
        !text
            .iter()
            .any(|(t, _, _)| t.starts_with("REGION") || t.contains("Region details"))
    );
    assert_eq!(fields.len(), 1);
    s.choose(0, Some(0));
    for expanded in [false, true] {
        s.inspector_details.set(expanded);
        let (text, fields, faders) = inspect();
        let y = |name: &str| {
            text.iter()
                .find(|(t, _, _)| t.starts_with(name))
                .unwrap_or_else(|| panic!("Missing {name}: {text:?}"))
                .1
                .y
        };
        assert!(y("REGION 1") < fields.last().unwrap().0.y);
        assert!(fields.last().unwrap().0.y < y("OUTPUT"));
        assert!(y("OUTPUT") < y("INSERTS") && y("INSERTS") < y("SENDS"));
        assert_eq!(fields.len(), if expanded { 3 } else { 1 });
        if expanded {
            assert!(y("Start ") < y("OUTPUT"));
            assert!(y("RANGE") < y("OUTPUT"));
            assert!(fields[0].0.y < fields[2].0.y);
            assert!(fields[1].0.y < fields[2].0.y);
        }
        assert_eq!(
            faders, initial_faders,
            "Expanding region metadata never reduces or moves the fader"
        );
        assert!(!text.iter().any(|(t, p, _)| p.y > y("SENDS")
            && (t.starts_with("REGION") || t.starts_with("RANGE") || t.starts_with("Start "))));
    }
    s.add_bus();
    let (text, fields, faders) = inspect();
    assert!(!text.iter().any(|(t, _, _)| t.starts_with("REGION")));
    assert_eq!(fields.len(), 1);
    let inputs = text.iter().find(|(t, _, _)| t == "INPUTS").unwrap().1.y;
    let output = text.iter().find(|(t, _, _)| t == "OUTPUT").unwrap().1.y;
    assert!(inputs < output);
    assert_eq!(faders, initial_faders);
}

#[test]
fn shared_channel_mute_solo_groups_center_on_fader_axes_without_aux_label_offset() {
    wait_for_test_font();
    let s = Daw::new(project());
    s.add_bus();
    let aux = s.model.borrow().selected_bus.unwrap();
    fn measure(
        view: AnyView,
        width: f32,
        height: f32,
    ) -> (Vec<(String, Point, Size)>, Vec<(Point, Size)>) {
        let mut tree = scarlet_ui::ElementTree::new();
        tree.set_root(view.create_element());
        tree.layout(scarlet_ui::LayoutConstraints::tight(width, height));
        fn controls(
            e: &dyn scarlet_ui::Element,
            parent: Point,
            out: &mut Vec<(String, Point, Size)>,
        ) {
            let origin = Point::new(parent.x + e.position().x, parent.y + e.position().y);
            if e.type_name_debug()
                .contains("::views::button::ButtonRenderObject")
            {
                let mut paint = scarlet_ui::renderer::PaintContext::new();
                e.render_object().unwrap().paint(&mut paint, origin);
                for command in paint.commands() {
                    if let scarlet_ui::renderer::PaintCommand::DrawText { text, .. } = command {
                        if text == "M" || text == "S" {
                            out.push((text.clone(), origin, e.bounds().size));
                        }
                    }
                }
            }
            for child in e.children() {
                controls(child.as_ref(), origin, out);
            }
        }
        let mut buttons = Vec::new();
        controls(tree.root().unwrap(), Point::ZERO, &mut buttons);
        let mut faders = Vec::new();
        control_bounds(
            tree.root().unwrap(),
            Point::ZERO,
            "::fader::FaderRender",
            &mut faders,
        );
        (buttons, faders)
    }
    let check = |buttons: &[(String, Point, Size)], fader: &(Point, Size), count: usize| {
        assert_eq!(buttons.len(), count);
        let left = buttons.first().unwrap().1.x;
        let last = buttons.last().unwrap();
        let right = last.1.x + last.2.width;
        let axis = fader.0.x + fader::Geometry::new(fader.1).axis;
        assert!(
            ((left + right) / 2. - axis).abs() < 0.01,
            "Buttons {buttons:?}, fader {fader:?}"
        );
        for (_, _, size) in buttons {
            assert_eq!(*size, Size::new(24., 24.));
        }
    };
    for width in [224., 260., 320.] {
        s.choose(0, None);
        let (buttons, faders) = measure(s.inspector_panel(), width, 616.);
        check(&buttons, &faders[0], 2);
        s.choose_bus(aux);
        let (buttons, faders) = measure(s.inspector_panel(), width, 616.);
        check(&buttons, &faders[0], 1);
    }
    for width in [560., 900., 1188.] {
        let (buttons, faders) = measure(s.mixer(), width, 382.);
        assert_eq!(buttons.len(), 5);
        check(&buttons[..2], &faders[0], 2);
        check(&buttons[2..4], &faders[1], 2);
        check(&buttons[4..], &faders[2], 1);
    }
}

#[test]
fn editor_session_survives_stop_seek_resume_and_eof_without_replacing_engine() {
    let mut app = Daw::new(project());
    app.play();
    // The device-free adapter models the editor pin on the real Playback owner.
    app.model.borrow_mut().audio.as_mut().unwrap().editor_open = true;
    let controls = app.model.borrow().audio.as_ref().unwrap().controls.clone();
    app.model.borrow().audio.as_ref().unwrap().render(1700);
    app.stop_audio(true);
    assert!(app.model.borrow().audio.is_none());
    let assert_retained = |app: &Daw| {
        let m = app.model.borrow();
        let audio = m
            .retired_audio
            .as_ref()
            .expect("Stop destroyed editor session");
        assert!(Arc::ptr_eq(&controls, &audio.controls));
        assert!(audio.has_open_editors());
        assert!(!audio.controls.playing.load(Ordering::Relaxed));
    };
    assert_retained(&app);
    app.seek(0.4);
    assert_retained(&app);
    {
        let m = app.model.borrow();
        let audio = m.retired_audio.as_ref().unwrap();
        audio.render(64);
        assert_eq!(controls.position.load(Ordering::Relaxed), 3200);
    }
    app.edit("Rename while stopped", |m| {
        m.project.tracks[0].name = "Same editor".into();
        Ok(())
    });
    app.play();
    assert!(Arc::ptr_eq(
        &controls,
        &app.model.borrow().audio.as_ref().unwrap().controls
    ));
    app.model.borrow().audio.as_ref().unwrap().render(64);
    assert_eq!(controls.position.load(Ordering::Relaxed), 3264);
    app.seek(0.7);
    app.model.borrow().audio.as_ref().unwrap().render(64);
    assert_eq!(controls.position.load(Ordering::Relaxed), 5664);
    app.model.borrow().audio.as_ref().unwrap().render(8000);
    app.last_playhead
        .set(Instant::now() - Duration::from_millis(40));
    app.on_idle();
    assert_retained(&app);
    app.seek(0.2);
    app.play();
    assert!(Arc::ptr_eq(
        &controls,
        &app.model.borrow().audio.as_ref().unwrap().controls
    ));
    app.model.borrow().audio.as_ref().unwrap().render(80);
    assert_eq!(controls.position.load(Ordering::Relaxed), 1680);
    assert!(!controls.error.load(Ordering::Relaxed));
}

#[test]
fn initially_stopped_editor_session_can_play_and_repeatedly_stop_without_replacement() {
    let app = Daw::new(project());
    let mut audio = TestAudio::start_paused(&app.model.borrow().project).unwrap();
    audio.editor_open = true;
    let controls = audio.controls.clone();
    audio.render(128);
    assert_eq!(controls.position.load(Ordering::Relaxed), 0);
    app.model.borrow_mut().retired_audio = Some(audio);
    app.play();
    assert!(Arc::ptr_eq(
        &controls,
        &app.model.borrow().audio.as_ref().unwrap().controls
    ));
    app.model.borrow().audio.as_ref().unwrap().render(256);
    assert_eq!(controls.position.load(Ordering::Relaxed), 256);
    app.stop_audio(true);
    app.stop_audio(true);
    assert!(Arc::ptr_eq(
        &controls,
        &app.model.borrow().retired_audio.as_ref().unwrap().controls
    ));
    assert!(!controls.playing.load(Ordering::Relaxed));
}

#[test]
#[ignore = "requires built bundled CLAP effect; run with RESONARA_CLAP_LIBRARY and --include-ignored"]
fn clap_bypass_preserves_editor_session_while_stopped_and_playing() {
    let mut p = project();
    p.tracks[0].routing.inserts.push(resonara_core::Insert {
        kind: resonara_core::InsertKind::Clap {
            plugin: resonara_core::plugins::load_bundled_gain().unwrap(),
        },
        bypass: true,
    });
    let app = Daw::new(p);
    let mut audio = TestAudio::start_paused(&app.model.borrow().project).unwrap();
    audio.editor_open = true;
    let controls = audio.controls.clone();
    app.model.borrow_mut().retired_audio = Some(audio);
    for _ in 0..3 {
        app.toggle_insert(RoutingTarget::Track(0), 0);
        {
            let m = app.model.borrow();
            let audio = m.retired_audio.as_ref().unwrap();
            assert!(audio.has_open_editors());
            assert!(Arc::ptr_eq(&controls, &audio.controls));
            audio.render(64);
            assert!(!controls.playing.load(Ordering::Relaxed));
        }
        app.play();
        app.toggle_insert(RoutingTarget::Track(0), 0);
        app.undo(false);
        app.undo(true);
        {
            let m = app.model.borrow();
            let audio = m.audio.as_ref().unwrap();
            assert!(audio.has_open_editors());
            assert!(Arc::ptr_eq(&controls, &audio.controls));
            audio.render(64);
            assert!(controls.playing.load(Ordering::Relaxed));
        }
        app.stop_audio(true);
    }
    assert!(!controls.error.load(Ordering::Relaxed));
}

#[test]
fn send_popups_and_pre_fader_changes_keep_valid_gpu_damage() {
    use scarlet_ui::renderer::{BackendFrame, PaintBackend, PaintContext};
    struct GpuDamageProbe(Size, u32, Rc<Cell<usize>>);
    impl PaintBackend for GpuDamageProbe {
        fn resize(&mut self, size: Size, scale: u32) {
            self.0 = size;
            self.1 = scale;
        }
        fn render<'a>(
            &'a mut self,
            _: &PaintContext<'_>,
            _: Color,
            logical: Option<&[scarlet_ui::geometry::Rect]>,
            physical: Option<&[scarlet_ui::compositor::DamageRect]>,
        ) -> scarlet_ui::Result<BackendFrame<'a>> {
            if let Some(damage) = physical {
                let width = (self.0.width * self.1 as f32 / 1000.).ceil() as u32;
                let height = (self.0.height * self.1 as f32 / 1000.).ceil() as u32;
                if !damage
                    .iter()
                    .any(|&(x, y, w, h)| w > 0 && h > 0 && x < width && y < height)
                {
                    eprintln!("Empty GPU damage: logical={logical:?}, physical={physical:?}");
                    return Err(scarlet_ui::Error::RenderError);
                }
            }
            self.2.set(self.2.get() + 1);
            Ok(BackendFrame::External)
        }
    }
    wait_for_test_font();
    let app = Daw::new(project());
    let aux = app.model.borrow_mut().project.add_bus("Aux", BusKind::Aux);
    let target = RoutingTarget::Track(0);
    app.add_send(target, aux);
    let mut pipeline = scarlet_ui::RenderingPipeline::new();
    pipeline.set_root(
        Window::new("Send GPU damage", app.clone())
            .size(Size::new(1280., 850.))
            .create_element(),
    );
    pipeline.layout_initial();
    let submissions = Rc::new(Cell::new(0));
    pipeline.set_paint_backend(Box::new(GpuDamageProbe(
        Size::ZERO,
        1000,
        submissions.clone(),
    )));
    let settle = |pipeline: &mut scarlet_ui::RenderingPipeline, action: &str| {
        let before = submissions.get();
        for frame in 0..8 {
            assert!(
                pipeline.render_for_present().is_ok(),
                "{action}: frame {frame}"
            );
        }
        assert!(
            submissions.get() > before,
            "{action} must still repaint visible changes"
        );
    };
    settle(&mut pipeline, "initial");
    for scale in [1000, 2000] {
        pipeline.set_scale_milli(scale);
        if scale != 1000 {
            settle(&mut pipeline, "scale change");
        }
        app.open_send_picker(target, Some(0));
        settle(&mut pipeline, "open send destination");
        app.choose_send_destination(target, Some(0), 0);
        settle(&mut pipeline, "choose Aux");
        app.open_send_actions(target, 0);
        settle(&mut pipeline, "open send actions");
        app.send_action(target, 0, 4);
        settle(&mut pipeline, "switch pre/post");
    }
    pipeline.teardown();
}
