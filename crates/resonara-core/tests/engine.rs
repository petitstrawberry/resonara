use resonara_core::*;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::{Arc, atomic::Ordering},
};
thread_local! {static GUARD:Cell<bool>=const {Cell::new(false)};static ALLOCS:Cell<usize>=const {Cell::new(0)};static FREES:Cell<usize>=const {Cell::new(0)};}
struct Audit;
unsafe impl GlobalAlloc for Audit {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        GUARD.with(|g| {
            if g.get() {
                ALLOCS.with(|c| c.set(c.get() + 1))
            }
        });
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        GUARD.with(|g| {
            if g.get() {
                FREES.with(|c| c.set(c.get() + 1))
            }
        });
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        GUARD.with(|g| {
            if g.get() {
                ALLOCS.with(|c| c.set(c.get() + 1))
            }
        });
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator]
static ALLOCATOR: Audit = Audit;
fn constant(value: f32, start: u64, frames: usize) -> Track {
    Track {
        name: "test".into(),
        clips: vec![Clip {
            source_channels: 2,
            start,
            source_offset: 0,
            frames,
            samples: Arc::new(vec![[value, value]; frames]),
        }],
        gain: 1.,
        pan: 0.,
        mute: false,
        solo: false,
    }
}
fn project() -> Project {
    Project {
        tracks: vec![constant(0.25, 0, 4), constant(0.5, 2, 2)],
        master: 1.,
        ..Project::default()
    }
}
fn render(p: &Project, rate: u32, frames: usize) -> Vec<f32> {
    let controls = Arc::new(Controls::new(p));
    let mut e = Engine::new(p, controls, rate, 0);
    let mut out = vec![0.; frames * 2];
    e.render(&mut out, 2);
    out
}
#[test]
fn timeline_mix_and_eof() {
    let p = project();
    assert_eq!(
        render(&p, 48000, 6),
        vec![
            0.25, 0.25, 0.25, 0.25, 0.75, 0.75, 0.75, 0.75, 0., 0., 0., 0.
        ]
    );
}
#[test]
fn mute_solo_pan_master_and_clipping() {
    let mut p = project();
    p.tracks[1].solo = true;
    p.tracks[1].pan = 1.;
    p.master = 0.5;
    assert_eq!(
        render(&p, 48000, 4),
        vec![0., 0., 0., 0., 0., 0.25, 0., 0.25]
    );
    p.tracks[1].mute = true;
    assert_eq!(render(&p, 48000, 4), vec![0.; 8]);
    p.tracks[0].solo = true;
    p.tracks[0].gain = 2.;
    p.tracks[1].mute = false;
    p.tracks[1].gain = 2.;
    p.master = 2.;
    assert_eq!(&render(&p, 48000, 4)[4..], [1., 1., 1., 1.]);
}
#[test]
fn split_is_nondestructive_and_trim_uses_source_offset() {
    let mut p = project();
    let before = render(&p, 48000, 4);
    p.split(0, 2).unwrap();
    assert_eq!(p.tracks[0].clips.len(), 2);
    assert!(Arc::ptr_eq(
        &p.tracks[0].clips[0].samples,
        &p.tracks[0].clips[1].samples
    ));
    assert_eq!(render(&p, 48000, 4), before);
    p.trim(0, 1, 3).unwrap();
    assert_eq!(p.tracks[0].clips[0].source_offset, 1);
    assert_eq!(p.tracks[0].clips[1].frames, 1);
    assert!(p.split(0, 0).is_err());
    assert!(p.trim(0, 2, 1).is_err());
}
#[test]
fn split_preserves_varying_audio_at_device_rates() {
    let mut track = constant(0., 7, 251);
    track.clips[0].source_offset = 3;
    track.clips[0].samples = Arc::new(
        (0..263)
            .map(|i| {
                [
                    ((i * 37 % 31) as f32 - 15.) / 32.,
                    ((i * 17 % 23) as f32 - 11.) / 24.,
                ]
            })
            .collect(),
    );
    let original = Project {
        tracks: vec![track],
        master: 1.,
        ..Project::default()
    };
    let mut split = original.clone();
    for at in [8, 83, 257] {
        split.split(0, at).unwrap();
    }
    // Source/timeline adjacency must not depend on the clips' storage order.
    split.tracks[0].clips.reverse();
    split.validate().unwrap();
    let serialized = serde_json::to_vec(&split).unwrap();
    let reloaded: Project = serde_json::from_slice(&serialized).unwrap();
    reloaded.validate().unwrap();
    assert!(!Arc::ptr_eq(
        &reloaded.tracks[0].clips[0].samples,
        &reloaded.tracks[0].clips[1].samples
    ));
    for rate in [44100, 96000] {
        let expected = render(&original, rate, 640);
        for (name, p) in [("split", &split), ("reloaded", &reloaded)] {
            assert_eq!(
                render(p, rate, 640),
                expected,
                "{name} changed the rendered audio at {rate} Hz"
            );
        }
    }
}
fn split_boundary_project() -> Project {
    let mut track = constant(0., 0, 6);
    track.clips[0].samples = Arc::new(
        [0., 0.8, -0.4, 0.6, -0.2, 0.3]
            .into_iter()
            .map(|v| [v, -v])
            .collect(),
    );
    let mut p = Project {
        tracks: vec![track],
        master: 1.,
        ..Project::default()
    };
    p.split(0, 2).unwrap();
    p
}
#[test]
fn resampling_does_not_bridge_moved_or_trimmed_clip_gaps() {
    let mut moved = split_boundary_project();
    moved.tracks[0].clips[1].start += 1;
    moved.validate().unwrap();
    let out = render(&moved, 96000, 16);
    // At project frame 1.5, hold the left clip's endpoint. Frames 2 and
    // 2.5 belong to the gap, so neither source may bleed into them.
    assert_eq!(&out[6..8], &[0.8, -0.8]);
    assert_eq!(&out[8..12], &[0.; 4]);
    assert_eq!(&out[12..14], &[-0.4, 0.4]);

    let mut trimmed = split_boundary_project();
    // Trim one source frame from the next clip, then place it against the
    // previous clip. The timeline touches, but the source is discontinuous.
    trimmed.tracks[0].clips[1].source_offset += 1;
    trimmed.tracks[0].clips[1].frames -= 1;
    trimmed.validate().unwrap();
    let out = render(&trimmed, 96000, 12);
    assert_eq!(&out[6..8], &[0.8, -0.8]);
    assert_eq!(&out[8..10], &[0.6, -0.6]);

    let mut ending = split_boundary_project();
    ending.trim(0, 0, 2).unwrap();
    let out = render(&ending, 96000, 6);
    assert_eq!(&out[6..8], &[0.8, -0.8]);
    assert_eq!(&out[8..], &[0.; 4]);
}
#[test]
fn resampling_does_not_bridge_different_source_assets() {
    let mut p = split_boundary_project();
    let right = &mut p.tracks[0].clips[1];
    let mut different_source = right.samples.as_ref().clone();
    different_source[2] = [0.2, -0.2];
    right.samples = Arc::new(different_source);
    p.validate().unwrap();
    let out = render(&p, 96000, 12);
    assert_eq!(&out[6..8], &[0.8, -0.8]);
    assert_eq!(&out[8..10], &[0.2, -0.2]);
}
#[test]
fn output_rate_and_mono_surround() {
    let p = project();
    assert_eq!(render(&p, 96000, 8).len(), 16);
    let c = Arc::new(Controls::new(&p));
    let mut e = Engine::new(&p, c.clone(), 24000, 0);
    let mut out = [0.; 3];
    e.render(&mut out, 1);
    assert_eq!(out, [0.25, 0.75, 0.]);
    let mut e = Engine::new(&p, Arc::new(Controls::new(&p)), 48000, 0);
    let mut out = [1.; 6];
    e.render(&mut out, 6);
    assert_eq!(out, [0.25, 0.25, 0., 0., 0., 0.]);
}
#[test]
fn live_atomics_and_pause() {
    let p = project();
    let c = Arc::new(Controls::new(&p));
    let mut e = Engine::new(&p, c.clone(), 48000, 0);
    let mut out = [0.; 2];
    e.render(&mut out, 2);
    c.tracks[0].gain.store(0.5f32.to_bits(), Ordering::Relaxed);
    e.render(&mut out, 2);
    assert_eq!(out, [0.125; 2]);
    c.playing.store(false, Ordering::Relaxed);
    e.render(&mut out, 2);
    assert_eq!(out, [0.; 2]);
    assert_eq!(c.position.load(Ordering::Relaxed), 2);
}

