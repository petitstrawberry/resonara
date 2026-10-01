//! Small paint-only leaves keep animation out of the build/layout dependency graph.
use scarlet_ui::{
    element::{Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult},
    prelude::*,
    renderer::PaintContext,
    state::{InvalidationKind, SubscriptionId},
};
use std::{any::Any, cell::RefCell, sync::Arc, time::Duration};
pub(crate) const PLAYHEAD_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 30);
pub(crate) const METER_INTERVAL: Duration = PLAYHEAD_INTERVAL;
#[derive(Clone)]
pub(crate) struct PaintState<T: 'static>(pub State<T>);
impl<T: 'static> Listenable for PaintState<T> {
    fn subscribe_any(&self, cb: Arc<dyn Fn() + Send + Sync>) -> SubscriptionId {
        self.0.subscribe_any(cb)
    }
    fn unsubscribe(&self, id: SubscriptionId) -> bool {
        self.0.unsubscribe(id)
    }
    fn invalidation_kind(&self) -> InvalidationKind {
        InvalidationKind::Paint
    }
}
#[derive(Clone)]
pub(crate) struct Playhead {
    pub position: PaintState<f64>,
    pub offset: f64,
    pub span: f64,
    pub size: Size,
}
impl Playhead {
    pub fn new(position: State<f64>, offset: f64, span: f64, size: Size) -> Self {
        Self {
            position: PaintState(position),
            offset,
            span,
            size,
        }
    }
}
impl View for Playhead {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |s| CursorRender(s.clone()),
            |r, s| {
                r.0 = s.clone();
                UpdateResult::Updated
            },
            |_| vec![],
        ))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.position]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
struct CursorRender(Playhead);
impl ElementRenderObject for CursorRender {
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
        let x =
            ((self.0.position.0.get() - self.0.offset) / self.0.span) as f32 * self.0.size.width;
        if x >= 0. && x < self.0.size.width {
            ctx.fill_rect(
                Rect::from_xywh(
                    origin.x + x,
                    origin.y,
                    1.5f32.min(self.0.size.width - x),
                    self.0.size.height,
                ),
                crate::ui::TEXT,
            );
        }
        true
    }
}
#[derive(Clone)]
pub(crate) struct Readout {
    pub text: PaintState<String>,
    pub font: f32,
    pub color: Color,
    pub size: Size,
}
impl Readout {
    pub fn new(text: State<String>, font: f32, color: Color, size: Size) -> Self {
        Self {
            text: PaintState(text),
            font,
            color,
            size,
        }
    }
}
impl View for Readout {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |s| ReadoutRender {
                view: s.clone(),
                ink: RefCell::new((String::new(), Point::ZERO)),
            },
            |r, s| {
                r.view = s.clone();
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
struct ReadoutRender {
    view: Readout,
    ink: RefCell<(String, Point)>,
}
impl ElementRenderObject for ReadoutRender {
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
        let mut ink = self.ink.borrow_mut();
        if ink.0 != text {
            *ink = (
                text.clone(),
                crate::fader::ink_center(&text, self.view.font),
            );
        }
        ctx.draw_text(
            Point::new(
                origin.x + self.view.size.width / 2. - ink.1.x,
                origin.y + self.view.size.height / 2. - ink.1.y,
            ),
            &text,
            self.view.color,
            self.view.font,
        );
        true
    }
}
