# Resonara Gain

A small, cleanroom, `no_std` Rust CLAP effect for testing Resonara's native
shared-object hosting. This is a separate Cargo workspace: it does not alter
or join the DAW's normal builds. It has no GUI, network, filesystem, heap,
OS calls, allocator, locks, or thread-local storage. The only dependency is
the local MIT-licensed no_std subset of `clap-sys 0.5.0` (CLAP 1.2.2).

## Host contract

- Exported **data** symbol: `clap_entry`; factory: `clap.plugin-factory`
- Stable plugin ID: `org.resonara.gain`; name: `Resonara Gain`
- One main stereo float32 input, one main stereo float32 output; both port
  IDs are `0`, mutually paired for in-place processing
- Parameter ID `0`, `Gain`: plain linear multiplier `0.0..=2.0`, default `1.0`
- Extensions: `clap.audio-ports`, `clap.params`, `clap.state`
- Global `CLAP_EVENT_PARAM_VALUE` events are applied at their sample offset.
  Invalid IDs, note/channel/port/key-specific events, nonfinite values,
  undersized events, foreign event spaces, and out-of-block or backwards
  sample offsets are ignored. Finite event values are clamped to the range
- `params.flush` accepts the same global parameter events. It must run on
  the main thread while inactive or on the audio thread while active
- No smoothing, modulation, 64-bit samples, note ports, latency, or tail
- `reset` preserves the parameter, as CLAP requires
- State save emits exactly 16 bytes: ASCII `RSGN`, little-endian `u32` version
  `1`, then little-endian IEEE754 `f64` gain. Load consumes those 16 bytes,
  supports short reads, and changes nothing on truncated data, bad magic,
  unsupported version, nonfinite gain, or gain outside `0..=2`. It does not
  consume subsequent bytes in an enclosing stream. Successful load requests
  `CLAP_PARAM_RESCAN_VALUES` if the host provides its params extension

Entry initialization is reference-counted. Independent instances occupy one
of 64 statically reserved slots; the 65th concurrent instance fails cleanly
with a null pointer. Destroy releases its slot. Concurrent factory calls
use atomic admission; parameters and lifecycle are atomically stored.
Creation/destroy are not realtime operations. CLAP lifecycle ordering and the
host's optional `clap.thread-check` extension are checked without assertions,
logging, or unwinding. Invalid-thread void calls are ignored; invalid-thread
boolean/process calls fail. Hosts must still obey CLAP pointer lifetime,
alignment, buffer sizing, destruction, and nonoverlapping audio callback
requirements. This is native in-process code, not a security sandbox.

The processing callback has no intended panic path, allocation/deallocation,
locks, system imports, or I/O. It invokes the host's optional thread-check
and input-event callbacks, whose own realtime behavior remains the host's
responsibility. Fixed-size compiler-generated copies use local freestanding
`memcpy`/`memset`, not a system libc. ELF builds bind them locally.

## Build and test on Linux

Use an installed Rust toolchain and `readelf` (or `llvm-readelf` on macOS):

```sh
cd plugins/resonara-gain
cargo test --locked
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
python3 build.py --arch linux --output artifacts/linux
```

Output: `artifacts/linux/resonara-gain.clap` (x86_64 Linux).
A direct `cargo build --release --locked` also creates
`target/release/libresonara_gain.so`, useful as a host test fixture.
Use `--offline` after registry dependencies used by tests have been cached.
The distributed effect itself has no registry dependency.

The test suite checks ABI layouts against unmodified registry `clap-sys
0.5.0`, lifecycle/thread rejection, parameter metadata/text, sample-accurate
in-place DSP, flush validation, versioned state rollback and short I/O,
64-instance isolation/reuse, concurrent entry/factory calls, malformed-buffer
rejection without audio writes, and zero
allocations/frees across 1,000 processing blocks. These unit tests supplement,
and do not replace, loading the emitted shared object through the real host.

## Build native Scarlet shared objects

Derived from Scarlet's `tools/loader-smoke/build-rust-dso.py`; no installed
sysroot or target files are modified. Use a pinned native-Scarlet Rust
compiler with real `rust-src` and its matching linker/runtime libraries:

```sh
export SCARLET_TOOLCHAIN=/path/to/scarlet-rust
# Configure PATH, LD_LIBRARY_PATH, and CARGO_HOME for that toolchain first.
python3 build.py --arch aarch64 --toolchain "$SCARLET_TOOLCHAIN" --output artifacts/aarch64
python3 build.py --arch riscv64 --toolchain "$SCARLET_TOOLCHAIN" --output artifacts/riscv64
```

Each build generates an isolated target JSON from the compiler's real
`aarch64-unknown-scarlet` or `riscv64gc-unknown-scarlet` target, preserving its
native ABI and ISA while enabling PIC shared-object output. Cargo rebuilds
`core` and `compiler_builtins` for that exact target. Initial Cargo resolution
can download Rust sysroot build dependencies even though only these two
components are built; subsequent runs support `--offline`.

Each output has its own Cargo target directory. Packaged Scarlet artifacts:
`artifacts/{aarch64,riscv64}/staging/system/plugins/resonara-gain.clap`.
The ELF OSABI byte is set to `83` (`ELFOSABI_SCARLET`) as in Scarlet's loader
smoke fixture. This alone does not make a Linux plugin Scarlet-compatible.
Only an actual native-Scarlet compilation is packaged.

`build.py` verifies ELF64 ET_DYN/machine, the `clap_entry` data export,
architecture-appropriate relocations, no undefined imports, no DT_NEEDED,
no TLS/IFUNC, no symbol versioning, and no RPATH/RUNPATH/TEXTREL. `build.json`
records the exact rustc version, build command, artifact SHA256, and audit.
`resonara-gain.LICENSE.txt` accompanies each packaged artifact; retain it when
distributing the binary.
Compilation and ELF audit are not proof of execution: native loader,
CLAP lifecycle, and audio output require a separate Scarlet runtime test.

## Attribution

Effect implementation: MIT, `LICENSE`. Vendored binding provenance and exact
changes: `vendor/clap-sys/README.md`; binding license: `LICENSE-MIT` there.
CLAP API: https://github.com/free-audio/clap, MIT, copyright Alexandre BIQUE;
its license is included as `vendor/clap-sys/LICENSE-CLAP`.
No third-party commercial effects, plugin binaries, or proprietary SDKs are
included or downloaded by this project.
