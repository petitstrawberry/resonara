use resonara_core::{
    audio_edit::{AudioEdit, edit_region_audio, remove_region_range},
    *,
};
use std::sync::{Arc, atomic::AtomicBool};

fn clip(values: &[f32]) -> Clip {
    Clip {
        start: 100,
        frames: values.len(),
        samples: Arc::new(values.iter().map(|&v| [v, -v]).collect()),
        source_offset: 0,
        source_channels: 2,
        edit: Default::default(),
    }
}
fn apply(c: &Clip, edit: AudioEdit) -> Clip {
    edit_region_audio(c, &edit, &AtomicBool::new(false)).unwrap()
}
fn values(c: &Clip) -> Vec<f32> {
    (0..c.frames).map(|at| c.sample_at(at as f64)[0]).collect()
}

#[test]
fn deleting_preserves_time_shared_source_and_later_samples() {
    let original = clip(&[0.1, 0.2, 0.3, 0.4, 0.5]);
    let remaining = remove_region_range(&original, 1..3).unwrap();
    assert_eq!(
        remaining
            .iter()
            .map(|c| (c.start, c.frames))
            .collect::<Vec<_>>(),
        vec![(100, 1), (103, 2)]
    );
    assert_eq!(values(&remaining[0]), vec![0.1]);
    assert_eq!(values(&remaining[1]), vec![0.4, 0.5]);
    for c in &remaining {
        assert!(Arc::ptr_eq(&original.samples, &c.samples));
    }
    assert!(remove_region_range(&original, 0..5).unwrap().is_empty());
    assert_eq!(remove_region_range(&original, 2..2).unwrap().len(), 1);
}
#[test]
fn silence_and_reverse_do_not_change_duration_gain_or_edge_envelope() {
    let mut original = clip(&[0.1, 0.2, 0.3, 0.4, 0.5]);
    original.set_fades(2, 2).unwrap();
    original.set_gain_db(-6.).unwrap();
    let silent = apply(&original, AudioEdit::Silence(1..3));
    assert_eq!(silent.frames, original.frames);
    assert_eq!(silent.edit, original.edit);
    assert_eq!(silent.sample_at(1.), [0.; 2]);
    assert_eq!(silent.sample_at(2.), [0.; 2]);
    assert_eq!(silent.sample_at(3.), original.sample_at(3.));
    let reversed = apply(&original, AudioEdit::Reverse(1..4));
    assert_eq!(reversed.edit, original.edit);
    assert_eq!(reversed.samples[1], original.samples[3]);
    assert_eq!(reversed.samples[3], original.samples[1]);
}
#[test]
fn reversed_destination_maintains_direction_after_editing_playback_order() {
    let mut original = clip(&[0.1, 0.2, 0.3, 0.4]);
    original.set_reversed(true);
    let removed = remove_region_range(&original, 1..2).unwrap();
    assert!(removed.iter().all(|c| c.edit.reversed));
    assert_eq!(values(&removed[0]), vec![0.4]);
    assert_eq!(values(&removed[1]), vec![0.2, 0.1]);
    assert_eq!(removed[1].start, 102);
    let reversed = apply(&original, AudioEdit::Reverse(0..3));
    assert_eq!(values(&reversed), vec![0.2, 0.3, 0.4, 0.1]);
}
#[test]
fn removing_range_preserves_inherited_fades_and_audible_remaining_audio() {
    let mut original = clip(&[0.5; 12]);
    original.set_fades(8, 6).unwrap();
    original.trim_relative(2, 10).unwrap();
    original.set_gain_db(-3.).unwrap();
    let before = values(&original);
    let remaining = remove_region_range(&original, 2..4).unwrap();
    assert!(
        remaining
            .iter()
            .all(|c| c.edit.gain_db == original.edit.gain_db)
    );
    for (actual, expected) in remaining
        .iter()
        .flat_map(values)
        .collect::<Vec<_>>()
        .iter()
        .zip(before[..2].iter().chain(before[4..].iter()))
    {
        assert!((actual - expected).abs() < 1e-6);
    }
}
#[test]
fn invalid_ranges_and_cancellation_leave_input_untouched() {
    let original = clip(&[0.5; 8]);
    for edit in [AudioEdit::Silence(3..9), AudioEdit::Reverse(7..4)] {
        assert!(edit_region_audio(&original, &edit, &AtomicBool::new(false)).is_err());
    }
    assert!(
        edit_region_audio(&original, &AudioEdit::Silence(0..2), &AtomicBool::new(true)).is_err()
    );
    assert!(remove_region_range(&original, 3..9).is_err());
    assert_eq!(values(&original), vec![0.5; 8]);
}
#[test]
fn materialized_region_edits_render_and_serialize_without_special_playback_paths() {
    let result = apply(&clip(&[0.1, 0.2, 0.3, 0.4]), AudioEdit::Silence(1..3));
    let project = Project {
        tracks: vec![Track {
            name: "Edited".into(),
            clips: vec![result],
            gain: 1.,
            pan: 0.,
            mute: false,
            solo: false,
            routing: Default::default(),
        }],
        master: 1.,
        ..Project::default()
    };
    let reopened: Project =
        serde_json::from_str(&serde_json::to_string(&project).unwrap()).unwrap();
    let mut engine = Engine::new(&reopened, Arc::new(Controls::new(&reopened)), 48000, 100);
    let mut output = [0.; 8];
    engine.render(&mut output, 2);
    assert_eq!(output, [0.1, -0.1, 0., 0., 0., 0., 0.4, -0.4]);
}
