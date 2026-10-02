use resonara_core::{live::Playback, *};
use std::sync::{Arc, atomic::Ordering};

fn clip(values: &[f32]) -> Clip {
    Clip {
        source_channels: 2,
        start: 0,
        source_offset: 0,
        frames: values.len(),
        samples: Arc::new(values.iter().map(|value| [*value, -*value]).collect()),
        edit: Default::default(),
    }
}

fn project(clip: Clip) -> Project {
    Project {
        tracks: vec![Track {
            name: "Edited region".into(),
            clips: vec![clip],
            gain: 1.,
            pan: 0.,
            mute: false,
            solo: false,
            routing: Default::default(),
        }],
        ..Project::default()
    }
}

fn render(project: &Project, rate: u32, frames: usize) -> Vec<f32> {
    let mut engine = Engine::new(project, Arc::new(Controls::new(project)), rate, 0);
    let mut output = vec![0.; frames * 2];
    engine.render(&mut output, 2);
    output
}

fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
}

#[test]
fn edits_preserve_source_samples_and_apply_gain_fades_and_reverse() {
    let mut region = clip(&[0.1, 0.2, 0.3, 0.4, 0.5, 0.6]);
    let original = region.samples.clone();
    region.edit.reversed = true;
    region.edit.gain_db = -6.020_6;
    region.set_fades(3, 3).unwrap();
    for (index, expected) in [0., 0.125, 0.2, 0.15, 0.05, 0.].into_iter().enumerate() {
        near(region.sample_at(index as f64)[0], expected);
        near(region.sample_at(index as f64)[1], -expected);
    }
    assert!(Arc::ptr_eq(&original, &region.samples));
    assert_eq!(original[0], [0.1, -0.1]);
    assert_eq!(original[5], [0.6, -0.6]);
}

#[test]
fn sample_interpolation_holds_the_regions_own_endpoint() {
    let mut region = clip(&[0.1, 0.2, 0.4, 0.8]);
    region.source_offset = 1;
    region.frames = 2;
    assert_eq!(region.sample_at(0.5), [0.3, -0.3]);
    assert_eq!(region.sample_at(1.5), [0.4, -0.4]);
    assert_eq!(region.sample_at(-0.1), [0.; 2]);
    assert_eq!(region.sample_at(2.), [0.; 2]);
    region.edit.reversed = true;
    assert_eq!(region.sample_at(0.), [0.4, -0.4]);
    assert_eq!(region.sample_at(1.5), [0.2, -0.2]);
}

#[test]
fn source_ranges_follow_direction_trim_offsets_and_clamped_region_bounds() {
    let mut region = clip(&[0.; 8]);
    region.source_offset = 2;
    region.frames = 5;
    assert_eq!(region.source_range(0, 2), 2..4);
    assert_eq!(region.source_range(1, usize::MAX), 3..7);
    assert_eq!(region.source_range(usize::MAX, 0), 7..7);
    assert_eq!(region.source_range(3, 1), 5..5);
    let mut reversed = region.clone();
    reversed.set_reversed(true);
    assert_eq!(reversed.source_range(0, 2), 5..7);
    assert_eq!(reversed.source_range(1, usize::MAX), 2..6);
    assert_eq!(reversed.source_range(usize::MAX, 0), 2..2);
    assert_eq!(reversed.source_range(3, 1), 4..4);
    region.trim_relative(1, 4).unwrap();
    reversed.trim_relative(1, 4).unwrap();
    assert_eq!(region.source_offset, 3);
    assert_eq!(reversed.source_offset, 3);
    assert_eq!(region.source_range(0, usize::MAX), 3..6);
    assert_eq!(reversed.source_range(0, usize::MAX), 3..6);
    assert_eq!(region.source_range(0, 1), 3..4);
    assert_eq!(reversed.source_range(0, 1), 5..6);
}

#[test]
fn reversed_trim_keeps_the_audible_slice_and_original_fade_envelope() {
    let mut region = clip(&[0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8]);
    region.start = 20;
    region.edit.reversed = true;
    region.edit.gain_db = -3.;
    region.set_fades(6, 4).unwrap();
    let original = region.clone();
    region.trim_relative(2, 7).unwrap();
    assert_eq!(region.start, 22);
    assert_eq!(region.frames, 5);
    assert!(Arc::ptr_eq(&region.samples, &original.samples));
    for i in 0..region.frames {
        assert_eq!(
            region.sample_at(i as f64),
            original.sample_at((i + 2) as f64)
        );
    }
    // Repeated trimming must not compound or restart the inherited envelope.
    region.trim_relative(1, 4).unwrap();
    assert_eq!(region.start, 23);
    for i in 0..region.frames {
        assert_eq!(
            region.sample_at(i as f64),
            original.sample_at((i + 3) as f64)
        );
    }
}

