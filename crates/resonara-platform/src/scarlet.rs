//! Scarlet Audio Server adapter. The worker exclusively owns the SAS mapping.
use crate::pcm::{CHANNELS, PERIOD_FRAMES, Pump, Ring, Step};
use resonara_core::{Controls, Engine, PluginOwner, Project, Result};
use sas_client::{SasClient, SasStream, StreamConfig};
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const SAMPLE_RATE: u32 = 48_000;
// Scarlet's native PCM ABI value; this adapter selects S16LE stereo.
const FORMAT_S16LE: u32 = 1;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const STALL_TIMEOUT: Duration = Duration::from_secs(3);

pub struct Audio {
    stop: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    plugin_owners: Vec<PluginOwner>,
    _owner_thread: PhantomData<Rc<()>>,
    pub controls: Arc<Controls>,
    pub device: String,
}

impl Audio {
    /// Completion means the final PCM has reached SAS. Keep this Audio alive
    /// afterward: SAS can still have samples queued at the output device.
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub fn start(project: &Project, start: u64) -> Result<Self> {
        Self::start_with_metronome(project, start, false)
    }

    pub fn start_with_metronome(project: &Project, start: u64, metronome: bool) -> Result<Self> {
        project.validate()?;
        let controls = Arc::new(Controls::new(project));
        controls.metronome.store(metronome, Ordering::Relaxed);
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
        let mut stream = client
            .configure_cancellable(&config, || started.elapsed() >= CONNECT_TIMEOUT)
            .map_err(|e| format!("Scarlet audio configuration: {}", e.as_str()))?
            .ok_or("Scarlet audio configuration timed out")?;
        // Resolve all fallible device setup before loading/activating plugins.
        // After guard extraction, only spawn can fail; its captured Engine is
        // dropped before the caller's guards if thread creation fails.
        let mut engine = Engine::try_new(project, controls.clone(), SAMPLE_RATE, start)?;
        let plugin_owners = engine.take_plugin_owners();
        let stop = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker_finished = finished.clone();
        let worker_controls = controls.clone();
        let worker = thread::Builder::new()
            .name("resonara-sas".into())
            .spawn(move || {
                let mut pump = Pump::new();
                let mut progressed = Instant::now();
                loop {
                    match pump.step(
                        &mut engine,
                        &mut stream,
                        worker_stop.load(Ordering::Acquire),
                    ) {
                        Ok(Step::Finished) => {
                            worker_finished.store(true, Ordering::Release);
                            // SAS reports client-ring consumption, not hardware
                            // playback. Closing its final client stops the device
                            // and discards that tail. The application retains this
                            // idle connection until the next explicit action.
                            while !worker_stop.load(Ordering::Acquire) {
                                thread::sleep(Duration::from_millis(10));
                            }
                            break;
                        }
                        Ok(Step::Cancelled) => break,
                        Ok(Step::Progress) => progressed = Instant::now(),
                        Ok(Step::Waiting) if progressed.elapsed() < STALL_TIMEOUT => {
                            thread::sleep(Duration::from_millis(1));
                        }
                        _ => {
                            worker_controls.error.store(true, Ordering::Relaxed);
                            worker_controls.playing.store(false, Ordering::Relaxed);
                            break;
                        }
                    }
                }
                // Stop/seek drops without a blocking drain/close RPC to SAS.
                drop(stream);
                drop(client);
            })?;
        Ok(Self {
            stop,
            finished,
            worker: Some(worker),
            plugin_owners,
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
        self.plugin_owners.clear();
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
