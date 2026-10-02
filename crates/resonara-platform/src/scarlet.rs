//! Scarlet Audio Server adapter. The worker exclusively owns the SAS mapping.
use crate::pcm::{CHANNELS, PERIOD_FRAMES, Pump, Ring, Step};
use resonara_core::{Controls, Project, Result, live::Playback};
use sas_client::{SasClient, SasStream, StreamConfig};
use scarlet_os::scheduler::{self, ConfiguredScheduler, DeadlineConfig, SchedulerError};
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const SAMPLE_RATE: u32 = 48_000;
// Scarlet's native PCM ABI value; this adapter selects S16LE stereo.
const FORMAT_S16LE: u32 = 1;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const STALL_TIMEOUT: Duration = Duration::from_secs(3);
// Implicit deadline: one 256-frame block every ~5.33 ms, with half a CPU's
// runtime reserved. SAS itself reserves 25%, so both fit below Scarlet's 90%
// admission limit even when placed on the same CPU.
const DEADLINE_PERIOD: Duration =
    Duration::from_nanos(PERIOD_FRAMES as u64 * 1_000_000_000 / SAMPLE_RATE as u64);
const DEADLINE_RUNTIME: Duration = Duration::from_nanos(DEADLINE_PERIOD.as_nanos() as u64 / 2);

struct AudioDeadline {
    restore: ConfiguredScheduler,
}

impl AudioDeadline {
    // Current-task APIs must run inside the producer, never on the UI thread.
    fn reserve() -> std::result::Result<Self, SchedulerError> {
        let restore = scheduler::configured()?;
        let cpu = scheduler::runtime_state()?
            .current_cpu_id()
            .ok_or(SchedulerError::InvalidDeadlineCpu)?;
        let deadline = DeadlineConfig::new(DEADLINE_RUNTIME, DEADLINE_PERIOD, cpu)?;
        let mut configured = restore.clone();
        configured.activate_deadline(deadline);
        configured.apply()?;
        eprintln!(
            "[Resonara audio] deadline enabled: cpu={cpu} runtime={}ns period={}ns",
            DEADLINE_RUNTIME.as_nanos(),
            DEADLINE_PERIOD.as_nanos(),
        );
        Ok(Self { restore })
    }
}

fn reserve_deadline() -> Option<AudioDeadline> {
    if std::env::var("RESONARA_SCARLET_DEADLINE").as_deref() == Ok("0") {
        return None;
    }
    match AudioDeadline::reserve() {
        Ok(reservation) => Some(reservation),
        Err(error) => {
            eprintln!(
                "[Resonara audio] deadline unavailable: {error:?}; retaining current scheduler"
            );
            None
        }
    }
}
fn connect_stream(cancelled: impl Fn() -> bool) -> Result<(SasClient, SasStream)> {
    let mut client = SasClient::connect()
        .map_err(|e| format!("Scarlet audio: {}. Check that SAS is running.", e.as_str()))?;
    let config = StreamConfig {
        format: FORMAT_S16LE,
        rate: SAMPLE_RATE,
        channels: CHANNELS as u16,
        period_frames: PERIOD_FRAMES as u32,
        buffer_frames: (PERIOD_FRAMES * 4) as u32,
    };
    let started = Instant::now();
    let stream = client
        .configure_cancellable(&config, || {
            cancelled() || started.elapsed() >= CONNECT_TIMEOUT
        })
        .map_err(|e| format!("Scarlet audio configuration: {}", e.as_str()))?
        .ok_or("Scarlet audio configuration timed out or cancelled")?;
    Ok((client, stream))
}

impl Drop for AudioDeadline {
    fn drop(&mut self) {
        // Report after rendering stops, never log or query scheduler state per
        // block. These are kernel scheduling counters, not SAS underrun counts.
        match scheduler::runtime_state() {
            Ok(state) => eprintln!(
                "[Resonara audio] deadline stats: misses={} overruns={}",
                state.deadline_miss_count(),
                state.deadline_overrun_count(),
            ),
            Err(error) => eprintln!("[Resonara audio] deadline stats unavailable: {error:?}"),
        }
        if let Err(error) = self.restore.apply() {
            eprintln!("[Resonara audio] could not restore scheduler: {error:?}");
        }
    }
}

