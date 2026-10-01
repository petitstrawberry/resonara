//! A counter with a dedicated monospace face and stable font-metric alignment.
use crate::{animation::PaintState, ui};
use scarlet_ui::{
    buffer::Buffer,
    element::{Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult},
    graphics::{self, Canvas, FontStack},
    prelude::*,
    renderer::PaintContext,
};
use std::{
    any::Any,
    cell::RefCell,
    sync::{Arc, OnceLock},
};

const FONT_SIZE: f32 = 24.;
pub(crate) fn font() -> &'static FontStack {
    static FONT: OnceLock<FontStack> = OnceLock::new();
    FONT.get_or_init(|| {
        FontStack::new(include_bytes!("../assets/fonts/DejaVuSansMono.ttf"))
            .expect("the bundled counter font is valid")
    })
}

#[derive(Clone)]
pub(crate) struct Counter {
    text: PaintState<String>,
    size: Size,
}
impl Counter {
    pub fn new(text: State<String>) -> Self {
        Self {
            text: PaintState(text),
            size: Size::new(ui::COUNTER_WIDTH, ui::CONTROL_HEIGHT),
        }
    }
}
impl View for Counter {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| CounterRender {
                view: view.clone(),
                raster: RefCell::new(None),
            },
            |render, view| {
                render.view = view.clone();
                UpdateResult::Updated
            },
            |_| vec![],
        ))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.text]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
struct Raster {
    text: String,
    size: Size,
    scale: u32,
    buffer: Arc<Buffer>,
}
pub(crate) struct CounterRender {
    view: Counter,
    raster: RefCell<Option<Raster>>,
}
impl CounterRender {
    #[cfg(test)]
    pub(crate) fn text(&self) -> String {
        self.view.text.0.get()
    }
}
impl ElementRenderObject for CounterRender {
    fn layout(&mut self, c: LayoutConstraints) -> Size {
        self.view.size = self.view.size.constrain(
            Size::new(c.min_width, c.min_height),
            Size::new(c.max_width, c.max_height),
        );
        self.view.size
    }
    fn size(&self) -> Size {
        self.view.size
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn render(&mut self) {}
    fn paint(&self, ctx: &mut PaintContext<'_>, origin: Point) -> bool {
        let text = self.view.text.0.get();
        let scale = graphics::current_scale_milli();
        let size = self.view.size;
        let mut raster = self.raster.borrow_mut();
        if raster
            .as_ref()
            .is_none_or(|r| r.text != text || r.size != size || r.scale != scale)
        {
            let mut buffer = Buffer::from_logical_dimensions_with_scale(
                size.width.ceil() as u32,
                size.height.ceil() as u32,
                scale,
            );
            let (width, height) =
                graphics::measure_text_sized_with_font_stack(&text, FONT_SIZE, font());
            // Right alignment also keeps the units digit anchored when the sample count gains a digit.
            let x = (size.width - 8. - width as f32).round() as i32;
            let y = ((size.height - height as f32) / 2.).round() as i32;
            Canvas::for_buffer(&mut buffer).draw_text_sized_with_font_stack(
                x,
                y,
                &text,
                ui::ACCENT,
                FONT_SIZE,
                font(),
            );
            *raster = Some(Raster {
                text,
                size,
                scale,
                buffer: Arc::new(buffer),
            });
        }
        let buffer = raster.as_ref().unwrap().buffer.clone();
        ctx.draw_buffer_rect_shared(
            Rect::new(origin, size),
            Rect::new(Point::ZERO, size),
            buffer,
            1.,
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn digits_have_equal_advances_and_counter_reuses_its_retained_raster() {
        let expected =
            graphics::measure_text_sized_with_font_stack("000.00.000", FONT_SIZE, font());
        for digit in '0'..='9' {
            let value = format!("{0}{0}{0}.{0}{0}.{0}{0}{0}", digit);
            assert_eq!(
                graphics::measure_text_sized_with_font_stack(&value, FONT_SIZE, font()),
                expected
            );
        }
        let text = crate::state(9800, "111.11.111".to_string());
        let view = Counter::new(text.clone());
        assert!(
            view.listenables()
                .iter()
                .all(|s| s.invalidation_kind() == scarlet_ui::state::InvalidationKind::Paint)
        );
        let mut element = view.create_element();
        element.layout(LayoutConstraints::tight(
            ui::COUNTER_WIDTH,
            ui::CONTROL_HEIGHT,
        ));
        let paint = || {
            let mut context = PaintContext::new();
            element
                .render_object()
                .unwrap()
                .paint(&mut context, Point::ZERO);
            let buffer = context.buffers()[0].as_buffer();
            assert!(buffer.data().iter().any(|&b| b != 0));
            (buffer.identity(), context.commands().to_vec())
        };
        let first = paint();
        assert_eq!(paint().0, first.0);
        for value in [
            "888.88.888",
            "11:11.111",
            "88:88.888",
            "9",
            "10",
            "999999999",
        ] {
            text.set(value.to_string());
            let next = paint();
            assert_eq!(
                element.bounds().size,
                Size::new(ui::COUNTER_WIDTH, ui::CONTROL_HEIGHT)
            );
            assert_eq!(paint().0, next.0);
            let [scarlet_ui::renderer::PaintCommand::DrawBufferRect { dst, .. }] =
                next.1.as_slice()
            else {
                panic!("the counter uses one retained buffer");
            };
            assert_eq!(dst.size, Size::new(ui::COUNTER_WIDTH, ui::CONTROL_HEIGHT));
        }
    }
}
