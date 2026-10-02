# CLAP editor integration contract

Independent plug-in windows are part of the intended host design. The current
implementation adds owner-thread `HostPlugin::gui_support(api)` negotiation and
keeps installed-library discovery separate from the generic parameter editor.
It does not yet create or display a plug-in GUI. The generic editor remains the
fallback when a plug-in has no GUI or its window API is unavailable.

The [CLAP GUI extension](https://github.com/free-audio/clap/blob/main/include/clap/ext/gui.h)
defines embedded and floating windows, API negotiation, main-thread creation and
destruction, resize requests, and asynchronous host notifications. A GUI belongs
to the same CLAP instance as its DSP. Opening a second inactive instance, editing
it and repeatedly replacing the playing graph is not the native GUI design.

## Ownership and playback

Before exposing native editors, split `InstanceCell` into immutable shared
instance identity/context, audio-owned scratch/process state, and owner-thread
editor state. Do not add concurrent `&mut Instance` access to its existing
`UnsafeCell`. `PluginOwner` retains the native editor; the realtime proxy retains
only processing state. No raw GUI handle crosses into the audio callback.

Track open editors by a stable insert instance ID, not the current slot number
or an engine pointer. Moving an insert retains its editor/DSP relationship;
removing it, rebuilding it, opening a different project, or closing the app closes
the editor before retiring/destroying its instance. A queued graph replacement
must not silently leave a window attached to a superseded processor. Until
instance-preserving graph updates exist, explicitly close the affected editor
on replacement; transport still continues at the audio block boundary.

Normal editor close calls hide/destroy on the creator thread and leaves DSP
running. Audio teardown joins/destroys the renderer before deactivation and final
plug-in destruction. GUI resource lifetime must not depend on cached project
metadata or the library discovery catalog.

## Parameters, state and notifications

Add bounded control/audio mailboxes for parameter values, gestures, flush requests
and main-thread notifications. Apply active parameter events inside CLAP process
or audio-thread flush; drain output events into the editor/project model outside
the callback. Undo coalesces a GUI gesture, rather than replacing the DSP for each
mouse movement. Reject overflowing queues visibly without allocating on audio.

Service `request_callback`, `clap_host_gui` show/hide/resize/closed notifications,
and required timer/fd extensions from the main event loop. The current host stops
on active main-thread requests; this must be replaced before enabling a native
editor. State saving/loading requires an explicit quiescence/ownership protocol;
do not call state callbacks concurrently with processing unless the extension
allows it. Preserve opaque state, parameter metadata and transport on failure.

## Platform adapters

- macOS: negotiate `cocoa`; an embedded parent is an NSView, not an NSWindow.
  Cocoa uses logical sizes, so do not apply `set_scale` to it.
- Linux: negotiate the backend actually in use. X11 supports an embedded window;
  the standard Wayland contract currently supports floating windows.
- Scarlet: standard CLAP has no SWS window API. Define and test a shared native
  ABI with the plug-in SDK before advertising a custom API string. Specify the
  window reference representation, ownership, embedding/floating behavior,
  resize and output-scale units, event dispatch and disconnection behavior.
  Do not pass an SWS integer window ID as a Cocoa/X11 handle. Native Scarlet
  plug-ins also need GUI code built for Scarlet, just as their DSP does.

`gui_support` reports plug-in capability only; the host must intersect it with
an implemented platform adapter. The host currently returns no `clap_host_gui`
extension, and does not advertise editor callbacks that it cannot service.

## Next implementation and acceptance checks

1. Split host ownership, add stable insert identity and bounded event mailboxes.
2. Add a test CLAP GUI fixture with create/show/hide/destroy counters, resize
   requests, callback requests and a GUI parameter controlling audible gain.
3. Implement the platform adapter and host notification/timer plumbing.
4. Add a native-editor action alongside the generic fallback. Verify resize,
   scale, repeated open/close, removal, movement, bypass, undo, session reopening,
   device stop/EOF and application close.
5. Verify the GUI controls the actual playing instance, transport keeps advancing,
   no owner callback runs on audio, and GUI/DSO resources are released exactly
   once. Run first/repeated audio callback allocation audits with the editor open.

These are outstanding implementation steps, not claims of completed GUI support.
