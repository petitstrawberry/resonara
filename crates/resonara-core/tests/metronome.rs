use resonara_core::{Controls, Engine, Project, TimeSignature};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};
fn engine(p: &Project, rate: u32, start: u64) -> (Engine, Arc<Controls>) {
    let c = Arc::new(Controls::new(p));
    c.metronome.store(true, Ordering::Relaxed);
    (Engine::new(p, Arc::clone(&c), rate, start), c)
}
#[test]
fn clicks_follow_tempo_meter_device_rate_and_absolute_seek_position() {
    for rate in [44100, 48000, 96000] {
        for meter in [
            TimeSignature {
                numerator: 3,
                denominator: 4,
            },
            TimeSignature {
                numerator: 7,
                denominator: 8,
            },
            TimeSignature {
                numerator: 6,
                denominator: 8,
            },
        ] {
            let p = Project {
                tempo: 137.5,
                time_signature: meter,
                ..Project::default()
            };
            let beat = meter.beat_seconds(p.tempo);
            let count = usize::from(meter.numerator) + 1;
            let frames = (beat * (count as f64 + 0.2) * rate as f64).ceil() as usize;
            let (mut e, c) = engine(&p, rate, 0);
            let mut out = vec![0.; frames * 2];
            e.render(&mut out, 2);
            assert!(c.playing.load(Ordering::Relaxed));
            let mut peaks = Vec::new();
            for index in 0..count {
                let at = (index as f64 * beat * rate as f64).ceil() as usize;
                let onset = (at..at + 3)
                    .find(|i| out[i * 2].abs() > 0.000001)
                    .expect("missing click at beat boundary");
                assert!(onset - at <= 2);
                let len = (rate as f64 * 0.035).ceil() as usize;
                let peak = out[at * 2..(at + len) * 2]
                    .iter()
                    .copied()
                    .map(f32::abs)
                    .fold(0., f32::max);
                peaks.push(peak);
                assert!(
                    out[(at + len + 2) * 2..(at + len + 10) * 2]
                        .iter()
                        .all(|s| *s == 0.)
                );
            }
            assert!(peaks[0] > peaks[1] * 1.2, "missing bar accent");
            assert!((peaks[0] - peaks[usize::from(meter.numerator)]).abs() < 0.001);
            assert!(out.chunks_exact(2).all(|s| s[0] == s[1]));
            let start = (beat * 1.4 * p.sample_rate as f64).round() as u64;
            let (mut seek, sc) = engine(&p, rate, start);
            assert_eq!(sc.position.load(Ordering::Relaxed), start);
            let mut resumed = vec![0.; (beat * rate as f64) as usize * 2];
            seek.render(&mut resumed, 2);
            let next =
                ((2. * beat - start as f64 / p.sample_rate as f64) * rate as f64).ceil() as usize;
            assert!(resumed[..next * 2].iter().all(|s| *s == 0.));
            assert!(
                resumed[next * 2..(next + 4) * 2]
                    .iter()
                    .any(|s| s.abs() > 0.000001)
            );
        }
    }
}
#[test]
fn live_toggle_pause_and_output_mapping_do_not_change_transport_or_export() {
    let p = Project::default();
    let (mut e, c) = engine(&p, 48000, 0);
    let mut mono = vec![0.; 2048];
    e.render(&mut mono, 1);
    assert!(mono.iter().any(|s| *s != 0.));
    let peak = f32::from_bits(c.master_peak_left.load(Ordering::Relaxed));
    assert!(peak > 0.);
    c.metronome.store(false, Ordering::Relaxed);
    let mut out = [1.; 1024];
    e.render(&mut out, 2);
    assert_eq!(out, [0.; 1024]);
    let position = c.position.load(Ordering::Relaxed);
    assert!(position > 0);
    c.playing.store(false, Ordering::Relaxed);
    e.render(&mut out, 2);
    assert_eq!(c.position.load(Ordering::Relaxed), position);
    assert_eq!(out, [0.; 1024]);
    c.metronome.store(true, Ordering::Relaxed);
    c.playing.store(true, Ordering::Relaxed);
    let mut resumed = vec![0.; 48000];
    e.render(&mut resumed, 2);
    assert!(resumed.iter().any(|s| *s != 0.));
    let p = Project::demo();
    let (_, enabled) = engine(&p, 48000, 0);
    assert!(enabled.metronome.load(Ordering::Relaxed));
    let fresh = Arc::new(Controls::new(&p));
    assert!(!fresh.metronome.load(Ordering::Relaxed));
    let mut expected = vec![0.; p.duration() as usize * 2];
    Engine::new(&p, fresh, 48000, 0).render(&mut expected, 2);
    let file: PathBuf = std::env::temp_dir().join(format!(
        "resonara-metronome-export-{}.wav",
        std::process::id()
    ));
    p.export_wav(&file).unwrap();
    let got = hound::WavReader::open(&file)
        .unwrap()
        .samples::<f32>()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    std::fs::remove_file(file).unwrap();
    assert_eq!(got, expected);
}
#[test]
fn callback_partitioning_and_eof_do_not_create_extra_clicks() {
    let p = Project::demo();
    let (mut whole, _) = engine(&p, 44100, 0);
    let (mut split, _) = engine(&p, 44100, 0);
    let frames = (p.duration() as f64 / 48000. * 44100.).ceil() as usize + 2000;
    let mut out = vec![0.; frames * 2];
    whole.render(&mut out, 2);
    let mut partitioned = vec![0.; out.len()];
    for part in partitioned.chunks_mut(214) {
        split.render(part, 2);
    }
    assert_eq!(out, partitioned);
    assert!(out[out.len() - 3000..].iter().all(|s| *s == 0.));
}
