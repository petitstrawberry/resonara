//! Prepared monitoring clicks; the callback only looks up a sample by transport position.
use crate::Project;
pub(crate) struct Metronome {
    period: f64,
    step: f64,
    beats: u64,
    regular: Vec<f32>,
    accent: Vec<f32>,
}
impl Metronome {
    pub fn new(project: &Project, device_rate: u32) -> Self {
        let len = (device_rate as f64 * 0.035).ceil() as usize;
        let tone = |frequency: f64, gain: f64| {
            (0..len)
                .map(|i| {
                    let time = i as f64 / device_rate as f64;
                    let attack = (time / 0.001).min(1.);
                    let release = (-time / 0.007).exp() * (1. - i as f64 / len as f64);
                    (gain * attack * release * (std::f64::consts::TAU * frequency * time).sin())
                        as f32
                })
                .collect()
        };
        Self {
            period: project.sample_rate as f64 * project.time_signature.beat_seconds(project.tempo),
            step: project.sample_rate as f64 / device_rate as f64,
            beats: u64::from(project.time_signature.numerator),
            regular: tone(1000., 0.16),
            accent: tone(1600., 0.24),
        }
    }
    pub fn sample(&self, position: f64) -> f32 {
        let beat = (position / self.period).floor();
        let offset = ((position - beat * self.period) / self.step).round() as usize;
        let wave = if (beat as u64).is_multiple_of(self.beats) {
            &self.accent
        } else {
            &self.regular
        };
        wave.get(offset).copied().unwrap_or(0.)
    }
}
