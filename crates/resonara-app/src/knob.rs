//! Compact, retained native mixer pan control. Positive values pan right.
//! Drag upward to increase pan; Shift+arrow keys provide fine adjustment.
use crate::ui;
use scarlet_ui::{
    buffer::Buffer,
    element::{Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult},
    event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, Phase},
    prelude::*,
    renderer::PaintContext,
};
use std::{any::Any, cell::RefCell, f32::consts::PI, rc::Rc, sync::Arc};

const DIAMETER: f32 = 32.;
const DRAG_TRAVEL: f32 = 120.;
const SWEEP: f32 = 3. * PI / 4.;

#[derive(Clone)]
pub struct PanKnob {
    pub value: State<f32>,
    pub dragging: State<bool>,
    pub focused: State<bool>,
    pub changed: Rc<dyn Fn(f32)>,
}

impl PanKnob {
    pub fn new(
        value: State<f32>,
        dragging: State<bool>,
        focused: State<bool>,
        changed: impl Fn(f32) + 'static,
    ) -> Self {
        Self {
            value,
            dragging,
            focused,
            changed: Rc::new(changed),
        }
    }

    fn handle_key(&self, event: KeyEvent) -> bool {
        let KeyEvent::Pressed { keycode, modifiers } = event else {
            return false;
        };
        let step = if modifiers.shift { 0.002 } else { 0.02 };
        let next = match keycode {
            KeyCode::Up | KeyCode::Right => bounded(self.value.get()) + step,
            KeyCode::Down | KeyCode::Left => bounded(self.value.get()) - step,
            KeyCode::Home => 0.,
            _ => return false,
        };
        (self.changed)(bounded(next));
        true
    }
}

impl View for PanKnob {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(scarlet_ui::ComponentElement::new_with_builder(
            self.clone(),
            |s| {
                let key = s.clone();
                Box::new(
                    KnobFace(s.clone())
                        .focusable(s.focused.clone())
                        .on_key(move |e| PanKnob::handle_key(&key, e)),
                )
            },
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.value, &self.dragging, &self.focused]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
struct KnobFace(PanKnob);

impl View for KnobFace {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |s| KnobRender {
                control: s.0.clone(),
                size: Size::new(DIAMETER, DIAMETER),
                before: 0.,
                start_y: 0.,
                reset: false,
                raster: RefCell::new(None),
            },
            |r, s| {
                // Keep the gesture origin when model changes rebuild the view.
                r.control = s.0.clone();
                UpdateResult::Updated
            },
            |_| vec![],
        ))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct KnobRender {
    control: PanKnob,
    size: Size,
    before: f32,
    start_y: f32,
    reset: bool,
    raster: RefCell<Option<KnobRaster>>,
}

fn bounded(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-1., 1.)
    } else {
        0.
    }
}

fn drag_value(before: f32, start_y: f32, y: f32) -> f32 {
    bounded(before + (start_y - y) * 2. / DRAG_TRAVEL)
}

fn dial_point(center: Point, radius: f32, value: f32) -> Point {
    let angle = bounded(value) * SWEEP;
    Point::new(
        center.x + angle.sin() * radius,
        center.y - angle.cos() * radius,
    )
}

// The SGFX backend closes multi-point StrokePaths and emits un-antialiased
// segments. Rasterize only this tiny control with analytic edge coverage,
// retaining the native Buffer between paints. This uses the regular, recyclable
// texture path rather than consuming a retained SGFX canvas target per knob.
#[derive(Clone, Copy, PartialEq)]
struct RasterKey {
    width: u32,
    height: u32,
    scale_milli: u32,
    value: f32,
    dragging: bool,
    focused: bool,
}

struct KnobRaster {
    key: RasterKey,
    buffer: Arc<Buffer>,
}

fn coverage(distance: f32, pixel_width: f32) -> f32 {
    (0.5 - distance / pixel_width).clamp(0., 1.)
}