#[test]
fn resizing_back_to_trimmed_edges_restores_source_and_original_fade_envelope() {
    let values: Vec<_> = (0..14).map(|i| i as f32 / 20.).collect();
    for reversed in [false, true] {
        let mut region = clip(&values);
        region.start = 20;
        region.source_offset = 2;
        region.frames = 8;
        region.set_reversed(reversed);
        region.set_gain_db(-3.).unwrap();
        region.set_fades(5, 6).unwrap();
        let original = region.clone();
        region.trim_relative(2, 6).unwrap();
        assert_eq!(region.start, 22);
        assert_eq!(region.frames, 4);
        region.resize_relative(-2, 6).unwrap();
        assert_eq!(region.start, 20);
        assert_eq!(region.source_offset, 2);
        assert_eq!(region.frames, 8);
        assert!(Arc::ptr_eq(&region.samples, &original.samples));
        for half_frame in 0..16 {
            let position = half_frame as f64 / 2.;
            assert_eq!(region.sample_at(position), original.sample_at(position));
        }
        project(region).validate().unwrap();
    }
}

#[test]
fn source_edge_extensions_follow_playback_direction_and_reject_invalid_ranges() {
    let values: Vec<_> = (0..14).map(|i| i as f32 / 20.).collect();
    for reversed in [false, true] {
        let mut region = clip(&values);
        region.start = 20;
        region.source_offset = 2;
        region.frames = 8;
        region.set_reversed(reversed);
        let original = region.clone();
        region.resize_relative(-1, 10).unwrap();
        assert_eq!(region.start, 19);
        assert_eq!(region.frames, 11);
        assert_eq!(region.source_offset, if reversed { 0 } else { 1 });
        assert!(Arc::ptr_eq(&region.samples, &original.samples));
        for frame in 0..11 {
            let source = if reversed { 10 - frame } else { 1 + frame };
            assert_eq!(region.sample_at(frame as f64), original.samples[source]);
        }
        project(region).validate().unwrap();

        let mut invalid = original.clone();
        let before = serde_json::to_value(&invalid).unwrap();
        for (from, to) in [
            (0, 0),
            (4, 3),
            (-5, 8),
            (0, 13),
            (i64::MIN, 8),
            (0, i64::MAX),
        ] {
            assert!(invalid.resize_relative(from, to).is_err());
            assert_eq!(serde_json::to_value(&invalid).unwrap(), before);
            assert!(Arc::ptr_eq(&invalid.samples, &original.samples));
        }
        // Source headroom alone does not allow a negative timeline position or
        // a region end that overflows the timeline clock.
        for (start, from, to) in [(0, -1, 8), (u64::MAX - 8, 0, 9)] {
            let mut invalid = original.clone();
            invalid.start = start;
            let before = serde_json::to_value(&invalid).unwrap();
            assert!(invalid.resize_relative(from, to).is_err());
            assert_eq!(serde_json::to_value(&invalid).unwrap(), before);
        }
    }
}

#[test]
fn extending_beyond_inherited_envelope_reanchors_and_bounds_fades() {
    for reversed in [false, true] {
        let mut region = clip(&[0.5; 14]);
        region.start = 20;
        region.source_offset = 2;
        region.frames = 8;
        region.set_reversed(reversed);
        region.set_fades(6, 7).unwrap();
        region.trim_relative(2, 6).unwrap();
        // Restore the original interval plus one frame at each edge. Its new
        // envelope must be anchored to the ten-frame result.
        region.resize_relative(-3, 7).unwrap();
        assert_eq!(region.start, 19);
        assert_eq!(region.frames, 10);
        assert_eq!(region.edit.fade_in, 6);
        assert_eq!(region.edit.fade_out, 7);
        for frame in 0..10 {
            let gain = (frame as f32 / 5.).min(1.) * ((9 - frame) as f32 / 6.).min(1.);
            near(region.sample_at(frame as f64)[0], 0.5 * gain);
        }
        project(region.clone()).validate().unwrap();
        // Extending one side while heavily trimming the other may make the
        // result shorter than either fade. Both lengths must remain valid.
        region.resize_relative(-1, 3).unwrap();
        assert_eq!(region.frames, 4);
        assert_eq!(region.edit.fade_in, 4);
        assert_eq!(region.edit.fade_out, 4);
        assert_eq!(region.sample_at(0.), [0.; 2]);
        near(region.sample_at(1.)[0], 1. / 9.);
        near(region.sample_at(2.)[0], 1. / 9.);
        assert_eq!(region.sample_at(3.), [0.; 2]);
        project(region).validate().unwrap();
    }
}

