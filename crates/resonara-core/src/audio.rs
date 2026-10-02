//! Decode completely before changing a project; keep the tolerant WAV reader.
use crate::{Result, wav};
use std::{
    fs::File,
    io::{Read, Seek},
    path::Path,
};
use symphonia::core::{
    formats::{TrackType, probe::Hint},
    io::MediaSourceStream,
};
pub const EXTENSIONS: &[&str] = &[
    "wav", "wave", "mp3", "flac", "aif", "aiff", "aifc", "ogg", "oga", "m4a", "mp4", "aac", "caf",
];
pub fn supported_path(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| EXTENSIONS.iter().any(|ext| s.eq_ignore_ascii_case(ext)))
}
pub(crate) struct Decoded {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<[f32; 2]>,
}
pub(crate) fn read_wav(path: &Path) -> Result<Decoded> {
    let mut r = wav::open(path)?;
    let spec = r.spec();
    if !(1..=2).contains(&spec.channels) || spec.sample_rate == 0 {
        return Err("Only mono/stereo WAV is supported".into());
    }
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<std::result::Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 2f32.powi(i32::from(spec.bits_per_sample) - 1);
            r.samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<std::result::Result<_, _>>()?
        }
    };
    if raw.len() % spec.channels as usize != 0 || raw.iter().any(|s| !s.is_finite()) {
        return Err("Invalid WAV samples".into());
    }
    Ok(Decoded {
        rate: spec.sample_rate,
        channels: spec.channels,
        samples: raw
            .chunks_exact(spec.channels as usize)
            .map(|s| [s[0], *s.get(1).unwrap_or(&s[0])])
            .collect(),
    })
}
pub(crate) fn read(path: &Path) -> Result<Decoded> {
    let mut file = File::open(path)?;
    let mut header = [0u8; 12];
    let read = file.read(&mut header)?;
    if read == 12 && &header[..4] == b"RIFF" && &header[8..] == b"WAVE" {
        return read_wav(path);
    }
    file.rewind()?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe().probe(
        &hint,
        stream,
        Default::default(),
        Default::default(),
    )?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or("No audio track in file")?;
    let id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("Unsupported audio codec")?;
    let mut decoder =
        symphonia::default::get_codecs().make_audio_decoder(params, &Default::default())?;
    let mut spec = None;
    let mut samples = Vec::new();
    let mut interleaved = Vec::<f32>::new();
    while let Some(packet) = format.next_packet()? {
        if packet.track_id != id {
            continue;
        }
        let buffer = decoder.decode(&packet)?;
        let rate = buffer.spec().rate();
        let channels = buffer.spec().channels().count();
        if rate == 0 || !(1..=2).contains(&channels) {
            return Err("Only mono/stereo audio is supported".into());
        }
        if spec.is_some_and(|s| s != (rate, channels)) {
            return Err("Audio format changes within the file are unsupported".into());
        }
        spec = Some((rate, channels));
        interleaved.resize(buffer.samples_interleaved(), 0.);
        buffer.copy_to_slice_interleaved(&mut interleaved);
        if interleaved.iter().any(|s| !s.is_finite()) {
            return Err("Invalid audio samples".into());
        }
        samples.extend(
            interleaved
                .chunks_exact(channels)
                .map(|s| [s[0], *s.get(1).unwrap_or(&s[0])]),
        );
    }
    let (rate, channels) = spec.ok_or("No decoded audio in file")?;
    if samples.is_empty() {
        return Err("No decoded audio in file".into());
    }
    Ok(Decoded {
        rate,
        channels: channels as u16,
        samples,
    })
}
