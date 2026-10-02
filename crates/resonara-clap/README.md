# Resonara's bounded CLAP host

The application admits bundled and discovered installed native effects. The host
validates one main stereo input/output, mandatory float32 support,
no note ports, zero latency, parameter metadata and opaque state support.
Native plug-ins execute arbitrary code in the application. Loading and metadata
validation provide no sandbox or protection from a malicious/broken DSO.

## Main/audio ownership

1. `HostPlugin::load(path, Some(id))` discovers and initializes on the creating
   owner thread. Generic parameter get/format/set (inactive `flush`) and bounded
   opaque state save/load run only there.
2. `activate(rate, maximum_quantum)` returns `(PluginOwner, RealtimePlugin)`.
   Retain the owner outside the device callback. Move only the realtime proxy to
   the callback. Activation allocates its four planar float32 buffers up front.
3. `RealtimePlugin::process(&mut [[f32; 2]])` converts interleaved graph frames
   into planar CLAP buffers. Each quantum calls start/process/stop on the same
   serialized symbolic audio thread. This preserves DSP state between calls,
   never calls reset, and avoids an unmatched processing phase at backend teardown.
   Empty blocks do not invoke the plug-in. The host path allocates/deallocates
   nothing, takes no locks, loads no code and performs no state I/O.
4. Stop and destroy the audio backend / join its worker, releasing its realtime
   proxy first. Then drop `PluginOwner` on the creating thread. It deactivates,
   destroys, and releases the shared DSO. Alternatively call owner.deactivate()
   after the proxy is gone to recover an inactive `HostPlugin` for state saving.

`PluginOwner` is movable so an engine can temporarily contain owners during
preparation. This does not authorize main-thread callbacks on a different OS
thread. Wrong-thread or premature guard destruction quarantines/leaks the
instance and its DSO, rather than allowing unsafe audio-side destruction.
Successful application paths must never rely on this misuse fallback. The public
`quarantined_instance_count()` diagnostic must remain unchanged on normal teardown.

CLAP permits a symbolic audio thread to be the main thread, so offline rendering
uses the same lifecycle on its dedicated owner worker. GUI/timer integration and
arbitrary macOS plug-ins requiring the application's UI thread are not supported.
Thread-check callbacks identify the creating thread and the actual thread within
start/process/stop; they do not claim all concurrent threads are audio threads.

## Defined limitations and failure handling

- No plug-in GUI, MIDI/note ports, automation/modulation lanes, worker pool, timer,
  file-descriptor extension, transport integration, resampling, or latency
  compensation. Unknown host extensions return null.
- Main callback requests are serviced while inactive (bounded to 16 per pass).
  Requests made while processing stop the insert with `MainThreadRequest`.
  Restart and structural-change requests are explicit errors requiring reload;
  they are never falsely acknowledged. The integration reports and stops render.
- Parameter output/gesture events are accepted without recording automation;
  values/state are rescanned or saved on the inactive owner. Unsupported output
  event types are refused via `try_push = false`.
- State save/load is bounded to 16 MiB and uses checked partial-read/write streams.
  The application may impose a smaller per-insert project budget.
- Invalid process status, start failure, nonfinite output, and unsupported active
  changes produce fixed-size `ProcessError` values. The original dry input is
  untouched on failure. Plugin processing failures are sticky until reload;
  invalid caller blocks are rejected without poisoning the instance.
- Every graph insert must own a separate realtime proxy. Fanout belongs in the
  graph schedule, which should execute that node just once per quantum.

## Native editor preparation

`HostPlugin::gui_support(api)` queries embedded/floating capabilities on the
original owner thread without creating GUI resources. Incomplete GUI extensions
fall back to no editor capability and do not block audio-only hosting. Capability
is not a promise that the host supports that platform API. Native editor creation,
active main-thread service and GUI parameter transport remain to be implemented;
see [the editor integration contract](../../docs/clap-gui.md).

## Dynamic loading

