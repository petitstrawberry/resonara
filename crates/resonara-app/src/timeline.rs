//! Musical display over the unchanged sample-based audio timeline.
use resonara_core::TimeSignature;
/// Keep sample-accurate transport positions, while retaining compact millisecond values.
pub(crate) fn seconds_input(seconds: f64) -> String {
    let mut value = format!("{seconds:.9}");
    let minimum = value.find('.').unwrap() + 4;
    while value.len() > minimum && value.ends_with('0') {
        value.pop();
    }
    value
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bars,
    Seconds,
    Samples,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TickKind {
    Major,
    Bar,
    Quarter,
    Eighth,
    Sixteenth,
}
impl TickKind {
    pub fn length(self) -> f32 {
        match self {
            Self::Major | Self::Bar => 14.,
            Self::Quarter => 11.,
            Self::Eighth => 7.,
            Self::Sixteenth => 4.,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct RulerTick {
    pub seconds: f64,
    pub kind: TickKind,
}
pub(crate) struct RulerGrid {
    pub label_step: f64,
    pub ticks: Vec<RulerTick>,
}
pub(crate) struct SnapGrid {
    pub step: f64,
    pub bar: Option<f64>,
    pub caption: String,
}
impl SnapGrid {
    pub fn position(&self, seconds: f64) -> f64 {
        let seconds = seconds.max(0.);
        if let Some(bar) = self.bar {
            let start = (seconds / bar).floor() * bar;
            let offset = seconds - start;
            let nearest = (offset / self.step).round() * self.step;
            let snapped = if bar - offset < (nearest - offset).abs() {
                bar
            } else {
                nearest.min(bar)
            };
            start + snapped
        } else {
            (seconds / self.step).round() * self.step
        }
    }
}
pub(crate) fn snap_grid(
    format: Format,
    span: f64,
    width: f32,
    tempo: f64,
    rate: u32,
    meter: TimeSignature,
) -> SnapGrid {
    match format {
        Format::Seconds => SnapGrid {
            step: 0.1,
            bar: None,
            caption: "100 ms".into(),
        },
        Format::Samples => SnapGrid {
            step: 1. / rate as f64,
            bar: None,
            caption: "1 sample".into(),
        },
        Format::Bars => {
            let quarter = 60. / tempo;
            let bar = meter.beat_seconds(tempo) * f64::from(meter.numerator);
            for (step, caption) in [
                (quarter / 4., "1/16"),
                (quarter / 2., "1/8"),
                (quarter, "1/4"),
            ] {
                if step <= bar && step / span * width as f64 >= 12. {
                    return SnapGrid {
                        step,
                        bar: Some(bar),
                        caption: caption.into(),
                    };
                }
            }
            let mut bars = 1u64;
            while bar * bars as f64 / span * (width as f64) < 12. && bars < (1 << 32) {
                bars *= 2;
            }
            SnapGrid {
                step: bar * bars as f64,
                bar: None,
                caption: if bars == 1 {
                    "bar".into()
                } else {
                    format!("{bars} bars")
                },
            }
        }
    }
}
pub(crate) fn ruler_grid(
    format: Format,
    start: f64,
    span: f64,
    width: f32,
    tempo: f64,
    rate: u32,
    meter: TimeSignature,
) -> RulerGrid {
    if !start.is_finite()
        || !span.is_finite()
        || span <= 0.
        || !width.is_finite()
        || width <= 0.
        || !tempo.is_finite()
        || tempo <= 0.
        || !meter.valid()
        || rate == 0
    {
        return RulerGrid {
            label_step: 1.,
            ticks: vec![],
        };
    }
    let quarter = 60. / tempo;
    let mut label_step = format.step(span, tempo, rate, meter);
    if format == Format::Bars {
        label_step = label_step.max(quarter / 4.);
    }
    while label_step / span * (width as f64) < 100. {
        label_step *= 2.;
    }
    let mut ticks = Vec::new();
    let visible = |seconds: f64| seconds >= start && seconds < start + span;
    if format == Format::Bars {
        let bar_ticks = meter.ticks_per_beat() * u64::from(meter.numerator);
        let bar = meter.beat_seconds(tempo) * f64::from(meter.numerator);
        if bar / span * width as f64 >= 12. {
            let unit = [240u64, 480, 960]
                .into_iter()
                .find(|unit| (*unit as f64 / 960. * quarter) / span * width as f64 >= 12.)
                .unwrap_or(bar_ticks);
            let first_bar = (start / bar).floor().max(0.) as u64;
            let bar_count = (span / bar).ceil().min(4096.) as u64 + 1;
            for index in first_bar..=first_bar.saturating_add(bar_count) {
                for offset in (0..bar_ticks).step_by(unit as usize) {
                    let seconds = index as f64 * bar + offset as f64 / 960. * quarter;
                    if visible(seconds) {
                        let kind = if offset == 0 {
                            TickKind::Bar
                        } else if offset.is_multiple_of(960) {
                            TickKind::Quarter
                        } else if offset.is_multiple_of(480) {
                            TickKind::Eighth
                        } else {
                            TickKind::Sixteenth
                        };
                        ticks.push(RulerTick { seconds, kind });
                    }
                    if ticks.len() >= 4096 {
                        break;
                    }
                }
                if ticks.len() >= 4096 {
                    break;
                }
            }
        }
    }
    let first = (start / label_step).ceil().max(0.) as u64;
    let count = (span / label_step).ceil().min(4096.) as u64;
    for index in first..=first.saturating_add(count) {
        let seconds = index as f64 * label_step;
        if visible(seconds)
            && !ticks
                .iter()
                .any(|t| (t.seconds - seconds).abs() < span / width as f64 * 0.5)
        {
            ticks.push(RulerTick {
                seconds,
                kind: TickKind::Major,
            });
        }
    }
    ticks.sort_by(|a, b| a.seconds.total_cmp(&b.seconds));
    RulerGrid { label_step, ticks }
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
