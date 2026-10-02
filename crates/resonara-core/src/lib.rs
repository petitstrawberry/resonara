//! OS and UI independent project model and allocation-free render loop.
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
pub mod audio;
pub mod graph;
pub mod live;
pub mod plugins;
pub use plugins::{ClapInsert, ClapParameter};
pub use resonara_clap::PluginOwner;
pub mod routing;
pub use routing::{Bus, BusId, BusKind, ChannelRouting, Destination, Insert, InsertKind, Send};
mod metronome;
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
    #[serde(default)]
    pub routing: ChannelRouting,
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
    #[serde(default)]
    pub buses: Vec<Bus>,
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
            buses: vec![],
            master: 1.0,
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
        self.validate_routing()?;
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
        let decoded = audio::read_wav(path)?;
        self.insert_audio(path, decoded)
    }
    pub fn import_audio(&mut self, path: &std::path::Path) -> Result<()> {
        let decoded = audio::read(path)?;
        self.insert_audio(path, decoded)
    }
    fn insert_audio(&mut self, path: &std::path::Path, decoded: audio::Decoded) -> Result<()> {
        if !(8000..=192000).contains(&self.sample_rate) {
            return Err("Invalid project sample rate".into());
        }
        let source = decoded.samples;
        let length = (source.len() as u64 * self.sample_rate as u64 / decoded.rate as u64) as usize;
        let samples = (0..length)
            .map(|i| {
                let x = i as f64 * decoded.rate as f64 / self.sample_rate as f64;
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
                source_channels: decoded.channels,
                start: 0,
                source_offset: 0,
                frames: length,
                samples: Arc::new(samples),
            }],
            gain: 1.,
            pan: 0.,
            mute: false,
            solo: false,
            routing: ChannelRouting::default(),
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
        use std::io::{BufWriter, Write};
        self.validate()?;
        let tmp = path.with_extension("resonara.tmp");
        // JSON emits tiny writes for embedded samples; batch them before file I/O.
        let mut file = BufWriter::with_capacity(256 * 1024, std::fs::File::create(&tmp)?);
        serde_json::to_writer(&mut file, self)?;
        file.flush()?;
        file.get_ref().sync_all()?;
        drop(file);
        std::fs::rename(tmp, path)?;
        Ok(())
    }
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let p: Self = serde_json::from_reader(std::io::BufReader::with_capacity(
            256 * 1024,
            std::fs::File::open(path)?,
        ))?;
        p.validate()?;
        Ok(p)
    }
    pub fn export_wav(&self, path: &std::path::Path) -> Result<()> {
        self.validate()?;
        let controls = Arc::new(Controls::new(self));
        let mut engine = Engine::try_new(self, controls, self.sample_rate, 0)?;
        if engine.graph_info().unavailable_plugins > 0 {
            return Err(
                "Cannot export: an active CLAP plug-in is unavailable or incompatible".into(),
            );
        }
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
            if engine.controls.error.load(Ordering::Relaxed) {
                return Err("Export stopped after an audio processing failure".into());
            }
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
                routing: ChannelRouting::default(),
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
/// Live bus controls and post-fader stereo telemetry, in Project.buses order.
pub struct BusMixer {
    pub gain: AtomicU32,
    pub pan: AtomicU32,
    pub mute: AtomicBool,
    pub peak: AtomicU32,
    pub peak_left: AtomicU32,
    pub peak_right: AtomicU32,
}
impl BusMixer {
    pub fn set(&self, bus: &Bus) {
        self.gain.store(bus.gain.to_bits(), Ordering::Relaxed);
        self.pan.store(bus.pan.to_bits(), Ordering::Relaxed);
        self.mute.store(bus.mute, Ordering::Relaxed);
    }
}
pub struct Controls {
    pub tracks: Vec<Mixer>,
    pub buses: Vec<BusMixer>,
    /// Live send levels, as f32 bits, in track-then-bus/send-slot order.
    /// Includes disabled slots; use Project::send_control_index off the audio thread.
    pub send_gains: Vec<AtomicU32>,
    /// Live insert bypass, in track-then-bus/insert-slot order.
    pub insert_bypasses: Vec<AtomicBool>,
    pub master: AtomicU32,
    /// Playback monitoring only. Exports construct disabled controls.
    pub metronome: Arc<AtomicBool>,
    /// Actual stereo master sum after master gain, before clipping/device mapping.
    /// Positive f32 bits, accumulated until consumed with swap(0).
    pub master_peak_left: AtomicU32,
    pub master_peak_right: AtomicU32,
    pub position: Arc<AtomicU64>,
    /// Coalesced transport seek, consumed only by the audio thread.
    pub seek: Arc<AtomicU64>,
    pub playing: Arc<AtomicBool>,
    pub error: Arc<AtomicBool>,
    /// Active reachable CLAP inserts that could not be prepared. Playback uses
    /// dry placeholders; export refuses to create a file in this condition.
    pub unavailable_plugins: AtomicU32,
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
            buses: p
                .buses
                .iter()
                .map(|bus| BusMixer {
                    gain: AtomicU32::new(bus.gain.to_bits()),
                    pan: AtomicU32::new(bus.pan.to_bits()),
                    mute: AtomicBool::new(bus.mute),
                    peak: AtomicU32::new(0),
                    peak_left: AtomicU32::new(0),
                    peak_right: AtomicU32::new(0),
                })
                .collect(),
            send_gains: p
                .channel_routings()
                .flat_map(|routing| &routing.sends)
                .map(|send| AtomicU32::new(send.gain.to_bits()))
                .collect(),
            insert_bypasses: p
                .channel_routings()
                .flat_map(|routing| &routing.inserts)
                .map(|insert| AtomicBool::new(insert.bypass))
                .collect(),
            master: AtomicU32::new(p.master.to_bits()),
            metronome: Arc::new(AtomicBool::new(false)),
            master_peak_left: AtomicU32::new(0),
            master_peak_right: AtomicU32::new(0),
            position: Arc::new(AtomicU64::new(0)),
            seek: Arc::new(AtomicU64::new(u64::MAX)),
            playing: Arc::new(AtomicBool::new(true)),
            error: Arc::new(AtomicBool::new(false)),
            unavailable_plugins: AtomicU32::new(0),
        }
    }
}
/// Owns one prepared snapshot; render does not lock, allocate or destroy graphs.
pub struct Engine {
    tracks: Vec<Track>,
    clip_interpolation_ends: Vec<Vec<usize>>,
    pub controls: Arc<Controls>,
    position: f64,
    step: f64,
    duration: u64,
    clock_only: bool,
    metronome: metronome::Metronome,
    graph: graph::CompiledGraph,
    faulted: bool,
    positions: Vec<f64>,
    block_mixers: Vec<graph::BlockMixer>,
    block_bus_mixers: Vec<graph::BlockMixer>,
    block_send_gains: Vec<f32>,
    block_insert_bypasses: Vec<bool>,
}
impl Engine {
    pub fn new(p: &Project, controls: Arc<Controls>, device_rate: u32, start: u64) -> Self {
        Self::try_new(p, controls, device_rate, start)
            .expect("Project routing must be valid and within engine budgets")
    }
    /// Prepare persistent routing outside the callback; invalid routes fail cleanly.
    pub fn try_new(
        p: &Project,
        controls: Arc<Controls>,
        device_rate: u32,
        start: u64,
    ) -> Result<Self> {
        let (routing, insert_controls) = p.routing_graph_with_insert_controls()?;
        let limits = p.routing_limits();
        let mut engine = Self::with_graph(p, controls, device_rate, start, &routing, limits)?;
        engine.graph.set_insert_controls(&insert_controls);
        engine.refresh_insert_controls();
        Ok(engine)
    }
    /// Prepare an explicit runtime graph outside the audio callback, overriding
    /// persistent routing for this engine only. No graph mutation during playback.
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
        let sends = p
            .channel_routings()
            .map(|routing| routing.sends.len())
            .sum();
        if controls.send_gains.len() != sends {
            return Err("Send controls do not match project routing".into());
        }
        let graph = graph::CompiledGraph::compile_with_sends_at_rate(
            routing,
            p.tracks.len(),
            p.buses.len(),
            sends,
            limits,
            device_rate,
        )?;
        controls
            .unavailable_plugins
            .store(graph.info().unavailable_plugins, Ordering::Relaxed);
        let duration = p.duration();
        let clock_only = duration == 0 && controls.metronome.load(Ordering::Relaxed);
        let inserts = controls.insert_bypasses.len();
        // The UI may read transport state before the first device callback.
        // Publish the requested start immediately rather than briefly jumping
        // to zero (which can be outside a panned arrangement viewport).
        controls.position.store(
            if clock_only {
                start
            } else {
                start.min(duration)
            },
            Ordering::Relaxed,
        );
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
            clock_only,
            metronome: metronome::Metronome::new(p, device_rate),
            graph,
            faulted: false,
            positions: vec![0.0; limits.quantum],
            block_mixers: vec![graph::BlockMixer::default(); p.tracks.len()],
            block_bus_mixers: vec![graph::BlockMixer::default(); p.buses.len()],
            block_send_gains: vec![0.0; sends],
            block_insert_bypasses: vec![false; inserts],
        })
    }
    /// Transfer main-thread plug-in lifecycle guards before sending the Engine
    /// to an audio worker. Retain and drop them on this thread after the worker
    /// and all callbacks stop. Offline rendering can leave ownership here.
    pub fn take_plugin_owners(&mut self) -> Vec<PluginOwner> {
        self.graph.take_plugin_owners()
    }
    pub(crate) fn take_keyed_plugin_owners(&mut self) -> Vec<(usize, PluginOwner)> {
        self.graph.take_keyed_plugin_owners()
    }
    /// Adopt the exact next sample at a live graph boundary, including the
    /// fractional project-frame position when device and project rates differ.
    pub(crate) fn continue_from(&mut self, previous: &Self) {
        self.position = if self.step == previous.step {
            previous.position
        } else {
            previous.position * self.step / previous.step
        };
        self.faulted = previous.faulted;
        self.controls
            .position
            .store(self.position as u64, Ordering::Relaxed);
    }
    fn refresh_insert_controls(&mut self) {
        for (bypass, control) in self
            .block_insert_bypasses
            .iter_mut()
            .zip(&self.controls.insert_bypasses)
        {
            *bypass = control.load(Ordering::Relaxed);
        }
        let missing = self.graph.refresh_unavailable(&self.block_insert_bypasses);
        self.controls
            .unavailable_plugins
            .store(missing, Ordering::Relaxed);
    }
    /// Control thread requests a new position; DSP and GUI instances survive.
    pub fn request_seek(controls: &Controls, start: u64) {
        controls
            .seek
            .store(start.min((1u64 << 63) - 2), Ordering::Release);
    }
    /// Resume and seek are adopted together at the next audio block boundary,
    /// even when the previous block reached EOF while the request was queued.
    pub fn request_resume(controls: &Controls, start: u64) {
        controls.seek.store(
            start.min((1u64 << 63) - 2) | (1u64 << 63),
            Ordering::Release,
        );
        controls.playing.store(true, Ordering::Release);
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
        let metronome_enabled = self.controls.metronome.load(Ordering::Relaxed);
        self.clock_only |= self.duration == 0 && metronome_enabled;
        let quantum = self.graph.info().quantum;
        for chunk in out.chunks_mut(channels.saturating_mul(quantum)) {
            self.refresh_insert_controls();
            let seek = self.controls.seek.swap(u64::MAX, Ordering::AcqRel);
            if seek != u64::MAX {
                if seek & (1u64 << 63) != 0 {
                    self.controls.playing.store(true, Ordering::Relaxed);
                }
                let start = seek & !(1u64 << 63);
                self.position = if self.clock_only {
                    start
                } else {
                    start.min(self.duration)
                } as f64;
                self.graph.reset_transport();
            }
            let frames = chunk.len().div_ceil(channels);
            let mut active = 0;
            let playing = self.controls.playing.load(Ordering::Acquire);
            if !self.faulted && playing {
                for position in &mut self.positions[..frames] {
                    if !self.clock_only && self.position >= self.duration as f64 {
                        break;
                    }
                    *position = self.position;
                    self.position += self.step;
                    active += 1;
                }
            }
            if playing && active < frames && self.controls.seek.load(Ordering::Acquire) == u64::MAX
            {
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
                for (index, mix) in self.block_bus_mixers.iter_mut().enumerate() {
                    *mix = if let Some(control) = self.controls.buses.get(index) {
                        graph::BlockMixer {
                            gain: f32::from_bits(control.gain.load(Ordering::Relaxed)),
                            pan: f32::from_bits(control.pan.load(Ordering::Relaxed)),
                            audible: !control.mute.load(Ordering::Relaxed),
                            peak: [0.0; 2],
                        }
                    } else {
                        graph::BlockMixer::default()
                    };
                }
                for (gain, control) in self
                    .block_send_gains
                    .iter_mut()
                    .zip(&self.controls.send_gains)
                {
                    *gain = f32::from_bits(control.load(Ordering::Relaxed));
                }
                let tracks = &self.tracks;
                let interpolation_ends = &self.clip_interpolation_ends;
                let positions = &self.positions;
                let processed = self.graph.process(
                    active,
                    &mut self.block_mixers,
                    &mut self.block_bus_mixers,
                    &self.block_send_gains,
                    &self.block_insert_bypasses,
                    |track, block| {
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
                    },
                );
                if !processed {
                    self.faulted = true;
                    self.controls.error.store(true, Ordering::Relaxed);
                    self.controls.playing.store(false, Ordering::Relaxed);
                }
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
                for (mix, control) in self.block_bus_mixers.iter().zip(&self.controls.buses) {
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
            if active == 0 && !self.faulted && !self.graph.flush_plugins() {
                self.faulted = true;
                self.controls.error.store(true, Ordering::Relaxed);
                self.controls.playing.store(false, Ordering::Relaxed);
            }
            // Retain the existing hard EOF boundary for this foundation. Effect
            // tails, latency compensation and live graph replacement are separate work.
            let output = self.graph.output(active);
            let mut master_peak = [0.0f32; 2];
            for (index, frame) in chunk.chunks_mut(channels).enumerate() {
                let mut sum = output.get(index).copied().unwrap_or([0.0; 2]);
                if metronome_enabled && index < active {
                    let click = self.metronome.sample(self.positions[index]);
                    sum[0] += click;
                    sum[1] += click;
                }
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
        self.controls.position.store(
            if self.clock_only {
                self.position as u64
            } else {
                (self.position as u64).min(self.duration)
            },
            Ordering::Relaxed,
        );
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
