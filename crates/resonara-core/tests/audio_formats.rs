use resonara_core::{Project, audio};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "resonara-formats-{}-{}",
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
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/audio");
fn path(name: &str) -> PathBuf {
    PathBuf::from(FIXTURES).join(name)
}
#[test]
fn lossless_imports_preserve_channels_samples_resampling_and_roundtrip() {
    let mut expected = Project::default();
    expected.import_wav(&path("source.wav")).unwrap();
    let temp = Temp::new();
    for file in ["stereo.flac", "stereo.aiff", "stereo.caf", "alac.m4a"] {
        let mut p = Project::default();
        p.import_audio(&path(file)).unwrap();
        let clip = &p.tracks[0].clips[0];
        assert_eq!(clip.source_channels, 2, "{file}");
        assert_eq!(clip.frames, expected.tracks[0].clips[0].frames, "{file}");
        assert_eq!(clip.samples, expected.tracks[0].clips[0].samples, "{file}");
        p.save(&temp.0.join("session.json")).unwrap();
        assert_eq!(
            Project::load(&temp.0.join("session.json")).unwrap().tracks[0].clips[0].samples,
            clip.samples
        );
    }
    let mut p = Project::default();
    p.import_audio(&path("mono.flac")).unwrap();
    let clip = &p.tracks[0].clips[0];
    assert_eq!(clip.source_channels, 1);
    assert!(clip.samples.iter().all(|s| s[0] == s[1]));
}
#[test]
fn lossy_imports_decode_stereo_without_mix_changes_or_extension_assumptions() {
    let temp = Temp::new();
    for file in ["stereo.mp3", "stereo.ogg", "aac.m4a", "stereo.aac"] {
        let mut p = Project::default();
        p.import_audio(&path(file)).unwrap();
        assert_eq!(p.master, 1.);
        assert_eq!(p.tracks[0].gain, 1.);
        let clip = &p.tracks[0].clips[0];
        assert_eq!(clip.source_channels, 2);
        assert!(
            (4500..9000).contains(&clip.frames),
            "{file}: {}",
            clip.frames
        );
        let energy = |channel| {
            clip.samples
                .iter()
                .map(|s| s[channel] * s[channel])
                .sum::<f32>()
        };
        assert!(
            energy(0) > energy(1) * 3.,
            "lost stereo channel balance: {file}"
        );
        assert!(clip.samples.iter().all(|s| s.iter().all(|s| s.is_finite())));
    }
    let renamed = temp.0.join("音声.WAV");
    std::fs::copy(path("stereo.mp3"), &renamed).unwrap();
    let mut p = Project::default();
    p.import_audio(&renamed).unwrap();
    assert_eq!(p.tracks[0].name, "音声");
}
#[test]
fn failed_decoding_is_transactional_and_file_filters_match_supported_formats() {
    let temp = Temp::new();
    let mut p = Project::demo();
    let before = serde_json::to_value(&p).unwrap();
    for (name, bytes) in [
        ("broken.flac", &b"fLaC\x80\x00\x00\x22"[..]),
        ("broken.mp3", &b"not audio"[..]),
    ] {
        let file = temp.0.join(name);
        std::fs::write(&file, bytes).unwrap();
        assert!(p.import_audio(&file).is_err());
        assert_eq!(serde_json::to_value(&p).unwrap(), before);
    }
    for file in [
        "x.WAV", "x.MP3", "x.FLAC", "x.AIF", "x.AIFF", "x.OGG", "x.M4A", "x.AAC", "x.CAF",
    ] {
        assert!(audio::supported_path(std::path::Path::new(file)));
    }
    for file in ["x.json", "x.exe", "x.opus", "x.wma"] {
        assert!(!audio::supported_path(std::path::Path::new(file)));
    }
}
