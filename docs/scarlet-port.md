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

ScarletUI is pinned to `2e5e96a5c29086c3b555c85f7835e81ecf529290`, including
its merged Canvas resize fix. The former renderer patch is removed. The SWS
protocol package under `vendor/sws-protocol` is updated to library
sources from Scarlet `0639a916dfd652e9b2c1ea740cacc1c09743d9eb` (version 13).
Both ScarletUI and its SWS client resolve to that one protocol package.

The old SWS protocol dependency used capability version 11 while the inspected
server reports 13. Although the wire changes are additive, ScarletUI requires
exact version equality to enable its application SGFX renderer. The mismatch
silently selected CPU painting, which ignores Canvas extensions; region text
and playhead still appeared over the Canvas placeholder, but waveform, grid and
region color did not. The provided screenshot's region pixels were `(6, 8, 14)`,
exactly the placeholder color. Server-side GPU composition does not establish
that the application selected its GPU paint backend.

Updating the protocol supplies version 13 and preserves the upstream exact-match
check and SGFX Canvas path. `sws-client`, `sas-client` and SGFX retain the shared
Scarlet runtime pin `b3d2a55740a3d2ca49daad0ec7baba233f706f7a`: their connection
and audio implementations are unchanged in the inspected revision. Updating
only the client runtime would duplicate `scarlet-os` identities and its global
event-return trampoline. The protocol has no runtime dependency with normal
`std`, so it can be updated independently. Its optional legacy facade retains
the shared runtime pin. `scripts/check-dependencies` checks the protocol, UI and
runtime identities. Remove this protocol patch when upstream ScarletUI and SGFX
update their dependencies together.

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
builds/audits the bundled native Resonara Gain and Freeverb CLAP effects and installs
them with their licenses under `/usr/lib/clap`. Failed host or effect audits
prevent staging and image composition. Each selected profile receives its own
additive local rootfs layer and launcher entry.

For Scarlet's own image recipes, include the
[native Resonara bundle](../platforms/scarlet/README.md), which installs both
the app and the standard plugins. An executable-only Cargo layer installs no
CLAP plugins; the app build script bundles Freeverb only on macOS/Linux.

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

The Scarlet producer requests a current-thread, single-CPU Deadline reservation
before rendering: period and relative deadline are 5,333,333 ns (256 frames at
48 kHz), with 2,666,666 ns runtime per period. This reserves 50% of one CPU;
SAS's own output thread requests 25%. The current CPU is selected using
`scarlet_os::scheduler`, sharing the existing SAS/SGFX runtime dependency pin.
If admission or scheduler queries fail, playback continues under its original
policy and logs the failure. This reservation is not a guarantee against heavy
DSP, SAS/device underruns or QEMU/host scheduling delays.

`[Resonara audio]` logs the accepted CPU and budget once at startup, then the
kernel's deadline miss/overrun counters at Stop, failure or natural completion.
These counters are scheduler observations, not SAS underrun counts. The prior
policy is restored when rendering finishes, before retaining the idle SAS
connection for its final hardware tail. `RESONARA_SCARLET_DEADLINE=0` disables
the reservation for an A/B comparison. Queries and logs run outside the PCM loop.

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