pub struct Audio {
    stop: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    transport: Arc<AtomicU64>,
    desired_playing: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    playback: Playback,
    _owner_thread: PhantomData<Rc<()>>,
    pub controls: Arc<Controls>,
    pub device: String,
}

impl Audio {
    pub fn update(&mut self, project: &Project) -> Result<()> {
        self.playback.update(project)?;
        self.controls = self.playback.controls.clone();
        Ok(())
    }
    pub fn open_editor(&self, slot: usize) -> Result<bool> {
        self.playback.open_editor(slot)
    }
    pub fn has_open_editors(&self) -> bool {
        self.playback.has_open_editors().unwrap_or(false)
    }
    pub fn pause(&self) {
        self.desired_playing.store(false, Ordering::Release);
        self.controls.playing.store(false, Ordering::Release);
    }
    // Recreate only the SAS stream, retaining the renderer and CLAP owners.
    // The worker adopts transport commands after dropping any queued PCM.
    pub fn seek(&self, start: u64) -> bool {
        let resume = self.desired_playing.load(Ordering::Acquire);
        self.request_transport(start, resume)
    }
    pub fn resume(&self, start: u64, metronome: bool) -> bool {
        self.controls.metronome.store(metronome, Ordering::Relaxed);
        self.request_transport(start, true)
    }
    fn request_transport(&self, start: u64, resume: bool) -> bool {
        if self.controls.error.load(Ordering::Acquire) {
            return false;
        }
        self.desired_playing.store(resume, Ordering::Release);
        self.controls.position.store(start, Ordering::Relaxed);
        self.finished.store(false, Ordering::Release);
        self.transport.store(
            start.min((1u64 << 63) - 2) | ((resume as u64) << 63),
            Ordering::Release,
        );
        true
    }
    pub fn close_editors(&self) -> Result<()> {
        self.playback.close_editors()
    }
    pub fn poll_plugins(
        &mut self,
        force: bool,
    ) -> Result<Vec<(usize, resonara_core::plugins::ClapInsert)>> {
        self.playback.poll_plugins(force)
    }
    pub fn collect_retired(&mut self) {
        self.playback.collect_retired();
    }
    /// Completion means the final PCM has reached SAS. Keep this Audio alive
    /// afterward: SAS can still have samples queued at the output device.
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub fn start(project: &Project, start: u64) -> Result<Self> {
        Self::start_with_metronome(project, start, false)
    }

