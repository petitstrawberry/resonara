//! UI-only stereo peak display state. Audio publishes independent interval maxima.
//! Peak markers and overload indicators expire after one second, including at stop.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct StereoMeter {
    /// Exact maxima from the last audio polling interval.
    pub peak: [f32; 2],
    /// Display envelope: immediate attack, 24 dB/s release.
    pub display: [f32; 2],
    pub held: [f32; 2],
    pub hold_seconds: [f32; 2],
    pub clip_seconds: [f32; 2],
}

impl StereoMeter {
    pub fn advance(&mut self, peak: [f32; 2], elapsed: f32) {
        let elapsed = if elapsed.is_finite() {
            elapsed.max(0.)
        } else {
            0.
        };
        for (channel, value) in peak.into_iter().enumerate() {
            let value = if value.is_finite() { value.abs() } else { 0. };
            self.peak[channel] = value;
            self.display[channel] = self.display[channel].max(value);
            self.hold_seconds[channel] = (self.hold_seconds[channel] - elapsed).max(0.);
            if value >= self.held[channel] || self.hold_seconds[channel] <= 0. {
                self.held[channel] = value;
                self.hold_seconds[channel] = if value > 0. { 1. } else { 0. };
            }
            self.clip_seconds[channel] = if value >= 1. {
                1.
            } else {
                (self.clip_seconds[channel] - elapsed).max(0.)
            };
        }
    }

    pub fn release(&mut self, elapsed: f32) {
        if !elapsed.is_finite() || elapsed <= 0. {
            return;
        }
        let attenuation = 10f32.powf(-24. * elapsed / 20.);
        for channel in 0..2 {
            let level = self.peak[channel].max(self.display[channel] * attenuation);
            self.display[channel] = if level < 0.001 {
                self.peak[channel]
            } else {
                level
            };
        }
    }

    pub fn summary(self) -> String {
        format!("Peak {} dBFS", db(self.held[0].max(self.held[1])))
    }

    pub fn detail(self, master: bool) -> String {
        format!(
            "L {} dBFS · R {} dBFS · {} · peak/clip hold 1 s",
            db(self.peak[0]),
            db(self.peak[1]),
            if master {
                "post-master gain, pre-output clip"
            } else {
                "post-pan, post-fader"
            },
        )
    }
}

fn db(value: f32) -> String {
    if value <= 0.00001 {
        "−∞".into()
    } else if value >= 1. {
        format!("{:+.1}", 20. * value.log10())
    } else {
        format!("{:.1}", 20. * value.log10())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_release_is_time_based_without_smoothing_measured_peaks() {
        let mut meter = StereoMeter::default();
        meter.advance([1.25, 0.25], 0.05);
        assert_eq!(meter.display, [1.25, 0.25]);
        meter.advance([0.01, 0.], 0.05);
        assert_eq!(meter.peak, [0.01, 0.]);
        assert_eq!(meter.summary(), "Peak +1.9 dBFS");
        let mut one_step = meter;
        one_step.release(0.5);
        for _ in 0..30 {
            meter.release(0.5 / 30.);
        }
        for channel in 0..2 {
            assert!((one_step.display[channel] - meter.display[channel]).abs() < 1e-6);
        }
        assert!((20. * (meter.display[0] / 1.25).log10() + 12.).abs() < 1e-4);
        assert_eq!(meter.peak, [0.01, 0.]);
        assert_eq!(meter.held, [1.25, 0.25]);
        assert!(meter.clip_seconds[0] > 0.);
        meter.advance([2., 0.5], 0.05);
        assert_eq!(meter.display, [2., 0.5]);
        assert_eq!(meter.held, [2., 0.5]);
    }
}
