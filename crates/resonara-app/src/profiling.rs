//! Opt-in, bounded native submission timing. These are not GPU/scanout timestamps.
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
pub(crate) const CAP: usize = 4096;
pub(crate) const WINDOW: Duration = Duration::from_secs(15);
#[derive(Default)]
pub(crate) struct Profiler {
    pub(crate) path: Option<PathBuf>,
    pub(crate) start: Option<Instant>,
    pub(crate) sync: Option<Instant>,
    pub(crate) previous: Option<Instant>,
    pub(crate) intervals: Vec<f64>,
    pub(crate) submissions: Vec<f64>,
    pub(crate) frames: usize,
    pub(crate) failures: usize,
    pub body_builds: usize,
    pub waveform_refreshes: usize,
    pub playhead_updates: usize,
    pub meter_updates: usize,
    run: usize,
}
#[derive(Debug, Default)]
pub(crate) struct Stats {
    pub count: usize,
    pub median: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
    pub over_budget: usize,
    pub over_33_ms: usize,
    pub over_50_ms: usize,
}
pub(crate) fn stats(samples: &[f64]) -> Stats {
    if samples.is_empty() {
        return Stats::default();
    }
    let mut v = samples.to_vec();
    v.sort_by(f64::total_cmp);
    let at = |q: f64| v[((v.len() - 1) as f64 * q).ceil() as usize];
    Stats {
        count: v.len(),
        median: at(0.5),
        p95: at(0.95),
        p99: at(0.99),
        max: *v.last().unwrap(),
        over_budget: v.iter().filter(|x| **x > 1000. / 60.).count(),
        over_33_ms: v.iter().filter(|x| **x > 1000. / 30.).count(),
        over_50_ms: v.iter().filter(|x| **x > 50.).count(),
    }
}
impl Stats {
    fn json(&self) -> String {
        format!(
            "{{\"count\":{},\"median_ms\":{},\"p95_ms\":{},\"p99_ms\":{},\"max_ms\":{},\"over_16_667_ms\":{},\"over_33_333_ms\":{},\"over_50_ms\":{}}}",
            self.count,
            self.median,
            self.p95,
            self.p99,
            self.max,
            self.over_budget,
            self.over_33_ms,
            self.over_50_ms
        )
    }
}
impl Profiler {
    pub fn from_env() -> Self {
        Self {
            path: std::env::var_os("RESONARA_UI_PROFILE").map(PathBuf::from),
            ..Self::default()
        }
    }
    pub fn enabled(&self) -> bool {
        self.path.is_some()
    }
    pub fn active(&self) -> bool {
        self.start.is_some()
    }
    pub fn begin(&mut self) {
        self.begin_at(Instant::now());
    }
    pub fn begin_at(&mut self, now: Instant) {
        if !self.enabled() {
            return;
        }
        self.start = Some(now);
        self.sync = None;
        self.previous = None;
        self.intervals = Vec::with_capacity(CAP);
        self.submissions = Vec::with_capacity(CAP);
        self.frames = 0;
        self.failures = 0;
        self.body_builds = 0;
        self.waveform_refreshes = 0;
        self.playhead_updates = 0;
        self.meter_updates = 0;
    }
    pub fn sync(&mut self) {
        self.sync_at(Instant::now());
    }
    pub fn sync_at(&mut self, now: Instant) {
        if self.active() {
            self.sync = Some(now);
        }
    }
    pub fn presented(&mut self) {
        self.presented_at(Instant::now());
    }
    pub fn presented_at(&mut self, now: Instant) {
        if !self.active() {
            return;
        }
        self.frames += 1;
        if let Some(previous) = self.previous.replace(now) {
            if self.intervals.len() < CAP {
                self.intervals
                    .push(now.duration_since(previous).as_secs_f64() * 1000.);
            }
        }
        if let Some(sync) = self.sync.take() {
            if self.submissions.len() < CAP {
                self.submissions
                    .push(now.duration_since(sync).as_secs_f64() * 1000.);
            }
        }
    }
    pub fn failure(&mut self) {
        if self.active() {
            self.failures += 1;
        }
    }
    pub fn expired(&self) -> bool {
        self.expired_at(Instant::now())
    }
    pub fn expired_at(&self, now: Instant) -> bool {
        self.start
            .is_some_and(|s| now.duration_since(s) >= WINDOW || self.frames >= CAP)
    }
    pub fn finish(
        &mut self,
        mode: &str,
        tracks: usize,
        width: f32,
        height: f32,
    ) -> Option<std::io::Result<PathBuf>> {
        let start = self.start.take()?;
        let base = self.path.as_ref()?;
        self.run += 1;
        let path = base.with_extension(format!("{}.json", self.run));
        let report = format!(
            "{{\n\"measurement\":\"native accepted submission, not GPU completion or scanout\",\n\"sync_span_excludes\":\"input dispatch and on_idle before on_window_sync\",\n\"mode\":\"{}\",\"duration_seconds\":{},\"tracks\":{},\"window_width\":{},\"window_height\":{},\n\"accepted_frames\":{},\"render_failures\":{},\"sample_cap\":{},\n\"accepted_interval\":{},\n\"sync_to_accepted\":{},\n\"body_builds\":{},\"waveform_refreshes\":{},\"playhead_updates\":{},\"meter_updates\":{}\n}}\n",
            mode,
            start.elapsed().as_secs_f64(),
            tracks,
            width,
            height,
            self.frames,
            self.failures,
            CAP,
            stats(&self.intervals).json(),
            stats(&self.submissions).json(),
            self.body_builds,
            self.waveform_refreshes,
            self.playhead_updates,
            self.meter_updates
        );
        Some(std::fs::write(&path, report).map(|_| path))
    }
}
struct AdapterLogger;
static LOGGER: AdapterLogger = AdapterLogger;
impl log::Log for AdapterLogger {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.target() == "wgpu_core::instance" && m.level() <= log::Level::Debug
    }
    fn log(&self, r: &log::Record<'_>) {
        if self.enabled(r.metadata()) {
            let s = r.args().to_string();
            if s.starts_with("Request adapter result") {
                eprintln!("[Resonara UI profile] {s}");
            }
        }
    }
    fn flush(&self) {}
}
pub fn install_adapter_log() {
    if std::env::var_os("RESONARA_UI_PROFILE").is_some() && log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }
}
