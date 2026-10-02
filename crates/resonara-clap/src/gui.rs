//! Owner-thread GUI negotiation and Cocoa embedding, independent of SGFX.
use super::*;
use clap_sys::ext::gui::{CLAP_EXT_GUI, clap_plugin_gui};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GuiSupport {
    pub embedded: bool,
    pub floating: bool,
}

impl HostPlugin {
    /// Query a platform's CLAP window API without creating a GUI. This only
    /// reports the plug-in's capabilities; it does not promise host GUI support.
    /// Call on the original owner/main thread while inactive.
    pub fn gui_support(&self, api: &CStr) -> Result<GuiSupport> {
        let instance = self.instance();
        instance.context.check_owner()?;
        let extension = unsafe { instance.extension::<clap_plugin_gui>(CLAP_EXT_GUI) };
        Ok(extension.map_or(GuiSupport::default(), |gui| {
            support(&gui, instance.plugin, api)
        }))
    }
}

fn support(gui: &clap_plugin_gui, plugin: *const clap_plugin, api: &CStr) -> GuiSupport {
    // Incomplete optional extensions must not prevent audio-only loading.
    let Some(check) = gui.is_api_supported else {
        return GuiSupport::default();
    };
    if gui.create.is_none() || gui.destroy.is_none() || gui.show.is_none() || gui.hide.is_none() {
        return GuiSupport::default();
    }
    // SAFETY: the extension belongs to this live instance; queries are made on
    // its creator/main thread with a terminated platform API string.
    unsafe {
        GuiSupport {
            embedded: gui.get_size.is_some()
                && gui.set_parent.is_some()
                && check(plugin, api.as_ptr(), false),
            floating: check(plugin, api.as_ptr(), true),
        }
    }
}

// All callbacks touch bounded atomics. Window objects and timer deadlines are
// accessed exclusively by the original owner/main thread.
use clap_sys::ext::{gui::clap_host_gui, timer_support::*};
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

pub(super) struct HostGui {
    alive: AtomicBool,
    resize: AtomicU64,
    visibility: AtomicU32,
    closed: AtomicBool,
    periods: [AtomicU32; 16],
    deadlines: [Cell<Option<Instant>>; 16],
}
impl HostGui {
    pub fn new() -> Self {
        Self {
            alive: AtomicBool::new(false),
            resize: AtomicU64::new(0),
            visibility: AtomicU32::new(0),
            closed: AtomicBool::new(false),
            periods: std::array::from_fn(|_| AtomicU32::new(0)),
            deadlines: std::array::from_fn(|_| Cell::new(None)),
        }
    }
}
#[cfg(any(target_os = "macos", target_os = "scarlet"))]
pub(super) fn host_extension(id: &CStr) -> *const c_void {
    if id == CLAP_EXT_GUI {
        (&HOST_GUI as *const clap_host_gui).cast()
    } else if id == CLAP_EXT_TIMER_SUPPORT {
        (&HOST_TIMER as *const clap_host_timer_support).cast()
    } else {
        ptr::null()
    }
}
#[cfg(not(any(target_os = "macos", target_os = "scarlet")))]
pub(super) fn host_extension(_: &CStr) -> *const c_void {
    ptr::null()
}
unsafe extern "C" fn resize(h: *const clap_host, w: u32, height: u32) -> bool {
    if cfg!(target_os = "scarlet") {
        return false;
    }
    let g = &unsafe { context(h) }.gui;
    if !g.alive.load(Ordering::Relaxed) || !(1..=8192).contains(&w) || !(1..=8192).contains(&height)
    {
        return false;
    }
    g.resize
        .store(((w as u64) << 32) | height as u64, Ordering::Release);
    true
}
unsafe extern "C" fn hints(_: *const clap_host) {}
unsafe extern "C" fn show(h: *const clap_host) -> bool {
    unsafe { visibility(h, 1) }
}
unsafe extern "C" fn hide(h: *const clap_host) -> bool {
    unsafe { visibility(h, 2) }
}
unsafe fn visibility(h: *const clap_host, value: u32) -> bool {
    let g = &unsafe { context(h) }.gui;
    if !g.alive.load(Ordering::Relaxed) {
        return false;
    }
    g.visibility.store(value, Ordering::Release);
    true
}
unsafe extern "C" fn closed(h: *const clap_host, _: bool) {
    unsafe { context(h) }
        .gui
        .closed
        .store(true, Ordering::Release);
}
static HOST_GUI: clap_host_gui = clap_host_gui {
    resize_hints_changed: Some(hints),
    request_resize: Some(resize),
    request_show: Some(show),
    request_hide: Some(hide),
    closed: Some(closed),
};
unsafe extern "C" fn register_timer(h: *const clap_host, period: u32, id: *mut u32) -> bool {
    let c = unsafe { context(h) };
    if id.is_null() || period == 0 || c.check_owner().is_err() {
        return false;
    }
    for (index, slot) in c.gui.periods.iter().enumerate() {
        if slot.load(Ordering::Relaxed) == 0 {
            c.gui.deadlines[index].set(Some(Instant::now() + Duration::from_millis(period as u64)));
            slot.store(period, Ordering::Relaxed);
            unsafe {
                *id = index as u32 + 1;
            }
            return true;
        }
    }
    false
}
unsafe extern "C" fn unregister_timer(h: *const clap_host, id: u32) -> bool {
    let c = unsafe { context(h) };
    if c.check_owner().is_err() || !(1..=16).contains(&id) {
        return false;
    }
    c.gui.deadlines[id as usize - 1].set(None);
    c.gui.periods[id as usize - 1].swap(0, Ordering::Relaxed) != 0
}
static HOST_TIMER: clap_host_timer_support = clap_host_timer_support {
    register_timer: Some(register_timer),
    unregister_timer: Some(unregister_timer),
};

