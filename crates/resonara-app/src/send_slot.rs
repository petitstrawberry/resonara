//! One continuous native send-rack row, with the retained send gain control.
use crate::{send_knob::SendKnob, ui};
use scarlet_ui::{
    element::{Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult},
    event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, Phase},
    prelude::*,
    renderer::PaintContext,
};
use std::{any::Any, rc::Rc};

const WIDTH: f32 = 190.;
const HEIGHT: f32 = 26.;
const FONT: f32 = 10.;
const LEVEL_FONT: f32 = 9.;
const MENU_WIDTH: f32 = 20.;
const KNOB_SIZE: f32 = 24.;
const LEVEL_WIDTH: f32 = 42.;
const PADDING: f32 = 8.;

#[derive(Clone)]
pub struct SendSlot {
    destination: Option<String>,
    level_label: String,
    enabled: bool,
    knob: Option<SendKnob>,
    pub focused: State<bool>,
    open: Rc<dyn Fn()>,
    context: Rc<dyn Fn()>,
}

impl SendSlot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        destination: String,
        level_label: String,
        enabled: bool,
        gain: State<f32>,
        dragging: State<bool>,
        focused: State<bool>,
        gain_changed: impl Fn(f32) + 'static,
        open: impl Fn() + 'static,
        context: impl Fn() + 'static,
    ) -> Self {
        Self {
            destination: Some(destination),
            level_label,
            enabled,
            knob: Some(SendKnob::new(gain, dragging, focused.clone(), gain_changed)),
            focused,
            open: Rc::new(open),
            context: Rc::new(context),
        }
    }

    /// The entire empty native row opens destination selection. It has no knob
    /// or separate add button, and can also be reached with keyboard focus.
    pub fn empty(focused: State<bool>, add: impl Fn() + 'static) -> Self {
        let add: Rc<dyn Fn()> = Rc::new(add);
        Self {
            destination: None,
            level_label: String::new(),
            enabled: false,
            knob: None,
            focused,
            open: add.clone(),
            context: add,
        }
    }

    fn handle_key(&self, event: KeyEvent) -> bool {
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
            KeyCode::F(10) if modifiers.shift => (self.context)(),
            _ => return false,
        }
        true
    }
}

impl View for SendSlot {
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
struct SlotFace(SendSlot);

impl View for SlotFace {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| SlotRender::new(view.0.clone()),
            |render, view| {
                if render.slot.destination != view.0.destination {
                    // A destination replaced under the pointer is a new target.
                    render.pressed = None;
                }
                render.slot = view.0.clone();
                render.update_labels();
                UpdateResult::Updated
            },
            |view| {
                view.0
                    .knob
                    .as_ref()
                    .map(|knob| vec![Box::new(knob.clone()) as Box<dyn View>])
                    .unwrap_or_default()
            },
        ))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Open,
    Gain,
    Context,
}

struct Label {
    text: String,
    ink: Point,
    width: f32,
}

impl Label {
    fn new(text: String, font: f32) -> Self {
        let ink = crate::fader::ink_center(&text, font);
        let width = scarlet_ui::graphics::measure_text_sized(&text, font).0 as f32;
        Self { text, ink, width }
    }
}

struct SlotRender {
    slot: SendSlot,
    size: Size,
    hovered: Option<Hit>,
    pressed: Option<(MouseButton, Hit)>,
    destination: Label,
    level: Label,
}

impl SlotRender {
    fn new(slot: SendSlot) -> Self {
        let mut render = Self {
            slot,
            size: Size::new(WIDTH, HEIGHT),
            hovered: None,
            pressed: None,
            destination: Label::new(String::new(), FONT),
            level: Label::new(String::new(), LEVEL_FONT),
        };
        render.update_labels();
        render
    }

    fn knob_rect(&self) -> Rect {
        let side = KNOB_SIZE
            .min(self.size.height)
            .min((self.size.width - MENU_WIDTH).max(0.));
        Rect::from_xywh(
            (self.size.width - MENU_WIDTH - side).max(0.),
            (self.size.height - side) / 2.,
            side,
            side,
        )
    }

