//! Generic embedded SWS GUI parent. Foreign plugins use only the documented C ABI.
use super::*;
use scarlet_clap_gui::{API, Input, ParentV1};
use scarlet_ui::{
    buffer::Buffer,
    element::{ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult},
    event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, Phase, WindowEvent},
    pipeline::RenderingPipeline,
    platform::{PlatformBackend, PlatformWindow, WindowCreateRequest},
    prelude::{Color, Point, Rect, Size, State, View, ViewExt, Window},
    renderer::PaintContext,
};
use std::{cell::RefCell, collections::VecDeque, rc::Rc, sync::Arc};
struct Parent {
    frame: State<Arc<Buffer>>,
    input: RefCell<VecDeque<Input>>,
    scale: Cell<u32>,
    width: u32,
    height: u32,
}
impl Parent {
    fn enqueue(&self, input: Input) {
        let mut queue = self.input.borrow_mut();
        // Keep a bounded queue and coalesce motion without dropping release events.
        if input.kind == 3 && queue.back().is_some_and(|i| i.kind == 3) {
            queue.pop_back();
        }
        if queue.len() >= 256 {
            if let Some(i) = queue.iter().position(|i| i.kind == 3) {
                queue.remove(i);
            } else {
                queue.pop_front();
            }
        }
        queue.push_back(input);
    }
}

