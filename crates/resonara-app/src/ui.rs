//! Shared ScarletUI composition and visual tokens.
use scarlet_ui::{prelude::*, views::containers::ViewTuple};
use std::any::Any;

pub const BG: Color = Color::rgb_f32(0.075, 0.083, 0.101);
pub const PANEL: Color = Color::rgb_f32(0.112, 0.123, 0.145);
pub const RAISED: Color = Color::rgb_f32(0.155, 0.168, 0.194);
pub const LINE: Color = Color::rgb_f32(0.225, 0.243, 0.275);
pub const TEXT: Color = Color::rgb_f32(0.90, 0.92, 0.95);
pub const MUTED: Color = Color::rgb_f32(0.60, 0.65, 0.71);
pub const ACCENT: Color = Color::rgb_f32(0.35, 0.77, 0.75);
pub const GOLD: Color = Color::rgb_f32(0.98, 0.74, 0.34);
pub const ROW: f32 = 84.;
pub const HEADER: f32 = 210.;

#[derive(Clone)]
pub struct AnyView(pub Box<dyn View>);
impl AnyView {
    pub fn new(view: impl View + 'static) -> Self {
        Self(Box::new(view))
    }
}
impl View for AnyView {
    fn create_element(&self) -> Box<dyn scarlet_ui::Element> {
        Box::new(scarlet_ui::ComponentElement::new_with_builder(
            self.clone(),
            |s| s.0.clone(),
        ))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        self.0.listenables()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
#[derive(Clone)]
pub struct Children(pub Vec<Box<dyn View>>);
impl ViewTuple for Children {
    fn create_elements(&self) -> Vec<Box<dyn scarlet_ui::Element>> {
        self.0.iter().map(|v| v.create_element()).collect()
    }
    fn clone_views(&self) -> Vec<Box<dyn View>> {
        self.0.clone()
    }
    fn collect_listenables<'a>(&'a self, c: &mut Vec<&'a dyn Listenable>) {
        for v in &self.0 {
            c.extend(v.listenables());
        }
    }
}
pub fn label(text: impl Into<String>) -> Text {
    Text::new(text).font_size(12.).color(TEXT)
}
pub fn caption(text: impl Into<String>) -> Text {
    Text::new(text).font_size(11.).color(MUTED)
}
pub fn button(text: impl Into<String>) -> Button {
    Button::new(text)
        .font_size(12.)
        .padding(6.)
        .background_color(RAISED)
        .text_color(TEXT)
        .border_color(LINE)
}
pub fn field(state: State<String>) -> TextField {
    TextField::new(state)
        .font_size(12.)
        .padding(6.)
        .background_color(BG)
        .text_color(TEXT)
        .border_color(LINE)
        .focused_border_color(ACCENT)
}
pub fn color(index: usize) -> Color {
    [
        ACCENT,
        Color::rgb(0.62, 0.57, 0.92),
        GOLD,
        Color::rgb(0.43, 0.67, 0.94),
        Color::rgb(0.89, 0.51, 0.67),
        Color::rgb(0.55, 0.77, 0.43),
    ][index % 6]
}
pub fn db(gain: f32) -> String {
    if gain <= 0.00001 {
        "−∞ dB".into()
    } else {
        format!("{:+.1} dB", 20. * gain.log10())
    }
}
pub fn pan(value: f32) -> String {
    if value.abs() < 0.0005 {
        "C".into()
    } else {
        let amount = value.abs() * 100.;
        let number = if (amount - amount.round()).abs() < 0.05 {
            format!("{amount:.0}")
        } else {
            format!("{amount:.1}")
        };
        format!("{} {number}", if value < 0. { "L" } else { "R" })
    }
}
pub fn time(seconds: f64) -> String {
    format!("{:02}:{:06.3}", (seconds / 60.) as u64, seconds % 60.)
}

/// Keep physical printable keys from bubbling into DAW shortcuts while editing.
/// ScarletUI commits their text through a later Char/IME event.
pub trait InputGuard: View + Sized + Clone {
    fn input_guard(self) -> AnyView {
        AnyView::new(self.on_key(|_| true))
    }
}
impl<V: View + Clone> InputGuard for V {}

/// A native event boundary that keeps platform shortcut presses from also
/// becoming text. The current winit adapter emits both Pressed and Char for
/// Ctrl/Cmd+letter; editors should receive the shortcut but not that extra Char.
#[derive(Clone)]
pub struct ShortcutBoundary(pub AnyView);
impl View for ShortcutBoundary {
    fn create_element(&self) -> Box<dyn scarlet_ui::Element> {
        Box::new(ShortcutBoundaryElement {
            inner: self.0.create_element(),
            suppress: false,
            horizontal: None,
        })
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        self.0.listenables()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
/// A transparent Element decorator is needed because ScarletUI's generic
/// RenderElement handles keyboard events specially before custom render hooks.
struct ShortcutBoundaryElement {
    inner: Box<dyn scarlet_ui::Element>,
    suppress: bool,
    horizontal: Option<std::rc::Rc<dyn Fn(i32)>>,
}
impl scarlet_ui::Element for ShortcutBoundaryElement {
    fn id(&self) -> scarlet_ui::ElementId {
        self.inner.id()
    }
    fn type_name(&self) -> &str {
        "ShortcutBoundary"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn children(&self) -> &[Box<dyn scarlet_ui::Element>] {
        self.inner.children()
    }
    fn children_mut(&mut self) -> &mut [Box<dyn scarlet_ui::Element>] {
        self.inner.children_mut()
    }
    fn update(&mut self, v: &dyn View) -> scarlet_ui::element::UpdateResult {
        if let Some(v) = v.as_any().downcast_ref::<ShortcutBoundary>() {
            self.horizontal = None;
            self.inner.update(&v.0)
        } else if let Some(v) = v.as_any().downcast_ref::<HorizontalWheel>() {
            self.horizontal = Some(v.1.clone());
            self.inner.update(&v.0)
        } else {
            scarlet_ui::element::UpdateResult::Replaced
        }
    }
    fn rebuild(&mut self) -> scarlet_ui::element::UpdateResult {
        self.inner.rebuild()
    }
    fn mount(&mut self, c: &scarlet_ui::pipeline::MountContext) {
        self.inner.mount(c)
    }
    fn unmount(&mut self) {
        self.inner.unmount()
    }
    fn layout(&mut self, c: scarlet_ui::element::LayoutConstraints) -> Size {
        self.inner.layout(c)
    }
    fn last_layout_constraints(&self) -> Option<scarlet_ui::element::LayoutConstraints> {
        self.inner.last_layout_constraints()
    }
    fn set_last_layout_constraints(&mut self, c: scarlet_ui::element::LayoutConstraints) {
        self.inner.set_last_layout_constraints(c)
    }
    fn position(&self) -> Point {
        self.inner.position()
    }
    fn set_position(&mut self, p: Point) {
        self.inner.set_position(p)
    }
    fn bounds(&self) -> Rect {
        self.inner.bounds()
    }
    fn hit_test(&self, p: Point) -> bool {
        self.inner.hit_test(p)
    }
    fn set_viewport_hint(&mut self, v: Rect) -> bool {
        self.inner.set_viewport_hint(v)
    }
    fn fill_width(&self) -> bool {
        self.inner.fill_width()
    }
    fn fill_height(&self) -> bool {
        self.inner.fill_height()
    }
    fn flex_factor(&self) -> u32 {
        self.inner.flex_factor()
    }
    fn handle_event(
        &mut self,
        e: &scarlet_ui::event::Event,
        phase: scarlet_ui::event::Phase,
    ) -> bool {
        use scarlet_ui::event::{Event, KeyEvent, Phase};
        // After the first tick the dispatcher captures a wheel transaction
        // to this element, so subsequent ticks arrive at Target rather than Capture.
        if phase == Phase::Target {
            if let (
                Some(horizontal),
                Event::Mouse(scarlet_ui::event::MouseEvent::Wheel { delta_x, .. }),
            ) = (&self.horizontal, e)
            {
                if *delta_x != 0 {
                    horizontal(*delta_x);
                    return true;
                }
            }
        }
        if phase == Phase::Capture {
            if let Some(horizontal) = &self.horizontal {
                if let Event::Mouse(scarlet_ui::event::MouseEvent::Wheel { delta_x, .. }) = e {
                    if *delta_x != 0 {
                        horizontal(*delta_x);
                        return true;
                    }
                }
                return false;
            }
            match e {
                Event::Keyboard(KeyEvent::Pressed { modifiers, .. }) => {
                    self.suppress = (modifiers.control && !modifiers.alt) || modifiers.super_key;
                }
                Event::Keyboard(KeyEvent::Char { .. }) if self.suppress => {
                    self.suppress = false;
                    return true;
                }
                Event::Keyboard(KeyEvent::Released { .. }) => {
                    self.suppress = false;
                }
                _ => {}
            }
            return false;
        }
        self.inner.handle_event(e, phase)
    }
}

/// Route horizontal wheel motion before a vertical ScrollView captures it.
#[derive(Clone)]
pub struct HorizontalWheel(pub AnyView, pub std::rc::Rc<dyn Fn(i32)>);
impl View for HorizontalWheel {
    fn create_element(&self) -> Box<dyn scarlet_ui::Element> {
        Box::new(ShortcutBoundaryElement {
            inner: self.0.create_element(),
            suppress: false,
            horizontal: Some(self.1.clone()),
        })
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        self.0.listenables()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
pub fn elide(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.into()
    } else {
        format!(
            "{}…",
            text.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    }
}
pub fn name_label(text: &str, max: usize, size: f32, status: State<String>) -> AnyView {
    let full = text.to_owned();
    AnyView::new(
        label(elide(text, max))
            .font_size(size)
            .on_hover(move || status.set(full.clone())),
    )
}
