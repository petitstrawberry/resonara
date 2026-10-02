# Native CLAP effects

Resonara now has a small, separate CLAP host and a cleanroom gain effect. CLAP
is a C interface, not Linux binary compatibility: the effect must be built for
the actual operating system and CPU. The app can now discover installed native
CLAP effects as well as the bundled `org.resonara.gain`. Admission still requires
one main stereo float32 input/output, opaque state, no note ports and zero
latency. Custom plugin windows, instruments, MIDI, sidechains, latency
compensation and dynamic port changes are not implemented. See the
[GUI integration contract](clap-gui.md) for the intended editor architecture.

## Components and lifecycle

- `resonara-clap` owns the C ABI adapter, library registry, extension validation,
  parameter/state calls, and preallocated planar process buffers
- `plugins/resonara-gain` is a separate, dependency-free `no_std` effect workspace
  with minimally adapted MIT CLAP declarations and license notices. Gain is a
  stereo multiplier, parameter ID 0, range 0–2, default 1. Its opaque versioned
  state is 16 bytes. It uses a fixed 64-instance pool and no libc, TLS, or OS API
- `resonara-core::plugins` stores identity, opaque state, and cached generic
  parameter metadata. Saved project paths never authorize loading code. Only
  application search configuration and installation locations are resolved
- The graph creates one processor per active insert. Fanout reads that single
  output, so sends never process the same effect again. Bypass omits the effect
  from the prepared graph
- `PluginOwner` remains on the control/creator thread. The `Send` realtime proxy
  can move into CPAL or SAS. Audio teardown stops/drops the stream or joins the
  worker, releases the proxy, then destroys/deactivates through the retained
  owner. Export keeps preparation, rendering and cleanup on its own owner thread
- The prototype brackets each quantum with CLAP start/process/stop in a symbolic
  audio-thread scope, without resetting DSP state. This is a deliberate small-host
  tradeoff, not a measured guarantee for third-party plugins
- Host processing does not allocate, load libraries, lock, perform state I/O, or
  free plugin ownership. Planar storage is allocated during activation and
  included in graph scratch admission. Unsupported requests or invalid/nonfinite
  process output latch an audio error and stop playback safely

Missing or incompatible effects remain editable placeholders with their saved
state intact. Playback bypasses unavailable effects and reports the count.
Export fails before creating its WAV when an active required effect cannot be
prepared. Generic parameter changes prepare a separate inactive instance and
save its opaque state before one undoable graph edit; invalid values or failure
do not replace the saved state.

## UI

An empty insert slot opens the effect picker. `Resonara Gain` loads the bundled
CLAP effect. `Installed CLAP effects…` opens a scrollable catalog with Rescan,
load errors and keyboard selection. Multiple descriptors in one library are
separate choices. Its slot opens a scrollable generic parameter popup; hidden
parameters are omitted and read-only parameters are displayed without input.
One Apply restores one inactive instance, edits its parameters and saves once.
The native GUI negotiation API is available to the future platform adapter;
custom windows are not yet displayed. The dedicated native slot separates name, bypass,
and context hit areas; context actions reorder or remove without permanent
button clutter. Edits and CLAP bypass prepare a replacement graph and switch at
an audio block boundary, preserving transport and the open output device.
Replaced processors are retired outside rendering before their owners are
destroyed on the creating thread. Structural swaps initialize fresh DSP state;
crossfades and crackle-free switching are not guaranteed.
Keyboard operation after focus uses Enter, Space, and Shift-F10;
ScarletUI does not currently provide general Tab traversal.

