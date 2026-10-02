//! An embedded Cocoa view driven by the host event loop. No application runner.
use crate::{FreeverbEditor, LiveParameters, WIDTH};
use core::ffi::c_void;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{
    NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace, NSEvent, NSEventModifierFlags,
    NSView,
};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSPoint, NSRect, NSSize};
use scarlet_ui::{
    event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent},
    pipeline::RenderingPipeline,
    platform::WindowDecoration,
    prelude::*,
};
use std::{cell::RefCell, rc::Rc, sync::OnceLock};

// Runtime-registered Objective-C classes retain method pointers for the process
// lifetime. Pin this image before registering PluginView, independently of how
// any particular CLAP host manages dlopen/dlclose.
#[inline(never)]
fn keep_image_loaded() -> bool {
    use core::ffi::{c_char, c_int};
    #[repr(C)]
    struct DlInfo {
        filename: *const c_char,
        base: *mut c_void,
        symbol: *const c_char,
        address: *mut c_void,
    }
    unsafe extern "C" {
        fn dladdr(address: *const c_void, info: *mut DlInfo) -> c_int;
        fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> c_int;
    }
    static LOADED: OnceLock<bool> = OnceLock::new();
    *LOADED.get_or_init(|| unsafe {
        let mut info: DlInfo = core::mem::zeroed();
        if dladdr(keep_image_loaded as *const () as *const c_void, &mut info) == 0
            || info.filename.is_null()
        {
            return false;
        }
        let handle = dlopen(info.filename, 0x2 | 0x4 | 0x80); // NOW | LOCAL | NODELETE
        if handle.is_null() {
            return false;
        }
        dlclose(handle);
        true
    })
}