Linux uses `dlopen(RTLD_NOW | RTLD_LOCAL)`; macOS uses its native flag values and
accepts native files or `.clap` bundles resolved through CoreFoundation. Bundle
entry init receives the bundle path; canonical executable paths key the leases.
Scarlet imports the C entry points from `/bin/scarlet-ld` and uses exactly
`RTLD_NOW | RTLD_GLOBAL = 0x102`. It does **not** statically link `scarlet-dl`.
All instances of a canonical DSO path share one entry init/deinit pair; registry
work runs outside processing. Scarlet currently pins code mappings after dlclose.
Do not load aliases/hardlinks to the same DSO through different canonical paths.

## Real-library tests

Build the clean-room effect, then run the integration tests explicitly:

```sh
cargo build --release --manifest-path plugins/resonara-gain/Cargo.toml
cargo test --locked -p resonara-clap --test native_gain -- --include-ignored
```

Or set `RESONARA_TEST_CLAP` to the native gain binary. These tests load its real
`clap_entry`, query metadata, change parameters, round-trip opaque state, render
several block lengths, move processing and proxy destruction to another thread,
check independent instances and repeated teardown, and count allocations and
frees starting at the very first audio call. They are ignored in the ordinary
workspace run because they require an OS-matching separately built DSO.

## ABI references

- Official CLAP headers: https://github.com/free-audio/clap/tree/main/include/clap
- Lifecycle: https://github.com/free-audio/clap/blob/main/include/clap/plugin.h
- Thread roles: https://github.com/free-audio/clap/blob/main/include/clap/ext/thread-check.h
- Rust ABI definitions: pinned `clap-sys` 0.5.0 (CLAP 1.2.2)
- Scarlet: `docs/userspace/dynamic-linker.md` and `user/lib/scarlet-dl/README.md`
  in the sibling Scarlet checkout

## Scarlet executable link contract

The crate's Scarlet-only `build.rs` emits a 281-byte **link-time-only** ELF64
symbol-table container. It has ET_DYN, the target machine ID, no program segments,
no executable bytes, no definitions/imports, three section headers (null,
SHT_DYNSYM, SHT_STRTAB), one null symbol, and one empty string. This is supplied as
an as-needed native dylib so Cargo propagates it to downstream executables.
Nothing from it is loaded or installed: the final executable must have zero
DT_NEEDED entries.

Why: [LLVM commit 994cea3](https://github.com/llvm/llvm-project/commit/994cea3f0a2d0caf4d66321ad5a06ab330144d89)
changed LLD 20+ to suppress undefined dynamic symbols when linking a PIE without
any input DSO. Consequently, simply permitting the four interpreter imports can
produce a successfully linked binary with zero call targets. The empty discarded
DSO enables proper dynamic imports without linking another loader/runtime.
This workaround is native Scarlet only; Linux/macOS use their ordinary libraries.

The executable linker arguments are:

```text
-C relocation-model=pic
-C link-arg=-pie
-C link-arg=--dynamic-linker=/bin/scarlet-ld
-C link-arg=--export-dynamic
-C link-arg=--unresolved-symbols=ignore-all
-C link-arg=-z
-C link-arg=now
```

Allowing unresolved symbols is **not sufficient evidence of a successful build**.
Before staging, run the mandatory fail-closed allowlist audit:

```sh
python3 crates/resonara-clap/scripts/audit-scarlet-host.py <executable> --arch aarch64
# or --arch riscv64; --json <file> also saves hash and ELF provenance
python3 crates/resonara-clap/scripts/test-audit-scarlet-host.py
```

The audit requires native OSABI 83, matching ELF64 ET_DYN machine, exactly the
`/bin/scarlet-ld` interpreter, PIE/eager-binding flags, and exactly dlopen/dlsym/
dlclose/dlerror in both dynamic and full unresolved symbol tables. Every import
must have one dynamic relocation. It rejects startup DSOs (including an
accidentally retained link seed), TLS, IFUNC, versioning, RPATH/RUNPATH, TEXTREL,
RELR, and relocation types outside the native loader's documented subset.
Audit before stripping `.symtab`; otherwise hidden extra unresolved references
cannot be excluded reliably.

Both AArch64 and RV64 smoke binaries and an independent downstream Cargo consumer
have been built and passed this audit. These are static ELF checks; they do not
claim guest execution. `examples/native_smoke.rs` exercises the real DSO's owner/
worker lifecycle and emits `RESONARA_CLAP_NATIVE_OK` when run successfully.
