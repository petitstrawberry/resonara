# CLAP editor integration

Resonara opens native editors through `HostPlugin::open_editor`, with the generic
parameter editor as fallback. The bundled Freeverb exports `clap.gui` and uses
this same path as installed plugins. Its layout and controls live in the plugin;
the application has no Freeverb View dependency or editor dispatch by identity.

The [CLAP GUI extension](https://github.com/free-audio/clap/blob/main/include/clap/ext/gui.h)
negotiates the window API and embedded/floating mode. The host selects a supported
API, creates its own container and supplies the embedded parent. One CLAP instance
owns its DSP and GUI. Multiple insert instances can display independent windows.

## Ownership and transport

Immutable instance identity is shared; processing scratch belongs to the audio
proxy and GUI objects belong to the creating/main thread. No GUI handle crosses
into the callback. Hide/destroy releases GUI resources before the parent window
and DSO. Closing an editor leaves its DSP alive; bypass leaves its editor open.

A paused Audio owner retains the playback renderer and plugins. Resume/seek reuse
those instances. Scarlet recreates the SAS output connection, not the CLAP
processors, when transport restarts, and services paused parameter flushes without
sending audio. Natural EOF retains the SAS connection for its hardware tail.
Removal, structural graph replacement, project replacement and application exit
close affected editors before retiring their exact processor owners.

## Parameters and event loop

The main event loop services plugin callbacks, bounded host parameter output
queues, GUI notifications and `clap.timer-support` timers. Active parameter flush
runs only on the audio thread; inactive flush runs on the owner thread. State
snapshot uses the existing host quiescence protocol. GUI edits update the project
and coalesce host Undo without rebuilding the DSP per mouse move.

Freeverb queues normalized values atomically, requests a flush and emits live
begin/value/end events. A full output queue retains unfinished gestures for retry.
GUI/DSP parameter changes synchronize through the timer callback. Process, flush
and reset remain allocation-free. The UI formats values to two decimals; DSP state
retains its normal floating-point representation.

## Platform adapters

- macOS: embedded `cocoa`, parent `NSView*`, logical dimensions. The host's native
  NSWindow is the container. Freeverb's NSView uses a shared ScarletUI CPU layout
  and retains its bitmap for AppKit redraws with no new damage.
- Scarlet: experimental embedded `org.scarlet-os.sws/1`, parent `ParentV1*`.
  See the [C ABI and lifetime contract](../crates/scarlet-clap-gui/README.md).
  The host owns an SWS window and composites plugin BGRA frames using its SGFX
  backend. Fixed-size parents are supported; resize requests are declined.
  This is not a standard CLAP API or a Cocoa/X11 handle.
- Linux: native GUI hosting is not implemented. Generic parameter UI remains
  available; a future adapter must negotiate the window system actually in use.

Embedded support is implemented; floating support is not advertised by Freeverb.
`gui_support` reports plugin capabilities, which the host intersects with its
implemented adapters. Standard Cocoa plugins remain independent of ScarletUI.

## Validation

`clap_editor_smoke` opens two independent CLAP instances through the generic host
API. DSP/CLAP tests cover negotiation, queue backpressure, instance isolation and
allocation-free processing; editor tests cover presets, decimal formatting and
knob input. macOS embedded rendering and immediate parameter edits are manually
verified. Native cross-build/audit does not imply Scarlet GUI runtime verification.
