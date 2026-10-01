//! Musical display over the unchanged sample-based audio timeline.
use resonara_core::TimeSignature;
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
    pub fn position(self, seconds: f64, tempo: f64, rate: u32, meter: TimeSignature) -> String {
        match self {
            Self::Bars => {
                let ticks = (seconds.max(0.) * tempo / 60. * 960.).round() as u64;
                format!(
                    "{:03}.{:02}.{:03}",
                    ticks / (meter.ticks_per_beat() * meter.numerator as u64) + 1,
                    ticks / meter.ticks_per_beat() % meter.numerator as u64 + 1,
                    ticks % meter.ticks_per_beat()
                )
            }
            Self::Seconds => crate::ui::time(seconds),
            Self::Samples => format!("{}", (seconds.max(0.) * rate as f64).round() as u64),
        }
    }
    pub fn duration(self, seconds: f64, tempo: f64, rate: u32, meter: TimeSignature) -> String {
        match self {
            Self::Bars => format!("{:.2} beats", seconds / meter.beat_seconds(tempo)),
            _ => self.position(seconds, tempo, rate, meter),
        }
    }
    pub fn step(self, span: f64, tempo: f64, rate: u32, meter: TimeSignature) -> f64 {
        match self {
            Self::Bars => {
                let beats = (span / meter.beat_seconds(tempo) / 8.).max(1. / 4.);
                let step = if beats >= meter.numerator as f64 {
                    2f64.powf((beats / meter.numerator as f64).log2().ceil())
                        * meter.numerator as f64
                } else {
                    2f64.powf(beats.log2().ceil())
                };
                step * meter.beat_seconds(tempo)
            }
            Self::Seconds => crate::wave::tick_step(span),
            Self::Samples => {
                crate::wave::tick_step(span * rate as f64).max(1.).ceil() / rate as f64
            }
        }
    }
    pub fn tick(self, seconds: f64, tempo: f64, rate: u32, meter: TimeSignature) -> String {
        match self {
            Self::Bars => self.position(seconds, tempo, rate, meter),
            Self::Seconds => format!("{seconds:.2}s"),
            Self::Samples => self.position(seconds, tempo, rate, meter),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn musical_positions_carry_at_beats_and_bars_and_scale_with_tempo() {
        assert_eq!(
            Format::Bars.position(0., 120., 48000, TimeSignature::default()),
            "001.01.000"
        );
        assert_eq!(
            Format::Bars.position(0.5, 120., 48000, TimeSignature::default()),
            "001.02.000"
        );
        assert_eq!(
            Format::Bars.position(2., 120., 48000, TimeSignature::default()),
            "002.01.000"
        );
        assert_eq!(
            Format::Bars.position(2., 60., 48000, TimeSignature::default()),
            "001.03.000"
        );
        assert_eq!(
            Format::Bars.position(1.99999999, 120., 48000, TimeSignature::default()),
            "002.01.000"
        );
        assert_eq!(
            Format::Samples.position(1.5, 120., 44100, TimeSignature::default()),
            "66150"
        );
        assert_eq!(
            Format::Seconds.position(61.5, 120., 48000, TimeSignature::default()),
            "01:01.500"
        );
        for mode in [Format::Bars, Format::Seconds, Format::Samples] {
            for span in [0.001, 0.1, 5., 300., 86400.] {
                let step = mode.step(span, 123.5, 44100, TimeSignature::default());
                assert!(step > 0. && span / step <= 8.000001);
            }
        }
    }
}

#[cfg(test)]
mod meter_tests {
    use super::*;
    #[test]
    fn selectable_meters_carry_and_grid_aligns_to_odd_bars() {
        for (n, d, beat) in [
            (3, 4, 0.5),
            (6, 8, 0.25),
            (7, 8, 0.25),
            (5, 4, 0.5),
            (1, 32, 0.0625),
        ] {
            let meter = TimeSignature {
                numerator: n,
                denominator: d,
            };
            assert_eq!(
                Format::Bars.position(beat * n as f64, 120., 48000, meter),
                "002.01.000"
            );
            assert_eq!(
                Format::Bars.position(beat * (n - 1) as f64, 120., 48000, meter),
                format!("001.{n:02}.000")
            );
            assert_eq!(
                Format::Bars.position(beat / 2., 120., 48000, meter),
                format!("001.01.{:03}", meter.ticks_per_beat() / 2)
            );
            let bar = beat * n as f64;
            let step = Format::Bars.step(bar * 40., 120., 48000, meter);
            assert!((step / bar - (step / bar).round()).abs() < 1e-8);
            assert!(bar * 40. / step <= 8.);
            assert_eq!(
                Format::Bars.duration(bar, 120., 48000, meter),
                format!("{:.2} beats", n as f64)
            );
            assert_eq!(Format::Samples.position(1., 120., 48000, meter), "48000");
        }
    }
}
