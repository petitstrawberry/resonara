// Offscreen CLAP embedding/reload check. No host application or visible window.
// Build with the official CLAP SDK's include directory and Apple's Cocoa SDK.
#import <Cocoa/Cocoa.h>
#include <clap/clap.h>
#include <assert.h>
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>

static bool register_timer(const clap_host_t *host, uint32_t period, clap_id *id) {
    assert(period > 0); *id = 1; return true;
}
static bool unregister_timer(const clap_host_t *host, clap_id id) {
    assert(id == 1); return true;
}
static const clap_host_timer_support_t timers = {register_timer, unregister_timer};
static const void *host_extension(const clap_host_t *host, const char *id) {
    return strcmp(id, CLAP_EXT_TIMER_SUPPORT) == 0 ? &timers : NULL;
}
static const clap_host_t host = {
    .clap_version = CLAP_VERSION_INIT, .name = "Offscreen lifecycle test",
    .vendor = "Resonara", .url = "", .version = "1", .get_extension = host_extension
};
int main(int argc, char **argv) {
    assert(argc == 2);
    @autoreleasepool {
        for (int cycle = 0; cycle < 8; ++cycle) {
            void *library = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
            assert(library);
            const clap_plugin_entry_t *entry = dlsym(library, "clap_entry");
            assert(entry && entry->init(argv[1]));
            const clap_plugin_factory_t *factory = entry->get_factory(CLAP_PLUGIN_FACTORY_ID);
            assert(factory);
            const clap_plugin_descriptor_t *descriptor = factory->get_plugin_descriptor(factory, 0);
            const clap_plugin_t *plugin = factory->create_plugin(factory, &host, descriptor->id);
            assert(plugin && plugin->init(plugin));
            const clap_plugin_gui_t *gui = plugin->get_extension(plugin, CLAP_EXT_GUI);
            assert(gui && gui->is_api_supported(plugin, CLAP_WINDOW_API_COCOA, false));
            for (int reopen = 0; reopen < 2; ++reopen) {
                assert(gui->create(plugin, CLAP_WINDOW_API_COCOA, false));
                uint32_t width = 0, height = 0;
                assert(gui->get_size(plugin, &width, &height));
                NSView *parent = [[NSView alloc] initWithFrame:NSMakeRect(0,0,width,height)];
                clap_window_t window = {.api = CLAP_WINDOW_API_COCOA, .cocoa = (__bridge void *)parent};
                assert(gui->set_parent(plugin, &window));
                assert(parent.subviews.count == 1);
                assert(gui->show(plugin));
                const clap_plugin_timer_support_t *timer = plugin->get_extension(plugin, CLAP_EXT_TIMER_SUPPORT);
                timer->on_timer(plugin, 1);
                assert(gui->hide(plugin));
                gui->destroy(plugin);
                assert(parent.subviews.count == 0);
            }
            plugin->destroy(plugin);
            entry->deinit();
            assert(dlclose(library) == 0);
        }
    }
    puts("Cocoa CLAP lifecycle: 8 library cycles, 16 offscreen GUI creations passed");
}
