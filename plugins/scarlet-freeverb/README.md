# Scarlet Freeverb CLAP

A Scarlet-native stereo reverb using the MIT-licensed DSP from
[trevyn/freeverb at d89365ce8381751bea6b0e85294b7f6da3ad98ef](https://github.com/trevyn/freeverb/tree/d89365ce8381751bea6b0e85294b7f6da3ad98ef),
Copyright (c) 2018 Ian Hobson. The original source and license are preserved in
`vendor/freeverb`. This is a port of that DSP with a new CLAP adapter, not a
binary distribution of an existing macOS/Linux plugin.

The DSP arithmetic, tuning and sample-rate scaling are preserved. The port
replaces heap delay lines with fixed storage initialized before processing,
adds tail reset, and excludes freeze mode. Tests compare every output sample
against the original implementation at 8, 44.1, 48 and 96 kHz. The CLAP adapter
reuses Resonara Gain's MIT freestanding ABI bindings and lifecycle design.

Five normalized parameters: Wet (default 0.3), Dry (1), Room size (0.5),
Damping (0.5), Width (1). For an Aux send reverb use Wet=1 and Dry=0. Parameter
changes support CLAP sample offsets; reset clears the tail and preserves
parameters. Versioned opaque state stores all five values, with partial stream
I/O and atomic rejection of corrupt states. Bypass is controlled by the host.

Stereo float32, zero latency, 8–96 kHz, up to eight independent instances per
loaded library. Audio process/flush/reset allocate no memory and use no locks,
threads, TLS or OS imports. The fixed DSP storage occupies about 6 MiB per
library. The host must continue feeding silence to hear the full tail. Custom
CLAP GUI is not implemented; use Resonara's generic parameter editor.
Its rotary knobs share the exact numeric fields. Drag vertically or use arrows
(Shift for fine adjustment); Home/double-click restores the opening value.
Apply commits the edits and state, while Cancel leaves the plugin unchanged.

## Build and install

Run in Scarlet's pinned Nix development shell:

```sh
python3 plugins/scarlet-freeverb/build.py --arch aarch64 --output artifacts/freeverb-aarch64
# Or --arch riscv64 with an independent output directory.
```

The builder stages `system/plugins/scarlet-freeverb.clap` and its license
notices only after auditing the native ELF. Install both in `/system/plugins`
on the matching Scarlet CPU architecture. `scripts/scarlet-image` includes
Freeverb and Gain in its application overlay. Existing guest disks are never
replaced implicitly.

In Resonara: empty Insert → Installed CLAP effects… → Rescan → Scarlet Freeverb.
The identifier is `org.scarlet.freeverb`. Projects retain its state and library
basename, using the existing CLAP discovery/load path.

## Verify

```sh
cargo test --locked --manifest-path plugins/scarlet-freeverb/Cargo.toml
python3 plugins/resonara-gain/test-build.py
cargo build --locked --release -p resonara --example scarlet_native_smoke --target aarch64-unknown-scarlet
python3 crates/resonara-clap/scripts/audit-scarlet-host.py \
  target/aarch64-unknown-scarlet/release/examples/scarlet_native_smoke --arch aarch64
```

Run the audited example as `/bin/native-smoke clap /share/resonara-validation/clap`
in a Scarlet guest with the plugin installed. It verifies real dynamic loading,
parameter changes, wet impulse/tail, state save/reopen, WAV export and exact
bypass through Resonara Engine. `audio` additionally verifies SAS playback.
`picker` and `save-picker` exercise ScarletUI's native Files interface with
optional extension filters, without blocking the UI thread.

Runtime verification on Scarlet AArch64 passed dynamic loading, wet tail
energy 2.3789181011455605, state/reopen/export/exact bypass and SAS playback.
The captured output PCM is nonzero. Resonara's native window also displayed
all five parameters and retained a Room size edit after reopening the editor.
Logs and screenshots are in `artifacts/native-plugin-vm`; RISC-V64 is build/ELF
audited, with runtime execution currently verified only on AArch64.

Native linking uses `-Bsymbolic-functions`, not `-Bsymbolic`: the latter emits
DF_SYMBOLIC, which Scarlet's loader rejects. The builder's flag regression
checks this in addition to undefined imports, dynamic dependencies, TLS and
relocations. A successful ELF audit does not alone certify runtime execution.
