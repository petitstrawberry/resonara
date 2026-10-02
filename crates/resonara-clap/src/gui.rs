//! Owner-thread GUI capability negotiation. Native window creation is supplied
//! by a future platform editor adapter, never by catalog/project serialization.
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
