use resonara_core::*;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "resonara-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn save_load_and_export_preserve_edited_mix() {
    let d = Temp::new();
    let mut p = Project::demo();
    p.split(0, 48000).unwrap();
    p.trim(1, 30000, 70000).unwrap();
    p.tracks[2].pan = -0.5;
    p.save(&d.0.join("session.json")).unwrap();
    let q = Project::load(&d.0.join("session.json")).unwrap();
    assert_eq!(q.tracks.len(), 3);
    assert_eq!(q.tracks[0].clips.len(), 2);
    q.export_wav(&d.0.join("mix.wav")).unwrap();
    let mut wav = hound::WavReader::open(d.0.join("mix.wav")).unwrap();
    assert_eq!(wav.spec().channels, 2);
    assert_eq!(wav.spec().sample_rate, 48000);
    assert_eq!(wav.duration() as u64, p.duration());
    let mut engine = Engine::new(&p, Arc::new(Controls::new(&p)), 48000, 0);
    let mut expected = vec![0.; p.duration() as usize * 2];
    engine.render(&mut expected, 2);
    let got = wav
        .samples::<f32>()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(got, expected);
}
#[test]
fn imports_mono_pcm_and_resamples() {
    let d = Temp::new();
    for bits in [8, 16, 24, 32] {
        let path = d.0.join(format!("mono{bits}.wav"));
        let mut w = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 24000,
                bits_per_sample: bits,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..4 {
            w.write_sample(1i32 << (bits - 2)).unwrap();
        }
        w.finalize().unwrap();
        let mut p = Project::default();
        p.import_wav(&path).unwrap();
        let c = &p.tracks[0].clips[0];
        assert_eq!(c.frames, 8);
        assert_eq!(c.samples[0], [0.5, 0.5]);
    }
}
#[test]
fn rejects_surround_and_nonfinite_wav_and_corrupt_project() {
    let d = Temp::new();
    for (channels, value) in [(3, 0.1), (2, f32::NAN)] {
        let path = d.0.join(format!("invalid{channels}.wav"));
        let mut w = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for _ in 0..channels {
            w.write_sample(value).unwrap();
        }
        w.finalize().unwrap();
        assert!(Project::default().import_wav(&path).is_err());
    }
    std::fs::write(d.0.join("bad.json"), "{}").unwrap();
    assert!(Project::load(&d.0.join("bad.json")).is_err());
}
