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

Default / Room / Hall / Aux send presets edit drafts. **Apply** saves CLAP state
in the project and makes one Undo step while playback continues. **Cancel**,
Escape or an outside click discards drafts. Aux send sets Wet=1 / Dry=0.

The app’s macOS/Linux build creates the DSP under `<profile>/plugins/` beside
its executable. Move that directory with the app binary. Scarlet’s image
builder stages native DSP and license notices under `/system/plugins/`.
No separate repository, plugin installation or catalog scan is required.

## Structure

- `src/`: freestanding CLAP DSP; no GUI dependency.
- `editor/`: a workspace-local ScarletUI View used by Resonara on macOS and
  Scarlet. The host owns its popup, event loop, state commit, Undo and transport.
- `vendor/`: preserved upstream Freeverb reference, CLAP bindings and licenses.
- `build.py`: native DSP builder and platform ABI audits.

The DSP has an isolated Cargo workspace because its no_std/PIC/panic settings
are independent of the application. The editor belongs to Resonara’s workspace
and shares the application’s pinned ScarletUI identity. See
[editor/README.md](editor/README.md) for the host interface.

This host-integrated editor is not a `clap.gui` extension. Other CLAP hosts can
use their generic parameter editor. Each OS requires its own DSP binary.

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

Audio process/flush/reset allocate no memory and use no locks, threads, TLS or
OS imports. Fixed delay storage occupies about 6 MiB per loaded library, with
up to eight simultaneous instances.

## Build and verify

Run from the Resonara root in the host Rust shell:

```sh
cargo build -p resonara
cargo test -p resonara-freeverb-editor
cargo test --locked --manifest-path plugins/resonara-freeverb/Cargo.toml
python3 plugins/resonara-freeverb/test-build.py
```

In the Scarlet toolchain shell (rust-src and readelf/llvm-readelf required):

```sh
python3 plugins/resonara-freeverb/build.py --arch aarch64 --output artifacts/freeverb-aarch64
python3 plugins/resonara-freeverb/build.py --arch riscv64 --output artifacts/freeverb-riscv64
bash scripts/verify-scarlet
```

The native builder writes audited DSP, license notices and a build.json report.
It derives an isolated PIC target and rebuilds core/compiler_builtins, leaving
the installed Scarlet target unchanged. Linking uses `-Bsymbolic-functions`;
`-Bsymbolic` emits loader-rejected DF_SYMBOLIC.

`--arch macos` builds an ad-hoc signed DSP-only CLAP bundle with the current
macOS host toolchain and Xcode tools. `--arch linux` supports x86-64 Linux.
These outputs are distinct from Scarlet ELF binaries.

DSP tests compare every sample with the original implementation at 8, 44.1,
48 and 96 kHz. CLAP lifecycle, events, reset/tail, in-place audio, partial and
corrupt state, capacity, allocation-free processing and ABI/loader checks are
covered. Editor tests cover rounding/padding, presets and knob gestures. Host
integration tests cover invalid input, cancellation, state/save/reopen, Undo,
bypass and continued playback.

Native AArch64 runtime loading, wet tail (energy 2.3789181011455605), state,
export, exact bypass and SAS playback passed. The dedicated GUI rendered on
macOS/wgpu and Scarlet/virgl. RISC-V64 is cross-built/audited, not runtime-tested.
The final two-decimal behavior is covered by editor/integration tests.

Older projects using `scarlet-freeverb.clap` / `org.scarlet.freeverb` or
`freeverb-scarlet.clap` / `org.scarlet.freeverb-scarlet` require their original DSP
or manual insert replacement. Old state is not silently rewritten.

## License

MIT. [UPSTREAM.md](UPSTREAM.md) records source provenance. Adapter, DSP, CLAP
bindings and CLAP notices are preserved and bundled with each DSP artifact.