fn segment_distance(point: Point, from: Point, to: Point) -> f32 {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared > f32::EPSILON {
        ((point.x - from.x) * dx + (point.y - from.y) * dy) / length_squared
    } else {
        0.
    }
    .clamp(0., 1.);
    (point.x - from.x - t * dx).hypot(point.y - from.y - t * dy)
}

fn arc_distance(point: Point, radius: f32, from: f32, to: f32) -> f32 {
    let start = from.min(to) * SWEEP;
    let end = from.max(to) * SWEEP;
    let angle = point.x.atan2(-point.y).clamp(start, end);
    (point.x - radius * angle.sin()).hypot(point.y + radius * angle.cos())
}

fn composite(pixel: &mut [f32; 4], color: Color, amount: f32) {
    let alpha = color.a * amount;
    for (index, channel) in [color.r, color.g, color.b].into_iter().enumerate() {
        pixel[index] = channel * alpha + pixel[index] * (1. - alpha);
    }
    pixel[3] = alpha + pixel[3] * (1. - alpha);
}

fn rasterize(buffer: &mut Buffer, key: RasterKey) {
    let scale = key.scale_milli as f32 / 1000.;
    let pixel_width = 1. / scale;
    let width = buffer.width();
    let height = buffer.height();
    let center = Point::new(key.width as f32 / 2., key.height as f32 / 2.);
    let radius = (center.x.min(center.y) - 3.).max(0.);
    let zero = Point::new(0., 0.);
    let pointer_start = dial_point(zero, radius * 0.3, key.value);
    let pointer_end = dial_point(zero, radius * 0.7, key.value);
    let ticks = [-1., 0., 1.].map(|position| {
        (
            dial_point(zero, radius + 1., position),
            dial_point(zero, radius + 2., position),
        )
    });
    let pointer_color = if key.dragging { ui::ACCENT } else { ui::TEXT };
    let pixels = buffer.as_mut_slice();
    for y in 0..height {
        for x in 0..width {
            let point = Point::new(
                (x as f32 + 0.5) / scale - center.x,
                (y as f32 + 0.5) / scale - center.y,
            );
            // Store a straight-alpha buffer, but composite layers in
            // premultiplied space to keep transparent edge pixels halo-free.
            let mut pixel = [0.; 4];
            composite(
                &mut pixel,
                ui::LINE,
                coverage(arc_distance(point, radius, -1., 1.) - 0.75, pixel_width),
            );
            if key.value.abs() > f32::EPSILON {
                composite(
                    &mut pixel,
                    ui::ACCENT,
                    coverage(
                        arc_distance(point, radius, 0., key.value) - 0.75,
                        pixel_width,
                    ),
                );
            }
            for (from, to) in ticks {
                composite(
                    &mut pixel,
                    ui::MUTED,
                    coverage(segment_distance(point, from, to) - 0.5, pixel_width),
                );
            }
            let distance = point.x.hypot(point.y);
            composite(
                &mut pixel,
                ui::LINE,
                coverage(distance - (radius - 2.).max(0.), pixel_width),
            );
            composite(
                &mut pixel,
                ui::RAISED,
                coverage(distance - (radius - 3.).max(0.), pixel_width),
            );
            composite(
                &mut pixel,
                pointer_color,
                coverage(
                    segment_distance(point, pointer_start, pointer_end) - 1.,
                    pixel_width,
                ),
            );
            if key.focused {
                let corner = 5f32.min(center.x).min(center.y);
                let qx = point.x.abs() - (center.x - 0.5 - corner);
                let qy = point.y.abs() - (center.y - 0.5 - corner);
                let distance = qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - corner;
                composite(
                    &mut pixel,
                    ui::ACCENT,
                    coverage(distance.abs() - 0.5, pixel_width),
                );
            }
            pixels[(y * width + x) as usize] = if pixel[3] > 0. {
                Color::rgba_f32(
                    pixel[0] / pixel[3],
                    pixel[1] / pixel[3],
                    pixel[2] / pixel[3],
                    pixel[3],
                )
                .to_bgra()
            } else {
                0
            };
        }
    }
}

