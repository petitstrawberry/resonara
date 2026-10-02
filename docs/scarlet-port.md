# Scarlet native port

Resonara selects its platform at the Rust target boundary:

- Linux/macOS: ScarletUI `std,platform-winit` and CPAL
- `aarch64-unknown-scarlet` / `riscv64gc-unknown-scarlet`: ScarletUI
  `std,platform-sws` and the Scarlet Audio Server (`sas-client`)

This uses the Scarlet Rust toolchain's normal `std`, not the legacy `no_std`
facade. Native file I/O, worker threads, JSON persistence and audio decoding
remain shared. The application uses ScarletUI's common asynchronous file-dialog
API; the current Files provider cannot honor Resonara's extension filters, so
the existing in-app browser is used on Scarlet.

## Dependencies and build

ScarletUI remains pinned to `cdd852eb22678b7b0d1c7e035c1f2b2ba4d057a7`.
The portable renderer patch under `vendor/` is retained. Native audio uses
`sas-client` from Scarlet `b3d2a55740a3d2ca49daad0ec7baba233f706f7a`, matching
ScarletUI's native dependency source; that client and its SAS protocol are
unchanged in the inspected Scarlet dev revision
`0639a916dfd652e9b2c1ea740cacc1c09743d9eb`.

The pinned SWS client defines capability-protocol version 11; that server reports
13. Source comparison found only additive scene/GPU-extension messages and
capability bits, with existing wire fields unchanged. Capability discovery
accepts the server's reported version. This supports retaining the upstream UI
pin, but is not a substitute for testing the actual SWS connection and renderer.

Enter the inspected Scarlet checkout's pinned development shell, then run:

```sh
cd /path/to/Scarlet
nix develop
cd /path/to/resonara
bash scripts/verify-scarlet
```

The script builds both native targets and runs the mandatory native host ELF
audit before accepting an output. An optional target argument limits the build
to that architecture. Scarlet PIC applies to dependency code; executable-only
link flags in `resonara-app/build.rs` retain the resident loader's four `dl*`
imports. The audit rejects all other unresolved imports and unsupported ELF
features. See the [CLAP host contract](../crates/resonara-clap/README.md). Git dependencies and `Cargo.lock` are used for
normal builds; adjacent source checkouts and machine-specific Cargo patches
are unnecessary.

## Image profiles and wrappers

For the native Resonara/SWS/SAS runtime without the unrelated Debian/Wine image,
select the separate native-desktop profile explicitly:

```sh
./scripts/scarlet-image /path/to/Scarlet --profile native-desktop
./scripts/scarlet-run /path/to/Scarlet --profile native-desktop --no-build
```

This generates `projects/aarch64-limine-resonara-native` in the Scarlet checkout,
with its own BSP copy, manifest, SDK locks/cache, images and VM lock. It preserves
the selected checkout's exact desktop bundle, including native SWS/SAS services,
base/CLI utilities, fonts, graphics assets and native desktop applications. The
original `aarch64-limine-full` project is not modified. Debian/Wine, experimental
apps and the in-guest Rust-toolchain top-level bundles are not selected. Native
staging and audit reports use `artifacts/scarlet-native` in Resonara.

The native rootfs has a 2,048 MiB minimum, and the SDK grows it if required by the
staged payload. It uses the SDK's sparse ext2 construction; the final GPT has no
explicit hole-preserving copy guarantee. Budget at least about 2.06 GiB for the
GPT (more if the rootfs or ESP grows), plus staging, ext2 allocations, downloads
and compiler caches. Check actual available space before composition; this
estimate is not a measured image result.

The exact desktop bundle retains Mozc service metadata, but its actual
`/usr/lib/mozc/mozc_server` conversion engine requires the excluded Linux layer.
Mozc conversion is unavailable in this profile and can log a service failure.
Native SKK, SWS and SAS remain present and do not depend on it. See the
[profile recipe and boundaries](../platforms/scarlet/native-desktop/README.md).

For a deliberate guest validation build, an optional local rootfs directory can
append fixtures after the audited app/plugin layer:

```sh
RESONARA_SCARLET_VALIDATION_ROOTFS=/path/to/validation-rootfs \
  ./scripts/scarlet-image /path/to/Scarlet --profile native-desktop
```

