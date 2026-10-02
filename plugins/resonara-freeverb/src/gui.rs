//! CLAP-owned GUI. Only the main thread accesses GUI objects; DSP sees atomics.
use super::*;
use alloc::boxed::Box;
use clap_sys::ext::{gui::*, timer_support::*};
#[cfg(target_os = "macos")]
use resonara_freeverb_editor::cocoa::{CocoaEditor as Editor, HEIGHT};
#[cfg(target_os = "scarlet")]
use resonara_freeverb_editor::sws::{HEIGHT, SwsEditor as Editor};
#[cfg(target_os = "macos")]
const API: &CStr = c"cocoa";
#[cfg(target_os = "scarlet")]
const API: &CStr = scarlet_clap_gui::API;
pub struct Gui {
    editor: Editor,
    timer: u32,
    parent: bool,
}
pub static GUI: clap_plugin_gui = clap_plugin_gui {
    is_api_supported: Some(supported),
    get_preferred_api: Some(preferred),
    create: Some(create),
    destroy: Some(destroy),
    set_scale: Some(scale),
    get_size: Some(size),
    can_resize: Some(no_resize),
    get_resize_hints: None,
    adjust_size: None,
    set_size: Some(set_size),
    set_parent: Some(parent),
    set_transient: None,
    suggest_title: None,
    show: Some(show),
    hide: Some(hide),
};
pub static TIMER: clap_plugin_timer_support = clap_plugin_timer_support {
    on_timer: Some(timer),
};
unsafe extern "C" fn supported(p: *const clap_plugin, api: *const c_char, floating: bool) -> bool {
    !floating
        && unsafe { slot(p) }.is_some_and(|s| s.thread_ok(false))
        && unsafe { is_id(api, API) }
}
unsafe extern "C" fn preferred(
    _: *const clap_plugin,
    api: *mut *const c_char,
    floating: *mut bool,
) -> bool {
    if api.is_null() || floating.is_null() {
        return false;
    }
    unsafe {
        *api = API.as_ptr();
        *floating = false;
    }
    true
}
fn timers(s: &Slot) -> Option<&clap_host_timer_support> {
    let host = s.host.load(Ordering::Acquire);
    unsafe {
        ((*host).get_extension?)(host, CLAP_EXT_TIMER_SUPPORT.as_ptr())
            .cast::<clap_host_timer_support>()
            .as_ref()
    }
}
unsafe extern "C" fn create(p: *const clap_plugin, api: *const c_char, floating: bool) -> bool {
    if !unsafe { supported(p, api, floating) } {
        return false;
    }
    let s = unsafe { slot(p) }.unwrap();
    if unsafe { &*s.gui.get() }.is_some() {
        return false;
    }
    let Some(t) = timers(s) else {
        return false;
    };
    let (Some(register), Some(_)) = (t.register_timer, t.unregister_timer) else {
        return false;
    };
    let mut id = 0;
    if !unsafe { register(s.host.load(Ordering::Acquire), 16, &mut id) } {
        return false;
    }
    let Some(editor) = Editor::new(core::array::from_fn(|i| s.value(i)), move |values| {
        s.queue_gui(values)
    }) else {
        unsafe { t.unregister_timer.unwrap()(s.host.load(Ordering::Acquire), id) };
        return false;
    };
    unsafe {
        *s.gui.get() = Some(Box::new(Gui {
            editor,
            timer: id,
            parent: false,
        }));
    }
    true
}
unsafe extern "C" fn destroy(p: *const clap_plugin) {
    if let Some(s) = unsafe { slot(p) }.filter(|s| s.thread_ok(false)) {
        if let Some(g) = unsafe { &mut *s.gui.get() }.take() {
            if let Some(t) = timers(s).and_then(|t| t.unregister_timer) {
                unsafe { t(s.host.load(Ordering::Acquire), g.timer) };
            }
            drop(g);
        }
    }
}
unsafe extern "C" fn scale(_: *const clap_plugin, _: f64) -> bool {
    false
}
unsafe extern "C" fn size(p: *const clap_plugin, w: *mut u32, h: *mut u32) -> bool {
    if w.is_null()
        || h.is_null()
        || !unsafe { slot(p) }
            .is_some_and(|s| s.thread_ok(false) && unsafe { &*s.gui.get() }.is_some())
    {
        return false;
    }
    unsafe {
        *w = resonara_freeverb_editor::WIDTH as u32;
        *h = HEIGHT;
    }
    true
}
unsafe extern "C" fn no_resize(_: *const clap_plugin) -> bool {
    false
}
unsafe extern "C" fn set_size(p: *const clap_plugin, w: u32, h: u32) -> bool {
    let (mut ew, mut eh) = (0, 0);
    (unsafe { size(p, &mut ew, &mut eh) }) && w == ew && h == eh
}
unsafe extern "C" fn parent(p: *const clap_plugin, window: *const clap_window) -> bool {
    let Some(s) = unsafe { slot(p) }.filter(|s| s.thread_ok(false)) else {
        return false;
    };
    let Some(w) = (unsafe { window.as_ref() }) else {
        return false;
    };
    if !unsafe { is_id(w.api, API) } {
        return false;
    }
    let Some(g) = unsafe { &mut *s.gui.get() }.as_mut() else {
        return false;
    };
    g.parent = unsafe { g.editor.set_parent(w.specific.ptr) };
    g.parent
}
unsafe extern "C" fn show(p: *const clap_plugin) -> bool {
    visibility(p, true)
}
unsafe extern "C" fn hide(p: *const clap_plugin) -> bool {
    visibility(p, false)
}
fn visibility(p: *const clap_plugin, visible: bool) -> bool {
    let Some(s) = (unsafe { slot(p) }).filter(|s| s.thread_ok(false)) else {
        return false;
    };
    let Some(g) = (unsafe { &mut *s.gui.get() }).as_mut().filter(|g| g.parent) else {
        return false;
    };
    g.editor.show(visible);
    true
}
unsafe extern "C" fn timer(p: *const clap_plugin, id: u32) {
    let Some(s) = unsafe { slot(p) }.filter(|s| s.thread_ok(false)) else {
        return;
    };
    if let Some(g) = unsafe { &mut *s.gui.get() }
        .as_mut()
        .filter(|g| g.timer == id)
    {
        // Keep unsent GUI values visible until the next audio process/flush consumes them.
        let values = core::array::from_fn(|i| {
            if s.pending_mask.load(Ordering::Acquire) & (1 << i) != 0
                || s.gesture_stage[i].load(Ordering::Relaxed) == 1
            {
                f64::from_bits(s.pending[i].load(Ordering::Acquire))
            } else {
                s.value(i)
            }
        });
        g.editor.sync(values);
    }
}