unsafe extern "C" fn parent_scale(p: *mut c_void) -> u32 {
    unsafe { &*p.cast::<Parent>() }.scale.get()
}
unsafe extern "C" fn poll(p: *mut c_void, out: *mut Input) -> bool {
    if out.is_null() {
        return false;
    }
    let Some(i) = unsafe { &*p.cast::<Parent>() }
        .input
        .borrow_mut()
        .pop_front()
    else {
        return false;
    };
    unsafe {
        *out = i;
    }
    true
}
unsafe extern "C" fn present(
    p: *mut c_void,
    pixels: *const u8,
    w: u32,
    h: u32,
    scale: u32,
) -> bool {
    let parent = unsafe { &*p.cast::<Parent>() };
    let ew = (parent.width as u64 * scale as u64 + 999) / 1000;
    let eh = (parent.height as u64 * scale as u64 + 999) / 1000;
    if pixels.is_null()
        || scale == 0
        || scale != parent.scale.get()
        || w as u64 != ew
        || h as u64 != eh
        || w > 8192
        || h > 8192
    {
        return false;
    }
    let mut buffer = Buffer::from_logical_dimensions_with_scale(parent.width, parent.height, scale);
    let data = buffer.data_mut();
    unsafe {
        core::ptr::copy_nonoverlapping(pixels, data.as_mut_ptr(), data.len());
    }
    parent.frame.set(Arc::new(buffer));
    true
}
#[derive(Clone)]
struct Content(Rc<Parent>);
impl View for Content {
    fn create_element(&self) -> Box<dyn scarlet_ui::Element> {
        Box::new(RenderElement::new(
            self.clone(),
            ContentRender {
                parent: self.0.clone(),
                size: Size::new(self.0.width as f32, self.0.height as f32),
            },
        ))
    }
    fn listenables(&self) -> Vec<&dyn scarlet_ui::Listenable> {
        vec![&self.0.frame]
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
struct ContentRender {
    parent: Rc<Parent>,
    size: Size,
}
impl ElementRenderObject for ContentRender {
    fn layout(&mut self, c: LayoutConstraints) -> Size {
        self.size = Size::new(self.parent.width as f32, self.parent.height as f32).constrain(
            Size::new(c.min_width, c.min_height),
            Size::new(c.max_width, c.max_height),
        );
        self.size
    }
    fn size(&self) -> Size {
        self.size
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn render(&mut self) {}
    fn paint(&self, c: &mut PaintContext<'_>, p: Point) -> bool {
        c.draw_buffer_rect_shared(
            Rect::new(p, self.size),
            Rect::from_xywh(0., 0., self.parent.width as f32, self.parent.height as f32),
            self.parent.frame.get(),
            1.,
        );
        true
    }
    fn handle_event(&mut self, e: &Event, phase: Phase) -> bool {
        if !matches!(phase, Phase::Target) {
            return false;
        }
        let Some(input) = input(e) else {
            return false;
        };
        self.parent.enqueue(input);
        true
    }
    fn update(&mut self, _: &dyn View) -> UpdateResult {
        UpdateResult::Updated
    }
}
fn input(e: &Event) -> Option<Input> {
    let mut i = Input::default();
    match e {
        Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x,
            y,
            click_count,
        }) => {
            i.kind = 1;
            i.x = *x;
            i.y = *y;
            i.click_count = *click_count as u32;
        }
        Event::Mouse(MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x,
            y,
            click_count,
        }) => {
            i.kind = 2;
            i.x = *x;
            i.y = *y;
            i.click_count = *click_count as u32;
        }
        Event::Mouse(MouseEvent::ButtonCancelled {
            button: MouseButton::Left,
            x,
            y,
            ..
        }) => {
            i.kind = 7;
            i.x = *x;
            i.y = *y;
        }
        Event::Mouse(MouseEvent::Moved { x, y }) => {
            i.kind = 3;
            i.x = *x;
            i.y = *y;
        }
        Event::Keyboard(KeyEvent::Char { c }) => {
            i.kind = 6;
            i.key = *c as u32;
        }
        Event::Keyboard(
            KeyEvent::Pressed { keycode, modifiers } | KeyEvent::Released { keycode, modifiers },
        ) => {
            i.kind = if matches!(e, Event::Keyboard(KeyEvent::Pressed { .. })) {
                4
            } else {
                5
            };
            i.modifiers = modifiers.shift as u32
                | (modifiers.control as u32) << 1
                | (modifiers.alt as u32) << 2
                | (modifiers.super_key as u32) << 3;
            i.key = match keycode {
                KeyCode::Escape => 1,
                KeyCode::Enter => 2,
                KeyCode::Tab => 3,
                KeyCode::Backspace => 4,
                KeyCode::Delete => 5,
                KeyCode::Left => 6,
                KeyCode::Right => 7,
                KeyCode::Up => 8,
                KeyCode::Down => 9,
                KeyCode::Home => 10,
                KeyCode::End => 11,
                KeyCode::Space => 12,
                KeyCode::Char(c) => 0x10000 + *c as u32,
                _ => return None,
            };
        }
        _ => return None,
    }
    Some(i)
}
pub(crate) struct Editor {
    window: Box<dyn PlatformWindow>,
    pipeline: RenderingPipeline,
    parent: Rc<Parent>,
    _bridge: Box<ParentV1>,
    api: clap_plugin_gui,
    destroyed: bool,
}
pub(crate) fn open(i: &Instance) -> Result<bool> {
    i.context.check_owner()?;
    if let Some(editor) = unsafe { &mut *i.editor.get() }.as_mut() {
        let _ = editor.window.restore();
        let _ = editor.window.focus();
        unsafe {
            editor.api.show.unwrap()(i.plugin);
        }
        return Ok(true);
    }
    let Some(api) = (unsafe { i.extension::<clap_plugin_gui>(CLAP_EXT_GUI) }) else {
        return Ok(false);
    };
    if !support(&api, i.plugin, API).embedded {
        return Ok(false);
    }
    if !unsafe { api.create.unwrap()(i.plugin, API.as_ptr(), false) } {
        return Err(Error::new("CLAP GUI creation failed"));
    }
    let result = (|| {
        let (mut w, mut h) = (0, 0);
        if !unsafe { api.get_size.unwrap()(i.plugin, &mut w, &mut h) }
            || !(1..=4096).contains(&w)
            || !(1..=4096).contains(&h)
        {
            return Err(Error::new("Invalid CLAP GUI size"));
        }
        let mut backend = scarlet_ui::SwsBackend::new();
        let scale = backend.output_scale_milli();
        let parent = Rc::new(Parent {
            frame: State::new(
                scarlet_ui::state::generate_state_id(),
                Arc::new(Buffer::from_logical_dimensions_with_scale(w, h, scale)),
            ),
            input: RefCell::new(VecDeque::new()),
            scale: Cell::new(scale),
            width: w,
            height: h,
        });
        let decoration = scarlet_ui::platform::WindowDecoration::CUSTOM;
        let chrome =
            scarlet_ui::views::WindowContentLayout::for_decoration(decoration).decoration_size();
        let keyboard_parent = parent.clone();
        let content = Window::new(
            format!("{} · Resonara", i.descriptor.name),
            Content(parent.clone())
                .focusable(State::new(scarlet_ui::state::generate_state_id(), false))
                .on_key(move |event| {
                    if let Some(input) = input(&Event::Keyboard(event)) {
                        keyboard_parent.enqueue(input);
                        true
                    } else {
                        false
                    }
                }),
        )
        .decoration(decoration)
        .size(Size::new(w as f32 + chrome.width, h as f32 + chrome.height))
        .resizable(false)
        .background_color(Color::rgb_f32(0.075, 0.083, 0.101));
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(content.create_element());
        let info = pipeline.layout_initial();
        let limits = scarlet_ui::element::WindowSizeLimits {
            min: Some(info.size),
            max: Some(info.size),
            resizable: false,
        };
        // Custom chrome paints translucent corners and shadows around the
        // opaque plugin content. SWS must blend alpha for the whole surface.
        let opaque = info.platform_surface_is_opaque();
        let mut window = backend
            .create_window(WindowCreateRequest {
                app_id: info.app_id,
                title: info.title,
                size: info.size,
                size_limits: limits,
                window_type: info.window_type,
                menu_titles: String::new(),
                focus_on_create: true,
                active_on_focus: true,
                opaque,
                decoration: info.decoration,
                placement: info.placement,
                window_geometry_insets: info.window_geometry_insets,
            })
            .map_err(|e| Error::new(format!("SWS GUI parent: {e:?}")))?;
        window
            .set_resizable(false)
            .map_err(|e| Error::new(format!("SWS GUI limits: {e:?}")))?;
        pipeline.set_scale_milli(window.output_scale_milli());
        pipeline.resize(window.size());
        if let Some(paint) = window
            .take_paint_backend()
            .map_err(|e| Error::new(format!("SWS GUI renderer: {e:?}")))?
        {
            pipeline.set_paint_backend(paint);
        }
        let bridge = Box::new(ParentV1 {
            version: 1,
            context: Rc::as_ptr(&parent).cast_mut().cast(),
            scale_milli: parent_scale,
            poll_input: poll,
            present,
        });
        Ok(Editor {
            window,
            pipeline,
            parent,
            _bridge: bridge,
            api,
            destroyed: false,
        })
    })();
    match result {
        Ok(mut editor) => {
            let parent_window = clap_sys::ext::gui::clap_window {
                api: API.as_ptr(),
                specific: clap_sys::ext::gui::clap_window_handle {
                    ptr: (&mut *editor._bridge as *mut ParentV1).cast(),
                },
            };
            i.context.gui.resize.store(0, Ordering::Relaxed);
            i.context.gui.visibility.store(0, Ordering::Relaxed);
            i.context.gui.closed.store(false, Ordering::Relaxed);
            i.context.gui.alive.store(true, Ordering::Relaxed);
            if !unsafe {
                api.set_parent.unwrap()(i.plugin, &parent_window) && api.show.unwrap()(i.plugin)
            } {
                editor.destroy(i); // Parent must remain valid until gui.destroy returns.
                i.context.gui.alive.store(false, Ordering::Relaxed);
                return Err(Error::new("CLAP SWS GUI embedding failed"));
            }
            unsafe {
                *i.editor.get() = Some(editor);
            }
            Ok(true)
        }
        Err(e) => {
            unsafe {
                api.destroy.unwrap()(i.plugin);
            }
            Err(e)
        }
    }
}
impl Editor {
    pub(crate) fn destroy(&mut self, i: &Instance) {
        if !self.destroyed {
            unsafe {
                self.api.hide.unwrap()(i.plugin);
                self.api.destroy.unwrap()(i.plugin);
            }
            self.destroyed = true;
            self.pipeline.teardown();
            let _ = self.window.close();
        }
    }
    fn window_action(&mut self, event: &Event) -> bool {
        match event {
            Event::Window(WindowEvent::CloseRequested) => return true,
            Event::Window(WindowEvent::MoveRequested) => {
                let _ = self.window.request_move();
            }
            Event::Window(WindowEvent::MinimizeRequested) => {
                let _ = self.window.minimize();
            }
            Event::Window(WindowEvent::RestoreRequested) => {
                let _ = self.window.restore();
            }
            _ => {}
        }
        false
    }
    pub(crate) fn service(&mut self, i: &Instance) -> bool {
        match i.context.gui.visibility.swap(0, Ordering::AcqRel) {
            1 => {
                let _ = self.window.restore();
                let _ = self.window.focus();
                unsafe {
                    self.api.show.unwrap()(i.plugin);
                }
            }
            2 => {
                unsafe {
                    self.api.hide.unwrap()(i.plugin);
                }
                let _ = self.window.minimize();
            }
            _ => {}
        }
        for _ in 0..64 {
            let Some(event) = self.window.poll_event() else {
                break;
            };
            if self.window_action(&event) {
                return true;
            }
            self.pipeline.handle_event(&event);
            for emitted in self.pipeline.take_emitted_events() {
                if self.window_action(&emitted) {
                    return true;
                }
            }
        }
        self.parent.scale.set(self.window.output_scale_milli());
        if self.pipeline.scale_milli() != self.parent.scale.get() {
            self.pipeline.set_scale_milli(self.parent.scale.get());
        }
        if self.pipeline.has_dirty() {
            match self.pipeline.render_for_present() {
                Ok(scarlet_ui::renderer::PresentedFrame::Cpu { buffer, damage }) => {
                    self.window.present_with_damage(buffer, damage)
                }
                Ok(_) => {}
                Err(e) => eprintln!("CLAP SWS GUI: {e:?}"),
            }
        }
        false
    }
}
