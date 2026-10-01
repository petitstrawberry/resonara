//! Musical display over the unchanged sample-based audio timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bars,
    Seconds,
    Samples,
}
impl Format {
    pub fn next(self) -> Self {
        match self {
            Self::Bars => Self::Seconds,
            Self::Seconds => Self::Samples,
            Self::Samples => Self::Bars,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Bars => "Bars / beats",
            Self::Seconds => "Time",
            Self::Samples => "Samples",
        }
    }
    pub fn caption(self) -> &'static str {
        match self {
            Self::Bars => "BAR · BEAT · TICK",
            Self::Seconds => "MIN · SEC",
            Self::Samples => "SAMPLE POSITION",
        }
    }
    pub fn position(self, seconds: f64, tempo: f64, rate: u32) -> String {
        match self {
            Self::Bars => {
                let ticks = (seconds.max(0.) * tempo / 60. * 960.).round() as u64;
                format!(
                    "{:03}.{:02}.{:03}",
                    ticks / 3840 + 1,
                    ticks / 960 % 4 + 1,
                    ticks % 960
                )
            }
            Self::Seconds => crate::ui::time(seconds),
            Self::Samples => format!("{}", (seconds.max(0.) * rate as f64).round() as u64),
        }
    }
    pub fn duration(self, seconds: f64, tempo: f64, rate: u32) -> String {
        match self {
            Self::Bars => format!("{:.2} beats", seconds * tempo / 60.),
            _ => self.position(seconds, tempo, rate),
        }
    }
    pub fn step(self, span: f64, tempo: f64, rate: u32) -> f64 {
        match self {
            Self::Bars => {
                let beats = (span * tempo / 60. / 8.).max(1. / 4.);
                2f64.powf(beats.log2().ceil()) * 60. / tempo
            }
            Self::Seconds => crate::wave::tick_step(span),
            Self::Samples => {
                crate::wave::tick_step(span * rate as f64).max(1.).ceil() / rate as f64
            }
        }
    }
    pub fn tick(self, seconds: f64, tempo: f64, rate: u32) -> String {
        match self {
            Self::Bars => self.position(seconds, tempo, rate),
            Self::Seconds => format!("{seconds:.2}s"),
            Self::Samples => self.position(seconds, tempo, rate),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn musical_positions_carry_at_beats_and_bars_and_scale_with_tempo() {
        assert_eq!(Format::Bars.position(0., 120., 48000), "001.01.000");
        assert_eq!(Format::Bars.position(0.5, 120., 48000), "001.02.000");
        assert_eq!(Format::Bars.position(2., 120., 48000), "002.01.000");
        assert_eq!(Format::Bars.position(2., 60., 48000), "001.03.000");
        assert_eq!(Format::Bars.position(1.99999999, 120., 48000), "002.01.000");
        assert_eq!(Format::Samples.position(1.5, 120., 44100), "66150");
        assert_eq!(Format::Seconds.position(61.5, 120., 48000), "01:01.500");
        for mode in [Format::Bars, Format::Seconds, Format::Samples] {
            for span in [0.001, 0.1, 5., 300., 86400.] {
                let step = mode.step(span, 123.5, 44100);
                assert!(step > 0. && span / step <= 8.000001);
            }
        }
    }
}
