//! Static tick marks share one render leaf, independent of transport animation.
use crate::{
    timeline::{RulerTick, TickKind},
    ui,
};
use scarlet_ui::{
    element::{Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult},
    prelude::*,
    renderer::PaintContext,
};
use std::any::Any;
#[derive(Clone)]
pub(crate) struct Marks {
    pub ticks: Vec<RulerTick>,
    pub start: f64,
    pub span: f64,
    pub size: Size,
}
impl View for Marks {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |v| MarksRender(v.clone()),
            |r, v| {
                r.0 = v.clone();
                UpdateResult::Updated
            },
            |_| vec![],
        ))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
struct MarksRender(Marks);
impl ElementRenderObject for MarksRender {
    fn layout(&mut self, c: LayoutConstraints) -> Size {
        self.0.size = self.0.size.constrain(
            Size::new(c.min_width, c.min_height),
            Size::new(c.max_width, c.max_height),
        );
        self.0.size
    }
    fn size(&self) -> Size {
        self.0.size
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn render(&mut self) {}
    fn paint(&self, ctx: &mut PaintContext<'_>, origin: Point) -> bool {
        for tick in &self.0.ticks {
            let x = ((tick.seconds - self.0.start) / self.0.span) as f32 * self.0.size.width;
            if x < 0. || x >= self.0.size.width {
                continue;
            }
            let length = tick.kind.length().min(self.0.size.height);
            let color = if matches!(tick.kind, TickKind::Bar | TickKind::Major) {
                ui::TEXT
            } else {
                ui::MUTED
            };
            ctx.fill_rect(
                Rect::from_xywh(
                    origin.x + x,
                    origin.y + self.0.size.height - length,
                    1f32.min(self.0.size.width - x),
                    length,
                ),
                color,
            );
        }
        true
    }
}