impl KnobRender {
    fn raster(&self, scale_milli: u32) -> Arc<Buffer> {
        let key = RasterKey {
            width: self.size.width.ceil().max(1.) as u32,
            height: self.size.height.ceil().max(1.) as u32,
            scale_milli: scale_milli.max(1),
            value: bounded(self.control.value.get()),
            dragging: self.control.dragging.get(),
            focused: self.control.focused.get(),
        };
        let mut cache = self.raster.borrow_mut();
        let entry = cache.get_or_insert_with(|| KnobRaster {
            key,
            buffer: {
                let mut buffer = Buffer::from_logical_dimensions_with_scale(
                    key.width,
                    key.height,
                    key.scale_milli,
                );
                rasterize(&mut buffer, key);
                Arc::new(buffer)
            },
        });
        if entry.key != key {
            if entry.key.width != key.width
                || entry.key.height != key.height
                || entry.key.scale_milli != key.scale_milli
            {
                entry.buffer = Arc::new(Buffer::from_logical_dimensions_with_scale(
                    key.width,
                    key.height,
                    key.scale_milli,
                ));
            }
            // Preserve identity where the previous paint snapshot has been
            // released; copy-on-write keeps any still-live snapshot immutable.
            rasterize(Arc::make_mut(&mut entry.buffer), key);
            entry.key = key;
        }
        entry.buffer.clone()
    }
}