#[test]
fn split_preserves_edited_audio_and_device_rate_interpolation_after_reload() {
    let values: Vec<_> = (0..67)
        .map(|i| ((i * 13 % 23) as f32 - 11.) / 32.)
        .collect();
    let mut region = clip(&values);
    region.start = 3;
    region.edit.reversed = true;
    region.edit.gain_db = -2.5;
    region.set_fades(31, 29).unwrap();
    let original = project(region);
    let mut split = original.clone();
    let region = &mut split.tracks[0].clips[0];
    let mut right = region.split_relative(19).unwrap();
    let last = right.split_relative(27).unwrap();
    split.tracks[0].clips.extend([right, last]);
    assert!(
        split.tracks[0]
            .clips
            .iter()
            .all(|region| Arc::ptr_eq(&region.samples, &original.tracks[0].clips[0].samples))
    );
    // Clip list order and a serde round trip may not change continuation matching.
    split.tracks[0].clips.reverse();
    let reloaded: Project = serde_json::from_slice(&serde_json::to_vec(&split).unwrap()).unwrap();
    split.validate().unwrap();
    reloaded.validate().unwrap();
    for rate in [44_100, 48_000, 96_000] {
        let expected = render(&original, rate, 180);
        assert_eq!(render(&split, rate, 180), expected, "split at {rate} Hz");
        assert_eq!(
            render(&reloaded, rate, 180),
            expected,
            "reloaded at {rate} Hz"
        );
    }
}

#[test]
fn resetting_fades_anchors_them_to_current_trimmed_region() {
    let mut region = clip(&[0.5; 10]);
    region.set_fades(8, 4).unwrap();
    region.trim_relative(2, 8).unwrap();
    assert!(region.sample_at(0.)[0] > 0.);
    region.set_fades(3, 3).unwrap();
    for (index, expected) in [0., 0.25, 0.5, 0.5, 0.25, 0.].into_iter().enumerate() {
        near(region.sample_at(index as f64)[0], expected);
    }
    region.set_fades(0, 0).unwrap();
    assert_eq!(region.sample_at(0.), [0.5, -0.5]);
    assert_eq!(region.sample_at(5.), [0.5, -0.5]);
}

#[test]
fn overlapping_and_single_frame_fades_have_finite_endpoint_values() {
    let mut region = clip(&[1.; 3]);
    region.set_fades(3, 3).unwrap();
    assert_eq!(region.sample_at(0.), [0.; 2]);
    assert_eq!(region.sample_at(1.), [0.25, -0.25]);
    assert_eq!(region.sample_at(2.), [0.; 2]);
    region.set_fades(1, 1).unwrap();
    assert_eq!(region.sample_at(0.), [0.; 2]);
    assert_eq!(region.sample_at(1.), [1., -1.]);
    assert_eq!(region.sample_at(2.), [0.; 2]);
}

#[test]
fn normalize_uses_effective_region_peak_and_is_idempotent() {
    let mut region = clip(&[0.8, 0.2, 0.4, 0.2, 0.8]);
    region.edit.gain_db = -9.;
    region.set_fades(2, 2).unwrap();
    // Both raw 0.8 peaks are faded out, so normalization must use the 0.4 peak.
    region.normalize(-6.).unwrap();
    let expected_peak = 10.0f32.powf(-6. / 20.);
    near(region.sample_at(2.)[0], expected_peak);
    let gain = region.edit.gain_db;
    region.normalize(-6.).unwrap();
    near(region.edit.gain_db, gain);
    near(region.sample_at(2.)[0], expected_peak);
}

