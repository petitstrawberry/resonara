//! Reusable vertical channel fader using ScarletUI's retained native paint/input API.
//! Spectrum-referenced piecewise gain and peak scales; the bottom gain stop is silence.
use crate::{meter::StereoMeter, ui};
use scarlet_ui::{
    element::{Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult},
    event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, Phase},
    prelude::*,
    renderer::PaintContext,
};
use std::{any::Any, rc::Rc};

#[derive(Clone)]
pub struct Fader {
    pub gain: State<f32>,
    pub peak: State<StereoMeter>,
    paint_peak: crate::animation::PaintState<StereoMeter>,
    pub dragging: State<bool>,
    pub focused: State<bool>,
    pub changed: Rc<dyn Fn(f32)>,
}
impl Fader {
    pub fn new(
        gain: State<f32>,
        peak: State<StereoMeter>,
        dragging: State<bool>,
        focused: State<bool>,
        changed: impl Fn(f32) + 'static,
    ) -> Self {
        Self {
            gain,
            paint_peak: crate::animation::PaintState(peak.clone()),
            peak,
            dragging,
            focused,
            changed: Rc::new(changed),
        }
    }
    fn handle_key(&self, event: KeyEvent) -> bool {
        let KeyEvent::Pressed { keycode, modifiers } = event else {
            return false;
        };
        let step = if modifiers.shift { 0.1 } else { 1. };
        let db = if self.gain.get() > 0. {
            20. * self.gain.get().log10()
        } else {
            -100.
        };
        let gain = match keycode {
            KeyCode::Up | KeyCode::Right => 10f32.powf(((db + step).min(6.)) / 20.),
            KeyCode::Down | KeyCode::Left => {
                let next = db - step;
                if next <= -100. {
                    0.
                } else {
                    10f32.powf(next / 20.)
                }
            }
            KeyCode::Home => 1.,
            KeyCode::End => 0.,
            _ => return false,
        };
        (self.changed)(gain.min(2.));
        true
    }
}
impl View for Fader {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(scarlet_ui::ComponentElement::new_with_builder(
            self.clone(),
            |s| {
                let key = s.clone();
                Box::new(
                    FaderTrack(s.clone())
                        .focusable(s.focused.clone())
                        .on_key(move |e| Fader::handle_key(&key, e)),
                )
            },
        ))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.gain, &self.dragging, &self.focused]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
#[derive(Clone)]
struct FaderTrack(Fader);
impl View for FaderTrack {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |s| FaderRender {
                control: s.0.clone(),
                size: Size::new(90., 112.),
                labels: scale_labels(),
                stereo_labels: ["L", "R"].map(|text| (text.to_string(), ink_center(text, 7.))),
                before: 1.,
                fixed_gain: None,
            },
            |r, s| {
                r.control = s.0.clone();
                UpdateResult::Updated
            },
            |_| vec![],
        ))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.0.paint_peak]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
