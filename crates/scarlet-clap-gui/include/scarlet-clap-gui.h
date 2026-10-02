/* Experimental embedded CLAP API; see ../README.md for ownership and units. */
#ifndef SCARLET_CLAP_GUI_H
#define SCARLET_CLAP_GUI_H
#include <stdbool.h>
#include <stdint.h>
#define SCARLET_CLAP_GUI_API "org.scarlet-os.sws/1"
typedef struct ScarletClapInput {
    uint32_t kind;
    int32_t x, y;
    uint32_t key, modifiers, click_count;
} ScarletClapInput;
typedef struct ScarletClapParentV1 {
    uint32_t version;
    void *context;
    uint32_t (*scale_milli)(void *context);
    bool (*poll_input)(void *context, ScarletClapInput *out);
    bool (*present)(void *context, const uint8_t *bgra,
                    uint32_t width, uint32_t height, uint32_t scale_milli);
} ScarletClapParentV1;
#endif
