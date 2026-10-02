# Resonara Freeverb

Resonara’s standard stereo reverb, maintained here together with its dedicated
ScarletUI editor. Package/file name: `resonara-freeverb`; display name:
**Resonara Freeverb**; CLAP identifier: `org.resonara.freeverb`.

## Use

Choose an empty Insert → **Resonara Freeverb**, then open that slot’s editor.
Five knobs and numeric fields control Wet, Dry, Room size, Damping and Width.
Values use 0.01 increments and exactly two decimal places (`0.20`, `0.25`, `1.00`).
Drag vertically or use arrows; Shift+arrow uses the finer 0.01 step, versus the
normal 0.02 step. Home/double-click restores the opening value.

The preset selector offers Default / Room / Hall / Aux send / Custom.
Knob, field and preset edits take effect immediately through CLAP parameter
output events. The host persists state and groups Undo; there is no Apply/Cancel
popup. Aux send sets Wet=1 / Dry=0. Each insert owns its own editor.

The app build places the plugin under `<profile>/plugins/` beside its executable.
Move that directory with the app binary. Scarlet's image builder stages native
plugins and license notices under `/usr/lib/clap/`. Scarlet's own image recipes
must include the [Resonara bundle](../../platforms/scarlet/README.md) rather than
only the app's Cargo layer. No separate repository,
installation or catalog scan is required for Resonara's bundled Freeverb.

## Structure and GUI

- `src/`: freestanding CLAP DSP, parameter mailbox, `clap.gui` and timer adapters.
- `editor/`: plugin-owned ScarletUI controls plus platform embedding adapters.
- `vendor/`: preserved upstream Freeverb reference, CLAP bindings and licenses.
- `build.py`: platform-specific plugin builder and ABI audits.

The plugin and editor have isolated Cargo workspaces. The host has no dependency
on the editor, does not select a View by plugin identity, and opens Freeverb via
its normal CLAP GUI path, just like other native effects.

macOS supports embedded `cocoa`: the plugin attaches an NSView to the parent
supplied by the host. Scarlet supports the experimental
`org.scarlet-os.sws/1` embedded API documented in
[scarlet-clap-gui](../../crates/scarlet-clap-gui/README.md). Both use the same
ScarletUI layout; platform code supplies input and presents the rasterized
content. The host owns the containing window and event loop. Floating GUI and
Linux GUI are not implemented; unsupported hosts can use generic parameters.
`--no-default-features` produces the DSP without GUI dependencies.
Each OS/CPU requires its own binary.

## DSP

| Parameter | Range | Default |
| --- | --- | --- |
| Wet | 0–1 | 0.30 |
| Dry | 0–1 | 1.00 |
| Room size | 0–1 | 0.50 |
| Damping | 0–1 | 0.50 |
| Width | 0–1 | 1.00 |

Stereo float32, zero latency, 8–96 kHz, sample-offset CLAP events, versioned
48-byte state. Reset clears the tail and preserves parameters; the host controls
bypass. Keep feeding silence to hear the full tail. Freeze mode is omitted.

Audio process/flush/reset allocate no memory and use fixed DSP storage and
atomics; GUI/runtime dependencies stay off the audio path. Fixed delay storage occupies about 6 MiB per loaded library, with
up to eight simultaneous instances.

## Build and verify

Run from the Resonara root in the host Rust shell:

```sh
cargo build -p resonara
cargo test --manifest-path plugins/resonara-freeverb/editor/Cargo.toml
cargo test --locked --manifest-path plugins/resonara-freeverb/Cargo.toml
python3 plugins/resonara-freeverb/test-build.py
```

In the Scarlet toolchain shell (rust-src and readelf/llvm-readelf required):

```sh
python3 plugins/resonara-freeverb/build.py --arch aarch64 --output artifacts/freeverb-aarch64
python3 plugins/resonara-freeverb/build.py --arch riscv64 --output artifacts/freeverb-riscv64
bash scripts/verify-scarlet
```

The native builder writes audited plugins, license notices and a build.json report.
It derives an isolated PIC target and rebuilds core/alloc/compiler_builtins, leaving
the installed Scarlet target unchanged. Linking uses `-Bsymbolic-functions`;
`-Bsymbolic` emits loader-rejected DF_SYMBOLIC.

`--arch macos` builds an ad-hoc signed CLAP bundle with embedded Cocoa GUI using the
macOS host toolchain and Xcode tools. `--arch linux` supports x86-64 Linux.
These outputs are distinct from Scarlet ELF binaries.

DSP tests compare every sample with the original implementation at 8, 44.1,
48 and 96 kHz. CLAP lifecycle, events, reset/tail, in-place audio, partial and
corrupt state, capacity, allocation-free processing and ABI/loader checks are
covered. Editor tests cover rounding/padding, presets and knob gestures. CLAP tests cover
GUI negotiation, instance isolation, output-event backpressure and allocation-free
active parameter flushes. `clap_editor_smoke` opens two instances through the
host's generic CLAP interface for manual GUI verification.

On macOS, `tests/cocoa-lifecycle.m` verifies offscreen create/embed/hide/destroy
and library reopen cycles using the official CLAP SDK headers:

```sh
clang -fobjc-arc -I /path/to/clap/include -framework Cocoa \
  plugins/resonara-freeverb/tests/cocoa-lifecycle.m -o /tmp/freeverb-cocoa-lifecycle
/tmp/freeverb-cocoa-lifecycle "$PWD/target/release/plugins/resonara-freeverb.clap"
```

Earlier native AArch64 validation covered loading, wet tail, state, export, exact
bypass and SAS playback. Embedded Cocoa GUI operation is verified on macOS;
Scarlet's embedded GUI is cross-built/audited separately from runtime validation.

Older projects using `scarlet-freeverb.clap` / `org.scarlet.freeverb` or
`freeverb-scarlet.clap` / `org.scarlet.freeverb-scarlet` require their original DSP
or manual insert replacement. Old state is not silently rewritten.

## License

MIT. [UPSTREAM.md](UPSTREAM.md) records source provenance. Adapter, DSP, CLAP
bindings and CLAP notices are preserved and bundled with each DSP artifact.