That directory may contain only ordinary files and directories beneath
`share/resonara-validation`; symlinks and other guest paths are refused. It
cannot overwrite the audited application or plugin. The variable is unset by
default, applies only to native-desktop image composition, and never causes an
existing disk to rebuild during normal launch. Add `--replace-image` only when
replacement is intended after exporting guest files. Our exact app-only overlay
can be extended with this fixture layer; an edited overlay or a different
previous fixture source requires separate review rather than silent replacement.

The full compatibility profile remains the default and is explicitly selectable:

```sh
./scripts/scarlet-image /path/to/Scarlet --profile full
./scripts/scarlet-run /path/to/Scarlet --profile full --no-build
```

The image wrapper enters that checkout's Nix shell, builds the native app with
`--locked`, and runs the mandatory ELF host audit before staging it. It also
builds/audits the bundled native Resonara Gain CLAP effect and installs it with
its license at `/system/plugins/resonara-gain.clap`. Failed host or effect audits
prevent staging and image composition. Each selected profile receives its own
additive local rootfs layer and launcher entry.

Normal runs reuse the selected guest disk. Image reconstruction requires
`--replace-image` with the same `--profile`, after exporting guest-created files.
The wrappers share an advisory project lock and check a configured QMP socket
before writing or launching. Independently launched upstream VMs do not
participate in that lock and must be stopped separately. Existing or edited
`scarlet.local.toml` and generated native-project source are never silently
replaced. Generated SDK image/Cargo lock updates are retained; original full
project locks are not rewritten by the native profile. There are no committed
machine-specific paths.

The run wrapper defaults to TCG, persistent guest disk writes and WAV audio
capture. It does not promise audible host-speaker output. For disposable tests,
`SCARLET_QEMU_SNAPSHOT=1` discards guest file changes when QEMU exits; leave it
disabled when saving sessions. Native ScarletUI needs a GL-capable
GPU: use the official GTK/SDL display selection in a desktop session. Plain
QEMU VNC selects a non-GL VirtIO GPU and is insufficient for the SGFX renderer.

## Audio behavior

The adapter selects 48 kHz stereo S16LE from SAS's supported formats.
The existing engine resamples the session to
48 kHz and preserves its mixer, graph and metronome behavior. The adapter uses
256-frame producer blocks and a 1,024-frame shared ring (about 21.3 ms of queued
audio, plus up to 255 staged frames and SAS/device/QEMU latency). The engine's
published playhead reports produced audio; it can lead the audible position
by the staging and output queues.

The producer thread owns the engine, socket and mapped ring. CLAP plugin
ownership guards remain on the creating/UI thread, separate from realtime
proxies. Audio shutdown drops the CPAL stream or joins the SAS worker before
deactivation/destruction/unloading on that owner thread; Audio cannot be moved
between threads. Native device setup finishes before plugin activation, so a
connection/configuration error cannot strand a plugin on a worker thread. Buffers are fixed
arrays allocated before its loop. Ring backpressure never skips audio: a short
write retains its remainder, and an unwritable ring does not render a new block.
No file I/O, logs, locks, allocation or graph destruction occur in the producer
step. This does not guarantee that the OS, SAS, QEMU or Rust thread machinery
have real-time scheduling or zero allocations.

Configuration times out after three seconds. A closed ring or three seconds
without progress sets the common audio error flag. Natural completion flushes staged PCM and starts a cancellable shared-ring
drain. The finished output stays alive until the next transport/edit action,
allowing SAS's separate device queue to finish; this retains one idle SAS
connection rather than guessing a hardware drain time. The UI keeps this
finished output separate from active playback. Stop, seek and drop signal
cancellation and join the worker; teardown drops the socket without waiting for
a blocking SAS drain reply. Each new playback creates a fresh stream so a seek
cannot inherit the previous ring's queued samples.

Scarlet's full-image runner disables audio by default. On Linux, enable it with
`SCARLET_QEMU_AUDIO=1` and explicitly choose a supported Linux QEMU audio driver;
its default `coreaudio` driver is macOS-specific. WAV capture can verify guest
audio samples separately from real-speaker playback.

## Verification levels

Keep these results separate when recording a run:

1. Host workspace tests, including the host-testable SAS PCM pump
2. Cross-build for each native architecture
3. Image construction with the compiled application and launcher entry
4. QEMU boot and actual SWS window rendering, resizing and interaction
5. SAS callback progress and non-zero captured guest audio
6. Import, save, reopen and export inside the guest

A successful cross-build does not imply any later level. Runtime results and
the exact selected commits belong in `validation.md` after they are observed.
