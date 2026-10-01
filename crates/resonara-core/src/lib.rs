//! OS and UI independent project model and allocation-free render loop.
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Clip {
    pub start: u64,
    pub source_offset: usize,
    pub frames: usize,
    pub samples: Arc<Vec<[f32; 2]>>,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Track {
    pub name: String,
    pub clips: Vec<Clip>,
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Project {
    pub version: u32,
    pub sample_rate: u32,
    pub tracks: Vec<Track>,
    pub master: f32,
}
impl Default for Project {
    fn default() -> Self {
        Self {
            version: 1,
            sample_rate: 48000,
            tracks: vec![],
            master: 0.8,
        }
    }
}
impl Project {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || !(8000..=192000).contains(&self.sample_rate)
            || !self.master.is_finite()
            || !(0.0..=2.0).contains(&self.master)
        {
            return Err("Invalid project header".into());
        }
        for t in &self.tracks {
            if !t.gain.is_finite()
                || !(0.0..=2.0).contains(&t.gain)
                || !t.pan.is_finite()
                || !(-1.0..=1.0).contains(&t.pan)
            {
                return Err("Invalid mixer value".into());
            }
            for c in &t.clips {
                if c.source_offset
                    .checked_add(c.frames)
                    .is_none_or(|end| end > c.samples.len())
                    || c.start.checked_add(c.frames as u64).is_none()
                    || c.samples.iter().flatten().any(|v| !v.is_finite())
                {
                    return Err("Invalid clip".into());
                }
            }
        }
        Ok(())
    }
    pub fn duration(&self) -> u64 {
        self.tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(|c| c.start + c.frames as u64)
            .max()
            .unwrap_or(0)
    }
    pub fn import_wav(&mut self, path: &std::path::Path) -> Result<()> {
        let mut r = hound::WavReader::open(path)?;
        let spec = r.spec();
        if !(1..=2).contains(&spec.channels) || spec.sample_rate == 0 {
            return Err("Only mono/stereo WAV is supported".into());
        }
        let raw: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => {
                r.samples::<f32>().collect::<std::result::Result<_, _>>()?
            }
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
        let source: Vec<[f32; 2]> = raw
            .chunks_exact(spec.channels as usize)
            .map(|s| [s[0], *s.get(1).unwrap_or(&s[0])])
            .collect();
        let length =
            (source.len() as u64 * self.sample_rate as u64 / spec.sample_rate as u64) as usize;
        let samples = (0..length)
            .map(|i| {
                let x = i as f64 * spec.sample_rate as f64 / self.sample_rate as f64;
                let a = x as usize;
                let b = (a + 1).min(source.len() - 1);
                let f = x.fract() as f32;
                [
                    source[a][0] * (1. - f) + source[b][0] * f,
                    source[a][1] * (1. - f) + source[b][1] * f,
                ]
            })
            .collect();
        self.tracks.push(Track {
            name: path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            clips: vec![Clip {
                start: 0,
                source_offset: 0,
                frames: length,
                samples: Arc::new(samples),
            }],
            gain: 1.,
            pan: 0.,
            mute: false,
            solo: false,
        });
        Ok(())
    }
    pub fn split(&mut self, track: usize, at: u64) -> Result<()> {
        let t = self.tracks.get_mut(track).ok_or("Select a track")?;
        let i = t
            .clips
            .iter()
            .position(|c| at > c.start && at < c.start + c.frames as u64)
            .ok_or("Split position must be inside a clip")?;
        let mut right = t.clips[i].clone();
        let left = (at - right.start) as usize;
        right.start = at;
        right.source_offset += left;
        right.frames -= left;
        t.clips[i].frames = left;
        t.clips.insert(i + 1, right);
        Ok(())
    }
    pub fn trim(&mut self, track: usize, from: u64, to: u64) -> Result<()> {
        if from >= to {
            return Err("Trim end must follow start".into());
        }
        let t = self.tracks.get_mut(track).ok_or("Select a track")?;
        t.clips.retain_mut(|c| {
            let a = from.max(c.start);
            let b = to.min(c.start + c.frames as u64);
            if a >= b {
                return false;
            }
            c.source_offset += (a - c.start) as usize;
            c.start = a;
            c.frames = (b - a) as usize;
            true
        });
        Ok(())
    }
    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        self.validate()?;
        let tmp = path.with_extension("resonara.tmp");
        let mut file = std::fs::File::create(&tmp)?;
        serde_json::to_writer(&mut file, self)?;
        file.sync_all()?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let p: Self = serde_json::from_reader(std::fs::File::open(path)?)?;
        p.validate()?;
        Ok(p)
    }
    pub fn export_wav(&self, path: &std::path::Path) -> Result<()> {
        self.validate()?;
        let controls = Arc::new(Controls::new(self));
        let mut engine = Engine::new(self, controls, self.sample_rate, 0);
        let mut writer = hound::WavWriter::create(
            path,
            hound::WavSpec {
                channels: 2,
                sample_rate: self.sample_rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        let mut block = [0f32; 1024];
        let mut remaining = self.duration();
        while remaining > 0 {
            let frames = remaining.min(512) as usize;
            engine.render(&mut block[..frames * 2], 2);
            for s in &block[..frames * 2] {
                writer.write_sample(*s)?;
            }
            remaining -= frames as u64;
        }
        writer.finalize()?;
        Ok(())
    }
    pub fn demo() -> Self {
        let mut p = Self::default();
        for (name, hz, start) in [
            ("Warm keys", 220., 0),
            ("Air melody", 330., 24000),
            ("Bass", 110., 48000),
        ] {
            let samples = (0..144000)
                .map(|i| {
                    let env = (1. - i as f32 / 144000.) * 0.2;
                    let s = (i as f32 * hz * std::f32::consts::TAU / 48000.).sin() * env;
                    [s, s]
                })
                .collect();
            p.tracks.push(Track {
                name: name.into(),
                clips: vec![Clip {
                    start,
                    source_offset: 0,
                    frames: 144000,
                    samples: Arc::new(samples),
                }],
                gain: 1.,
                pan: 0.,
                mute: false,
                solo: false,
            });
        }
        p
    }
}
pub struct Mixer {
    pub gain: AtomicU32,
    pub pan: AtomicU32,
    pub mute: AtomicBool,
    pub solo: AtomicBool,
    pub peak: AtomicU32,
}
impl Mixer {
    pub fn set(&self, t: &Track) {
        self.gain.store(t.gain.to_bits(), Ordering::Relaxed);
        self.pan.store(t.pan.to_bits(), Ordering::Relaxed);
        self.mute.store(t.mute, Ordering::Relaxed);
        self.solo.store(t.solo, Ordering::Relaxed);
    }
}
pub struct Controls {
    pub tracks: Vec<Mixer>,
    pub master: AtomicU32,
    pub position: AtomicU64,
    pub playing: AtomicBool,
    pub error: AtomicBool,
}
impl Controls {
    pub fn new(p: &Project) -> Self {
        Self {
            tracks: p
                .tracks
                .iter()
                .map(|t| Mixer {
                    gain: AtomicU32::new(t.gain.to_bits()),
                    pan: AtomicU32::new(t.pan.to_bits()),
                    mute: AtomicBool::new(t.mute),
                    solo: AtomicBool::new(t.solo),
                    peak: AtomicU32::new(0),
                })
                .collect(),
            master: AtomicU32::new(p.master.to_bits()),
            position: AtomicU64::new(0),
            playing: AtomicBool::new(true),
            error: AtomicBool::new(false),
        }
    }
}
/// Holds its snapshot for the entire stream lifetime; no swaps, locks, allocation or destruction in render.
pub struct Engine {
    tracks: Vec<Track>,
    pub controls: Arc<Controls>,
    position: f64,
    step: f64,
    duration: u64,
}
impl Engine {
    pub fn new(p: &Project, controls: Arc<Controls>, device_rate: u32, start: u64) -> Self {
        Self {
            tracks: p.tracks.clone(),
            controls,
            position: start as f64,
            step: p.sample_rate as f64 / device_rate as f64,
            duration: p.duration(),
        }
    }
    pub fn render<T: OutputSample>(&mut self, out: &mut [T], channels: usize) {
        if channels == 0 {
            return;
        }
        let solo = self
            .controls
            .tracks
            .iter()
            .any(|m| m.solo.load(Ordering::Relaxed));
        let master = f32::from_bits(self.controls.master.load(Ordering::Relaxed));
        for frame in out.chunks_mut(channels) {
            let mut sum = [0f32; 2];
            if self.controls.playing.load(Ordering::Relaxed) && self.position < self.duration as f64
            {
                for (track, mix) in self.tracks.iter().zip(&self.controls.tracks) {
                    if mix.mute.load(Ordering::Relaxed)
                        || (solo && !mix.solo.load(Ordering::Relaxed))
                    {
                        continue;
                    }
                    let gain = f32::from_bits(mix.gain.load(Ordering::Relaxed));
                    let pan = f32::from_bits(mix.pan.load(Ordering::Relaxed));
                    let mut sample = [0f32; 2];
                    for c in &track.clips {
                        let x = self.position - c.start as f64;
                        if x < 0. || x >= c.frames as f64 {
                            continue;
                        }
                        let a = x as usize;
                        let b = (a + 1).min(c.frames - 1);
                        let f = x.fract() as f32;
                        for (ch, v) in sample.iter_mut().enumerate() {
                            *v += c.samples[c.source_offset + a][ch] * (1. - f)
                                + c.samples[c.source_offset + b][ch] * f;
                        }
                    }
                    sample[0] *= gain * (1. - pan.max(0.));
                    sample[1] *= gain * (1. + pan.min(0.));
                    let peak = sample[0].abs().max(sample[1].abs());
                    mix.peak.fetch_max(peak.to_bits(), Ordering::Relaxed);
                    sum[0] += sample[0];
                    sum[1] += sample[1];
                }
                self.position += self.step;
            } else {
                self.controls.playing.store(false, Ordering::Relaxed);
            }
            for (i, v) in frame.iter_mut().enumerate() {
                let s = if channels == 1 {
                    (sum[0] + sum[1]) * 0.5
                } else if i < 2 {
                    sum[i]
                } else {
                    0.
                };
                *v = T::from_f32((s * master).clamp(-1., 1.));
            }
        }
        self.controls
            .position
            .store((self.position as u64).min(self.duration), Ordering::Relaxed);
    }
}
pub trait OutputSample: Copy {
    fn from_f32(value: f32) -> Self;
}
impl OutputSample for f32 {
    fn from_f32(v: f32) -> Self {
        v
    }
}
impl OutputSample for i16 {
    fn from_f32(v: f32) -> Self {
        (v * 32767.).round() as i16
    }
}
impl OutputSample for u16 {
    fn from_f32(v: f32) -> Self {
        ((v + 1.) * 32767.5).round() as u16
    }
}
