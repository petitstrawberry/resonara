# Resonara Freeverb editor

Plugin-owned ScarletUI controls shared by the embedded Cocoa and Scarlet/SWS
adapters. Resonara does not depend on or instantiate this crate.

`FreeverbEditor::new([wet, dry, room_size, damping, width])` creates stable knob
and numeric field states. `.live()` enables immediate edits. Values are normalized
0–1, use 0.01 increments and always display two decimal places. The stock Select
component offers Default, Room, Hall, Aux send and a derived Custom selection.

`CocoaEditor` creates an NSView, attaches it through `clap.gui.set_parent`, forwards
input and caches the rendered bitmap. Every AppKit draw paints that cached image,
including redraws without ScarletUI damage. It uses flipped coordinates consistently. The Cocoa adapter pins its Mach-O image
after its first GUI creation because registered Objective-C classes retain method
pointers; editor instances and bitmaps are still destroyed normally.
`SwsEditor` consumes the experimental C parent bridge supplied by a host and
presents BGRA frames. Neither adapter creates an application or an event loop.

The CLAP plugin owns both adapters and their lifetime. Host timers service GUI
updates on the main thread. GUI changes request a CLAP parameter flush; DSP sees
only atomics and never accesses ScarletUI objects. Editor teardown precedes
parent/window destruction. The Scarlet adapter uses the legacy freestanding
Scarlet runtime; no SWS/window backend is linked into the plugin itself.

The crate has its own Cargo workspace and lockfile, using the same pinned
ScarletUI revision as the host.

```sh
cargo test --locked --manifest-path plugins/resonara-freeverb/editor/Cargo.toml
cargo fmt --manifest-path plugins/resonara-freeverb/editor/Cargo.toml -- --check
```

MIT. The rotary widget is adapted from Resonara's parameter knob.
