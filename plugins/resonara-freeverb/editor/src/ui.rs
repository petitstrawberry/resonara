use scarlet_ui::{prelude::*, views::containers::ViewTuple};
use std::any::Any;
pub const BG: Color = Color::rgb_f32(0.075, 0.083, 0.101);
pub const RAISED: Color = Color::rgb_f32(0.155, 0.168, 0.194);
pub const LINE: Color = Color::rgb_f32(0.225, 0.243, 0.275);
pub const TEXT: Color = Color::rgb_f32(0.90, 0.92, 0.95);
pub const MUTED: Color = Color::rgb_f32(0.60, 0.65, 0.71);
pub const ACCENT: Color = Color::rgb_f32(0.35, 0.77, 0.75);
pub const GOLD: Color = Color::rgb_f32(0.98, 0.74, 0.34);
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
