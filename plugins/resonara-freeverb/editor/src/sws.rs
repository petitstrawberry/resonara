//! Embedded SWS content driven by CLAP host timers; no window/event loop ownership.
use crate::{FreeverbEditor, LiveParameters, WIDTH};
use alloc::rc::Rc;
use scarlet_clap_gui::{Input, ParentV1};
use scarlet_ui::{
    event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent},
    pipeline::RenderingPipeline,
    platform::WindowDecoration,
    prelude::*,
};
pub const HEIGHT: u32 = 510;
pub struct SwsEditor {
    pipeline: RenderingPipeline,
    editor: FreeverbEditor,
    parent: Option<ParentV1>,
    visible: bool,
    parameters: LiveParameters,
    change: Rc<dyn Fn([f64; 5])>,
}
impl SwsEditor {
    pub fn new(values: [f64; 5], change: impl Fn([f64; 5]) + 'static) -> Option<Self> {
        let editor = FreeverbEditor::new(values).ok()?.live();
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(
            Window::new("Freeverb", editor.clone())
                .decoration(WindowDecoration::NONE)
                .size(Size::new(WIDTH, HEIGHT as f32))
                .background_color(crate::ui::BG)
                .create_element(),
        );
        pipeline.layout_initial();
        Some(Self {
            pipeline,
            editor,
            parent: None,
            visible: false,
            parameters: LiveParameters::new(values),
            change: Rc::new(change),
        })
    }
    pub unsafe fn set_parent(&mut self, parent: *mut core::ffi::c_void) -> bool {
        let Some(p) = (unsafe { parent.cast::<ParentV1>().as_ref() }) else {
            return false;
        };
        if p.version != 1 {
            return false;
        }
        self.parent = Some(*p);
        true
    }
    pub fn show(&mut self, visible: bool) {
        self.visible = visible;
        if visible {
            self.pipeline.request_redraw();
        }
    }
    pub fn sync(&mut self, values: [f64; 5]) {
        if self.parameters.observe(values) {
            self.editor.set_values(values);
        }
        let Some(p) = self.parent.filter(|_| self.visible) else {
            return;
        };
        let scale = unsafe { (p.scale_milli)(p.context) };
        if scale != self.pipeline.scale_milli() {
            self.pipeline.set_scale_milli(scale);
        }
        for _ in 0..64 {
            let mut input = Input::default();
            if !unsafe { (p.poll_input)(p.context, &mut input) } {
                break;
            }
            if let Some(e) = event(input) {
                self.pipeline.handle_event(&e);
            }
        }
        if let Ok(v) = self.editor.values() {
            if let Some(values) = self.parameters.changes(v) {
                (self.change)(values);
            }
        }
        if self.pipeline.has_dirty() {
            if let Some(buffer) = self.pipeline.render() {
                let _ = unsafe {
                    (p.present)(
                        p.context,
                        buffer.data().as_ptr(),
                        buffer.width(),
                        buffer.height(),
                        scale,
                    )
                };
            }
        }
    }
}
impl Drop for SwsEditor {
    fn drop(&mut self) {
        self.pipeline.teardown();
    }
}
fn event(i: Input) -> Option<Event> {
    let m = KeyModifiers {
        shift: i.modifiers & 1 != 0,
        control: i.modifiers & 2 != 0,
        alt: i.modifiers & 4 != 0,
        super_key: i.modifiers & 8 != 0,
    };
    let key = match i.key {
        1 => KeyCode::Escape,
        2 => KeyCode::Enter,
        3 => KeyCode::Tab,
        4 => KeyCode::Backspace,
        5 => KeyCode::Delete,
        6 => KeyCode::Left,
        7 => KeyCode::Right,
        8 => KeyCode::Up,
        9 => KeyCode::Down,
        10 => KeyCode::Home,
        11 => KeyCode::End,
        12 => KeyCode::Space,
        n if n >= 0x10000 => KeyCode::Char(core::char::from_u32(n - 0x10000)?),
        _ => KeyCode::Unknown,
    };
    Some(match i.kind {
        1 => Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: i.x,
            y: i.y,
            click_count: i.click_count.clamp(1, 255) as u8,
        }),
        2 => Event::Mouse(MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: i.x,
            y: i.y,
            click_count: i.click_count.clamp(1, 255) as u8,
        }),
        3 => Event::Mouse(MouseEvent::Moved { x: i.x, y: i.y }),
        4 => Event::Keyboard(KeyEvent::Pressed {
            keycode: key,
            modifiers: m,
        }),
        5 => Event::Keyboard(KeyEvent::Released {
            keycode: key,
            modifiers: m,
        }),
        6 => Event::Keyboard(KeyEvent::Char {
            c: core::char::from_u32(i.key)?,
        }),
        7 => Event::Mouse(MouseEvent::ButtonCancelled {
            button: MouseButton::Left,
            x: i.x,
            y: i.y,
        }),
        _ => return None,
    })
}
