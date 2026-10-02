//! Compact native send-level control with the channel fader's nonlinear gain scale.
//! Drag upward to increase the send, or use arrows (Shift for fine adjustment).
use crate::{
    fader::{gain_from_fraction, gain_to_fraction},
    ui,
};
use scarlet_ui::{
    buffer::Buffer,
    element::{Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult},
    event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, Phase},
    prelude::*,
    renderer::PaintContext,
};
use std::{any::Any, cell::Cell, cell::RefCell, f32::consts::PI, rc::Rc, sync::Arc};

const DIAMETER: f32 = 32.;
const DRAG_TRAVEL: f32 = 120.;
const SWEEP: f32 = 3. * PI / 4.;

#[derive(Clone, Copy, Default)]
struct Interaction {
    before: f32,
    last_y: f32,
    fraction: f32,
    reset: bool,
    fine: bool,
}

#[derive(Clone)]
pub struct SendKnob {
    pub gain: State<f32>,
    pub dragging: State<bool>,
    pub focused: State<bool>,
    pub changed: Rc<dyn Fn(f32)>,
    interaction: Rc<Cell<Interaction>>,
}

impl SendKnob {
    pub fn new(
        gain: State<f32>,
        dragging: State<bool>,
        focused: State<bool>,
        changed: impl Fn(f32) + 'static,
    ) -> Self {
        Self {
            gain,
            dragging,
            focused,
            changed: Rc::new(changed),
            interaction: Rc::new(Cell::new(Interaction::default())),
        }
    }

    fn handle_key(&self, event: KeyEvent) -> bool {
        // ScarletUI mouse events do not carry modifiers. Keep the most recent
        // focused keyboard modifiers for fine pointer movement as well.
        let modifiers = match event {
            KeyEvent::Pressed { modifiers, .. } | KeyEvent::Released { modifiers, .. } => modifiers,
            KeyEvent::Char { .. } => return false,
        };
        let mut gesture = self.interaction.get();
        gesture.fine = modifiers.shift;
        self.interaction.set(gesture);
        let KeyEvent::Pressed { keycode, .. } = event else {
            return false;
        };
        if keycode == KeyCode::Escape && self.dragging.get() {
            self.cancel();
            return true;
        }
        let gain = bounded(self.gain.get());
        let step = if modifiers.shift { 0.1 } else { 1. };
        let db = if gain > 0. {
            (20. * gain.log10()).max(-100.)
        } else {
            -100.
        };
        let next = match keycode {
            // Existing sessions permit 2.0 (+6.02 dB), just beyond the
            // fader's +6 dB endpoint. Increasing must never lower that value.
            KeyCode::Up | KeyCode::Right => gain.max(10f32.powf((db + step).min(6.) / 20.)),
            KeyCode::Down | KeyCode::Left => {
                let next_db = db - step;
                if next_db <= -100. {
                    0.
                } else {
                    10f32.powf(next_db / 20.)
                }
            }
            KeyCode::Home => 1.,
            KeyCode::End => 0.,
            _ => return false,
        };
        let next = bounded(next);
        gesture.fraction = gain_to_fraction(next);
        self.interaction.set(gesture);
        if next != gain {
            (self.changed)(next);
        }
        true
    }

    fn move_to(&self, y: f32) {
        let mut gesture = self.interaction.get();
        let delta = gesture.last_y - y;
        gesture.last_y = y;
        let before_fraction = gesture.fraction;
        if !gesture.reset {
            let sensitivity = if gesture.fine { 0.1 } else { 1. };
            gesture.fraction = (gesture.fraction + delta * sensitivity / DRAG_TRAVEL).clamp(0., 1.);
        }
        self.interaction.set(gesture);
        // A press/release without movement must preserve the exact gain. In
        // particular, inverse mapping must not turn a no-op into an Undo entry.
        if gesture.fraction != before_fraction {
            (self.changed)(bounded(gain_from_fraction(gesture.fraction)));
        }
    }

