//! A compact, retained native insert slot. The owner supplies editor and menu UI.
use crate::ui;
use scarlet_ui::{
    buffer::Buffer,
    element::{Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult},
    event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, Phase},
    prelude::*,
    renderer::PaintContext,
};
use std::{any::Any, cell::RefCell, f32::consts::FRAC_1_SQRT_2, rc::Rc, sync::Arc};

const WIDTH: f32 = 190.;
const HEIGHT: f32 = 26.;
const FONT: f32 = 10.;
const MENU_WIDTH: f32 = 20.;
const POWER_WIDTH: f32 = 22.;
const POWER_RASTER_SIZE: u32 = 14;
const POWER_RADIUS: f32 = 4.;
const POWER_STROKE: f32 = 1.2;

#[derive(Clone)]
pub struct InsertSlot {
    number: usize,
    name: Option<String>,
    bypassed: bool,
    selected: bool,
    pub focused: State<bool>,
    open: Rc<dyn Fn()>,
    toggle_bypass: Rc<dyn Fn()>,
    context: Rc<dyn Fn()>,
}

impl InsertSlot {
    /// `number` is the displayed, one-based slot number. The name is independent
    /// of effect type, so native effects and hosted plug-ins share this control.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        number: usize,
        name: impl Into<String>,
        bypassed: bool,
        selected: bool,
        focused: State<bool>,
        open: impl Fn() + 'static,
        toggle_bypass: impl Fn() + 'static,
        context: impl Fn() + 'static,
    ) -> Self {
        Self {
            number,
            name: Some(name.into()),
            bypassed,
            selected,
            focused,
            open: Rc::new(open),
            toggle_bypass: Rc::new(toggle_bypass),
            context: Rc::new(context),
        }
    }

    /// Clicking an empty slot, its chevron, or Enter opens the add-effect UI.
    pub fn empty(number: usize, focused: State<bool>, add: impl Fn() + 'static) -> Self {
        let add: Rc<dyn Fn()> = Rc::new(add);
        Self {
            number,
            name: None,
            bypassed: false,
            selected: false,
            focused,
            open: add.clone(),
            toggle_bypass: Rc::new(|| {}),
            context: add,
        }
    }

    fn handle_key(&self, event: KeyEvent) -> bool {
        // OnKey can be visited during root fallback as well as focused bubbling.
        // Never consume the DAW's keys just because the slot exists in the tree.
        if !self.focused.get() {
            return false;
        }
        let KeyEvent::Pressed { keycode, modifiers } = event else {
            return false;
        };
        if modifiers.control || modifiers.alt || modifiers.super_key {
            return false;
        }
        match keycode {
            KeyCode::Enter if !modifiers.shift => (self.open)(),
            KeyCode::Space if !modifiers.shift && self.name.is_some() => (self.toggle_bypass)(),
            // ScarletUI has no Menu key code; Shift-F10 is the portable equivalent.
            KeyCode::F(10) if modifiers.shift => (self.context)(),
            _ => return false,
        }
        true
    }
}

impl View for InsertSlot {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(scarlet_ui::ComponentElement::new_with_builder(
            self.clone(),
            |slot| {
                let key = slot.clone();
                Box::new(
                    SlotFace(slot.clone())
                        .focusable(slot.focused.clone())
                        .on_key(move |event| key.handle_key(event)),
                )
            },
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.focused]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
struct SlotFace(InsertSlot);

impl View for SlotFace {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| SlotRender::new(view.0.clone()),
            |render, view| {
                let labels_changed =
                    render.slot.number != view.0.number || render.slot.name != view.0.name;
                if labels_changed {
                    // Do not complete a press on an effect replaced under the pointer.
                    render.pressed = None;
                }
                render.slot = view.0.clone();
                if labels_changed {
                    render.update_labels();
                }
                UpdateResult::Updated
            },
            |_| vec![],
        ))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Open,
    Power,
    Context,
}

struct Label {
    text: String,
    ink: Point,
}

impl Label {
    fn new(text: String, font: f32) -> Self {
        let ink = crate::fader::ink_center(&text, font);
        Self { text, ink }
    }
}

