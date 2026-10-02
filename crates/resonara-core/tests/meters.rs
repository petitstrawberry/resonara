use resonara_core::*;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

fn track(samples: Vec<[f32; 2]>, gain: f32, pan: f32) -> Track {
    Track {
        name: "stereo meter".into(),
        clips: vec![Clip {
            source_channels: 2,
            start: 0,
            source_offset: 0,
            frames: samples.len(),
            samples: Arc::new(samples),
        }],
        gain,
        pan,
        mute: false,
        solo: false,
        routing: resonara_core::ChannelRouting::default(),
    }
}
fn take(peak: &AtomicU32) -> f32 {
    f32::from_bits(peak.swap(0, Ordering::Relaxed))
}
fn take_track(mix: &Mixer) -> [f32; 2] {
    [take(&mix.peak_left), take(&mix.peak_right)]
}
fn take_master(controls: &Controls) -> [f32; 2] {
    [
        take(&controls.master_peak_left),
        take(&controls.master_peak_right),
    ]
}
fn setup(tracks: Vec<Track>, master: f32) -> (Engine, Arc<Controls>) {
    let p = Project {
        tracks,
        master,
        ..Project::default()
    };
    let controls = Arc::new(Controls::new(&p));
    (Engine::new(&p, controls.clone(), 48000, 0), controls)
}

#[test]
fn track_stereo_peaks_follow_unequal_channels_and_live_left_center_right_pan() {
    let (mut engine, controls) = setup(vec![track(vec![[0.5, -0.25]; 32], 0.5, -1.0)], 1.0);
    assert_eq!(take_track(&controls.tracks[0]), [0.0; 2]);
    assert_eq!(take_master(&controls), [0.0; 2]);
    let mut out = [0.0; 8];
    for (pan, expected) in [
        (-1.0f32, [0.25, 0.0]),
        (0.0, [0.25, 0.125]),
        (1.0, [0.0, 0.125]),
    ] {
        controls.tracks[0]
            .pan
            .store(pan.to_bits(), Ordering::Relaxed);
        engine.render(&mut out, 2);
        assert_eq!(take_track(&controls.tracks[0]), expected);
        assert_eq!(take(&controls.tracks[0].peak), expected[0].max(expected[1]));
        assert_eq!(take_master(&controls), expected);
        assert_eq!([out[0].abs(), out[1].abs()], expected);
    }
}

#[test]
fn meters_accumulate_each_channel_peak_until_consumed_then_reset() {
    let (mut engine, controls) = setup(
        vec![track(
            vec![[0.75, 0.125], [-0.25, -0.5], [0.125, 0.0625]],
            1.0,
            0.0,
        )],
        0.5,
    );
    let mut out = [0.0; 2];
    engine.render(&mut out, 2);
    engine.render(&mut out, 2);
    assert_eq!(take_track(&controls.tracks[0]), [0.75, 0.5]);
    assert_eq!(take_master(&controls), [0.375, 0.25]);
    assert_eq!(take_track(&controls.tracks[0]), [0.0; 2]);
    assert_eq!(take_master(&controls), [0.0; 2]);
    engine.render(&mut out, 2);
    assert_eq!(take_track(&controls.tracks[0]), [0.125, 0.0625]);
    assert_eq!(take_master(&controls), [0.0625, 0.03125]);
}

#[test]
fn mute_and_solo_gate_each_track_meter_and_the_actual_master_sum() {
    let (mut engine, controls) = setup(
        vec![
            track(vec![[0.5, 0.25]; 16], 1.0, 0.0),
            track(vec![[0.125, -0.125]; 16], 1.0, 0.0),
        ],
        1.0,
    );
    let mut out = [0.0; 2];
    controls.tracks[0].mute.store(true, Ordering::Relaxed);
    engine.render(&mut out, 2);
    assert_eq!(take_track(&controls.tracks[0]), [0.0; 2]);
    assert_eq!(take_track(&controls.tracks[1]), [0.125; 2]);
    assert_eq!(take_master(&controls), [0.125; 2]);
    controls.tracks[0].mute.store(false, Ordering::Relaxed);
    controls.tracks[0].solo.store(true, Ordering::Relaxed);
    engine.render(&mut out, 2);
    assert_eq!(take_track(&controls.tracks[0]), [0.5, 0.25]);
    assert_eq!(take_track(&controls.tracks[1]), [0.0; 2]);
    assert_eq!(take_master(&controls), [0.5, 0.25]);
    controls.tracks[0].mute.store(true, Ordering::Relaxed);
    engine.render(&mut out, 2);
    assert_eq!(take_track(&controls.tracks[0]), [0.0; 2]);
    assert_eq!(take_track(&controls.tracks[1]), [0.0; 2]);
    assert_eq!(take_master(&controls), [0.0; 2]);
}