    fn cancel(&self) {
        (self.changed)(self.interaction.get().before);
        self.dragging.set(false);
    }
}

impl View for SendKnob {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(scarlet_ui::ComponentElement::new_with_builder(
            self.clone(),
            |s| {
                let key = s.clone();
                Box::new(
                    SendFace(s.clone())
                        .focusable(s.focused.clone())
                        .on_key(move |event| key.handle_key(event)),
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
struct SendFace(SendKnob);

impl View for SendFace {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |s| SendRender {
                control: s.0.clone(),
                size: Size::new(DIAMETER, DIAMETER),
                raster: RefCell::new(None),
            },
            |render, s| {
                // Preserve capture, cancellation origin and fine mode when a
                // model update constructs a new view during the same gesture.
                s.0.interaction.set(render.control.interaction.get());
                render.control = s.0.clone();
                UpdateResult::Updated
            },
            |_| vec![],
        ))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct SendRender {
    control: SendKnob,
    size: Size,
    raster: RefCell<Option<SendRaster>>,
}

fn bounded(gain: f32) -> f32 {
    if gain.is_finite() {
        gain.clamp(0., 2.)
    } else {
        0.
    }
}

fn dial_point(center: Point, radius: f32, fraction: f32) -> Point {
    let angle = (fraction.clamp(0., 1.) * 2. - 1.) * SWEEP;
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
    fraction: f32,
    dragging: bool,
    focused: bool,
}

struct SendRaster {
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
    let start = (from.min(to) * 2. - 1.) * SWEEP;
    let end = (from.max(to) * 2. - 1.) * SWEEP;
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
    let pointer_start = dial_point(zero, radius * 0.3, key.fraction);
    let pointer_end = dial_point(zero, radius * 0.7, key.fraction);
    let ticks = [0., gain_to_fraction(1.), 1.].map(|position| {
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
                coverage(arc_distance(point, radius, 0., 1.) - 0.75, pixel_width),
            );
            if key.fraction > 0. {
                composite(
                    &mut pixel,
                    ui::ACCENT,
                    coverage(
                        arc_distance(point, radius, 0., key.fraction) - 0.75,
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

impl SendRender {
    fn raster(&self, scale_milli: u32) -> Arc<Buffer> {
        let key = RasterKey {
            width: self.size.width.ceil().max(1.) as u32,
            height: self.size.height.ceil().max(1.) as u32,
            scale_milli: scale_milli.max(1),
            fraction: gain_to_fraction(bounded(self.control.gain.get())),
            dragging: self.control.dragging.get(),
            focused: self.control.focused.get(),
        };
        let mut cache = self.raster.borrow_mut();
        let entry = cache.get_or_insert_with(|| SendRaster {
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

impl ElementRenderObject for SendRender {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.size = Size::new(DIAMETER, DIAMETER).constrain(
            Size::new(constraints.min_width, constraints.min_height),
            Size::new(constraints.max_width, constraints.max_height),
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
                if self.control.dragging.get() {
                    return true;
                }
                let before = bounded(self.control.gain.get());
                let reset = *click_count >= 2;
                self.control.interaction.set(Interaction {
                    before,
                    last_y: *y as f32,
                    fraction: gain_to_fraction(before),
                    reset,
                    fine: self.control.interaction.get().fine,
                });
                self.control.dragging.set(true);
                self.control.focused.set(true);
                if reset && before != 1. {
                    (self.control.changed)(1.);
                }
                true
            }
            Event::Mouse(MouseEvent::Moved { y, .. }) if self.control.dragging.get() => {
                self.control.move_to(*y as f32);
                true
            }
            Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                y,
                ..
            }) if self.control.dragging.get() => {
                self.control.move_to(*y as f32);
                self.control.dragging.set(false);
                true
            }
            Event::Mouse(MouseEvent::ButtonCancelled {
                button: MouseButton::Left,
                ..
            }) if self.control.dragging.get() => {
                self.control.cancel();
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
    use scarlet_ui::{ElementTree, EventDispatcher, event::KeyModifiers, renderer::PaintCommand};

    struct Harness {
        knob: SendKnob,
        gain: State<f32>,
        dragging: State<bool>,
        focused: State<bool>,
        changes: Rc<RefCell<Vec<f32>>>,
        tree: ElementTree,
        dispatcher: EventDispatcher,
    }

    impl Harness {
        fn new(gain: f32) -> Self {
            let gain = State::new(StateId::new(731), gain);
            let dragging = State::new(StateId::new(732), false);
            let focused = State::new(StateId::new(733), false);
            let changes = Rc::new(RefCell::new(Vec::new()));
            let output = gain.clone();
            let changed = changes.clone();
            let knob = SendKnob::new(gain.clone(), dragging.clone(), focused.clone(), move |v| {
                changed.borrow_mut().push(v);
                output.set(v);
            });
            let mut tree = ElementTree::new();
            tree.set_root(knob.create_element());
            tree.layout(LayoutConstraints::tight(DIAMETER, DIAMETER));
            Self {
                knob,
                gain,
                dragging,
                focused,
                changes,
                tree,
                dispatcher: EventDispatcher::new(),
            }
        }

        fn event(&mut self, event: Event) -> bool {
            self.dispatcher.dispatch(&mut self.tree, &event)
        }

        fn press(&mut self, click_count: u8) {
            assert!(self.event(Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: 16,
                y: 16,
                click_count,
            })));
        }

        fn move_to(&mut self, y: i32) {
            assert!(self.event(Event::Mouse(MouseEvent::Moved { x: 16, y })));
        }

        fn release(&mut self, y: i32) {
            self.event(Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 16,
                y,
                click_count: 1,
            }));
        }

        fn key(&mut self, keycode: KeyCode, shift: bool) -> bool {
            self.event(Event::Keyboard(KeyEvent::Pressed {
                keycode,
                modifiers: KeyModifiers {
                    shift,
                    ..KeyModifiers::empty()
                },
            }))
        }

        fn rebuild(&mut self) {
            // Reconstruct as the inspector does, rather than merely clone the
            // original control. Active gesture data must survive either form.
            let output = self.gain.clone();
            let changes = self.changes.clone();
            self.knob = SendKnob::new(
                self.gain.clone(),
                self.dragging.clone(),
                self.focused.clone(),
                move |v| {
                    changes.borrow_mut().push(v);
                    output.set(v);
                },
            );
            self.tree.root_mut().unwrap().update(&self.knob);
            self.tree
                .layout(LayoutConstraints::tight(DIAMETER, DIAMETER));
        }
    }

    fn send_render(element: &dyn Element) -> Option<&SendRender> {
        element
            .render_object()
            .and_then(|render| render.as_any().downcast_ref::<SendRender>())
            .or_else(|| {
                element
                    .children()
                    .iter()
                    .find_map(|child| send_render(child.as_ref()))
            })
    }

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.00001,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn native_drag_is_relative_captured_and_preserved_across_rebuilds() {
        let mut h = Harness::new(0.25);
        let initial_fraction = gain_to_fraction(0.25);
        h.press(1);
        assert!(h.dragging.get() && h.focused.get());
        assert_eq!(h.gain.get(), 0.25);
        assert!(
            h.changes.borrow().is_empty(),
            "Pressing must never jump the gain"
        );
        h.move_to(4);
        close(h.gain.get(), gain_from_fraction(initial_fraction + 0.1));
        h.rebuild();
        h.release(-8);
        close(h.gain.get(), gain_from_fraction(initial_fraction + 0.2));
        assert!(!h.dragging.get());
        assert!(
            h.gain.get() > 0.25,
            "Release outside bounds must remain captured"
        );

        let before = h.gain.get();
        h.press(1);
        h.release(16);
        assert_eq!(
            h.gain.get(),
            before,
            "Repeated no-op clicks preserve exact gain"
        );
    }

    #[test]
    fn native_cancel_and_escape_restore_exact_gain_and_ignore_late_release() {
        let mut h = Harness::new(0.314_159_27);
        for escape in [false, true] {
            let before = h.gain.get();
            h.press(1);
            h.move_to(-42);
            h.rebuild();
            assert_ne!(h.gain.get(), before);
            if escape {
                assert!(h.key(KeyCode::Escape, false));
            } else {
                assert!(h.event(Event::Mouse(MouseEvent::ButtonCancelled {
                    button: MouseButton::Left,
                    x: 16,
                    y: -42,
                })));
            }
            assert_eq!(h.gain.get(), before);
            assert!(!h.dragging.get());
            h.release(-80);
            assert_eq!(h.gain.get(), before);
        }
        h.press(1);
        h.move_to(10);
        h.release(10);
        assert!(
            h.gain.get() > 0.314_159_27,
            "A new gesture works after cancellation"
        );
    }

    #[test]
    fn double_click_reset_remains_unity_until_release_and_can_be_cancelled() {
        let mut h = Harness::new(0.25);
        h.press(2);
        assert_eq!(h.gain.get(), 1.);
        h.move_to(-120);
        h.rebuild();
        h.release(80);
        assert_eq!(h.gain.get(), 1.);
        assert!(!h.dragging.get());

        h.gain.set(0.37);
        h.press(2);
        assert_eq!(h.gain.get(), 1.);
        assert!(h.key(KeyCode::Escape, false));
        assert_eq!(h.gain.get(), 0.37);
        h.release(16);
        assert_eq!(h.gain.get(), 0.37);
    }

    #[test]
    fn focused_keyboard_steps_decibels_and_handles_unity_silence_and_range() {
        let mut h = Harness::new(0.25);
        h.press(1);
        h.release(16);
        assert!(h.key(KeyCode::Home, false));
        assert_eq!(h.gain.get(), 1.);
        for (key, shift, db) in [
            (KeyCode::Up, false, 1.),
            (KeyCode::Right, true, 1.1),
            (KeyCode::Left, true, 1.),
            (KeyCode::Down, false, 0.),
        ] {
            assert!(h.key(key, shift));
            close(h.gain.get(), 10f32.powf(db / 20.));
        }
        assert!(h.key(KeyCode::End, false));
        assert_eq!(h.gain.get(), 0.);
        h.key(KeyCode::Down, false);
        assert_eq!(h.gain.get(), 0.);
        h.key(KeyCode::Up, true);
        assert!(h.gain.get() > 0.);
        h.key(KeyCode::Down, false);
        assert_eq!(h.gain.get(), 0.);
        h.gain.set(2.);
        h.key(KeyCode::Up, false);
        assert_eq!(
            h.gain.get(),
            2.,
            "Increasing a legal stored maximum must not reduce it"
        );
        h.key(KeyCode::Down, false);
        close(h.gain.get(), 2. * 10f32.powf(-1. / 20.));
        assert!(!h.key(KeyCode::Space, false));
        assert!(!h.event(Event::Keyboard(KeyEvent::Char { c: 's' })));
        assert!(!h.event(Event::Keyboard(KeyEvent::Released {
            keycode: KeyCode::Up,
            modifiers: KeyModifiers::empty()
        })));
    }

    #[test]
    fn fine_drag_tracks_focused_modifiers_without_jumping_when_shift_changes() {
        let mut h = Harness::new(0.25);
        let initial_fraction = gain_to_fraction(0.25);
        h.press(1);
        h.key(KeyCode::Unknown, true);
        h.move_to(4);
        close(h.gain.get(), gain_from_fraction(initial_fraction + 0.01));
        h.rebuild();
        h.key(KeyCode::Unknown, false);
        let gain = h.gain.get();
        h.move_to(4);
        assert_eq!(
            h.gain.get(),
            gain,
            "Modifier changes must not rebase the full drag"
        );
        h.move_to(-8);
        close(h.gain.get(), gain_from_fraction(initial_fraction + 0.11));
        h.release(-8);
    }

    #[test]
    fn gain_mapping_and_drag_are_monotonic_finite_and_bounded() {
        let mut h = Harness::new(0.);
        h.press(1);
        let mut previous = 0.;
        for y in (16 - DRAG_TRAVEL as i32..=16).rev() {
            h.move_to(y);
            let next = h.gain.get();
            assert!(next.is_finite() && (0. ..=2.).contains(&next));
            assert!(next >= previous);
            previous = next;
        }
        close(h.gain.get(), gain_from_fraction(1.));
        h.move_to(-10_000);
        close(h.gain.get(), gain_from_fraction(1.));
        h.move_to(10_000);
        assert_eq!(h.gain.get(), 0.);
        h.release(10_000);
        assert_eq!(bounded(f32::NAN), 0.);
        assert_eq!(bounded(f32::INFINITY), 0.);
        assert_eq!(bounded(-1.), 0.);
        assert_eq!(bounded(8.), 2.);
        let center = Point::new(16., 16.);
        let silence = dial_point(center, 10., 0.);
        let unity = dial_point(center, 10., gain_to_fraction(1.));
        let maximum = dial_point(center, 10., 1.);
        assert!(silence.x < center.x && silence.y > center.y);
        assert!(unity.x > center.x && unity.y < center.y);
        assert!(maximum.x > center.x && maximum.y > center.y);
    }

    #[test]
    fn native_element_paint_has_smooth_dpi_scaled_edges_open_arc_and_cached_buffer() {
        let h = Harness::new(0.25);
        let render = send_render(h.tree.root().unwrap()).expect("Native send render object");
        for scale in [1000, 2000] {
            let buffer = render.raster(scale);
            let dimension = 32 * scale / 1000;
            assert_eq!((buffer.width(), buffer.height()), (dimension, dimension));
            let alpha = |x: u32, y: u32| buffer.as_slice()[(y * dimension + x) as usize] >> 24;
            assert_eq!(alpha(0, 0), 0);
            assert_eq!(alpha(dimension / 2, dimension / 2), 255);
            assert_eq!(
                alpha(dimension / 2, 29 * scale / 1000),
                0,
                "No closing chord across the arc"
            );
            assert!(
                buffer
                    .as_slice()
                    .iter()
                    .filter(|pixel| (1..255).contains(&(*pixel >> 24)))
                    .count()
                    > 40 * (scale / 1000) as usize
            );
        }
        let first = render.raster(1000);
        let same = render.raster(1000);
        assert!(Arc::ptr_eq(&first, &same));
        let identity = first.identity();
        let revision = first.revision();
        drop(first);
        drop(same);
        h.gain.set(1.);
        let changed = render.raster(1000);
        assert_eq!(changed.identity(), identity);
        assert!(changed.revision() > revision);
        let pixels = changed.as_slice().to_vec();
        h.gain.set(0.);
        let next = render.raster(1000);
        assert_ne!(next.identity(), changed.identity());
        assert_eq!(
            changed.as_slice(),
            pixels,
            "Live paint snapshots stay immutable"
        );
        assert_ne!(next.as_slice(), pixels);

        let mut paint = PaintContext::new();
        assert!(render.paint(&mut paint, Point::new(8., 5.)));
        assert_eq!(paint.commands().len(), 1);
        assert!(matches!(
            paint.commands()[0],
            PaintCommand::DrawBufferRect { .. }
        ));
        assert!(arc_distance(Point::new(0., 13.), 13., 0., 1.) > 9.);
    }
}