struct SlotRender {
    slot: InsertSlot,
    size: Size,
    hovered: Option<Hit>,
    pressed: Option<(MouseButton, Hit)>,
    number: Label,
    name: Label,
    power_raster: RefCell<Option<PowerRaster>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PowerRasterKey {
    scale_milli: u32,
    bypassed: bool,
}

struct PowerRaster {
    key: PowerRasterKey,
    buffer: Arc<Buffer>,
}

// Exact distance to the open 270-degree circle, including its round end caps.
// Folding x preserves mirror symmetry and avoids a tessellated/closed path.
fn power_arc_distance(point: Point) -> f32 {
    if point.y >= -point.x.abs() {
        (point.x.hypot(point.y) - POWER_RADIUS).abs()
    } else {
        let end = POWER_RADIUS * FRAC_1_SQRT_2;
        (point.x.abs() - end).hypot(point.y + end)
    }
}

fn power_distance(point: Point) -> f32 {
    let stem = point.x.hypot(point.y - point.y.clamp(-5., 0.));
    power_arc_distance(point).min(stem) - POWER_STROKE / 2.
}

fn rasterize_power(buffer: &mut Buffer, key: PowerRasterKey) {
    let scale = key.scale_milli as f32 / 1000.;
    let half = POWER_RASTER_SIZE as f32 / 2.;
    let color = if key.bypassed { ui::MUTED } else { ui::ACCENT };
    let width = buffer.width();
    let height = buffer.height();
    let pixels = buffer.as_mut_slice();
    for y in 0..height {
        for x in 0..width {
            // The stem extends 1px above the ring. Shift the shape down 0.5px
            // to optically center its complete silhouette in the compact row.
            let point = Point::new(
                (x as f32 + 0.5) / scale - half,
                (y as f32 + 0.5) / scale - half - 0.5,
            );
            let alpha = (0.5 - power_distance(point) * scale).clamp(0., 1.) * color.a;
            pixels[(y * width + x) as usize] = if alpha > 0. {
                // Buffer uses straight alpha: retain the original ink even at
                // the edge so either the row or hover background stays clean.
                Color::rgba_f32(color.r, color.g, color.b, alpha).to_bgra()
            } else {
                0
            };
        }
    }
}

impl SlotRender {
    fn new(slot: InsertSlot) -> Self {
        let mut render = Self {
            slot,
            size: Size::new(WIDTH, HEIGHT),
            hovered: None,
            pressed: None,
            number: Label::new(String::new(), 9.),
            name: Label::new(String::new(), FONT),
            power_raster: RefCell::new(None),
        };
        render.update_labels();
        render
    }

    fn power_raster(&self, scale_milli: u32) -> Arc<Buffer> {
        let key = PowerRasterKey {
            scale_milli: scale_milli.max(1),
            bypassed: self.slot.bypassed,
        };
        let mut cache = self.power_raster.borrow_mut();
        let entry = cache.get_or_insert_with(|| {
            let mut buffer = Buffer::from_logical_dimensions_with_scale(
                POWER_RASTER_SIZE,
                POWER_RASTER_SIZE,
                key.scale_milli,
            );
            rasterize_power(&mut buffer, key);
            PowerRaster {
                key,
                buffer: Arc::new(buffer),
            }
        });
        if entry.key != key {
            if entry.key.scale_milli != key.scale_milli {
                entry.buffer = Arc::new(Buffer::from_logical_dimensions_with_scale(
                    POWER_RASTER_SIZE,
                    POWER_RASTER_SIZE,
                    key.scale_milli,
                ));
            }
            // Reuse the native texture identity unless a previous paint still
            // owns a snapshot; copy-on-write keeps that snapshot immutable.
            rasterize_power(Arc::make_mut(&mut entry.buffer), key);
            entry.key = key;
        }
        entry.buffer.clone()
    }

    fn name_left(&self) -> f32 {
        if self.slot.name.is_some() { 25. } else { 39. }
    }

    fn name_right(&self) -> f32 {
        self.size.width
            - MENU_WIDTH
            - if self.slot.name.is_some() {
                POWER_WIDTH
            } else {
                0.
            }
            - 5.
    }

    fn update_labels(&mut self) {
        let number = self.slot.number.to_string();
        if self.number.text != number {
            self.number = Label::new(number, 9.);
        }
        let name = fit_name(
            self.slot.name.as_deref().unwrap_or("Add insert"),
            (self.name_right() - self.name_left()).max(0.),
        );
        if self.name.text != name {
            self.name = Label::new(name, FONT);
        }
    }