#[test]
fn initial_transport_position_is_published_before_first_callback() {
    let p = project();
    let controls = Arc::new(Controls::new(&p));
    let mut engine = Engine::new(&p, controls.clone(), 48000, 2);
    assert_eq!(controls.position.load(Ordering::Relaxed), 2);
    let mut output = [0.0; 2];
    engine.render(&mut output, 2);
    assert_eq!(output, [0.75; 2]);

    let _beyond_end = Engine::new(&p, controls.clone(), 48000, 99);
    assert_eq!(controls.position.load(Ordering::Relaxed), p.duration());
}
#[test]
fn callback_never_allocates_or_frees() {
    let mut p = Project::demo();
    p.split(0, 12345).unwrap();
    p.split(0, 48000).unwrap();
    let c = Arc::new(Controls::new(&p));
    let mut e = Engine::new(&p, c, 44100, 0);
    let mut out = [0.; 512];
    ALLOCS.with(|v| v.set(0));
    FREES.with(|v| v.set(0));
    GUARD.with(|v| v.set(true));
    for _ in 0..1000 {
        e.render(&mut out, 2);
    }
    GUARD.with(|v| v.set(false));
    assert_eq!(ALLOCS.with(Cell::get), 0);
    assert_eq!(FREES.with(Cell::get), 0);
}
#[test]
fn validation_rejects_invalid_state() {
    let mut p = project();
    p.tracks[0].clips[0].source_offset = 100;
    assert!(p.validate().is_err());
    let mut p = project();
    p.master = f32::NAN;
    assert!(p.validate().is_err());
    let mut p = project();
    p.version = 9;
    assert!(p.validate().is_err());
}
#[test]
fn integer_conversions() {
    assert_eq!(i16::from_f32(-1.), -32767);
    assert_eq!(u16::from_f32(-1.), 0);
    assert_eq!(u16::from_f32(1.), 65535);
}

