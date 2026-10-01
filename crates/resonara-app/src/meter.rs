//! UI-only stereo peak display state. Audio publishes independent interval maxima.
//! Peak markers and overload indicators expire after one second, including at stop.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct StereoMeter {
    pub peak: [f32; 2],
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

    pub fn summary(self) -> String {
        format!("Peak {} dBFS", db(self.peak[0].max(self.peak[1])))
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