    fn hit(&self, x: i32, y: i32) -> Option<Hit> {
        let (x, y) = (x as f32, y as f32);
        if x < 0. || y < 0. || x >= self.size.width || y >= self.size.height {
            return None;
        }
        if x >= self.size.width - MENU_WIDTH {
            Some(Hit::Context)
        } else if self.slot.name.is_some() && x >= self.size.width - MENU_WIDTH - POWER_WIDTH {
            Some(Hit::Power)
        } else {
            Some(Hit::Open)
        }
    }

    fn activate(&self, hit: Hit) {
        match hit {
            Hit::Open => (self.slot.open)(),
            Hit::Power => (self.slot.toggle_bypass)(),
            Hit::Context => (self.slot.context)(),
        }
    }
}

/// Truncate at Unicode scalar boundaries, keeping text out of the power/menu
/// hit regions. The retained object only measures when its label or size changes.
fn fit_name(name: &str, width: f32) -> String {
    let measure = |text: &str| scarlet_ui::graphics::measure_text_sized(text, FONT).0 as f32;
    if width <= 0. {
        return String::new();
    }
    if measure(name) <= width {
        return name.into();
    }
    if measure("…") > width {
        return String::new();
    }
    let mut fitted = name.to_string();
    while !fitted.is_empty() {
        fitted.pop();
        let candidate = format!("{fitted}…");
        if measure(&candidate) <= width {
            return candidate;
        }
    }
    "…".into()
}

impl ElementRenderObject for SlotRender {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        let size = Size::new(WIDTH, HEIGHT).constrain(
            Size::new(constraints.min_width, constraints.min_height),
            Size::new(constraints.max_width, constraints.max_height),
        );
        if self.size != size {
            self.size = size;
            self.update_labels();
        }
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
        let (x, y) = (origin.x, origin.y);
        let width = self.size.width;
        let height = self.size.height;
        let center_y = y + height / 2.;
        ctx.fill_rounded_rect(
            Rect::new(origin, self.size),
            0.,
            if self.hovered.is_some() || self.slot.selected {
                ui::RAISED
            } else {
                ui::BG
            },
        );
        // Slots form one continuous rack. A single bottom rule separates rows;
        // focus gets an inset outline without adding gaps or rounded cards.
        ctx.draw_line(
            Point::new(x, y + height - 0.5),
            Point::new(x + width, y + height - 0.5),
            1.,
            ui::LINE,
        );
        if self.slot.focused.get() {
            ctx.stroke_rounded_rect(
                Rect::from_xywh(x + 0.5, y + 0.5, width - 1., height - 1.),
                0.,
                1.,
                ui::ACCENT,
            );
        }
        if self.slot.selected {
            ctx.fill_rounded_rect(
                Rect::from_xywh(x + 2., y + 5., 2., height - 10.),
                1.,
                ui::ACCENT,
            );
        }
        ctx.draw_text(
            Point::new(x + 13. - self.number.ink.x, center_y - self.number.ink.y),
            &self.number.text,
            ui::MUTED,
            9.,
        );
        ctx.draw_text(
            Point::new(x + self.name_left(), center_y - self.name.ink.y),
            &self.name.text,
            if self.slot.bypassed || self.slot.name.is_none() {
                ui::MUTED
            } else {
                ui::TEXT
            },
            FONT,
        );

        if self.slot.name.is_some() {
            let center = Point::new(x + width - MENU_WIDTH - POWER_WIDTH / 2., center_y);
            if self.hovered == Some(Hit::Power) {
                ctx.fill_rounded_rect(
                    Rect::from_xywh(center.x - 9., center.y - 9., 18., 18.),
                    3.,
                    ui::PANEL,
                );
            }
            // SGFX strokes are aliased and close multi-point arcs. A retained
            // analytic raster gives this tiny native glyph smooth open edges.
            let side = POWER_RASTER_SIZE as f32;
            ctx.draw_buffer_rect_shared(
                Rect::from_xywh(center.x - side / 2., center.y - side / 2., side, side),
                Rect::from_xywh(0., 0., side, side),
                self.power_raster(scarlet_ui::graphics::current_scale_milli()),
                1.,
            );
        } else {
            let center = Point::new(x + 30., center_y);
            ctx.draw_line(
                Point::new(center.x - 3., center.y),
                Point::new(center.x + 3., center.y),
                1.,
                ui::MUTED,
            );
            ctx.draw_line(
                Point::new(center.x, center.y - 3.),
                Point::new(center.x, center.y + 3.),
                1.,
                ui::MUTED,
            );
        }
        let menu_x = x + width - MENU_WIDTH / 2.;
        let menu_color = if self.hovered == Some(Hit::Context) {
            ui::TEXT
        } else {
            ui::MUTED
        };
        ctx.draw_line(
            Point::new(menu_x - 3., center_y - 1.),
            Point::new(menu_x, center_y + 2.),
            1.,
            menu_color,
        );
        ctx.draw_line(
            Point::new(menu_x, center_y + 2.),
            Point::new(menu_x + 3., center_y - 1.),
            1.,
            menu_color,
        );
        true
    }