impl HostPlugin {
    /// Returns false when this platform/plugin has no implemented window API.
    /// The application's native main event loop must already be running.
    pub fn open_editor(&self) -> Result<bool> {
        open(self.instance())
    }
    pub fn close_editor(&self) -> Result<()> {
        close(self.instance())
    }
    pub fn take_editor_dirty(&self) -> Result<bool> {
        self.instance().context.check_owner()?;
        Ok(self.instance().context.dirty.swap(false, Ordering::Relaxed))
    }
    pub fn editor_is_open(&self) -> Result<bool> {
        is_open(self.instance())
    }
}
pub(super) fn is_open(i: &Instance) -> Result<bool> {
    i.context.check_owner()?;
    Ok(unsafe { &*i.editor.get() }.is_some())
}
pub(super) fn close(i: &Instance) -> Result<()> {
    i.context.check_owner()?;
    i.context.gui.alive.store(false, Ordering::Relaxed);
    if let Some(mut editor) = unsafe { &mut *i.editor.get() }.take() {
        editor.destroy(i);
        i.context.dirty.store(true, Ordering::Relaxed);
    }
    Ok(())
}
pub(super) fn service(i: &Instance) -> Result<()> {
    i.context.check_owner()?;
    let now = Instant::now();
    if let Some(extension) =
        unsafe { i.extension::<clap_plugin_timer_support>(CLAP_EXT_TIMER_SUPPORT) }
        && let Some(timer) = extension.on_timer
    {
        for index in 0..16 {
            let period = i.context.gui.periods[index].load(Ordering::Relaxed);
            if period != 0
                && i.context.gui.deadlines[index]
                    .get()
                    .is_some_and(|d| now >= d)
            {
                i.context.gui.deadlines[index]
                    .set(Some(now + Duration::from_millis(period as u64)));
                unsafe {
                    timer(i.plugin, index as u32 + 1);
                }
            }
        }
    }
    // Do not hold a mutable editor borrow across plug-in callbacks: they may
    // reenter the host. Callbacks themselves only enqueue the atomics above.
    let should_close = unsafe { &mut *i.editor.get() }
        .as_mut()
        .is_some_and(|e| e.service(i));
    if should_close || i.context.gui.closed.swap(false, Ordering::AcqRel) {
        close(i)?;
    }
    Ok(())
}
#[cfg(not(any(target_os = "macos", target_os = "scarlet")))]
pub(super) fn open(i: &Instance) -> Result<bool> {
    i.context.check_owner()?;
    Ok(false)
}
#[cfg(not(any(target_os = "macos", target_os = "scarlet")))]
pub(super) struct Editor;
#[cfg(not(any(target_os = "macos", target_os = "scarlet")))]
impl Editor {
    fn destroy(&mut self, _: &Instance) {}
    fn service(&mut self, _: &Instance) -> bool {
        false
    }
}

