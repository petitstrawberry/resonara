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
    assert!(
        q.tracks
            .iter()
            .flat_map(|track| &track.clips)
            .all(|clip| clip.source_channels == 1)
    );
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
        assert_eq!(c.source_channels, 1);
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

#[test]
fn imported_channel_metadata_preserves_true_stereo_even_when_channels_match() {
    let d = Temp::new();
    for (name, samples) in [
        ("unequal", vec![[0.25f32, -0.5], [0.5, 0.125]]),
        ("dual-mono", vec![[0.25; 2], [0.5; 2]]),
    ] {
        let path = d.0.join(format!("{name}.wav"));
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for sample in samples.iter().flatten() {
            writer.write_sample(*sample).unwrap();
        }
        writer.finalize().unwrap();
        let mut p = Project::default();
        p.import_wav(&path).unwrap();
        assert_eq!(p.tracks[0].clips[0].source_channels, 2);
        assert_eq!(p.tracks[0].clips[0].samples.as_ref(), &samples);
        p.split(0, 1).unwrap();
        p.save(&d.0.join(format!("{name}.json"))).unwrap();
        let loaded = Project::load(&d.0.join(format!("{name}.json"))).unwrap();
        assert!(
            loaded.tracks[0]
                .clips
                .iter()
                .all(|clip| clip.source_channels == 2)
        );
    }
}

#[test]
fn legacy_projects_default_to_stereo_without_guessing_from_equal_samples() {
    let d = Temp::new();
    let p = Project {
        tracks: vec![Track {
            name: "known mono".into(),
            clips: vec![Clip {
                source_channels: 1,
                start: 0,
                source_offset: 0,
                frames: 3,
                samples: Arc::new(vec![[0.25; 2], [-0.5; 2], [0.125; 2]]),
            }],
            gain: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
        }],
        master: 1.0,
        ..Project::default()
    };
    let mut legacy = serde_json::to_value(&p).unwrap();
    legacy["tracks"][0]["clips"][0]
        .as_object_mut()
        .unwrap()
        .remove("source_channels");
    let path = d.0.join("legacy.json");
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let loaded = Project::load(&path).unwrap();
    assert_eq!(loaded.tracks[0].clips[0].source_channels, 2);
    assert_eq!(
        loaded.tracks[0].clips[0].samples,
        p.tracks[0].clips[0].samples
    );
    let mut old_audio = [0.0; 6];
    let mut new_audio = [0.0; 6];
    Engine::new(&p, Arc::new(Controls::new(&p)), 48000, 0).render(&mut old_audio, 2);
    Engine::new(&loaded, Arc::new(Controls::new(&loaded)), 48000, 0).render(&mut new_audio, 2);
    assert_eq!(old_audio, new_audio);
}

#[test]
fn validation_rejects_invalid_channels_and_mono_metadata_that_hides_stereo_audio() {
    let mut p = Project::demo();
    for channels in [0, 3, u16::MAX] {
        p.tracks[0].clips[0].source_channels = channels;
        assert!(p.validate().is_err());
    }
    let clip = &mut p.tracks[0].clips[0];
    clip.source_channels = 1;
    Arc::make_mut(&mut clip.samples)[0] = [0.25, 0.5];
    assert!(p.validate().is_err());
    p.tracks[0].clips[0].source_channels = 2;
    p.validate().unwrap();
}
