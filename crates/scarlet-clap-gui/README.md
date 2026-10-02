# Experimental Scarlet CLAP GUI parent

This local `no_std` crate specifies an embedded parent bridge shared by a CLAP
host and plugin. It is **not a standard CLAP window API** and is not yet a
published Scarlet SDK contract. API negotiation uses `org.scarlet-os.sws/1`
with `is_floating=false`. The matching `clap_window.specific.ptr` points to a
`ParentV1` (`ScarletClapParentV1` in the [C header](include/scarlet-clap-gui.h)).

The host owns the parent, context, window and event loop. They remain valid from
`set_parent` until `gui.destroy` returns, including failed embedding/show paths.
The plugin borrows the parent, creates its own GUI content and releases all
parent references on destroy. Both sides call the bridge exclusively on the CLAP
main thread. No Rust objects, trait objects, references or allocation ownership
cross the boundary; callbacks use only C-compatible values and borrowed pointers.

- `scale_milli`: physical pixels per logical pixel, in thousandths (1000 = 1×).
- `poll_input`: dequeue one input event. False means no event; output remains
  unchanged. Coordinates are logical pixels relative to the content's upper-left
  corner. Character events contain a Unicode scalar; key events use the listed
  key mapping. Mouse cancel ends capture without committing a click.
- `present`: BGRA8, tightly packed rows of `width * 4` bytes, top to bottom.
  Width/height are physical dimensions, each `ceil(logical_size * scale / 1000)`.
  Pixels are valid only during the callback. The host must copy before returning;
  true means the frame was accepted. False means retry after correcting size/scale.

Version must equal 1. Every callback and context must remain valid throughout the
borrow. A host closes the containing window only after destroying the plugin GUI.
Host timers drive plugin updates; disconnection closes/destroys the editor via its
owner. The current adapter uses fixed content dimensions; dynamic resize is
rejected. It forwards mouse and keyboard events. Wheel, touch, IME and native
clipboard requests are outside version 1.

Resonara creates an SWS window and composites submitted frames with SGFX.
Freeverb rasterizes its shared ScarletUI controls in the plugin. Neither side
passes an SWS ID as a Cocoa/X11 parent or depends on the other side's UI internals.