#[cfg(target_os = "scarlet")]
#[path = "gui_sws.rs"]
mod sws;
#[cfg(target_os = "macos")]
use objc2::{MainThreadOnly, rc::Retained};
#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSFloatingWindowLevel, NSWindow, NSWindowStyleMask,
};
#[cfg(target_os = "macos")]
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};
#[cfg(target_os = "scarlet")]
pub(super) use sws::{Editor, open};
#[cfg(target_os = "macos")]
pub(super) struct Editor {
    window: Retained<NSWindow>,
    api: clap_plugin_gui,
    hidden: bool,
    destroyed: bool,
}
#[cfg(target_os = "macos")]
pub(super) fn open(i: &Instance) -> Result<bool> {
    i.context.check_owner()?;
    if let Some(editor) = unsafe { &mut *i.editor.get() }.as_mut() {
        editor.window.makeKeyAndOrderFront(None);
        editor.hidden = false;
        unsafe {
            editor.api.show.unwrap()(i.plugin);
        }
        return Ok(true);
    }
    let Some(api) = (unsafe { i.extension::<clap_plugin_gui>(CLAP_EXT_GUI) }) else {
        return Ok(false);
    };
    if !support(&api, i.plugin, c"cocoa").embedded {
        return Ok(false);
    }
    let marker = MainThreadMarker::new()
        .ok_or_else(|| Error::new("Cocoa GUI requires the OS main thread"))?;
    let _application = NSApplication::sharedApplication(marker);
    i.context.gui.closed.store(false, Ordering::Relaxed);
    i.context.gui.resize.store(0, Ordering::Relaxed);
    i.context.gui.visibility.store(0, Ordering::Relaxed);
    if !unsafe { api.create.unwrap()(i.plugin, c"cocoa".as_ptr(), false) } {
        return Err(Error::new("CLAP GUI creation failed"));
    }
    let mut width = 0;
    let mut height = 0;
    if !unsafe { api.get_size.unwrap()(i.plugin, &mut width, &mut height) }
        || !(1..=8192).contains(&width)
        || !(1..=8192).contains(&height)
    {
        unsafe {
            api.destroy.unwrap()(i.plugin);
        }
        return Err(Error::new("Invalid CLAP GUI size"));
    }
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(marker),
            NSRect::new(
                NSPoint::new(0., 0.),
                NSSize::new(width as f64, height as f64),
            ),
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe {
        window.setReleasedWhenClosed(false);
    }
    // Keep the editor above the SGFX main window when focus returns there.
    // Ordering a normal-level main window forward otherwise covers the editor.
    window.setLevel(NSFloatingWindowLevel);
    window.setTitle(&NSString::from_str(&format!(
        "{} · Resonara",
        i.descriptor.name
    )));
    window.center();
    let mut editor = Editor {
        window,
        api,
        hidden: false,
        destroyed: false,
    };
    let Some(view) = editor.window.contentView() else {
        editor.destroy(i);
        return Err(Error::new("Cocoa content view is unavailable"));
    };
    let parent = clap_sys::ext::gui::clap_window {
        api: c"cocoa".as_ptr(),
        specific: clap_sys::ext::gui::clap_window_handle {
            cocoa: (&*view as *const _ as *mut c_void),
        },
    };
    i.context.gui.alive.store(true, Ordering::Relaxed);
    if !unsafe { api.set_parent.unwrap()(i.plugin, &parent) && api.show.unwrap()(i.plugin) } {
        i.context.gui.alive.store(false, Ordering::Relaxed);
        editor.destroy(i);
        return Err(Error::new("CLAP GUI embedding failed"));
    }
    editor.window.makeKeyAndOrderFront(None);
    unsafe {
        *i.editor.get() = Some(editor);
    }
    Ok(true)
}
#[cfg(target_os = "macos")]
impl Editor {
    fn destroy(&mut self, i: &Instance) {
        if !self.destroyed {
            // Keep the parent NSView alive until the plug-in releases its GUI.
            unsafe {
                self.api.hide.unwrap()(i.plugin);
                self.api.destroy.unwrap()(i.plugin);
            }
            self.destroyed = true;
            self.window.close();
        }
    }
    fn service(&mut self, i: &Instance) -> bool {
        let size = i.context.gui.resize.swap(0, Ordering::AcqRel);
        if size != 0 {
            self.window
                .setContentSize(NSSize::new((size >> 32) as f64, (size as u32) as f64));
        }
        match i.context.gui.visibility.swap(0, Ordering::AcqRel) {
            1 => {
                self.hidden = false;
                self.window.makeKeyAndOrderFront(None);
                unsafe {
                    self.api.show.unwrap()(i.plugin);
                }
            }
            2 => {
                self.hidden = true;
                self.window.orderOut(None);
                unsafe {
                    self.api.hide.unwrap()(i.plugin);
                }
            }
            _ => {}
        }
        !self.hidden && !self.window.isVisible() && !self.window.isMiniaturized()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    unsafe extern "C" fn check(_: *const clap_plugin, api: *const c_char, floating: bool) -> bool {
        unsafe { CStr::from_ptr(api) == c"cocoa" && !floating }
    }
    unsafe extern "C" fn create(_: *const clap_plugin, _: *const c_char, _: bool) -> bool {
        true
    }
    unsafe extern "C" fn destroy(_: *const clap_plugin) {}
    unsafe extern "C" fn show(_: *const clap_plugin) -> bool {
        true
    }
    unsafe extern "C" fn size(_: *const clap_plugin, _: *mut u32, _: *mut u32) -> bool {
        true
    }
    unsafe extern "C" fn parent(
        _: *const clap_plugin,
        _: *const clap_sys::ext::gui::clap_window,
    ) -> bool {
        true
    }
    #[test]
    fn negotiates_api_and_rejects_incomplete_gui_without_affecting_audio() {
        let mut gui = clap_plugin_gui {
            is_api_supported: Some(check),
            get_preferred_api: None,
            create: Some(create),
            destroy: Some(destroy),
            set_scale: None,
            get_size: Some(size),
            can_resize: None,
            get_resize_hints: None,
            adjust_size: None,
            set_size: None,
            set_parent: Some(parent),
            set_transient: None,
            suggest_title: None,
            show: Some(show),
            hide: Some(show),
        };
        assert_eq!(
            support(&gui, ptr::null(), c"cocoa"),
            GuiSupport {
                embedded: true,
                floating: false
            }
        );
        assert_eq!(support(&gui, ptr::null(), c"x11"), GuiSupport::default());
        gui.destroy = None;
        assert_eq!(support(&gui, ptr::null(), c"cocoa"), GuiSupport::default());
    }
}