#[test]
fn master_peak_measures_summed_samples_not_the_sum_of_track_peaks() {
    let (mut engine, controls) = setup(
        vec![
            track(vec![[0.5, 0.25], [0.0, -0.5]], 1.0, 0.0),
            track(vec![[-0.5, 0.125], [0.25, 0.25]], 1.0, 0.0),
        ],
        0.5,
    );
    let mut out = [0.0; 4];
    engine.render(&mut out, 2);
    assert_eq!(out, [0.0, 0.1875, 0.125, -0.125]);
    assert_eq!(take_track(&controls.tracks[0]), [0.5, 0.5]);
    assert_eq!(take_track(&controls.tracks[1]), [0.5, 0.25]);
    assert_eq!(take_master(&controls), [0.125, 0.1875]);
}

#[test]
fn master_meter_is_post_master_gain_pre_clip_and_live_gain_changes_are_real() {
    let (mut engine, controls) = setup(
        vec![
            track(vec![[0.75, -0.25]; 8], 1.0, 0.0),
            track(vec![[0.25, 0.125]; 8], 1.0, 0.0),
        ],
        2.0,
    );
    let mut out = [0.0; 2];
    engine.render(&mut out, 2);
    assert_eq!(out, [1.0, -0.25]);
    assert_eq!(take_master(&controls), [2.0, 0.25]);
    assert_eq!(take_track(&controls.tracks[0]), [0.75, 0.25]);
    controls.master.store(0.5f32.to_bits(), Ordering::Relaxed);
    engine.render(&mut out, 2);
    assert_eq!(out, [0.5, -0.0625]);
    assert_eq!(take_master(&controls), [0.5, 0.0625]);
    assert_eq!(take_track(&controls.tracks[0]), [0.75, 0.25]);
}

#[test]
fn master_stereo_meter_precedes_mono_mapping_and_pause_generates_no_peaks() {
    let (mut engine, controls) = setup(vec![track(vec![[0.5, -0.25]; 8], 1.0, 0.0)], 0.5);
    let mut out = [0.0; 1];
    engine.render(&mut out, 1);
    assert_eq!(out, [0.0625]);
    assert_eq!(take_master(&controls), [0.25, 0.125]);
    take_track(&controls.tracks[0]);
    controls.playing.store(false, Ordering::Relaxed);
    engine.render(&mut out, 1);
    assert_eq!(out, [0.0]);
    assert_eq!(take_master(&controls), [0.0; 2]);
    assert_eq!(take_track(&controls.tracks[0]), [0.0; 2]);
}

#[test]
fn master_meter_includes_actual_pre_post_send_return_output_not_dry_track_peaks() {
    use resonara_core::graph::*;
    let p = Project {
        tracks: vec![
            track(vec![[0.25, -0.125]; 8], 0.5, 0.5),
            track(vec![[0.75; 2]; 8], 1.0, 0.0),
        ],
        master: 0.5,
        ..Project::default()
    };
    let controls = Arc::new(Controls::new(&p));
    let graph = RoutingGraph {
        nodes: vec![
            Node {
                id: NodeId(1),
                processor: Processor::TrackSource { track: 0 },
            },
            Node {
                id: NodeId(2),
                processor: Processor::TrackFader { track: 0 },
            },
            Node {
                id: NodeId(3),
                processor: Processor::Bus,
            },
            Node {
                id: NodeId(4),
                processor: Processor::Gain { gain: 2.0 },
            },
            Node {
                id: NodeId(5),
                processor: Processor::Bus,
            },
        ],
        routes: [
            (1, 2, 1.0),
            (1, 3, 0.5),
            (2, 3, 0.25),
            (3, 4, 1.0),
            (4, 5, 1.0),
            (2, 5, 1.0),
        ]
        .into_iter()
        .map(|(from, to, gain)| Route {
            from: NodeId(from),
            to: NodeId(to),
            gain,
        })
        .collect(),
        output: NodeId(5),
    };
    let mut engine = Engine::with_graph(
        &p,
        controls.clone(),
        48000,
        0,
        &graph,
        GraphLimits::default(),
    )
    .unwrap();
    let mut out = [0.0; 4];
    engine.render(&mut out, 2);
    assert_eq!(out, [0.171875, -0.109375, 0.171875, -0.109375]);
    assert_eq!(take_track(&controls.tracks[0]), [0.0625, 0.0625]);
    assert_eq!(take_track(&controls.tracks[1]), [0.0; 2]);
    assert_eq!(take_master(&controls), [0.171875, 0.109375]);
}
