use resonara_core::{live::*, *};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::{Arc, atomic::Ordering},
};

thread_local! {
    static AUDITING: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
    static FREES: Cell<usize> = const { Cell::new(0) };
}
struct Audit;
fn allocation() {
    AUDITING.with(|guard| {
        if guard.get() {
            ALLOCS.with(|count| count.set(count.get() + 1));
        }
    });
}
fn deallocation() {
    AUDITING.with(|guard| {
        if guard.get() {
            FREES.with(|count| count.set(count.get() + 1));
        }
    });
}
unsafe impl GlobalAlloc for Audit {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        deallocation();
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        allocation();
        deallocation();
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Audit = Audit;

fn project(sample: [f32; 2], frames: usize) -> Project {
    Project {
        tracks: vec![Track {
            name: "Live source".into(),
            clips: vec![Clip {
                source_channels: 2,
                edit: Default::default(),
                start: 0,
                source_offset: 0,
                frames,
                samples: Arc::new(vec![sample; frames]),
            }],
            gain: 1.,
            pan: 0.,
            mute: false,
            solo: false,
            routing: Default::default(),
        }],
        ..Project::default()
    }
}

fn identity(renderer: &mut PlaybackRenderer) -> usize {
    renderer.engine_mut() as *mut Engine as usize
}

fn audited_render(renderer: &mut PlaybackRenderer, output: &mut [f32]) {
    ALLOCS.with(|count| count.set(0));
    FREES.with(|count| count.set(0));
    AUDITING.with(|guard| guard.set(true));
    renderer.render(output, 2);
    AUDITING.with(|guard| guard.set(false));
    assert_eq!(ALLOCS.with(Cell::get), 0, "audio callback allocated");
    assert_eq!(FREES.with(Cell::get), 0, "audio callback freed memory");
}

#[test]
fn every_region_edit_preserves_engine_controls_and_already_rendered_delay_tail() {
    for edit in [
        "gain", "fades", "reverse", "trim", "split", "source", "duration",
    ] {
        let mut p = project([0.; 2], 32);
        let mut source = vec![[0.; 2]; 64];
        source[0] = [0.25, -0.5];
        p.tracks[0].clips[0].samples = Arc::new(source);
        p.tracks[0].routing.inserts.push(Insert {
            kind: InsertKind::Delay { frames: 3 },
            bypass: false,
        });
        let (mut playback, mut renderer) = Playback::new(&p, 48_000, 0, false).unwrap();
        let controls = playback.controls.clone();
        let engine = identity(&mut renderer);
        let mut first = [1.; 2];
        audited_render(&mut renderer, &mut first);
        assert_eq!(first, [0.; 2]);
        match edit {
            "gain" => p.tracks[0].clips[0].set_gain_db(-6.).unwrap(),
            "fades" => p.tracks[0].clips[0].set_fades(4, 4).unwrap(),
            "reverse" => p.tracks[0].clips[0].set_reversed(true),
            "trim" => p.tracks[0].clips[0].trim_relative(1, 32).unwrap(),
            "split" => {
                let right = p.tracks[0].clips[0].split_relative(8).unwrap();
                p.tracks[0].clips.push(right);
            }
            "source" => p.tracks[0].clips[0].samples = Arc::new(vec![[0.; 2]; 64]),
            "duration" => p.tracks[0].clips[0].frames = 48,
            _ => unreachable!(),
        }
        playback.update(&p).unwrap();
        assert!(
            Arc::ptr_eq(&controls, &playback.controls),
            "{edit} replaced controls"
        );
        assert_eq!(controls.position.load(Ordering::Relaxed), 1);
        let mut tail = [1.; 6];
        audited_render(&mut renderer, &mut tail);
        assert_eq!(
            identity(&mut renderer),
            engine,
            "{edit} replaced the DSP engine"
        );
        assert_eq!(
            tail,
            [0., 0., 0., 0., 0.25, -0.5],
            "{edit} erased the delay tail"
        );
        assert_eq!(controls.position.load(Ordering::Relaxed), 4);
        assert!(controls.playing.load(Ordering::Relaxed));
        drop(renderer);
        playback.collect_retired();
    }
}

#[test]
fn source_duration_extension_updates_eof_without_replacing_the_engine() {
    let mut p = project([0.25, -0.5], 12);
    p.tracks[0].clips[0].frames = 4;
    let (mut playback, mut renderer) = Playback::new(&p, 48_000, 0, false).unwrap();
    let engine = identity(&mut renderer);
    let mut output = [0.; 4];
    audited_render(&mut renderer, &mut output);
    p.tracks[0].clips[0].frames = 10;
    playback.update(&p).unwrap();
    let mut extended = [0.; 18];
    audited_render(&mut renderer, &mut extended);
    assert_eq!(identity(&mut renderer), engine);
    assert_eq!(&extended[..16], &[0.25, -0.5].repeat(8));
    assert_eq!(&extended[16..], &[0.; 2]);
    assert_eq!(playback.controls.position.load(Ordering::Relaxed), 10);
    assert!(!playback.controls.playing.load(Ordering::Relaxed));
    drop(renderer);
}

#[test]
fn source_adoption_keeps_the_fractional_device_clock_through_repeated_splits() {
    let mut p = project([0.; 2], 2_000);
    p.sample_rate = 44_100;
    p.tracks[0].clips[0].samples = Arc::new(
        (0..2_000)
            .map(|i| [i as f32 / 4_000., -(i as f32) / 4_000.])
            .collect(),
    );
    let (mut playback, mut renderer) = Playback::new(&p, 48_000, 73, false).unwrap();
    let controls = playback.controls.clone();
    let engine = identity(&mut renderer);
    let mut oracle = Engine::new(&p, Arc::new(Controls::new(&p)), 48_000, 73);
    let mut output = [0.; 256];
    let mut expected = [0.; 256];
    for (frames, split) in [(1, 99), (127, 203), (3, 251), (128, 400)] {
        audited_render(&mut renderer, &mut output[..frames * 2]);
        oracle.render(&mut expected[..frames * 2], 2);
        assert_eq!(&output[..frames * 2], &expected[..frames * 2]);
        p.split(0, split).unwrap();
        playback.update(&p).unwrap();
        assert!(Arc::ptr_eq(&controls, &playback.controls));
    }
    audited_render(&mut renderer, &mut output);
    oracle.render(&mut expected, 2);
    assert_eq!(output, expected);
    assert_eq!(identity(&mut renderer), engine);
    assert!(controls.playing.load(Ordering::Relaxed));
    drop(renderer);
}

#[test]
fn rapid_region_updates_coalesce_and_release_old_sources_only_on_control_thread() {
    let mut p = project([0.1, -0.2], 1_000);
    let original_source = Arc::downgrade(&p.tracks[0].clips[0].samples);
    let (mut playback, mut renderer) = Playback::new(&p, 48_000, 0, false).unwrap();
    let engine = identity(&mut renderer);
    let controls = playback.controls.clone();
    for value in [0.2, 0.3, 0.4] {
        p.tracks[0].clips[0].samples = Arc::new(vec![[value, -value]; 1_000]);
        playback.update(&p).unwrap();
    }
    assert!(original_source.upgrade().is_some());
    let mut output = [0.; 2];
    audited_render(&mut renderer, &mut output);
    assert_eq!(output, [0.4, -0.4]);
    assert_eq!(identity(&mut renderer), engine);
    assert!(Arc::ptr_eq(&controls, &playback.controls));
    // The audio callback must transfer ownership of the replaced source into
    // retirement; the last strong reference is released on this thread.
    assert!(original_source.upgrade().is_some());
    playback.collect_retired();
    assert!(original_source.upgrade().is_none());
    for step in 0..20 {
        let value = if step % 2 == 0 { 0.25 } else { 0.5 };
        p.tracks[0].clips[0].samples = Arc::new(vec![[value, -value]; 1_000]);
        playback.update(&p).unwrap();
        audited_render(&mut renderer, &mut output);
        assert_eq!(output, [value, -value]);
    }
    assert_eq!(playback.controls.position.load(Ordering::Relaxed), 21);
    assert_eq!(identity(&mut renderer), engine);
    drop(renderer);
}

#[test]
fn rejected_source_edits_preserve_pending_audio_and_live_mixer_controls() {
    let mut p = project([0.25, -0.125], 64);
    let (mut playback, mut renderer) = Playback::new(&p, 48_000, 0, false).unwrap();
    let engine = identity(&mut renderer);
    let controls = playback.controls.clone();
    p.tracks[0].clips[0].samples = Arc::new(vec![[0.5, -0.25]; 64]);
    playback.update(&p).unwrap();
    for invalid_gain in [true, false] {
        let mut invalid = p.clone();
        invalid.master = 0.;
        invalid.tracks[0].gain = 0.;
        if invalid_gain {
            invalid.tracks[0].clips[0].edit.gain_db = f32::NAN;
        } else {
            invalid.tracks[0].clips[0].source_offset = usize::MAX;
        }
        assert!(playback.update(&invalid).is_err());
        assert!(Arc::ptr_eq(&controls, &playback.controls));
        assert_eq!(f32::from_bits(controls.master.load(Ordering::Relaxed)), 1.);
        assert_eq!(
            f32::from_bits(controls.tracks[0].gain.load(Ordering::Relaxed)),
            1.
        );
    }
    let mut output = [0.; 2];
    audited_render(&mut renderer, &mut output);
    assert_eq!(output, [0.5, -0.25]);
    assert_eq!(identity(&mut renderer), engine);
    assert_eq!(controls.position.load(Ordering::Relaxed), 1);
    assert!(controls.playing.load(Ordering::Relaxed));
    drop(renderer);
}

#[test]
fn mixed_graph_and_source_updates_publish_the_latest_complete_project() {
    let mut p = project([0.1, -0.2], 1_000);
    let (mut playback, mut renderer) = Playback::new(&p, 48_000, 0, false).unwrap();
    let position = playback.controls.position.clone();
    let original_engine = identity(&mut renderer);
    let mut output = [0.; 2];
    audited_render(&mut renderer, &mut output);
    // A source snapshot pending for the old engine is superseded by a graph
    // update; another source edit must amend that complete pending graph.
    p.tracks[0].clips[0].samples = Arc::new(vec![[0.25, -0.5]; 1_000]);
    playback.update(&p).unwrap();
    p.tracks[0].routing.inserts.push(Insert {
        kind: InsertKind::Gain { gain: 0.5 },
        bypass: false,
    });
    playback.update(&p).unwrap();
    let graph_controls = playback.controls.clone();
    p.tracks[0].clips[0].samples = Arc::new(vec![[0.5, -0.75]; 1_000]);
    playback.update(&p).unwrap();
    assert!(Arc::ptr_eq(&graph_controls, &playback.controls));
    assert!(Arc::ptr_eq(&position, &playback.controls.position));
    audited_render(&mut renderer, &mut output);
    assert_eq!(output, [0.25, -0.375]);
    let graph_engine = identity(&mut renderer);
    assert_ne!(graph_engine, original_engine);
    assert_eq!(position.load(Ordering::Relaxed), 2);

    // Retire the old engine, then adopt a source-only change against the graph
    // that actually became active, retaining its engine and controls.
    p.tracks[0].clips[0].samples = Arc::new(vec![[0.75, -0.25]; 1_000]);
    playback.update(&p).unwrap();
    audited_render(&mut renderer, &mut output);
    assert_eq!(output, [0.375, -0.125]);
    assert_eq!(identity(&mut renderer), graph_engine);
    assert!(Arc::ptr_eq(&graph_controls, &playback.controls));

    // Multiple full graph generations can also coalesce around source edits.
    p.tracks[0].routing.inserts[0].kind = InsertKind::Gain { gain: 0.25 };
    playback.update(&p).unwrap();
    p.tracks[0].clips[0].samples = Arc::new(vec![[0.4, -0.2]; 1_000]);
    playback.update(&p).unwrap();
    p.tracks[0].routing.inserts[0].kind = InsertKind::Gain { gain: 2. };
    playback.update(&p).unwrap();
    p.tracks[0].clips[0].samples = Arc::new(vec![[0.2, -0.1]; 1_000]);
    playback.update(&p).unwrap();
    audited_render(&mut renderer, &mut output);
    assert_eq!(output, [0.4, -0.2]);
    assert_eq!(position.load(Ordering::Relaxed), 4);
    assert!(playback.controls.playing.load(Ordering::Relaxed));
    drop(renderer);
    playback.collect_retired();
}

#[test]
fn changing_track_layouts_never_adopts_sources_from_another_graph_generation() {
    let mut p = project([0.1, -0.2], 256);
    let (mut playback, mut renderer) = Playback::new(&p, 48_000, 0, false).unwrap();
    let position = playback.controls.position.clone();
    let original_engine = identity(&mut renderer);
    let mut output = [0.; 2];
    audited_render(&mut renderer, &mut output);
    assert_eq!(output, [0.1, -0.2]);

    p.tracks.push(project([0.2, -0.1], 256).tracks.remove(0));
    playback.update(&p).unwrap();
    let two_track_controls = playback.controls.clone();
    assert_eq!(two_track_controls.tracks.len(), 2);
    p.tracks[1].clips[0].samples = Arc::new(vec![[0.4, -0.3]; 256]);
    playback.update(&p).unwrap();
    assert!(Arc::ptr_eq(&two_track_controls, &playback.controls));

    // Invalid source preparation while a two-track graph is pending must not
    // discard that graph or leak the failed edit's mixer changes into it.
    let mut invalid = p.clone();
    invalid.tracks[1].clips[0].source_offset = usize::MAX;
    invalid.master = 0.;
    assert!(playback.update(&invalid).is_err());
    assert!(Arc::ptr_eq(&two_track_controls, &playback.controls));
    assert_eq!(identity(&mut renderer), original_engine);
    audited_render(&mut renderer, &mut output);
    assert_eq!(output, [0.5, -0.5]);
    let two_track_engine = identity(&mut renderer);
    assert_ne!(two_track_engine, original_engine);
    assert_eq!(position.load(Ordering::Relaxed), 2);

    // A pending snapshot for two tracks must be superseded by the one-track
    // graph, then amended with the latest one-track source before adoption.
    p.tracks[1].clips[0].samples = Arc::new(vec![[0.25, -0.125]; 256]);
    playback.update(&p).unwrap();
    p.tracks.remove(0);
    playback.update(&p).unwrap();
    let one_track_controls = playback.controls.clone();
    assert_eq!(one_track_controls.tracks.len(), 1);
    p.tracks[0].clips[0].samples = Arc::new(vec![[0.75, -0.5]; 256]);
    playback.update(&p).unwrap();
    assert!(Arc::ptr_eq(&one_track_controls, &playback.controls));
    audited_render(&mut renderer, &mut output);
    assert_eq!(output, [0.75, -0.5]);
    assert_ne!(identity(&mut renderer), two_track_engine);
    assert!(Arc::ptr_eq(&position, &playback.controls.position));
    assert_eq!(position.load(Ordering::Relaxed), 3);
    assert!(playback.controls.playing.load(Ordering::Relaxed));
    assert!(!playback.controls.error.load(Ordering::Relaxed));
    drop(renderer);
    playback.collect_retired();
}

#[test]
fn concurrent_graph_and_source_publication_keeps_transport_and_final_audio_consistent() {
    let mut p = project([0.125, -0.25], 100_000);
    let (mut playback, renderer) = Playback::new(&p, 48_000, 0, false).unwrap();
    let position = playback.controls.position.clone();
    let worker = std::thread::spawn(move || {
        let mut renderer = renderer;
        let mut samples = [0.; 2];
        for _ in 0..10_000 {
            audited_render(&mut renderer, &mut samples);
            assert!(samples.iter().all(|sample| sample.is_finite()));
            std::thread::yield_now();
        }
        renderer
    });
    for index in 0..200 {
        let value = if index % 2 == 0 { 0.125 } else { 0.25 };
        p.tracks[0].clips[0].samples = Arc::new(vec![[value, -value]; 100_000]);
        playback.update(&p).unwrap();
        if index % 3 == 0 {
            p.tracks[0].routing.inserts = vec![Insert {
                kind: InsertKind::Gain {
                    gain: if index % 2 == 0 { 0.5 } else { 2. },
                },
                bypass: false,
            }];
            playback.update(&p).unwrap();
        }
        std::thread::yield_now();
    }
    // The final source and full graph may race with an earlier adoption. They
    // must eventually arrive together, with no unannounced transport reset.
    p.tracks[0].routing.inserts = vec![Insert {
        kind: InsertKind::Gain { gain: 0.5 },
        bypass: false,
    }];
    playback.update(&p).unwrap();
    p.tracks[0].clips[0].samples = Arc::new(vec![[0.75, -0.25]; 100_000]);
    playback.update(&p).unwrap();
    let mut renderer = worker.join().unwrap();
    let mut output = [0.; 2];
    // Drain the bounded retirement slot before each callback so any update
    // deferred by backpressure can be adopted.
    for _ in 0..3 {
        playback.collect_retired();
        audited_render(&mut renderer, &mut output);
    }
    assert_eq!(output, [0.375, -0.125]);
    assert!(Arc::ptr_eq(&position, &playback.controls.position));
    assert_eq!(position.load(Ordering::Relaxed), 10_003);
    assert!(playback.controls.playing.load(Ordering::Relaxed));
    assert!(!playback.controls.error.load(Ordering::Relaxed));
    drop(renderer);
    playback.collect_retired();
}