    fn handle_event(&mut self, event: &Event, phase: Phase) -> bool {
        if !matches!(phase, Phase::Target | Phase::Bubble) {
            return false;
        }
        match event {
            Event::Mouse(MouseEvent::Entered { x, y }) => {
                // Right presses have no dispatcher capture. Re-entry must never
                // revive one whose outside release was delivered elsewhere.
                if self
                    .pressed
                    .is_some_and(|(button, _)| button == MouseButton::Right)
                {
                    self.pressed = None;
                }
                self.hovered = self.hit(*x, *y);
                true
            }
            Event::Mouse(MouseEvent::Moved { x, y }) => {
                let hovered = self.hit(*x, *y);
                let changed = self.hovered != hovered;
                self.hovered = hovered;
                changed || self.pressed.is_some()
            }
            Event::Mouse(MouseEvent::Exited { .. }) => {
                let changed = self.hovered.take().is_some();
                // ScarletUI captures left presses. Right presses are not captured,
                // so leaving must invalidate one even if its release goes elsewhere.
                if self
                    .pressed
                    .is_some_and(|(button, _)| button == MouseButton::Right)
                {
                    self.pressed = None;
                    return true;
                }
                changed
            }
            Event::Mouse(MouseEvent::ButtonPressed { button, x, y, .. })
                if matches!(button, MouseButton::Left | MouseButton::Right) =>
            {
                let Some(hit) = self.hit(*x, *y) else {
                    return false;
                };
                self.pressed = Some((
                    *button,
                    if *button == MouseButton::Right {
                        Hit::Context
                    } else {
                        hit
                    },
                ));
                self.hovered = Some(hit);
                self.slot.focused.set(true);
                true
            }
            Event::Mouse(MouseEvent::ButtonReleased { button, x, y, .. }) => {
                let Some((pressed_button, hit)) = self.pressed else {
                    return false;
                };
                if *button != pressed_button {
                    return false;
                }
                self.pressed = None;
                self.hovered = self.hit(*x, *y);
                if self.hovered.is_some()
                    && (pressed_button == MouseButton::Right || self.hovered == Some(hit))
                {
                    self.activate(hit);
                }
                true
            }
            Event::Mouse(MouseEvent::ButtonCancelled { button, .. })
                if self
                    .pressed
                    .is_some_and(|(pressed_button, _)| pressed_button == *button) =>
            {
                self.pressed = None;
                self.hovered = None;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarlet_ui::{ElementTree, EventDispatcher, event::KeyModifiers, renderer::PaintCommand};
    use std::cell::Cell;

    #[derive(Clone, Default)]
    struct Actions {
        open: Rc<Cell<usize>>,
        bypass: Rc<Cell<usize>>,
        context: Rc<Cell<usize>>,
    }

    impl Actions {
        fn counts(&self) -> (usize, usize, usize) {
            (self.open.get(), self.bypass.get(), self.context.get())
        }
    }

    fn slot() -> (InsertSlot, Actions) {
        let actions = Actions::default();
        let (open, bypass, context) = (
            actions.open.clone(),
            actions.bypass.clone(),
            actions.context.clone(),
        );
        (
            InsertSlot::new(
                1,
                "Stereo delay",
                false,
                false,
                State::new(StateId::new(1701), false),
                move || open.set(open.get() + 1),
                move || bypass.set(bypass.get() + 1),
                move || context.set(context.get() + 1),
            ),
            actions,
        )
    }

    fn tree(slot: &InsertSlot) -> (ElementTree, EventDispatcher) {
        let mut tree = ElementTree::new();
        tree.set_root(slot.create_element());
        tree.layout(LayoutConstraints::tight(WIDTH, HEIGHT));
        (tree, EventDispatcher::new())
    }

    fn press(button: MouseButton, x: i32, y: i32) -> Event {
        Event::Mouse(MouseEvent::ButtonPressed {
            button,
            x,
            y,
            click_count: 1,
        })
    }

    fn release(button: MouseButton, x: i32, y: i32) -> Event {
        Event::Mouse(MouseEvent::ButtonReleased {
            button,
            x,
            y,
            click_count: 1,
        })
    }

    fn key(keycode: KeyCode, shift: bool) -> Event {
        Event::Keyboard(KeyEvent::Pressed {
            keycode,
            modifiers: KeyModifiers {
                shift,
                ..KeyModifiers::empty()
            },
        })
    }

    #[test]
    fn native_slot_clicks_only_activate_on_release_and_repeat_reliably() {
        let (slot, actions) = slot();
        let (mut tree, mut dispatcher) = tree(&slot);
        dispatcher.dispatch(&mut tree, &Event::Mouse(MouseEvent::Moved { x: 70, y: 13 }));
        assert_eq!(actions.counts(), (0, 0, 0));
        for expected in 1..=4 {
            assert!(dispatcher.dispatch(&mut tree, &press(MouseButton::Left, 70, 13)));
            assert!(slot.focused.get());
            assert_eq!(actions.open.get(), expected - 1);
            // Focus/model reconciliation must preserve the in-flight gesture.
            tree.root_mut().unwrap().update(&slot);
            tree.layout(LayoutConstraints::tight(WIDTH, HEIGHT));
            assert!(dispatcher.dispatch(&mut tree, &release(MouseButton::Left, 70, 13)));
            assert_eq!(actions.counts(), (expected, 0, 0));
        }
        dispatcher.dispatch(&mut tree, &release(MouseButton::Left, 70, 13));
        assert_eq!(actions.counts(), (4, 0, 0));
    }

    #[test]
    fn native_power_and_context_targets_do_not_open_the_editor() {
        let (slot, actions) = slot();
        let (mut tree, mut dispatcher) = tree(&slot);
        for (x, expected) in [(159, (0, 1, 0)), (180, (0, 1, 1)), (159, (0, 2, 1))] {
            dispatcher.dispatch(&mut tree, &press(MouseButton::Left, x, 13));
            dispatcher.dispatch(&mut tree, &release(MouseButton::Left, x, 13));
            assert_eq!(actions.counts(), expected);
        }
        for (start, end) in [(70, 159), (159, 70), (180, 159), (159, 180)] {
            dispatcher.dispatch(&mut tree, &press(MouseButton::Left, start, 13));
            dispatcher.dispatch(&mut tree, &release(MouseButton::Left, end, 13));
            assert_eq!(
                actions.counts(),
                (0, 2, 1),
                "Cross-target release must cancel"
            );
        }
    }

    #[test]
    fn native_capture_cancels_outside_and_platform_cancel_without_stale_activation() {
        let (slot, actions) = slot();
        let (mut tree, mut dispatcher) = tree(&slot);
        for (x, y) in [(-1, 13), (190, 13), (70, -1), (70, 26)] {
            dispatcher.dispatch(&mut tree, &press(MouseButton::Left, 70, 13));
            dispatcher.dispatch(&mut tree, &Event::Mouse(MouseEvent::Moved { x, y }));
            assert!(dispatcher.dispatch(&mut tree, &release(MouseButton::Left, x, y)));
            dispatcher.dispatch(&mut tree, &release(MouseButton::Left, 70, 13));
            assert_eq!(actions.counts(), (0, 0, 0));
        }
        dispatcher.dispatch(&mut tree, &press(MouseButton::Left, 159, 13));
        assert!(dispatcher.dispatch(
            &mut tree,
            &Event::Mouse(MouseEvent::ButtonCancelled {
                button: MouseButton::Left,
                x: 159,
                y: 13,
            })
        ));
        dispatcher.dispatch(&mut tree, &release(MouseButton::Left, 159, 13));
        assert_eq!(actions.counts(), (0, 0, 0));
        dispatcher.dispatch(&mut tree, &press(MouseButton::Left, 159, 13));
        dispatcher.dispatch(&mut tree, &release(MouseButton::Left, 159, 13));
        assert_eq!(actions.counts(), (0, 1, 0));
    }

    #[test]
    fn native_right_click_opens_context_only_on_release_and_exit_cancels() {
        let (slot, actions) = slot();
        let (mut tree, mut dispatcher) = tree(&slot);
        dispatcher.dispatch(&mut tree, &press(MouseButton::Right, 70, 13));
        assert_eq!(actions.counts(), (0, 0, 0));
        dispatcher.dispatch(&mut tree, &release(MouseButton::Right, 70, 13));
        assert_eq!(actions.counts(), (0, 0, 1));
        dispatcher.dispatch(&mut tree, &press(MouseButton::Right, 70, 13));
        dispatcher.dispatch(
            &mut tree,
            &Event::Mouse(MouseEvent::Moved { x: 250, y: 13 }),
        );
        dispatcher.dispatch(&mut tree, &release(MouseButton::Right, 250, 13));
        dispatcher.dispatch(&mut tree, &Event::Mouse(MouseEvent::Moved { x: 70, y: 13 }));
        dispatcher.dispatch(&mut tree, &release(MouseButton::Right, 70, 13));
        assert_eq!(actions.counts(), (0, 0, 1));
    }

    #[test]
    fn native_keyboard_respects_focus_and_leaves_navigation_keys_to_the_app() {
        let (slot, actions) = slot();
        let (mut tree, mut dispatcher) = tree(&slot);
        for keycode in [KeyCode::Enter, KeyCode::Space, KeyCode::Up, KeyCode::Down] {
            assert!(!dispatcher.dispatch(&mut tree, &key(keycode, false)));
        }
        assert_eq!(actions.counts(), (0, 0, 0));
        // The owner can focus a slot without a click. ScarletUI's dispatcher
        // discovers this native focusable State; it does not implement Tab traversal.
        slot.focused.set(true);
        assert!(slot.focused.get());
        assert!(dispatcher.dispatch(&mut tree, &key(KeyCode::Enter, false)));
        assert!(dispatcher.dispatch(&mut tree, &key(KeyCode::Space, false)));
        assert!(dispatcher.dispatch(&mut tree, &key(KeyCode::F(10), true)));
        assert_eq!(actions.counts(), (1, 1, 1));
        for keycode in [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::F(10),
        ] {
            assert!(!dispatcher.dispatch(&mut tree, &key(keycode, false)));
        }
        assert!(!dispatcher.dispatch(
            &mut tree,
            &Event::Keyboard(KeyEvent::Released {
                keycode: KeyCode::Space,
                modifiers: KeyModifiers::empty(),
            })
        ));
        assert!(!dispatcher.dispatch(&mut tree, &Event::Keyboard(KeyEvent::Char { c: ' ' })));
        assert!(!dispatcher.dispatch(
            &mut tree,
            &Event::Keyboard(KeyEvent::Pressed {
                keycode: KeyCode::Enter,
                modifiers: KeyModifiers {
                    control: true,
                    ..KeyModifiers::empty()
                },
            })
        ));
        slot.focused.set(false);
        assert!(!dispatcher.dispatch(&mut tree, &key(KeyCode::Space, false)));
        assert_eq!(actions.counts(), (1, 1, 1));
    }

    #[test]
    fn native_empty_slot_adds_without_exposing_a_bypass_target() {
        let added = Rc::new(Cell::new(0));
        let output = added.clone();
        let slot = InsertSlot::empty(3, State::new(StateId::new(1702), false), move || {
            output.set(output.get() + 1)
        });
        let (mut tree, mut dispatcher) = tree(&slot);
        for (index, x) in [50, 159, 180].into_iter().enumerate() {
            dispatcher.dispatch(&mut tree, &press(MouseButton::Left, x, 13));
            assert_eq!(added.get(), index);
            dispatcher.dispatch(&mut tree, &release(MouseButton::Left, x, 13));
            assert_eq!(added.get(), index + 1);
        }
        assert!(dispatcher.dispatch(&mut tree, &key(KeyCode::Enter, false)));
        assert_eq!(added.get(), 4);
        assert!(!dispatcher.dispatch(&mut tree, &key(KeyCode::Space, false)));
    }

    #[test]
    fn replacement_during_capture_does_not_activate_the_replacement() {
        let (mut slot, actions) = slot();
        let (mut tree, mut dispatcher) = tree(&slot);
        dispatcher.dispatch(&mut tree, &press(MouseButton::Left, 70, 13));
        slot.name = Some("Replacement effect".into());
        tree.root_mut().unwrap().update(&slot);
        tree.layout(LayoutConstraints::tight(WIDTH, HEIGHT));
        dispatcher.dispatch(&mut tree, &release(MouseButton::Left, 70, 13));
        assert_eq!(actions.counts(), (0, 0, 0));
    }

    #[test]
    fn power_glyph_has_smooth_open_arc_and_vertical_stem_at_native_dpi() {
        let (slot, _) = slot();
        let render = SlotRender::new(slot);
        for scale_milli in [1000, 1500, 2000] {
            let buffer = render.power_raster(scale_milli);
            let side = POWER_RASTER_SIZE * scale_milli / 1000;
            let scale = scale_milli as f32 / 1000.;
            assert_eq!((buffer.width(), buffer.height()), (side, side));
            assert_eq!((buffer.logical_width(), buffer.logical_height()), (14, 14));
            let alpha = |x: u32, y: u32| buffer.as_slice()[(y * side + x) as usize] >> 24;
            let sample = |x: f32, y: f32| {
                alpha(
                    ((x + 7.) * scale - 0.5).round() as u32,
                    ((y + 7.5) * scale - 0.5).round() as u32,
                )
            };
            assert!(
                buffer
                    .as_slice()
                    .iter()
                    .filter(|pixel| (1..255).contains(&(*pixel >> 24)))
                    .count()
                    >= 20,
                "Curved edges must have partial-alpha pixels at {scale_milli}"
            );
            for coordinate in 0..side {
                assert_eq!(alpha(coordinate, 0), 0);
                assert_eq!(alpha(coordinate, side - 1), 0);
                assert_eq!(alpha(0, coordinate), 0);
                assert_eq!(alpha(side - 1, coordinate), 0);
                for x in 0..side {
                    assert_eq!(alpha(x, coordinate), alpha(side - x - 1, coordinate));
                }
            }
            for (x, y) in [(0., -4.), (0., -1.), (-4., 0.), (4., 0.), (0., 4.)] {
                assert!(
                    sample(x, y) > 100,
                    "Missing arc/stem at ({x}, {y}), {scale_milli}"
                );
            }
            for (x, y) in [(-1.5, -3.), (1.5, -3.), (0., 2.)] {
                assert_eq!(
                    sample(x, y),
                    0,
                    "Keep the opening/interior clear at {scale_milli}"
                );
            }
            // Every covered pixel stores the original ink, without a dark
            // premultiplied fringe when the hover background changes.
            let rgb = ui::ACCENT.to_bgra() & 0x00ff_ffff;
            assert!(
                buffer
                    .as_slice()
                    .iter()
                    .all(|pixel| *pixel == 0 || pixel & 0x00ff_ffff == rgb)
            );
        }
        assert!(power_arc_distance(Point::new(0., -4.)) > 3.);
        assert!(power_distance(Point::new(0., -5.)) < 0.);
        assert!(power_distance(Point::new(0., 0.)) < 0.);
        assert!(power_distance(Point::new(0., 2.)) > 1.);
    }

    #[test]
    fn power_glyph_cache_preserves_snapshots_and_reuses_unchanged_native_buffer() {
        let (slot, _) = slot();
        let mut render = SlotRender::new(slot);
        let first = render.power_raster(1000);
        let same = render.power_raster(1000);
        assert!(Arc::ptr_eq(&first, &same));
        let identity = first.identity();
        let revision = first.revision();
        let enabled = first.as_slice().to_vec();
        drop(first);
        drop(same);
        render.hovered = Some(Hit::Power);
        render.slot.selected = true;
        render.slot.focused.set(true);
        let hover = render.power_raster(1000);
        assert_eq!((hover.identity(), hover.revision()), (identity, revision));
        drop(hover);
        render.slot.bypassed = true;
        let bypassed = render.power_raster(1000);
        assert_eq!(bypassed.identity(), identity);
        assert!(bypassed.revision() > revision);
        let muted = ui::MUTED.to_bgra() & 0x00ff_ffff;
        for (&active, &inactive) in enabled.iter().zip(bypassed.as_slice()) {
            assert_eq!(active >> 24, inactive >> 24);
            assert!(inactive == 0 || inactive & 0x00ff_ffff == muted);
        }
        let pixels = bypassed.as_slice().to_vec();
        render.slot.bypassed = false;
        let active = render.power_raster(1000);
        assert_ne!(active.identity(), bypassed.identity());
        assert_eq!(
            bypassed.as_slice(),
            pixels,
            "A live paint snapshot must stay immutable"
        );
        assert_eq!(active.as_slice(), enabled);
        let scaled = render.power_raster(1500);
        assert_ne!(scaled.identity(), active.identity());
        assert_eq!(active.as_slice(), enabled);
    }

    #[test]
    fn native_paint_keeps_compact_power_geometry_and_hover_background() {
        let (slot, _) = slot();
        let mut render = SlotRender::new(slot);
        render.hovered = Some(Hit::Power);
        let mut paint = PaintContext::new();
        assert!(render.paint(&mut paint, Point::new(8., 5.)));
        assert_eq!(render.size(), Size::new(190., 26.));
        assert_eq!(render.hit(147, 13), Some(Hit::Open));
        assert_eq!(render.hit(148, 13), Some(Hit::Power));
        assert_eq!(render.hit(169, 13), Some(Hit::Power));
        assert_eq!(render.hit(170, 13), Some(Hit::Context));
        let rasters: Vec<_> = paint
            .commands()
            .iter()
            .filter_map(|command| match command {
                PaintCommand::DrawBufferRect {
                    dst, src, opacity, ..
                } => Some((dst, src, opacity)),
                _ => None,
            })
            .collect();
        assert_eq!(rasters.len(), 1);
        assert_eq!(*rasters[0].0, Rect::from_xywh(160., 11., 14., 14.));
        assert_eq!(*rasters[0].1, Rect::from_xywh(0., 0., 14., 14.));
        assert_eq!(*rasters[0].2, 1.);
        assert!(paint.commands().iter().any(|command| matches!(command,
            PaintCommand::FillRoundedRect { rect, color, .. }
                if *rect == Rect::from_xywh(158., 9., 18., 18.) && *color == ui::PANEL
        )));
        render.slot.name = None;
        let mut paint = PaintContext::new();
        assert!(render.paint(&mut paint, Point::ZERO));
        assert!(
            !paint
                .commands()
                .iter()
                .any(|command| matches!(command, PaintCommand::DrawBufferRect { .. }))
        );
    }

    #[test]
    fn text_fits_and_native_paint_distinguishes_enabled_bypassed_and_empty() {
        assert_eq!(fit_name("Delay", 120.), "Delay");
        assert_eq!(fit_name("Delay", 0.), "");
        let long = "A very long plug-in name with 日本語 and 🎵";
        let fitted = fit_name(long, 90.);
        assert!(fitted.ends_with('…'));
        assert!(scarlet_ui::graphics::measure_text_sized(&fitted, FONT).0 <= 90);

        let (slot, _) = slot();
        let mut render = SlotRender::new(slot);
        assert_eq!(
            render.layout(LayoutConstraints::loose(300., 50.)),
            Size::new(WIDTH, HEIGHT)
        );
        assert_eq!(render.hit(147, 13), Some(Hit::Open));
        assert_eq!(render.hit(148, 13), Some(Hit::Power));
        assert_eq!(render.hit(169, 13), Some(Hit::Power));
        assert_eq!(render.hit(170, 13), Some(Hit::Context));
        for bypassed in [false, true] {
            render.slot.bypassed = bypassed;
            let mut paint = PaintContext::new();
            assert!(render.paint(&mut paint, Point::ZERO));
            assert!(paint.commands().iter().any(|command| matches!(command,
                scarlet_ui::renderer::PaintCommand::DrawText { text, color, .. }
                    if text == "Stereo delay" && *color == if bypassed { ui::MUTED } else { ui::TEXT }
            )));
        }
        render.slot.name = None;
        render.update_labels();
        assert_eq!(render.hit(159, 13), Some(Hit::Open));
        let mut paint = PaintContext::new();
        render.paint(&mut paint, Point::ZERO);
        assert!(paint.commands().iter().any(|command| matches!(command,
            scarlet_ui::renderer::PaintCommand::DrawText { text, .. } if text == "Add insert"
        )));
    }
}