    fn level_right(&self) -> f32 {
        (self.knob_rect().origin.x - 3.).max(0.)
    }

    fn name_right(&self) -> f32 {
        if self.slot.destination.is_some() {
            (self.level_right() - LEVEL_WIDTH - 1.).max(PADDING)
        } else {
            (self.size.width - MENU_WIDTH - PADDING).max(PADDING)
        }
    }

    fn update_labels(&mut self) {
        let destination = fit_label(
            self.slot
                .destination
                .as_deref()
                .unwrap_or("Select destination…"),
            self.name_right() - PADDING,
            FONT,
        );
        if self.destination.text != destination {
            self.destination = Label::new(destination, FONT);
        }
        let level = fit_label(
            &self.slot.level_label,
            LEVEL_WIDTH.min(self.level_right()),
            LEVEL_FONT,
        );
        if self.level.text != level {
            self.level = Label::new(level, LEVEL_FONT);
        }
    }

    fn hit(&self, x: i32, y: i32) -> Option<Hit> {
        let (x, y) = (x as f32, y as f32);
        if x < 0. || y < 0. || x >= self.size.width || y >= self.size.height {
            return None;
        }
        if self.slot.destination.is_none() {
            Some(Hit::Open)
        } else if x >= self.size.width - MENU_WIDTH {
            Some(Hit::Context)
        } else if x >= self.knob_rect().origin.x {
            Some(Hit::Gain)
        } else {
            Some(Hit::Open)
        }
    }

    fn activate(&self, hit: Hit) {
        match hit {
            Hit::Open => (self.slot.open)(),
            Hit::Context => (self.slot.context)(),
            Hit::Gain => {}
        }
    }
}

