# Resonara Freeverb editor

A shared ScarletUI View for macOS and Scarlet. `FreeverbEditor::new([wet, dry,
room_size, damping, width])` validates normalized values and creates stable knob
and numeric-field states. `fields()` returns the five draft fields in CLAP ID
order; `values()` validates numeric drafts and rounds them to 0.01 increments.
`normalize_drafts()` applies that resolution before the host commits. `preset()` changes drafts only.

Supply `.on_apply(...)` and `.on_cancel(...)`, optionally `.error(State<String>)`.
The host owns the popup/window, keyboard cancellation, input focus and parameter
commit. Apply must validate values, update the inactive CLAP instance, snapshot
state, commit through the host’s Undo history and preserve transport. No DSP or
thread/event-loop work runs in this crate. A failed commit should keep the editor
open and display the error. No `clap.gui` interface is exported by the DSP.

Use `PLUGIN_ID`, `PARAMETER_IDS`, `DEFAULTS` and `WIDTH` when connecting the host.
Check the plugin identity and all five normalized writable continuous parameters
before choosing this editor; fall back to generic UI on incompatible metadata.

The ScarletUI revision is pinned to match Resonara. Platform/window/renderer
features are enabled by the host (Winit on macOS, SWS on Scarlet), not this crate.
This crate belongs to the Resonara workspace and uses its lockfile.

```sh
cargo test --locked -p resonara-freeverb-editor
cargo fmt --all -- --check
```

MIT. The rotary widget is adapted from Resonara’s parameter knob.