#[test]
fn compiled_default_matches_independent_flat_mixer_at_multiple_rates() {
    let source = Arc::new(
        (0..128)
            .map(|i| {
                [
                    ((i * 7 % 19) as f32 - 9.) / 16.,
                    ((i * 13 % 31) as f32 - 15.) / 32.,
                ]
            })
            .collect::<Vec<_>>(),
    );
    let mut p = Project {
        master: 0.73,
        ..Project::default()
    };
    for index in 0..6 {
        p.tracks.push(Track {
            name: format!("Reference {index}"),
            clips: (0..8)
                .rev()
                .map(|part| Clip {
                    source_channels: 2,
                    start: part * 14 + index * 3,
                    source_offset: part as usize * 7,
                    frames: 20,
                    samples: source.clone(),
                })
                .collect(),
            gain: 0.2 + index as f32 * 0.1,
            pan: index as f32 / 3.0 - 0.8,
            mute: index == 0,
            solo: false,
        });
    }
    for solo_enabled in [false, true] {
        p.tracks[3].solo = solo_enabled;
        p.tracks[4].solo = solo_enabled;
        for rate in [44100, 48000, 96000] {
            for start in [0, 17, 201] {
                // Direct sample-at-a-time oracle for the pre-graph mixer. These
                // overlapping clips have no source-contiguous end neighbors.
                let mut expected = Vec::new();
                let mut position = start as f64;
                for _ in 0..399 {
                    let mut sum = [0.0f32; 2];
                    if position < p.duration() as f64 {
                        for track in &p.tracks {
                            if track.mute || (solo_enabled && !track.solo) {
                                continue;
                            }
                            let mut sample = [0.0f32; 2];
                            for clip in &track.clips {
                                let x = position - clip.start as f64;
                                if x < 0.0 || x >= clip.frames as f64 {
                                    continue;
                                }
                                let a = x as usize;
                                let b = (a + 1).min(clip.frames - 1);
                                let f = x.fract() as f32;
                                for (channel, value) in sample.iter_mut().enumerate() {
                                    *value += clip.samples[clip.source_offset + a][channel]
                                        * (1.0 - f)
                                        + clip.samples[clip.source_offset + b][channel] * f;
                                }
                            }
                            sample[0] *= track.gain * (1.0 - track.pan.max(0.0));
                            sample[1] *= track.gain * (1.0 + track.pan.min(0.0));
                            sum[0] += sample[0];
                            sum[1] += sample[1];
                        }
                        position += p.sample_rate as f64 / rate as f64;
                    }
                    expected.extend(sum.map(|value| (value * p.master).clamp(-1.0, 1.0)));
                }
                let mut e = Engine::new(&p, Arc::new(Controls::new(&p)), rate, start);
                let mut actual = vec![0.0; expected.len()];
                let mut offset = 0;
                for frames in [13, 128, 257, 1] {
                    e.render(&mut actual[offset * 2..(offset + frames) * 2], 2);
                    offset += frames;
                }
                assert_eq!(
                    actual, expected,
                    "rate {rate}, start {start}, solo {solo_enabled}"
                );
            }
        }
    }
}