struct FaderRender {
    control: Fader,
    size: Size,
    before: f32,
    fixed_gain: Option<f32>,
    labels: Vec<ScaleLabel>,
    stereo_labels: [(String, Point); 2],
}
// Reference: Spectrum MixerPanel.tsx at 965f8dbf328c3fb0c0bbc465cb84974cd0f6def9.
// Gain and signal level deliberately have distinct reference curves.
pub(crate) const GAIN_SCALE: &[(f32, f32)] = &[
    (-100., 0.),
    (-40., 0.082),
    (-30., 0.123),
    (-20., 0.209),
    (-15., 0.291),
    (-10., 0.399),
    (-6., 0.485),
    (-3., 0.612),
    (0., 0.743),
    (3., 0.869),
    (6., 1.),
];
pub(crate) const PEAK_SCALE: &[(f32, f32)] = &[
    (-60., 0.),
    (-50., 0.082),
    (-45., 0.157),
    (-40., 0.231),
    (-35., 0.302),
    (-30., 0.377),
    (-24., 0.466),
    (-21., 0.534),
    (-18., 0.601),
    (-15., 0.668),
    (-12., 0.735),
    (-9., 0.799),
    (-6., 0.866),
    (-3., 0.933),
    (0., 1.),
];
pub(crate) fn interpolate(value: f32, anchors: &[(f32, f32)]) -> f32 {
    if value <= anchors[0].0 {
        return anchors[0].1;
    }
    for pair in anchors.windows(2) {
        let [(a, x), (b, y)] = pair else {
            unreachable!()
        };
        if value <= *b {
            return x + (value - a) / (b - a) * (y - x);
        }
    }
    anchors.last().unwrap().1
}
pub(crate) fn gain_fraction(db: f32) -> f32 {
    interpolate(db, GAIN_SCALE)
}
pub(crate) fn peak_fraction(db: f32) -> f32 {
    interpolate(db, PEAK_SCALE)
}
pub(crate) fn gain_to_fraction(gain: f32) -> f32 {
    if gain <= 0. {
        0.
    } else {
        gain_fraction(20. * gain.log10())
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Geometry {
    pub axis: f32,
    pub top: f32,
    pub bottom: f32,
    pub gain_label: f32,
    pub meter_left: f32,
    pub peak_label: f32,
}
impl Geometry {
    pub fn new(size: Size) -> Self {
        let axis = size.width / 2.;
        Self {
            axis,
            top: 6.,
            bottom: (size.height - 6.).max(7.),
            gain_label: axis - 31.,
            meter_left: axis + 18.,
            peak_label: axis + 36.,
        }
    }
    pub fn y(self, fraction: f32) -> f32 {
        self.bottom - fraction.clamp(0., 1.) * (self.bottom - self.top)
    }
}
pub(crate) fn gain_at(y: f32, height: f32) -> f32 {
    let g = Geometry::new(Size::new(90., height));
    gain_from_fraction((g.bottom - y) / (g.bottom - g.top))
}
pub(crate) fn gain_from_fraction(f: f32) -> f32 {
    let f = f.clamp(0., 1.);
    if f <= 0. {
        return 0.;
    }
    for pair in GAIN_SCALE.windows(2) {
        let [(a, x), (b, y)] = pair else {
            unreachable!()
        };
        if f <= *y {
            let db = a + (f - x) / (y - x) * (b - a);
            return 10f32.powf(db / 20.);
        }
    }
    10f32.powf(6. / 20.)
}
/// Center the visible glyph ink, not the font's em box or advance width.
/// Cache each static label's offset once in the retained render object.
pub(crate) fn ink_center(text: &str, font: f32) -> Point {
    let glyphs = scarlet_ui::graphics::rasterize_text(text, font, 1000);
    let mut bounds: Option<(i32, i32, i32, i32)> = None;
    for glyph in glyphs {
        for (i, alpha) in glyph.mask.iter().enumerate() {
            if *alpha == 0 {
                continue;
            }
            let x = glyph.x + (i as u32 % glyph.width) as i32;
            let y = glyph.y + (i as u32 / glyph.width) as i32;
            bounds = Some(match bounds {
                Some((left, top, right, bottom)) => {
                    (left.min(x), top.min(y), right.max(x + 1), bottom.max(y + 1))
                }
                None => (x, y, x + 1, y + 1),
            });
        }
    }
    match bounds {
        Some((l, t, r, b)) => Point::new((l + r) as f32 / 2., (t + b) as f32 / 2.),
        None => {
            let (w, h) = scarlet_ui::graphics::measure_text_sized(text, font);
            Point::new(w as f32 / 2., h as f32 / 2.)
        }
    }
}
struct ScaleLabel {
    db: f32,
    text: String,
    ink: Point,
}
fn scale_labels() -> Vec<ScaleLabel> {
    [
        (6., "+6"),
        (0., "0"),
        (-6., "−6"),
        (-18., "−18"),
        (-48., "−48"),
        (-100., "−∞"),
        (-60., "−60"),
    ]
    .into_iter()
    .map(|(db, text)| ScaleLabel {
        db,
        text: text.into(),
        ink: ink_center(text, 9.),
    })
    .collect()
}
impl ElementRenderObject for FaderRender {
    fn layout(&mut self, c: LayoutConstraints) -> Size {
        self.size = Size::new(90., 112.).constrain(
            Size::new(c.min_width, c.min_height),
            Size::new(c.max_width, c.max_height),
        );
        self.size
    }
    fn size(&self) -> Size {
        self.size
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn render(&mut self) {}
    fn paint(&self, ctx: &mut PaintContext<'_>, origin: Point) -> bool {
        let h = self.size.height;
        let x = origin.x;
        let y = origin.y;
        let g = Geometry::new(self.size);
        ctx.fill_rect(
            Rect::from_xywh(x + g.axis - 3., y + g.top - 1., 6., g.bottom - g.top + 2.),
            ui::BG,
        );
        for label in &self.labels {
            if label.db == -60. {
                continue;
            }
            let yy = y + g.y(gain_fraction(label.db));
            ctx.draw_line(
                Point::new(x + g.axis - 18., yy),
                Point::new(x + g.axis - 12., yy),
                1.,
                if label.db == 0. { ui::TEXT } else { ui::LINE },
            );
            ctx.draw_text(
                Point::new(x + g.gain_label - label.ink.x, yy - label.ink.y),
                &label.text,
                ui::MUTED,
                9.,
            );
        }
        let yy = y + g.y(gain_to_fraction(self.control.gain.get()));
        ctx.fill_rounded_rect(
            Rect::from_xywh(x + g.axis - 12., yy - 5., 24., 10.),
            2.,
            ui::TEXT,
        );
        ctx.draw_line(
            Point::new(x + g.axis - 9., yy),
            Point::new(x + g.axis + 9., yy),
            1.,
            ui::BG,
        );
        let reading = self.control.peak.get();
        for (channel, peak) in reading.display.into_iter().enumerate() {
            let left = x + g.meter_left + channel as f32 * 5.;
            let amount = if peak > 0. {
                peak_fraction(20. * peak.log10())
            } else {
                0.
            };
            let meter_top = g.y(amount);
            ctx.fill_rect(
                Rect::from_xywh(left, y + g.top, 4., g.bottom - g.top),
                ui::BG,
            );
            if amount > 0. {
                ctx.fill_rect(
                    Rect::from_xywh(left, y + meter_top, 4., g.bottom - meter_top),
                    if peak >= 1. {
                        Color::rgb(0.98, 0.4, 0.4)
                    } else if peak > 0.7 {
                        ui::GOLD
                    } else {
                        ui::ACCENT
                    },
                );
            }
            if reading.held[channel] > 0. {
                let hold_y = y + g.y(peak_fraction(20. * reading.held[channel].log10()));
                ctx.fill_rect(Rect::from_xywh(left, hold_y, 4., 1.), ui::TEXT);
            }
            ctx.fill_rect(
                Rect::from_xywh(left, y + g.top - 4., 4., 2.),
                if reading.clip_seconds[channel] > 0. {
                    Color::rgb(0.98, 0.4, 0.4)
                } else {
                    ui::LINE
                },
            );
            let (label, center) = &self.stereo_labels[channel];
            ctx.draw_text(
                Point::new(left + 2. - center.x, y + g.bottom + 3. - center.y),
                label,
                ui::MUTED,
                7.,
            );
        }
        for label in &self.labels {
            if !matches!(label.db, 0. | -6. | -18. | -48. | -60.) {
                continue;
            }
            let yy = y + g.y(peak_fraction(label.db));
            ctx.draw_line(
                Point::new(x + g.meter_left - 2., yy),
                Point::new(x + g.meter_left, yy),
                1.,
                ui::LINE,
            );
            ctx.draw_text(
                Point::new(x + g.peak_label - label.ink.x, yy - label.ink.y),
                &label.text,
                ui::MUTED,
                9.,
            );
        }
        if self.control.focused.get() {
            ctx.stroke_rect(
                Rect::from_xywh(x + 1., y + 1., self.size.width - 2., h - 2.),
                1.,
                ui::ACCENT,
            );
        }
        true
    }
    fn handle_event(&mut self, e: &Event, phase: Phase) -> bool {
        if !matches!(phase, Phase::Target | Phase::Bubble) {
            return false;
        }
        let change = |y: f32| (self.control.changed)(gain_at(y, self.size.height));
        match e {
            Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                y,
                click_count,
                ..
            }) => {
                let g = Geometry::new(self.size);
                let tick = self.gain_tick_at(*x as f32, *y as f32);
                if tick.is_none() && (*x as f32 - g.axis).abs() > 12. {
                    return false;
                }
                self.before = self.control.gain.get();
                self.fixed_gain = tick.or_else(|| (*click_count >= 2).then_some(1.));
                self.control.dragging.set(true);
                self.control.focused.set(true);
                if let Some(gain) = self.fixed_gain {
                    (self.control.changed)(gain);
                } else {
                    change(*y as f32);
                }
                true
            }
            Event::Mouse(MouseEvent::Moved { y, .. }) if self.control.dragging.get() => {
                if self.fixed_gain.is_none() {
                    change(*y as f32);
                }
                true
            }
            Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                y,
                ..
            }) if self.control.dragging.get() => {
                if self.fixed_gain.is_none() {
                    change(*y as f32);
                }
                self.control.dragging.set(false);
                true
            }
            Event::Mouse(MouseEvent::ButtonCancelled { .. }) if self.control.dragging.get() => {
                (self.control.changed)(self.before);
                self.control.dragging.set(false);
                true
            }
            Event::Keyboard(k) => self.control.handle_key(*k),
            _ => false,
        }
    }
}
impl FaderRender {
    fn gain_tick_at(&self, x: f32, y: f32) -> Option<f32> {
        let g = Geometry::new(self.size);
        // The left label/tick column owns these targets. The thumb, meter and
        // neighboring strips cannot hit them. Close bottom ticks use the nearest
        // painted center so silence and -48 dB remain distinct at short heights.
        if !(0. ..g.axis - 12.).contains(&x) || !(0. ..self.size.height).contains(&y) {
            return None;
        }
        let (label, distance) = self
            .labels
            .iter()
            .filter(|label| label.db != -60.)
            .map(|label| (label, (y - g.y(gain_fraction(label.db))).abs()))
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        if distance > 8. {
            return None;
        }
        Some(if label.db == -100. {
            0.
        } else {
            10f32.powf(label.db / 20.)
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fader_mapping_has_unity_and_silence() {
        assert_eq!(gain_at(108., 108.), 0.);
        assert!(
            (gain_at(
                Geometry::new(Size::new(90., 108.)).y(gain_fraction(0.)),
                108.
            ) - 1.)
                .abs()
                < 0.0001
        );
        assert!(gain_at(0., 108.) <= 2.);
    }
}
