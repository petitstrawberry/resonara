//! OS and UI independent project model and allocation-free render loop.
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
pub mod graph;
mod wav;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Clip {
    /// Original source channel count. Internal mono samples are duplicated to L/R.
    /// Older projects did not retain this information; conservatively show stereo.
    #[serde(default = "default_source_channels")]
    pub source_channels: u16,
    pub start: u64,
    pub source_offset: usize,
    pub frames: usize,
    pub samples: Arc<Vec<[f32; 2]>>,
}
fn default_source_channels() -> u16 {
    2
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
/// Display meter; tempo remains quarter notes per minute and audio stays sample based.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct TimeSignature {
    pub numerator: u8,
    pub denominator: u8,
}
impl Default for TimeSignature {
    fn default() -> Self {
        Self {
            numerator: 4,
            denominator: 4,
        }
    }
}
impl TimeSignature {
    pub fn valid(self) -> bool {
        (1..=32).contains(&self.numerator) && [1, 2, 4, 8, 16, 32].contains(&self.denominator)
    }
    pub fn beat_seconds(self, tempo: f64) -> f64 {
        60. / tempo * 4. / self.denominator as f64
    }
    pub fn ticks_per_beat(self) -> u64 {
        3840 / self.denominator as u64
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Project {
    pub version: u32,
    pub sample_rate: u32,
    pub tracks: Vec<Track>,
    pub master: f32,
    #[serde(default = "default_tempo")]
    pub tempo: f64,
    #[serde(default)]
    pub time_signature: TimeSignature,
}
fn default_tempo() -> f64 {
    120.
}
impl Default for Project {
    fn default() -> Self {
        Self {
            version: 1,
            sample_rate: 48000,
            tracks: vec![],
            master: 0.8,
            tempo: default_tempo(),
            time_signature: TimeSignature::default(),
        }
    }
}
impl Project {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || !(8000..=192000).contains(&self.sample_rate)
            || !self.master.is_finite()
            || !(0.0..=2.0).contains(&self.master)
            || !self.time_signature.valid()
            || !self.tempo.is_finite()
            || !(20.0..=400.0).contains(&self.tempo)
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
                if !(1..=2).contains(&c.source_channels)
                    || c.source_offset
                        .checked_add(c.frames)
                        .is_none_or(|end| end > c.samples.len())
                    || c.start.checked_add(c.frames as u64).is_none()
                    || c.samples.iter().any(|frame| {
                        !frame[0].is_finite()
                            || !frame[1].is_finite()
                            || (c.source_channels == 1 && frame[0] != frame[1])
                    })
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
        let mut r = wav::open(path)?;
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
                source_channels: spec.channels,
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
                    source_channels: 1,
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
    /// Compatibility max(L, R) peak. UI consumers clear with swap(0).
    pub peak: AtomicU32,
    /// Independent post-fader/pan peaks, positive f32 bits; clear with swap(0).
    pub peak_left: AtomicU32,
    pub peak_right: AtomicU32,
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
    /// Actual stereo master sum after master gain, before clipping/device mapping.
    /// Positive f32 bits, accumulated until consumed with swap(0).
    pub master_peak_left: AtomicU32,
    pub master_peak_right: AtomicU32,
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
                    peak_left: AtomicU32::new(0),
                    peak_right: AtomicU32::new(0),
                })
                .collect(),
            master: AtomicU32::new(p.master.to_bits()),
            master_peak_left: AtomicU32::new(0),
            master_peak_right: AtomicU32::new(0),
            position: AtomicU64::new(0),
            playing: AtomicBool::new(true),
            error: AtomicBool::new(false),
        }
    }
}
/// Holds its snapshot for the entire stream lifetime; no swaps, locks, allocation or destruction in render.
pub struct Engine {
    tracks: Vec<Track>,
    clip_interpolation_ends: Vec<Vec<usize>>,
    pub controls: Arc<Controls>,
    position: f64,
    step: f64,
    duration: u64,
    graph: graph::CompiledGraph,
    faulted: bool,
    positions: Vec<f64>,
    block_mixers: Vec<graph::BlockMixer>,
}
impl Engine {
    pub fn new(p: &Project, controls: Arc<Controls>, device_rate: u32, start: u64) -> Self {
        let routing = graph::RoutingGraph::tracks_to_master(p.tracks.len());
        // Keep the original constructor usable for arbitrary project sizes.
        // Opt-in graphs use explicit admission budgets through with_graph.
        let defaults = graph::GraphLimits::default();
        let limits = graph::GraphLimits {
            max_nodes: defaults.max_nodes.max(routing.nodes.len()),
            max_edges: defaults.max_edges.max(routing.routes.len()),
            max_scratch_bytes: defaults.max_scratch_bytes.max(
                routing
                    .nodes
                    .len()
                    .saturating_mul(defaults.quantum)
                    .saturating_mul(8),
            ),
            ..defaults
        };
        Self::with_graph(p, controls, device_rate, start, &routing, limits)
            .expect("the default track-to-master graph is valid")
    }
    /// Prepare an opt-in routing graph outside the audio callback. This does not
    /// alter Project serialization or enable graph mutation during playback.
    pub fn with_graph(
        p: &Project,
        controls: Arc<Controls>,
        device_rate: u32,
        start: u64,
        routing: &graph::RoutingGraph,
        limits: graph::GraphLimits,
    ) -> Result<Self> {
        if device_rate == 0 {
            return Err("Device sample rate must be nonzero".into());
        }
        let graph = graph::CompiledGraph::compile(routing, p.tracks.len(), limits)?;
        let duration = p.duration();
        // The UI may read transport state before the first device callback.
        // Publish the requested start immediately rather than briefly jumping
        // to zero (which can be outside a panned arrangement viewport).
        controls
            .position
            .store(start.min(duration), Ordering::Relaxed);
        Ok(Self {
            tracks: p.tracks.clone(),
            // Splitting a clip must not change the interpolation at the cut.
            // Prepare each clip's lookahead before the real-time render loop,
            // and allow it only for an actual continuation of the same source.
            clip_interpolation_ends: p
                .tracks
                .iter()
                .map(|track| {
                    track
                        .clips
                        .iter()
                        .map(|clip| {
                            let end = clip.start.checked_add(clip.frames as u64);
                            let source_end = clip.source_offset.checked_add(clip.frames);
                            let continues = clip.frames > 0
                                && track.clips.iter().any(|next| {
                                    next.frames > 0
                                        && Some(next.start) == end
                                        && Some(next.source_offset) == source_end
                                        // JSON restores separate Arcs for shared audio.
                                        // Compare full buffers only for adjacent source
                                        // ranges, during setup rather than rendering.
                                        && (Arc::ptr_eq(&clip.samples, &next.samples)
                                            || clip.samples.as_ref() == next.samples.as_ref())
                                });
                            if continues {
                                clip.frames
                            } else {
                                clip.frames.saturating_sub(1)
                            }
                        })
                        .collect()
                })
                .collect(),
            controls,
            position: start as f64,
            step: p.sample_rate as f64 / device_rate as f64,
            duration,
            graph,
            faulted: false,
            positions: vec![0.0; limits.quantum],
            block_mixers: vec![graph::BlockMixer::default(); p.tracks.len()],
        })
    }
    pub fn graph_info(&self) -> &graph::GraphInfo {
        self.graph.info()
    }
    pub fn render_counts(&self) -> graph::RenderCounts {
        self.graph.counts()
    }
    pub fn node_process_count(&self, id: graph::NodeId) -> Option<u64> {
        self.graph.node_process_count(id)
    }
    pub fn render<T: OutputSample>(&mut self, out: &mut [T], channels: usize) {
        if channels == 0 {
            return;
        }
        if self.faulted {
            out.fill(T::from_f32(0.0));
            self.controls.error.store(true, Ordering::Relaxed);
            self.controls.playing.store(false, Ordering::Relaxed);
            return;
        }
        let solo = self
            .controls
            .tracks
            .iter()
            .any(|m| m.solo.load(Ordering::Relaxed));
        let master = f32::from_bits(self.controls.master.load(Ordering::Relaxed));
        let quantum = self.graph.info().quantum;
        for chunk in out.chunks_mut(channels.saturating_mul(quantum)) {
            let frames = chunk.len().div_ceil(channels);
            let mut active = 0;
            if !self.faulted && self.controls.playing.load(Ordering::Relaxed) {
                for position in &mut self.positions[..frames] {
                    if self.position >= self.duration as f64 {
                        break;
                    }
                    *position = self.position;
                    self.position += self.step;
                    active += 1;
                }
            }
            if active < frames {
                self.controls.playing.store(false, Ordering::Relaxed);
            }
            if active > 0 {
                for (index, mix) in self.block_mixers.iter_mut().enumerate() {
                    *mix = if let Some(control) = self.controls.tracks.get(index) {
                        graph::BlockMixer {
                            gain: f32::from_bits(control.gain.load(Ordering::Relaxed)),
                            pan: f32::from_bits(control.pan.load(Ordering::Relaxed)),
                            audible: !control.mute.load(Ordering::Relaxed)
                                && (!solo || control.solo.load(Ordering::Relaxed)),
                            peak: [0.0; 2],
                        }
                    } else {
                        graph::BlockMixer::default()
                    };
                }
                let tracks = &self.tracks;
                let interpolation_ends = &self.clip_interpolation_ends;
                let positions = &self.positions;
                self.graph
                    .process(active, &mut self.block_mixers, |track, block| {
                        // Visit each clip's block intersection, retaining clip
                        // summation order and sample-by-sample transport positions.
                        let positions = &positions[..block.len()];
                        for (clip, &interpolation_end) in
                            tracks[track].clips.iter().zip(&interpolation_ends[track])
                        {
                            let begin =
                                positions.partition_point(|&p| p - (clip.start as f64) < 0.0);
                            let end = begin
                                + positions[begin..].partition_point(|&p| {
                                    p - (clip.start as f64) < clip.frames as f64
                                });
                            for (sample, &position) in
                                block[begin..end].iter_mut().zip(&positions[begin..end])
                            {
                                let x = position - clip.start as f64;
                                let a = x as usize;
                                let b = (a + 1).min(interpolation_end);
                                let f = x.fract() as f32;
                                for (channel, value) in sample.iter_mut().enumerate() {
                                    *value += clip.samples[clip.source_offset + a][channel]
                                        * (1.0 - f)
                                        + clip.samples[clip.source_offset + b][channel] * f;
                                }
                            }
                        }
                    });
                for (mix, control) in self.block_mixers.iter().zip(&self.controls.tracks) {
                    control
                        .peak
                        .fetch_max(mix.peak[0].max(mix.peak[1]).to_bits(), Ordering::Relaxed);
                    control
                        .peak_left
                        .fetch_max(mix.peak[0].to_bits(), Ordering::Relaxed);
                    control
                        .peak_right
                        .fetch_max(mix.peak[1].to_bits(), Ordering::Relaxed);
                }
            }
            // Retain the existing hard EOF boundary for this foundation. Effect
            // tails, latency compensation and live graph replacement are separate work.
            let output = self.graph.output(active);
            let mut master_peak = [0.0f32; 2];
            for (index, frame) in chunk.chunks_mut(channels).enumerate() {
                let sum = output.get(index).copied().unwrap_or([0.0; 2]);
                if !self.faulted {
                    for channel in 0..2 {
                        // Keep overrange peaks visible. Nonfinite faults saturate
                        // telemetry while the output guard below fail-stops audio.
                        master_peak[channel] =
                            master_peak[channel].max((sum[channel] * master).abs().min(f32::MAX));
                    }
                }
                for (channel, value) in frame.iter_mut().enumerate() {
                    let sample = if channels == 1 {
                        (sum[0] + sum[1]) * 0.5
                    } else if channel < 2 {
                        sum[channel]
                    } else {
                        0.0
                    };
                    let scaled = sample * master;
                    // Finite parameters can still overflow a large graph. Never
                    // hand NaN/Inf to the device. Faults are latched: normal
                    // playback requires a newly prepared Engine, never reusing
                    // potentially poisoned filter/delay state after a gain reset.
                    let safe = if self.faulted {
                        0.0
                    } else if scaled.is_finite() {
                        scaled.clamp(-1.0, 1.0)
                    } else {
                        self.faulted = true;
                        self.controls.error.store(true, Ordering::Relaxed);
                        self.controls.playing.store(false, Ordering::Relaxed);
                        0.0
                    };
                    *value = T::from_f32(safe);
                }
            }
            self.controls
                .master_peak_left
                .fetch_max(master_peak[0].to_bits(), Ordering::Relaxed);
            self.controls
                .master_peak_right
                .fetch_max(master_peak[1].to_bits(), Ordering::Relaxed);
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