The slot workflow was checked against [Logic’s plugin controls](https://support.apple.com/guide/logicpro/add-remove-move-and-copy-plug-ins-lgcpbc218a22/mac)
and [Cubase’s insert controls](https://www.steinberg.help/r/cubase-pro/15.0/en/cubase_nuendo/topics/mixconsole/mixconsole_bypassing_insert_effects_c.html).
Cubase's live bypass keeps DSP running. Resonara's CLAP bypass replaces the
prepared graph; its built-in bypass retains and freezes the effect's state.

## Build and run on Linux

Use the normal repository toolchain, then:

```sh
cargo build --release --manifest-path plugins/resonara-gain/Cargo.toml --locked
export RESONARA_CLAP_LIBRARY="$PWD/plugins/resonara-gain/target/release/libresonara_gain.so"
cargo run --locked -p resonara
```

The environment override must be an existing absolute file. Without it, the
application checks `plugins/resonara-gain.clap` beside its executable, then the
same installation catalog used for other effects. Freeverb follows the same
rule, with `RESONARA_FREEVERB_LIBRARY` as its override. The filename stored in a project is an
identity, never a path passed directly to `dlopen`.

Other effects are discovered recursively as `.clap` libraries in absolute
`CLAP_PATH` directories (colon-separated on Unix) and `plugins` beside the executable.
Linux and Scarlet also search `/usr/lib/clap`, `/usr/local/lib/clap`, `~/.clap`
and `~/.local/lib/clap`;
macOS searches `~/Library/Audio/Plug-Ins/CLAP` and `/Library/Audio/Plug-Ins/CLAP`.
macOS bundles use CoreFoundation to resolve `CFBundleExecutable`; entry init
receives the bundle path, while leases deduplicate by canonical binary path.
The [CLAP entry contract](https://github.com/free-audio/clap/blob/main/include/clap/entry.h)
specifies the Linux `~/.clap` and `/usr/lib/clap` paths; the other two Unix
locations are additional compatibility paths.

Projects retain a basename and plug-in ID; absolute paths and path traversal never
become loader input. Distinct installed files with the same basename are rejected
as ambiguous. Symlink cycles and duplicate canonical locations are handled during
indexing. Search is bounded to 16,384 entries, depth 32, and 1,024 library names;
the catalog lists at most 1,024 effects. Rescan refreshes the filesystem inventory
and executes library entry callbacks for descriptor inspection on the control
thread. A cached inventory avoids repeated recursive scans during UI painting.
After changing installations, rescan before reloading effects.

## Verification

Ordinary tests do not require a deployed binary. Build the actual effect and run
these additional tests so loading, C ABI, and process behavior are exercised:

```sh
cargo test --locked --workspace
cargo test --manifest-path plugins/resonara-gain/Cargo.toml --locked
export RESONARA_CLAP_LIBRARY="$PWD/plugins/resonara-gain/target/release/libresonara_gain.so"
export RESONARA_TEST_CLAP="$RESONARA_CLAP_LIBRARY"
mkdir -p target/clap-tests
cp "$RESONARA_CLAP_LIBRARY" target/clap-tests/external-gain.clap
export CLAP_PATH="$PWD/target/clap-tests"
cargo test --locked -p resonara-clap --test native_gain -- --include-ignored
cargo test --locked -p resonara-core --test clap -- --include-ignored
cargo test --locked -p resonara clap_ -- --include-ignored
RESONARA_SMOKE_ROUTING=1 RESONARA_SMOKE_CLAP=1 bash scripts/audio-smoke
```

Linux tests have exercised real loading, independent instances, generic parameter
and opaque-state roundtrips, exact samples, failure latching, export, owner-thread
teardown, and first/repeated callback allocation audits. CPAL/ALSA file-sink
playback with the real effect has produced nonzero PCM. The final 2026-10-02
earlier cloud handoff run completed 301 ordinary workspace tests, all nine explicitly
enabled real-CLAP cases, ten effect tests, and nine executable-auditor tests.
The installed-effect update passed 323 workspace cases on macOS with actual
native gain binaries (one long stress workload excluded), including external
catalog loading, continued playback during edits, save/reopen/export, allocation
audits and bundle executable resolution. Both Scarlet release builds passed their
ELF audits. These checks do not measure real hardware latency or dropouts.

## Scarlet status and boundaries

The effect build helper produces separate native AArch64 and RISC-V ELF shared
objects and audits imports, dynamic tags, and relocation kinds. See the plugin’s
README and generated `artifacts/<arch>/build.json` for provenance. The packaging
path is `/usr/lib/clap/resonara-gain.clap`.

Scarlet’s resident `/bin/scarlet-ld` provides the loader C API. It requires eager
`RTLD_NOW | RTLD_GLOBAL`; this host must not embed a second scarlet-dl runtime.
TLS, symbol versions, IFUNC, RPATH, and unloading are outside the current loader
scope. Building an ELF is insufficient: native executable imports and dynamic
relocations must be audited, then the effect must actually run in the guest.
The native host link now uses a documented Scarlet-only link input to retain
the four resident-loader imports with current LLD. A strict post-link auditor
rejects unexpected dynamic symbols, dependencies, interpreters, protections, or
relocations before staging; see `resonara-clap` documentation. Both native application builds (AArch64 and RISC-V64, before the later M/S-only
alignment refinement), as well as the
small host/consumer fixtures, passed their strict ELF audits. The application
audits report exactly the four expected `dl*` imports, no `DT_NEEDED` or TLS,
and only `JUMP_SLOT`/`RELATIVE` relocations. Reports are written to
`artifacts/audit-{aarch64-unknown-scarlet,riscv64gc-unknown-scarlet}.json`.
Actual guest execution remains a separate required check and is not inferred
from compilation.