#[test]
fn invalid_region_operations_leave_the_previous_audio_intact() {
    let mut region = clip(&[0.2, 0.4, 0.6, 0.8]);
    region.set_fades(2, 2).unwrap();
    let before = serde_json::to_value(&region).unwrap();
    for (from, to) in [(0, 0), (3, 2), (0, 5), (usize::MAX, usize::MAX)] {
        assert!(region.trim_relative(from, to).is_err());
        assert_eq!(serde_json::to_value(&region).unwrap(), before);
    }
    for at in [0, 4, usize::MAX] {
        assert!(region.split_relative(at).is_err());
        assert_eq!(serde_json::to_value(&region).unwrap(), before);
    }
    for (fade_in, fade_out) in [(5, 0), (0, 5), (usize::MAX, 0)] {
        assert!(region.set_fades(fade_in, fade_out).is_err());
        assert_eq!(serde_json::to_value(&region).unwrap(), before);
    }
    for target in [-60.1, 0.1, f32::NAN, f32::INFINITY] {
        assert!(region.normalize(target).is_err());
        assert_eq!(serde_json::to_value(&region).unwrap(), before);
    }
    assert!(clip(&[0.; 4]).normalize(-1.).is_err());
    for nonfinite in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut invalid = clip(&[0.25, nonfinite, 0.5]);
        invalid.edit.gain_db = -3.;
        assert!(invalid.normalize(-1.).is_err());
        assert_eq!(invalid.edit.gain_db, -3.);
    }
}

#[test]
fn legacy_regions_default_to_neutral_edits_and_invalid_metadata_is_rejected() {
    let original = project(clip(&[0.2, -0.4, 0.6]));
    let mut value = serde_json::to_value(&original).unwrap();
    value["tracks"][0]["clips"][0]
        .as_object_mut()
        .unwrap()
        .remove("edit");
    let legacy: Project = serde_json::from_value(value).unwrap();
    legacy.validate().unwrap();
    assert_eq!(render(&legacy, 48_000, 3), render(&original, 48_000, 3));
    let edit = &legacy.tracks[0].clips[0].edit;
    assert_eq!(edit.gain_db, 0.);
    assert_eq!(edit.fade_in, 0);
    assert_eq!(edit.fade_out, 0);
    assert!(!edit.reversed);
    for gain in [-60.1, 24.1, f32::NAN, f32::INFINITY] {
        let mut invalid = original.clone();
        invalid.tracks[0].clips[0].edit.gain_db = gain;
        assert!(invalid.validate().is_err());
        assert!(Engine::try_new(&invalid, Arc::new(Controls::new(&invalid)), 48_000, 0).is_err());
    }
    for gain in [-60., 24.] {
        let mut valid = original.clone();
        valid.tracks[0].clips[0].edit.gain_db = gain;
        valid.validate().unwrap();
    }
    for field in ["fade_in", "fade_out", "envelope_offset"] {
        let mut invalid = serde_json::to_value(&original).unwrap();
        invalid["tracks"][0]["clips"][0]["edit"][field] = serde_json::json!(usize::MAX);
        let invalid: Project = serde_json::from_value(invalid).unwrap();
        assert!(invalid.validate().is_err(), "accepted invalid {field}");
        assert!(Engine::try_new(&invalid, Arc::new(Controls::new(&invalid)), 48_000, 0).is_err());
    }
    let mut invalid = serde_json::to_value(&original).unwrap();
    invalid["tracks"][0]["clips"][0]["edit"]["envelope_frames"] = serde_json::json!(2);
    let invalid: Project = serde_json::from_value(invalid).unwrap();
    assert!(invalid.validate().is_err());
    assert!(Engine::try_new(&invalid, Arc::new(Controls::new(&invalid)), 48_000, 0).is_err());
}

#[test]
fn live_region_changes_preserve_playback_position_and_apply_at_next_callback() {
    let mut p = project(clip(&[0.2, 0.4, 0.6, 0.8, 0.5, 0.3, 0.1, 0.7]));
    let (mut playback, mut renderer) = Playback::new(&p, 48_000, 0, false).unwrap();
    let mut output = [0.; 4];
    renderer.render(&mut output, 2);
    assert_eq!(output, [0.2, -0.2, 0.4, -0.4]);
    let position = playback.controls.position.clone();
    p.tracks[0].clips[0].edit.reversed = true;
    p.tracks[0].clips[0].edit.gain_db = -6.020_6;
    playback.update(&p).unwrap();
    assert!(Arc::ptr_eq(&position, &playback.controls.position));
    assert_eq!(position.load(Ordering::Relaxed), 2);
    assert!(playback.controls.playing.load(Ordering::Relaxed));
    renderer.render(&mut output, 2);
    for (actual, expected) in output.into_iter().zip([0.15, -0.15, 0.25, -0.25]) {
        near(actual, expected);
    }
    assert_eq!(position.load(Ordering::Relaxed), 4);
    assert!(playback.controls.playing.load(Ordering::Relaxed));
    drop(renderer);
}