pub const HEIGHT: u32 = 510;
struct Content {
    pipeline: RenderingPipeline,
    editor: FreeverbEditor,
    parameters: LiveParameters,
    change: Rc<dyn Fn([f64; 5])>,
}
impl Content {
    fn changes(&mut self) {
        if let Ok(values) = self.editor.values() {
            if let Some(values) = self.parameters.changes(values) {
                (self.change)(values);
            }
        }
    }
}
struct Ivars {
    content: RefCell<Content>,
    bitmap: RefCell<Option<Retained<NSBitmapImageRep>>>,
}
define_class!(
    #[unsafe(super = NSView)]
    #[name = "ResonaraFreeverbClapView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct PluginView;
    unsafe impl NSObjectProtocol for PluginView {}
    impl PluginView {
        #[unsafe(method(isFlipped))] fn flipped(&self)->bool { true }
        #[unsafe(method(acceptsFirstResponder))] fn accepts(&self)->bool { true }
        #[unsafe(method(viewDidChangeBackingProperties))] fn backing_changed(&self) {
            if let Some(window) = self.window() {
                let scale = (window.backingScaleFactor() * 1000.) as u32;
                let mut c = self.ivars().content.borrow_mut();
                if c.pipeline.scale_milli() != scale { c.pipeline.set_scale_milli(scale); }
                drop(c);
                self.setNeedsDisplay(true);
            }
        }
        #[unsafe(method(drawRect:))] fn draw(&self, _:NSRect) {
            let mut c = self.ivars().content.borrow_mut();
            let mut bitmap = self.ivars().bitmap.borrow_mut();
            if let Some(buffer) = c.pipeline.render() {
                if bitmap.as_ref().is_none_or(|b| b.pixelsWide() != buffer.width() as isize || b.pixelsHigh() != buffer.height() as isize) {
                    *bitmap = unsafe { NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                        NSBitmapImageRep::alloc(), core::ptr::null_mut(), buffer.width() as isize, buffer.height() as isize, 8, 4, true, false, NSDeviceRGBColorSpace, buffer.width() as isize * 4, 32) };
                }
                if let Some(b) = bitmap.as_ref() {
                    let dst = b.bitmapData();
                    if !dst.is_null() {
                        for (i, pixel) in buffer.data().chunks_exact(4).enumerate() {
                            unsafe { core::ptr::copy_nonoverlapping([pixel[2],pixel[1],pixel[0],pixel[3]].as_ptr(), dst.add(i*4), 4); }
                        }
                    }
                }
            }
            // AppKit may request another draw after a menu/focus change even
            // when ScarletUI has no damage. Idle means reuse the last frame;
            // leaving drawRect empty exposes the window's cleared background.
            if let Some(b) = bitmap.as_ref() {
                unsafe { b.drawInRect_fromRect_operation_fraction_respectFlipped_hints(self.bounds(),NSRect::new(NSPoint::new(0.,0.),b.size()),NSCompositingOperation::Copy,1.,true,None); }
            }
        }
        #[unsafe(method(mouseDown:))] fn down(&self,e:&NSEvent) {
            if let Some(w) = self.window() { w.makeFirstResponder(Some(self)); }
            self.mouse(e,0);
        }
        #[unsafe(method(mouseUp:))] fn up(&self,e:&NSEvent) { self.mouse(e,1); }
        #[unsafe(method(mouseDragged:))] fn drag(&self,e:&NSEvent) { self.mouse(e,2); }
        #[unsafe(method(mouseMoved:))] fn moved(&self,e:&NSEvent) { self.mouse(e,2); }
        #[unsafe(method(keyDown:))] fn key(&self,e:&NSEvent) {
            let f=e.modifierFlags();
            let m=KeyModifiers {shift:f.contains(NSEventModifierFlags::Shift), control:f.contains(NSEventModifierFlags::Control), alt:f.contains(NSEventModifierFlags::Option), super_key:f.contains(NSEventModifierFlags::Command)};
            let chars=e.characters().map(|s|s.to_string()).unwrap_or_default();
            let key=match e.keyCode() { 53=>KeyCode::Escape,36|76=>KeyCode::Enter,48=>KeyCode::Tab,51=>KeyCode::Backspace,117=>KeyCode::Delete,123=>KeyCode::Left,124=>KeyCode::Right,125=>KeyCode::Down,126=>KeyCode::Up,115=>KeyCode::Home,119=>KeyCode::End,_=>chars.chars().next().map(KeyCode::Char).unwrap_or(KeyCode::Unknown) };
            self.event(Event::Keyboard(KeyEvent::Pressed{keycode:key,modifiers:m}));
            if matches!(key,KeyCode::Char(_)) && !m.control && !m.super_key {
                for c in chars.chars().filter(|c|!c.is_control()) { self.event(Event::Keyboard(KeyEvent::Char{c})); }
            }
        }
    }
);
impl PluginView {
    fn event(&self, e: Event) {
        let mut content = self.ivars().content.borrow_mut();
        content.pipeline.handle_event(&e);
        content.changes();
        drop(content);
        self.setNeedsDisplay(true);
    }
    fn mouse(&self, e: &NSEvent, kind: u8) {
        let p = self.convertPoint_fromView(e.locationInWindow(), None);
        let (x, y) = (p.x as i32, p.y as i32);
        let event = match kind {
            0 => MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                y,
                click_count: e.clickCount().clamp(1, 255) as u8,
            },
            1 => MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x,
                y,
                click_count: e.clickCount().clamp(1, 255) as u8,
            },
            _ => MouseEvent::Moved { x, y },
        };
        self.event(Event::Mouse(event));
    }
}
pub struct CocoaEditor {
    view: Retained<PluginView>,
}
impl CocoaEditor {
    pub fn new(values: [f64; 5], change: impl Fn([f64; 5]) + 'static) -> Option<Self> {
        let marker = MainThreadMarker::new()?;
        if !keep_image_loaded() {
            return None;
        }
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
        let view = PluginView::alloc(marker).set_ivars(Ivars {
            content: RefCell::new(Content {
                pipeline,
                editor,
                parameters: LiveParameters::new(values),
                change: Rc::new(change),
            }),
            bitmap: RefCell::new(None),
        });
        let view: Retained<PluginView> = unsafe {
            msg_send![super(view),initWithFrame:NSRect::new(NSPoint::new(0.,0.),NSSize::new(WIDTH as f64,HEIGHT as f64))]
        };
        Some(Self { view })
    }
    /// Parent is the live NSView supplied by clap.gui.set_parent on the main thread.
    pub unsafe fn set_parent(&self, parent: *mut c_void) -> bool {
        let Some(parent) = (unsafe { parent.cast::<NSView>().as_ref() }) else {
            return false;
        };
        parent.addSubview(&self.view);
        if let Some(w) = parent.window() {
            self.view
                .ivars()
                .content
                .borrow_mut()
                .pipeline
                .set_scale_milli((w.backingScaleFactor() * 1000.) as u32);
        }
        true
    }
    pub fn show(&self, show: bool) {
        self.view.setHidden(!show);
        if show {
            self.view.setNeedsDisplay(true);
        }
    }
    pub fn sync(&self, values: [f64; 5]) {
        let mut c = self.view.ivars().content.borrow_mut();
        if c.parameters.observe(values) {
            c.editor.set_values(values);
        }
        if c.pipeline.has_dirty() {
            self.view.setNeedsDisplay(true);
        }
    }
}
impl Drop for CocoaEditor {
    fn drop(&mut self) {
        self.view.removeFromSuperview();
        self.view.ivars().content.borrow_mut().pipeline.teardown();
    }
}