fn fit_label(text: &str, width: f32, font: f32) -> String {
    let measure = |text: &str| scarlet_ui::graphics::measure_text_sized(text, font).0 as f32;
    if width <= 0. {
        return String::new();
    }
    if measure(text) <= width {
        return text.into();
    }
    if measure("…") > width {
        return String::new();
    }
    let mut fitted = text.to_string();
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

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        let size = self.layout(constraints);
        let knob = self.knob_rect();
        for child in children {
            child.layout(LayoutConstraints::tight(knob.size.width, knob.size.height));
            child.set_position(knob.origin);
        }
        size
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
        let center_y = y + self.size.height / 2.;
        ctx.fill_rounded_rect(
            Rect::new(origin, self.size),
            0.,
            if self.hovered.is_some() {
                ui::RAISED
            } else {
                ui::BG
            },
        );
        ctx.draw_line(
            Point::new(x, y + self.size.height - 0.5),
            Point::new(x + self.size.width, y + self.size.height - 0.5),
            1.,
            ui::LINE,
        );
        if self.slot.focused.get() {
            ctx.stroke_rounded_rect(
                Rect::from_xywh(
                    x + 0.5,
                    y + 0.5,
                    self.size.width - 1.,
                    self.size.height - 1.,
                ),
                0.,
                1.,
                ui::ACCENT,
            );
        }
        ctx.draw_text(
            Point::new(x + PADDING, center_y - self.destination.ink.y),
            &self.destination.text,
            if self.slot.enabled {
                ui::TEXT
            } else {
                ui::MUTED
            },
            FONT,
        );
        if self.slot.destination.is_some() {
            ctx.draw_text(
                Point::new(
                    x + self.level_right() - self.level.width,
                    center_y - self.level.ink.y,
                ),
                &self.level.text,
                if self.slot.enabled {
                    ui::TEXT
                } else {
                    ui::MUTED
                },
                LEVEL_FONT,
            );
        }
        let menu_x = x + self.size.width - MENU_WIDTH / 2.;
        let color = if self.hovered == Some(Hit::Context) {
            ui::TEXT
        } else {
            ui::MUTED
        };
        ctx.draw_line(
            Point::new(menu_x - 3., center_y - 1.),
            Point::new(menu_x, center_y + 2.),
            1.,
            color,
        );
        ctx.draw_line(
            Point::new(menu_x, center_y + 2.),
            Point::new(menu_x + 3., center_y - 1.),
            1.,
            color,
        );
        true
    }

    fn handle_event(&mut self, event: &Event, phase: Phase) -> bool {
        if !matches!(phase, Phase::Target | Phase::Bubble) {
            return false;
        }
        match event {
            Event::Mouse(MouseEvent::Entered { x, y }) => {
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
                // Only left presses have dispatcher capture. A right release
                // elsewhere must not leave a context action armed for re-entry.
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
                if *button == MouseButton::Left && hit == Hit::Gain {
                    return false;
                }
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
    use crate::fader::{gain_from_fraction, gain_to_fraction};
    use scarlet_ui::{ElementTree, EventDispatcher, event::KeyModifiers, renderer::PaintCommand};
    use std::cell::{Cell, RefCell};

    struct Harness {
        slot: SendSlot,
        gain: State<f32>,
        dragging: State<bool>,
        open: Rc<Cell<usize>>,
        context: Rc<Cell<usize>>,
        changes: Rc<RefCell<Vec<f32>>>,
        tree: ElementTree,
        dispatcher: EventDispatcher,
    }

    impl Harness {
        fn new() -> Self {
            let gain = State::new(StateId::new(1811), 0.5);
            let dragging = State::new(StateId::new(1812), false);
            let focused = State::new(StateId::new(1813), false);
            let open = Rc::new(Cell::new(0));
            let context = Rc::new(Cell::new(0));
            let changes = Rc::new(RefCell::new(Vec::new()));
            let output = gain.clone();
            let change_count = changes.clone();
            let opened = open.clone();
            let menu = context.clone();
            let slot = SendSlot::new(
                "Room reverb".into(),
                "−6.0 dB".into(),
                true,
                gain.clone(),
                dragging.clone(),
                focused,
                move |gain| {
                    output.set(gain);
                    change_count.borrow_mut().push(gain);
                },
                move || opened.set(opened.get() + 1),
                move || menu.set(menu.get() + 1),
            );
            let mut tree = ElementTree::new();
            tree.set_root(slot.create_element());
            tree.layout(LayoutConstraints::tight(WIDTH, HEIGHT));
            Self {
                slot,
                gain,
                dragging,
                open,
                context,
                changes,
                tree,
                dispatcher: EventDispatcher::new(),
            }
        }

        fn event(&mut self, event: Event) -> bool {
            self.dispatcher.dispatch(&mut self.tree, &event)
        }

        fn click(&mut self, button: MouseButton, x: i32) {
            assert!(self.event(press(button, x, 13)));
            assert!(self.event(release(button, x, 13)));
        }

        fn update(&mut self) {
            self.tree.root_mut().unwrap().update(&self.slot);
            self.tree.layout(LayoutConstraints::tight(WIDTH, HEIGHT));
        }

        fn counts(&self) -> (usize, usize, usize) {
            (
                self.open.get(),
                self.context.get(),
                self.changes.borrow().len(),
            )
        }
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

    fn slot_element(element: &dyn Element) -> &dyn Element {
        if element
            .render_object()
            .is_some_and(|render| render.as_any().is::<SlotRender>())
        {
            return element;
        }
        element
            .children()
            .iter()
            .find_map(|child| find_slot(child.as_ref()))
            .unwrap()
    }

    fn find_slot(element: &dyn Element) -> Option<&dyn Element> {
        if element
            .render_object()
            .is_some_and(|render| render.as_any().is::<SlotRender>())
        {
            Some(element)
        } else {
            element
                .children()
                .iter()
                .find_map(|child| find_slot(child.as_ref()))
        }
    }

    fn render(tree: &ElementTree) -> &SlotRender {
        slot_element(tree.root().unwrap())
            .render_object()
            .unwrap()
            .as_any()
            .downcast_ref()
            .unwrap()
    }

    fn close(left: f32, right: f32) {
        assert!((left - right).abs() < 0.00001, "{left} != {right}");
    }

    #[test]
    fn compact_inline_layout_retains_the_real_gain_control() {
        let mut h = Harness::new();
        let element = slot_element(h.tree.root().unwrap());
        assert_eq!(element.bounds().size, Size::new(WIDTH, HEIGHT));
        assert_eq!(element.children().len(), 1);
        let knob = &element.children()[0];
        assert!(knob.as_any().is::<scarlet_ui::ComponentElement<SendKnob>>());
        assert_eq!(knob.position(), Point::new(146., 1.));
        assert_eq!(knob.bounds().size, Size::new(24., 24.));
        let knob_id = knob.id();
        let row = render(&h.tree);
        assert!(PADDING + row.destination.width <= row.name_right());
        assert!(row.name_right() < row.level_right() - row.level.width);
        assert!(row.level_right() < row.knob_rect().origin.x);
        for (x, hit) in [
            (0, Hit::Open),
            (145, Hit::Open),
            (146, Hit::Gain),
            (169, Hit::Gain),
            (170, Hit::Context),
            (189, Hit::Context),
        ] {
            assert_eq!(row.hit(x, 13), Some(hit));
        }
        for (x, y) in [(-1, 13), (190, 13), (100, -1), (100, 26)] {
            assert_eq!(row.hit(x, y), None);
        }
        h.slot.level_label = "+1.0 dB".into();
        h.slot.enabled = false;
        h.update();
        assert_eq!(
            slot_element(h.tree.root().unwrap()).children()[0].id(),
            knob_id
        );
        assert_eq!(render(&h.tree).level.text, "+1.0 dB");
    }

    #[test]
    fn name_and_chevron_activate_on_release_and_repeated_clicks_are_stable() {
        let mut h = Harness::new();
        for expected in 1..=4 {
            assert!(h.event(press(MouseButton::Left, 50, 13)));
            assert_eq!(h.open.get(), expected - 1);
            assert!(!h.dragging.get());
            h.update();
            assert!(h.event(release(MouseButton::Left, 50, 13)));
            assert_eq!(h.counts(), (expected, 0, 0));
        }
        for expected in 1..=3 {
            h.click(MouseButton::Left, 180);
            assert_eq!(h.counts(), (4, expected, 0));
        }
        h.event(release(MouseButton::Left, 180, 13));
        assert_eq!(h.counts(), (4, 3, 0));
    }

    #[test]
    fn crossing_targets_outside_release_and_cancel_never_activate() {
        let mut h = Harness::new();
        for (start, end) in [
            (50, 180),
            (180, 50),
            (50, 158),
            (180, 158),
            (50, -1),
            (50, 190),
        ] {
            h.event(press(MouseButton::Left, start, 13));
            h.event(release(MouseButton::Left, end, 13));
            assert_eq!(h.counts(), (0, 0, 0));
        }
        for y in [-1, 26] {
            h.event(press(MouseButton::Left, 50, 13));
            h.event(release(MouseButton::Left, 50, y));
        }
        h.event(press(MouseButton::Left, 180, 13));
        assert!(h.event(Event::Mouse(MouseEvent::ButtonCancelled {
            button: MouseButton::Left,
            x: 180,
            y: 13
        })));
        h.event(release(MouseButton::Left, 180, 13));
        assert_eq!(h.counts(), (0, 0, 0));
        h.click(MouseButton::Left, 180);
        assert_eq!(h.counts(), (0, 1, 0));
    }

    #[test]
    fn right_click_on_destination_level_knob_or_chevron_opens_only_context() {
        let mut h = Harness::new();
        for (index, x) in [50, 122, 158, 180].into_iter().enumerate() {
            assert!(h.event(press(MouseButton::Right, x, 13)));
            assert_eq!(h.context.get(), index);
            assert!(!h.dragging.get());
            assert!(h.event(release(MouseButton::Right, x, 13)));
            assert_eq!(h.counts(), (0, index + 1, 0));
        }
        h.event(press(MouseButton::Right, 158, 13));
        h.event(Event::Mouse(MouseEvent::Moved { x: 220, y: 13 }));
        h.event(release(MouseButton::Right, 220, 13));
        h.event(Event::Mouse(MouseEvent::Moved { x: 158, y: 13 }));
        h.event(release(MouseButton::Right, 158, 13));
        assert_eq!(h.counts(), (0, 4, 0));
        h.event(press(MouseButton::Right, 158, 13));
        h.event(Event::Mouse(MouseEvent::ButtonCancelled {
            button: MouseButton::Right,
            x: 158,
            y: 13,
        }));
        h.event(release(MouseButton::Right, 158, 13));
        assert_eq!(h.counts(), (0, 4, 0));
    }

    #[test]
    fn captured_relative_gain_drag_survives_label_rebuild_and_cancels() {
        let mut h = Harness::new();
        let initial = h.gain.get();
        let knob_id = slot_element(h.tree.root().unwrap()).children()[0].id();
        h.event(press(MouseButton::Left, 158, 13));
        assert!(h.dragging.get());
        assert_eq!(h.gain.get(), initial);
        h.event(Event::Mouse(MouseEvent::Moved { x: 220, y: -11 }));
        close(
            h.gain.get(),
            gain_from_fraction(gain_to_fraction(initial) + 0.2),
        );
        h.slot.level_label = ui::db(h.gain.get());
        h.update();
        assert_eq!(
            slot_element(h.tree.root().unwrap()).children()[0].id(),
            knob_id
        );
        h.event(Event::Mouse(MouseEvent::Moved { x: 220, y: -23 }));
        close(
            h.gain.get(),
            gain_from_fraction(gain_to_fraction(initial) + 0.3),
        );
        assert!(h.event(Event::Mouse(MouseEvent::ButtonCancelled {
            button: MouseButton::Left,
            x: 220,
            y: -23
        })));
        assert_eq!(h.gain.get(), initial);
        assert!(!h.dragging.get());
        assert_eq!((h.open.get(), h.context.get()), (0, 0));
        h.event(release(MouseButton::Left, 50, 13));
        assert_eq!((h.open.get(), h.context.get()), (0, 0));
        h.click(MouseButton::Left, 158);
        assert_eq!(h.gain.get(), initial);
        assert!(!h.dragging.get());
    }

    #[test]
    fn captured_drag_releases_outside_after_fresh_gain_view_reconciliation() {
        let mut h = Harness::new();
        let initial_fraction = gain_to_fraction(h.gain.get());
        h.event(press(MouseButton::Left, 158, 13));
        h.event(Event::Mouse(MouseEvent::Moved { x: 158, y: 1 }));
        let changed = h.slot.knob.as_ref().unwrap().changed.clone();
        // Application reconciliation constructs a new SendKnob, not merely a
        // clone of the old view. Its native child must keep capture/drag origin.
        h.slot.knob = Some(SendKnob::new(
            h.gain.clone(),
            h.dragging.clone(),
            h.slot.focused.clone(),
            move |gain| changed(gain),
        ));
        h.slot.level_label = ui::db(h.gain.get());
        h.update();
        h.event(Event::Mouse(MouseEvent::Moved { x: 220, y: -11 }));
        close(h.gain.get(), gain_from_fraction(initial_fraction + 0.2));
        assert!(h.event(release(MouseButton::Left, 220, -11)));
        assert!(!h.dragging.get());
        assert_eq!((h.open.get(), h.context.get()), (0, 0));
        let committed = h.gain.get();
        h.event(release(MouseButton::Left, 50, 13));
        assert_eq!(h.gain.get(), committed);
        assert_eq!((h.open.get(), h.context.get()), (0, 0));
    }

    #[test]
    fn gain_keyboard_and_row_menu_shortcuts_keep_the_existing_behavior() {
        let mut h = Harness::new();
        assert!(!h.event(key(KeyCode::Up, false)));
        assert_eq!(h.gain.get(), 0.5);
        h.click(MouseButton::Left, 158);
        assert!(h.slot.focused.get());
        assert!(h.event(key(KeyCode::Home, false)));
        assert_eq!(h.gain.get(), 1.);
        assert!(h.event(key(KeyCode::Down, true)));
        close(h.gain.get(), 10f32.powf(-0.1 / 20.));
        assert!(h.event(key(KeyCode::End, false)));
        assert_eq!(h.gain.get(), 0.);
        assert!(h.event(key(KeyCode::Enter, false)));
        assert!(h.event(key(KeyCode::F(10), true)));
        assert_eq!((h.open.get(), h.context.get()), (1, 1));
        h.event(press(MouseButton::Left, 158, 13));
        h.event(Event::Mouse(MouseEvent::Moved { x: 158, y: -11 }));
        assert!(h.gain.get() > 0.);
        assert!(h.event(key(KeyCode::Escape, false)));
        assert_eq!(h.gain.get(), 0.);
        assert!(!h.dragging.get());
    }

    #[test]
    fn empty_native_row_is_one_full_width_add_target_without_gain_control() {
        let added = Rc::new(Cell::new(0));
        let callback = added.clone();
        let slot = SendSlot::empty(State::new(StateId::new(1821), false), move || {
            callback.set(callback.get() + 1)
        });
        let mut tree = ElementTree::new();
        tree.set_root(slot.create_element());
        tree.layout(LayoutConstraints::tight(WIDTH, HEIGHT));
        let mut dispatcher = EventDispatcher::new();
        assert!(slot_element(tree.root().unwrap()).children().is_empty());
        for x in [1, 50, 158, 189] {
            dispatcher.dispatch(&mut tree, &press(MouseButton::Left, x, 13));
            dispatcher.dispatch(&mut tree, &release(MouseButton::Left, x, 13));
        }
        assert_eq!(added.get(), 4);
        assert!(dispatcher.dispatch(&mut tree, &key(KeyCode::Enter, false)));
        assert_eq!(added.get(), 5);
        let mut paint = PaintContext::new();
        render(&tree).paint(&mut paint, Point::ZERO);
        assert_eq!(
            paint
                .commands()
                .iter()
                .filter(|command| matches!(command, PaintCommand::DrawText { .. }))
                .count(),
            1
        );
        assert!(paint.commands().iter().any(|command| matches!(command, PaintCommand::DrawText { text, color, .. } if text == "Select destination…" && *color == ui::MUTED)));
    }

    #[test]
    fn text_is_bounded_and_disabled_rows_have_muted_native_labels() {
        let mut h = Harness::new();
        h.slot.destination = Some("A very long 日本語 🎵 destination name".into());
        h.slot.level_label = "−100.0 dB".into();
        h.slot.enabled = false;
        h.update();
        let row = render(&h.tree);
        assert!(row.destination.text.ends_with('…'));
        assert!(PADDING + row.destination.width <= row.name_right());
        assert!(row.level.width <= LEVEL_WIDTH);
        let mut paint = PaintContext::new();
        row.paint(&mut paint, Point::ZERO);
        for command in paint.commands() {
            if let PaintCommand::DrawText { color, .. } = command {
                assert_eq!(*color, ui::MUTED);
            }
        }
        assert_eq!(fit_label("anything", 0., FONT), "");
        let mut render = SlotRender::new(h.slot);
        assert_eq!(
            render.layout(LayoutConstraints::loose(300., 60.)),
            Size::new(WIDTH, HEIGHT)
        );
        render.layout(LayoutConstraints::tight(120., HEIGHT));
        assert!(PADDING + render.destination.width <= render.name_right());
        assert!(render.level_right() < render.knob_rect().origin.x);
    }

    #[test]
    fn destination_replaced_during_name_press_does_not_activate_new_target() {
        let mut h = Harness::new();
        h.event(press(MouseButton::Left, 50, 13));
        h.slot.destination = Some("Replacement bus".into());
        h.update();
        h.event(release(MouseButton::Left, 50, 13));
        assert_eq!(h.counts(), (0, 0, 0));
    }
}