    pub fn start_with_metronome(project: &Project, start: u64, metronome: bool) -> Result<Self> {
        Self::start_transport(project, start, metronome, true)
    }
    pub fn start_paused(project: &Project) -> Result<Self> {
        Self::start_transport(project, 0, false, false)
    }
    fn start_transport(
        project: &Project,
        start: u64,
        metronome: bool,
        playing: bool,
    ) -> Result<Self> {
        project.validate()?;
        let connection = connect_stream(|| false)?;
        // Resolve all fallible device setup before loading/activating plugins.
        // After guard extraction, only spawn can fail; its captured Engine is
        // dropped before the caller's guards if thread creation fails.
        let (playback, mut engine) = Playback::new(project, SAMPLE_RATE, start, metronome)?;
        let controls = playback.controls.clone();
        controls.playing.store(playing, Ordering::Release);
        let transport = Arc::new(AtomicU64::new(u64::MAX));
        let worker_transport = transport.clone();
        let desired_playing = Arc::new(AtomicBool::new(playing));
        let worker_desired_playing = desired_playing.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(!playing));
        let worker_stop = stop.clone();
        let worker_finished = finished.clone();
        let worker_controls = controls.clone();
        let worker = thread::Builder::new()
            .name("resonara-sas".into())
            .spawn(move || {
                let mut connection = Some(connection);
                let mut pump = Pump::new();
                let mut idle = !playing;
                let mut deadline = if playing { reserve_deadline() } else { None };
                let mut progressed = Instant::now();
                let mut silence = [0i16; PERIOD_FRAMES * CHANNELS];
                while !worker_stop.load(Ordering::Acquire) {
                    let command = worker_transport.swap(u64::MAX, Ordering::AcqRel);
                    if command != u64::MAX {
                        drop(deadline.take());
                        drop(connection.take());
                        match connect_stream(|| worker_stop.load(Ordering::Acquire)) {
                            Ok(next) => connection = Some(next),
                            Err(error) => {
                                if !worker_stop.load(Ordering::Acquire) {
                                    eprintln!("[Resonara audio] resume failed: {error}");
                                    worker_controls.error.store(true, Ordering::Release);
                                    worker_controls.playing.store(false, Ordering::Release);
                                }
                                break;
                            }
                        }
                        let start = command & !(1u64 << 63);
                        // A Stop received while configuration was pending must
                        // still win over the already-consumed Resume command.
                        idle = command & (1u64 << 63) == 0
                            || !worker_desired_playing.load(Ordering::Acquire);
                        if idle {
                            worker_controls.playing.store(false, Ordering::Release);
                            resonara_core::Engine::request_seek(&worker_controls, start);
                        } else {
                            // This worker alone renders, so seek and play can be
                            // adopted here without a deferred resume bit that
                            // could override a subsequent UI Stop at render time.
                            resonara_core::Engine::request_seek(&worker_controls, start);
                            worker_controls.playing.store(true, Ordering::Release);
                            if !worker_desired_playing.load(Ordering::Acquire) {
                                worker_controls.playing.store(false, Ordering::Release);
                                idle = true;
                            } else {
                                deadline = reserve_deadline();
                            }
                        }
                        pump = Pump::new();
                        progressed = Instant::now();
                        worker_finished.store(idle, Ordering::Release);
                    }
                    engine.apply_pending();
                    if idle {
                        // Service active CLAP flushes and graph swaps while stopped,
                        // without feeding SAS or advancing the transport.
                        engine.render(&mut silence, CHANNELS);
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    match pump.step(
                        engine.engine_mut(),
                        &mut connection.as_mut().unwrap().1,
                        false,
                    ) {
                        Ok(Step::Finished) => {
                            drop(deadline.take());
                            worker_finished.store(true, Ordering::Release);
                            // Preserve SAS's hardware tail and the exact plugin
                            // instances until a transport command or Audio drop.
                            idle = true;
                        }
                        Ok(Step::Cancelled) => break,
                        Ok(Step::Progress) => progressed = Instant::now(),
                        Ok(Step::Waiting) if progressed.elapsed() < STALL_TIMEOUT => {
                            thread::sleep(Duration::from_millis(1));
                        }
                        _ => {
                            worker_controls.error.store(true, Ordering::Release);
                            worker_controls.playing.store(false, Ordering::Release);
                            break;
                        }
                    }
                }
                drop(deadline);
                // Connection teardown never issues a blocking drain RPC.
                drop(connection);
            })?;
        Ok(Self {
            stop,
            finished,
            transport,
            desired_playing,
            worker: Some(worker),
            playback,
            _owner_thread: PhantomData,
            controls,
            device: "Scarlet Audio Server · 48 kHz stereo S16LE".into(),
        })
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        // The worker has dropped Engine/realtime proxies. Deactivate, destroy
        // and unload only now, on the original Audio::start caller thread.
    }
}

impl Ring for SasStream {
    fn closed(&self) -> bool {
        self.is_closed()
    }
    fn writable_frames(&self) -> usize {
        self.writable_frames()
    }
    fn write(&mut self, bytes: &[u8]) -> usize {
        self.write(bytes)
    }
    fn begin_drain(&mut self) {
        self.begin_drain();
    }
    fn is_empty(&self) -> bool {
        self.is_empty()
    }
    fn read_frames(&self) -> u64 {
        self.read_frames()
    }
}