impl ElementRenderObject for KnobRender {
    fn layout(&mut self, c: LayoutConstraints) -> Size {
        self.size = Size::new(DIAMETER, DIAMETER).constrain(
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
        if self.size.width <= 0. || self.size.height <= 0. {
            return true;
        }
        let buffer = self.raster(scarlet_ui::graphics::current_scale_milli());
        // Both backend implementations scale the source rect from logical
        // coordinates; the Buffer itself already owns physical-resolution pixels.
        let source = Rect::from_xywh(
            0.,
            0.,
            buffer.logical_width() as f32,
            buffer.logical_height() as f32,
        );
        ctx.draw_buffer_rect_shared(Rect::new(origin, self.size), source, buffer, 1.);
        true
    }

    fn handle_event(&mut self, event: &Event, phase: Phase) -> bool {
        if !matches!(phase, Phase::Target | Phase::Bubble) {
            return false;
        }
        match event {
            Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                y,
                click_count,
                ..
            }) => {
                self.before = bounded(self.control.value.get());
                self.start_y = *y as f32;
                self.reset = *click_count >= 2;
                self.control.dragging.set(true);
                self.control.focused.set(true);
                // A single press never jumps to an absolute angle or value.
                if self.reset {
                    (self.control.changed)(0.);
                }
                true
            }
            Event::Mouse(MouseEvent::Moved { y, .. }) if self.control.dragging.get() => {
                if !self.reset {
                    (self.control.changed)(drag_value(self.before, self.start_y, *y as f32));
                }
                true
            }
            Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                y,
                ..
            }) if self.control.dragging.get() => {
                if !self.reset {
                    (self.control.changed)(drag_value(self.before, self.start_y, *y as f32));
                }
                self.control.dragging.set(false);
                true
            }
            Event::Mouse(MouseEvent::ButtonCancelled {
                button: MouseButton::Left,
                ..
            }) if self.control.dragging.get() => {
                (self.control.changed)(self.before);
                self.control.dragging.set(false);
                true
            }
            Event::Keyboard(key) => self.control.handle_key(*key),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarlet_ui::{ElementTree, EventDispatcher, event::KeyModifiers};

    fn control() -> (PanKnob, State<f32>, State<bool>, State<bool>) {
        let value = State::new(StateId::new(701), 0.4);
        let dragging = State::new(StateId::new(702), false);
        let focused = State::new(StateId::new(703), false);
        let output = value.clone();
        (
            PanKnob::new(value.clone(), dragging.clone(), focused.clone(), move |v| {
                output.set(v)
            }),
            value,
            dragging,
            focused,
        )
    }

    fn render_control(control: PanKnob) -> KnobRender {
        KnobRender {
            control,
            size: Size::new(DIAMETER, DIAMETER),
            before: 0.,
            start_y: 0.,
            reset: false,
            raster: RefCell::new(None),
        }
    }

    #[test]
    fn knob_raster_has_antialiased_edges_at_one_and_two_times_dpi() {
        let (knob, value, _, _) = control();
        let render = render_control(knob);
        value.set(0.);
        for scale in [1000, 2000] {
            let buffer = render.raster(scale);
            let dimension = 32 * scale / 1000;
            assert_eq!((buffer.width(), buffer.height()), (dimension, dimension));
            assert_eq!(buffer.scale_milli(), scale);
            let alpha = |x: u32, y: u32| buffer.as_slice()[(y * dimension + x) as usize] >> 24;
            assert_eq!(alpha(dimension / 2, dimension / 2), 255);
            assert_eq!(alpha(0, 0), 0);
            assert_eq!(
                alpha(dimension / 2, 29 * scale / 1000),
                0,
                "The open bottom of the arc must not acquire a closing chord"
            );
            let partial_edges = buffer
                .as_slice()
                .iter()
                .filter(|pixel| (1..255).contains(&(*pixel >> 24)))
                .count();
            assert!(
                partial_edges > 40 * (scale / 1000) as usize,
                "Curved edges must contain coverage pixels at {scale} milli-scale"
            );
        }
        assert!(arc_distance(Point::new(0., 13.), 13., -1., 1.) > 9.);
    }

    #[test]
    fn knob_raster_cache_reuses_identity_and_keeps_live_snapshots_immutable() {
        let (knob, value, _, _) = control();
        let render = render_control(knob);
        let first = render.raster(1000);
        let identity = first.identity();
        let revision = first.revision();
        let same = render.raster(1000);
        assert!(Arc::ptr_eq(&first, &same));
        assert_eq!(same.revision(), revision);
        drop(same);
        drop(first);

        value.set(-0.8);
        let changed = render.raster(1000);
        assert_eq!(changed.identity(), identity);
        assert!(changed.revision() > revision);
        let snapshot = changed.as_slice().to_vec();
        value.set(0.8);
        let next = render.raster(1000);
        assert_ne!(next.identity(), changed.identity());
        assert_eq!(changed.as_slice(), snapshot);
        assert_ne!(next.as_slice(), snapshot);

        let mut paint = PaintContext::new();
        render.paint(&mut paint, Point::new(0., 0.));
        assert_eq!(paint.commands().len(), 1);
        assert!(matches!(
            paint.commands()[0],
            scarlet_ui::renderer::PaintCommand::DrawBufferRect { .. }
        ));
    }

    #[test]
    fn pan_geometry_has_left_center_right_and_bounded_full_travel() {
        let center = Point::new(16., 16.);
        let left = dial_point(center, 10., -1.);
        let middle = dial_point(center, 10., 0.);
        let right = dial_point(center, 10., 1.);
        assert!(left.x < center.x && left.y > center.y);
        assert_eq!(middle, Point::new(16., 6.));
        assert!(right.x > center.x && right.y > center.y);
        assert!((left.x + right.x - 2. * center.x).abs() < 0.0001);
        assert_eq!(drag_value(-1., 100., -20.), 1.);
        assert_eq!(drag_value(1., 100., 220.), -1.);
        assert_eq!(drag_value(0.4, 16., 16.), 0.4);
        assert_eq!(drag_value(0., 16., -1000.), 1.);
        assert_eq!(drag_value(0., 16., 1000.), -1.);
        assert_eq!(bounded(f32::NAN), 0.);
    }

    #[test]
    fn keyboard_steps_fine_adjustment_center_and_bounds() {
        let (knob, value, _, _) = control();
        let press = |keycode, shift| {
            knob.handle_key(KeyEvent::Pressed {
                keycode,
                modifiers: KeyModifiers {
                    shift,
                    ..KeyModifiers::empty()
                },
            })
        };
        for (key, shift, expected) in [
            (KeyCode::Right, false, 0.42),
            (KeyCode::Up, true, 0.422),
            (KeyCode::Left, true, 0.420),
            (KeyCode::Down, false, 0.40),
            (KeyCode::Home, false, 0.),
        ] {
            assert!(press(key, shift));
            assert!((value.get() - expected).abs() < 0.00001);
        }
        value.set(0.999);
        press(KeyCode::Right, false);
        assert_eq!(value.get(), 1.);
        value.set(-0.999);
        press(KeyCode::Down, true);
        assert_eq!(value.get(), -1.);
        assert!(!press(KeyCode::Space, false));
        assert!(!knob.handle_key(KeyEvent::Char { c: 's' }));
    }

    #[test]
    fn native_dispatch_keeps_drag_origin_reset_and_cancel() {
        let (knob, value, dragging, focused) = control();
        let mut tree = ElementTree::new();
        tree.set_root(knob.create_element());
        tree.layout(LayoutConstraints::tight(DIAMETER, DIAMETER));
        let mut dispatcher = EventDispatcher::new();
        let press = |click_count| {
            Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: 16,
                y: 16,
                click_count,
            })
        };
        let release = |y, click_count| {
            Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 16,
                y,
                click_count,
            })
        };
        assert!(dispatcher.dispatch(&mut tree, &press(1)));
        assert!(focused.get() && dragging.get());
        assert_eq!(value.get(), 0.4);
        assert!(dispatcher.dispatch(&mut tree, &Event::Mouse(MouseEvent::Moved { x: 16, y: 4 })));
        assert!((value.get() - 0.6).abs() < 0.00001);
        tree.root_mut().unwrap().update(&knob);
        tree.layout(LayoutConstraints::tight(DIAMETER, DIAMETER));
        assert!(dispatcher.dispatch(&mut tree, &release(-8, 1)));
        assert!((value.get() - 0.8).abs() < 0.00001);
        assert!(!dragging.get());

        assert!(dispatcher.dispatch(&mut tree, &press(2)));
        assert_eq!(value.get(), 0.);
        dispatcher.dispatch(
            &mut tree,
            &Event::Mouse(MouseEvent::Moved { x: 16, y: -60 }),
        );
        dispatcher.dispatch(&mut tree, &release(25, 2));
        assert_eq!(value.get(), 0., "Reset must survive movement and release");
        assert!(!dragging.get());

        value.set(-0.3);
        dispatcher.dispatch(&mut tree, &press(1));
        dispatcher.dispatch(
            &mut tree,
            &Event::Mouse(MouseEvent::Moved { x: 16, y: -30 }),
        );
        assert_ne!(value.get(), -0.3);
        assert!(dispatcher.dispatch(
            &mut tree,
            &Event::Mouse(MouseEvent::ButtonCancelled {
                button: MouseButton::Left,
                x: 16,
                y: -30,
            }),
        ));
        assert_eq!(value.get(), -0.3);
        assert!(!dragging.get());
        dispatcher.dispatch(&mut tree, &release(-30, 1));
        assert_eq!(value.get(), -0.3);

        assert!(dispatcher.dispatch(
            &mut tree,
            &Event::Keyboard(KeyEvent::Pressed {
                keycode: KeyCode::Home,
                modifiers: KeyModifiers::empty(),
            }),
        ));
        assert_eq!(value.get(), 0., "Native focus must route keyboard input");
    }
}
