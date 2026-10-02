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
fn tempo_defaults_for_legacy_projects_and_preserves_audio_when_saved() {
    let d = Temp::new();
    let mut p = Project::demo();
    let mut legacy = serde_json::to_value(&p).unwrap();
    legacy.as_object_mut().unwrap().remove("tempo");
    let legacy: Project = serde_json::from_value(legacy).unwrap();
    legacy.validate().unwrap();
    assert_eq!(legacy.tempo, 120.);
    p.export_wav(&d.0.join("before.wav")).unwrap();
    p.tempo = 96.5;
    p.save(&d.0.join("tempo.json")).unwrap();
    let saved = Project::load(&d.0.join("tempo.json")).unwrap();
    assert_eq!(saved.tempo, 96.5);
    saved.export_wav(&d.0.join("after.wav")).unwrap();
    assert_eq!(
        std::fs::read(d.0.join("before.wav")).unwrap(),
        std::fs::read(d.0.join("after.wav")).unwrap()
    );
    for tempo in [0., 19.9, 400.1, f64::INFINITY, f64::NAN] {
        p.tempo = tempo;
        assert!(p.validate().is_err());
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
fn non_destructive_region_edits_survive_save_load_and_wav_export() {
    let d = Temp::new();
    let mut p = Project::demo();
    p.tracks.truncate(1);
    p.master = 1.;
    p.tracks[0].gain = 1.;
    p.tracks[0].pan = 0.;
    let region = &mut p.tracks[0].clips[0];
    let source = region.samples.clone();
    region.edit.reversed = true;
    region.edit.gain_db = -3.;
    region.set_fades(8_000, 6_000).unwrap();
    region.trim_relative(2_000, 16_000).unwrap();
    let right = region.split_relative(5_000).unwrap();
    p.tracks[0].clips.push(right);
    let expected = p.tracks[0]
        .clips
        .iter()
        .flat_map(|region| (0..region.frames).flat_map(move |frame| region.sample_at(frame as f64)))
        .collect::<Vec<_>>();
    assert!(
        p.tracks[0]
            .clips
            .iter()
            .all(|region| Arc::ptr_eq(&region.samples, &source))
    );
    let session = d.0.join("region-edits.json");
    p.save(&session).unwrap();
    let loaded = Project::load(&session).unwrap();
    assert_eq!(loaded.tracks[0].clips[0].samples, source);
    assert_eq!(
        serde_json::to_value(&loaded).unwrap(),
        serde_json::to_value(&p).unwrap()
    );
    let exported = d.0.join("region-edits.wav");
    loaded.export_wav(&exported).unwrap();
    let mut wav = hound::WavReader::open(exported).unwrap();
    let samples = wav
        .samples::<f32>()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(&samples[..4_000], &[0.; 4_000]);
    assert_eq!(&samples[4_000..], expected);
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

fn counted_padding_wav(sample_rate: u32, channels: u16, samples: &[i32], tail: &[u8]) -> Vec<u8> {
    let mut bytes = b"RIFF\0\0\0\0WAVEJUNK\x04\0\0\0\0\0\0\0fmt \x10\0\0\0".to_vec();
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * u32::from(channels) * 3).to_le_bytes());
    bytes.extend_from_slice(&(channels * 3).to_le_bytes());
    bytes.extend_from_slice(&24u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    let length = (samples.len() * 3 + tail.len()) as u32;
    bytes.extend_from_slice(&length.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes()[..3]);
    }
    bytes.extend_from_slice(tail);
    if length % 2 != 0 {
        bytes.push(0);
    }
    bytes.extend_from_slice(b"LGWV\x04\0\0\0meta");
    let riff_length = bytes.len() as u32 - 8;
    bytes[4..8].copy_from_slice(&riff_length.to_le_bytes());
    bytes
}

#[test]
fn imports_logic_counted_padding_without_losing_samples_or_reading_metadata() {
    let d = Temp::new();
    let source = [1 << 22, -(1 << 22), 1 << 21];
    for rate in [44100, 48000, 96000] {
        let path = d.0.join(format!("logic-{rate}.wav"));
        let bytes = counted_padding_wav(rate, 1, &source, &[0]);
        std::fs::write(&path, &bytes).unwrap();
        assert!(hound::WavReader::open(&path).is_err());
        let mut p = Project {
            sample_rate: rate,
            ..Project::default()
        };
        p.import_wav(&path).unwrap();
        let clip = &p.tracks[0].clips[0];
        assert_eq!(clip.source_channels, 1);
        assert_eq!(clip.frames, 3);
        assert_eq!(clip.samples.as_ref(), &vec![[0.5; 2], [-0.5; 2], [0.25; 2]]);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        p.validate().unwrap();
    }
}

#[test]
fn rejects_partial_samples_and_frames_instead_of_discarding_audio() {
    let d = Temp::new();
    for (channels, samples, tail) in [
        (1, vec![1, 2, 3], vec![1]),    // The extra byte is not zero padding.
        (1, vec![1, 2], vec![0]),       // Even sample counts need no RIFF padding.
        (1, vec![1, 2, 3], vec![0, 0]), // More than one byte is incomplete audio.
        (2, vec![1, 2, 3], vec![0]),    // A partial stereo frame must not be accepted.
    ] {
        let path = d.0.join("incomplete.wav");
        std::fs::write(&path, counted_padding_wav(48000, channels, &samples, &tail)).unwrap();
        let mut p = Project::demo();
        assert!(p.import_wav(&path).is_err());
        assert_eq!(p.tracks.len(), 3);
    }
    let path = d.0.join("truncated.wav");
    let mut bytes = counted_padding_wav(48000, 1, &[1, 2, 3], &[0]);
    bytes.truncate(56); // Ends before the declared data payload is complete.
    std::fs::write(&path, bytes).unwrap();
    assert!(Project::default().import_wav(&path).is_err());
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
                edit: Default::default(),
                start: 0,
                source_offset: 0,
                frames: 3,
                samples: Arc::new(vec![[0.25; 2], [-0.5; 2], [0.125; 2]]),
            }],
            gain: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
            routing: resonara_core::ChannelRouting::default(),
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

#[test]
fn meter_persistence_legacy_default_validation_and_export_are_sample_neutral() {
    use resonara_core::TimeSignature;
    let d = Temp::new();
    let mut p = Project::demo();
    let mut legacy = serde_json::to_value(&p).unwrap();
    legacy.as_object_mut().unwrap().remove("time_signature");
    let old: Project = serde_json::from_value(legacy).unwrap();
    assert_eq!(old.time_signature, TimeSignature::default());
    p.export_wav(&d.0.join("before-meter.wav")).unwrap();
    let duration = p.duration();
    for (n, den) in [(3, 4), (6, 8), (7, 8), (5, 4), (32, 32)] {
        p.time_signature = TimeSignature {
            numerator: n,
            denominator: den,
        };
        p.save(&d.0.join("meter.json")).unwrap();
        let loaded = Project::load(&d.0.join("meter.json")).unwrap();
        assert_eq!(loaded.time_signature, p.time_signature);
        assert_eq!(loaded.duration(), duration);
        loaded.export_wav(&d.0.join("after-meter.wav")).unwrap();
        assert_eq!(
            std::fs::read(d.0.join("before-meter.wav")).unwrap(),
            std::fs::read(d.0.join("after-meter.wav")).unwrap()
        );
    }
    for (n, den) in [(0, 4), (33, 4), (4, 0), (4, 3), (4, 64)] {
        p.time_signature = TimeSignature {
            numerator: n,
            denominator: den,
        };
        assert!(p.validate().is_err());
    }
}
