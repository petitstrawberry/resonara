use resonara_core::{live::*, *};
use std::sync::{Arc, atomic::Ordering};

fn source() -> Project {
    let mut p = Project::demo();
    p.tracks.truncate(1);
    p.tracks[0].gain = 1.;
    p.tracks[0].pan = 0.;
    p.tracks[0].clips = vec![Clip {
        source_channels: 2,
        start: 0,
        source_offset: 0,
        frames: 100_000,
        samples: Arc::new(vec![[0.125, -0.25]; 100_000]),
    }];
    p.master = 1.;
    p
}
fn render(renderer: &mut PlaybackRenderer, frames: usize) -> Vec<f32> {
    let mut output = vec![0.; frames * 2];
    renderer.render(&mut output, 2);
    output
}

#[test]
fn graph_changes_continue_at_exact_fractional_position_without_a_silent_block() {
    let mut p = source();
    p.sample_rate = 44_100;
    p.tracks[0].clips[0].samples = Arc::new(
        (0..100_000)
            .map(|i| [i as f32 / 200_000., -(i as f32) / 200_000.])
            .collect(),
    );
    let (mut playback, mut renderer) = Playback::new(&p, 48_000, 73, false).unwrap();
    let mut oracle = Engine::new(&p, Arc::new(Controls::new(&p)), 48_000, 73);
    for frames in [1, 127, 255, 3, 128] {
        let mut expected = vec![0.; frames * 2];
        oracle.render(&mut expected, 2);
        assert_eq!(render(&mut renderer, frames), expected);
        // Adding a unity insert changes the graph without changing the samples.
        p.tracks[0].routing.inserts.push(Insert {
            kind: InsertKind::Gain { gain: 1. },
            bypass: false,
        });
        let position = playback.controls.position.load(Ordering::Relaxed);
        playback.update(&p).unwrap();
        assert_eq!(playback.controls.position.load(Ordering::Relaxed), position);
        assert!(playback.controls.playing.load(Ordering::Relaxed));
    }
    let mut expected = vec![0.; 64];
    oracle.render(&mut expected, 2);
    assert_eq!(render(&mut renderer, 32), expected);
    drop(renderer);
}

#[test]
fn metadata_mixer_and_builtin_bypass_preserve_engine_and_delay_state() {
    let mut p = source();
    p.tracks[0].clips[0].samples = Arc::new({
        let mut s = vec![[0.; 2]; 100_000];
        s[0] = [0.125, -0.25];
        s
    });
    p.tracks[0].routing.inserts.push(Insert {
        kind: InsertKind::Delay { frames: 2 },
        bypass: false,
    });
    let (mut playback, mut renderer) = Playback::new(&p, p.sample_rate, 0, false).unwrap();
    let controls = playback.controls.clone();
    assert_eq!(render(&mut renderer, 1), [0., 0.]);
    p.tracks[0].name = "Renamed while playing".into();
    p.tracks[0].routing.inserts[0].bypass = true;
    playback.update(&p).unwrap();
    assert!(Arc::ptr_eq(&controls, &playback.controls));
    assert_eq!(render(&mut renderer, 1), [0., 0.]);
    p.tracks[0].routing.inserts[0].bypass = false;
    playback.update(&p).unwrap();
    assert_eq!(render(&mut renderer, 1), [0., 0.]);
    assert_eq!(render(&mut renderer, 1), [0.125, -0.25]);
    assert!(controls.playing.load(Ordering::Relaxed));
    drop(renderer);
}

#[test]
fn rapid_edits_coalesce_and_retirement_backpressure_never_stalls_audio() {
    let mut p = source();
    let (mut playback, mut renderer) = Playback::new(&p, p.sample_rate, 0, false).unwrap();
    p.tracks[0].routing.inserts.push(Insert {
        kind: InsertKind::Gain { gain: 0.5 },
        bypass: false,
    });
    playback.update(&p).unwrap();
    assert_eq!(render(&mut renderer, 1), [0.0625, -0.125]);
    p.tracks[0].routing.inserts[0].kind = InsertKind::Gain { gain: 0.25 };
    playback.update(&p).unwrap(); // Collects the preceding retired graph.
    assert_eq!(render(&mut renderer, 1), [0.03125, -0.0625]);
    p.tracks[0].routing.inserts[0].kind = InsertKind::Gain { gain: 0.75 };
    playback.update(&p).unwrap();
    p.tracks[0].routing.inserts[0].kind = InsertKind::Gain { gain: 2. };
    playback.update(&p).unwrap(); // Supersedes the unpublished 0.75 graph.
    assert_eq!(render(&mut renderer, 1), [0.25, -0.5]);
    for _ in 0..20 {
        assert_eq!(render(&mut renderer, 1), [0.25, -0.5]);
    }
    assert_eq!(playback.controls.position.load(Ordering::Relaxed), 23);
    drop(renderer);
}

#[test]
fn rejected_graph_keeps_transport_controls_and_the_previous_sound() {
    let mut p = source();
    let (mut playback, mut renderer) = Playback::new(&p, p.sample_rate, 0, false).unwrap();
    let controls = playback.controls.clone();
    let bus = p.add_bus("Cycle", BusKind::Aux);
    p.bus_mut(bus).unwrap().routing.output = Destination::Bus(bus);
    assert!(playback.update(&p).is_err());
    assert!(Arc::ptr_eq(&controls, &playback.controls));
    assert_eq!(render(&mut renderer, 3), [0.125, -0.25].repeat(3));
    assert_eq!(controls.position.load(Ordering::Relaxed), 3);
    drop(renderer);
}

#[test]
fn concurrent_graph_publication_and_retirement_remain_bounded() {
    let mut p = source();
    let (mut playback, renderer) = Playback::new(&p, p.sample_rate, 0, false).unwrap();
    let worker = std::thread::spawn(move || {
        let mut renderer = renderer;
        let mut samples = [0.; 2];
        for _ in 0..10_000 {
            renderer.render(&mut samples, 2);
            std::thread::yield_now();
        }
        renderer
    });
    for i in 0..500 {
        p.tracks[0].routing.inserts = vec![Insert {
            kind: InsertKind::Gain {
                gain: if i % 2 == 0 { 0.5 } else { 2. },
            },
            bypass: false,
        }];
        playback.update(&p).unwrap();
        std::thread::yield_now();
    }
    let mut renderer = worker.join().unwrap();
    playback.collect_retired();
    assert_eq!(render(&mut renderer, 1), [0.25, -0.5]);
    assert_eq!(playback.controls.position.load(Ordering::Relaxed), 10_001);
    drop(renderer);
}
