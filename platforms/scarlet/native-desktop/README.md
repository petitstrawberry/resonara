# Native Scarlet desktop profile

This is the source recipe for an explicitly selected native Resonara runtime.
It is a separate image profile, not the full Debian/Wine compatibility image.

```sh
./scripts/scarlet-image /path/to/Scarlet --profile native-desktop
./scripts/scarlet-run /path/to/Scarlet --profile native-desktop --no-build
```

The wrapper generates `projects/aarch64-limine-resonara-native` in the supplied
Scarlet checkout. It copies the official full project's BSP source/configuration
and initially seeds its Cargo/project locks. Its own manifest, SDK cache, rootfs,
ESP, GPT image, local app overlay and VM lock remain separate from
`aarch64-limine-full`. The full project is not modified. Local staging and audit
reports are under Resonara's ignored `artifacts/scarlet-native` directory.

The recipe keeps the selected checkout's exact `bundles/desktop/bundle.toml`.
That bundle includes base and CLI utilities, SWS, SAS, the desktop shell, native
applications, fonts, cursors, icons and service definitions. The initramfs uses
the same base and CLI bundles as the full project. Kernel features, static
modules, BSP and the official QEMU runner are retained. The image wrapper adds
only audited Resonara, its launcher, and the audited native Gain / Freeverb effects
and their license notices.
The top-level Debian/Wine, experimental and in-guest Rust-toolchain bundles are
not selected. This does not remove individual layers from the desktop bundle.

The full-profile wrapper remains available and is still the default:

```sh
./scripts/scarlet-image /path/to/Scarlet --profile full
```

## Storage and preservation

The rootfs uses the SDK's official ext2 format and a 2,048 MiB minimum. The SDK
initially creates it with sparse `truncate`, then grows its logical size as
needed to fit the staged payload: approximately 4/3 of the staging allocation
plus 64 MiB, aligned to 16 MiB. The minimum is not a maximum.

The ESP is at least 64 MiB and grows with kernel/initramfs payload. The resulting
GPT therefore starts around **2.06 GiB or larger**. The pinned SDK's GPT copy has
no explicit hole-preserving implementation, so budget a fully allocated final
disk, plus ext2 allocated blocks, staging, downloads and compiler caches. Actual
sizes and remaining disk space must be checked after the build; no fitted-image
or boot claim is implied by this source recipe.

Normal runs reuse the existing disk, preserving guest saves. Reconstruction
requires `--replace-image`, after exporting anything needed from the guest.
The wrapper's project lock spans compilation, composition and VM runtime; it
cannot coordinate independently launched upstream runners. Existing or edited
local overlays and project source files are refused rather than overwritten.
SDK-managed lock-file updates are retained. If the selected Scarlet source or
profile template changes, review the generated project before regenerating it.

## Known dependency boundary

The exact desktop bundle includes the native `mozc-server` launcher and Mozc
client service. Its conversion engine is a Linux ABI binary at
`/usr/lib/mozc/mozc_server`; that engine is not provided by this native-only
profile. Mozc conversion is unavailable and may log a service failure. Native
SKK and the SWS/SAS dependency chain are retained and do not depend on Mozc.
Wayland bridge metadata is retained, but this profile supplies no Linux apps.

Other external native desktop app builds remain pinned by the selected Scarlet
bundle. Any missing build dependency must be addressed rather than silently
removing the affected application. The same exact AArch64 EDK2 firmware and
GL-enabled VirtIO GPU requirements apply as for the full project. Audio capture
and actual guest playback must be verified separately from image construction.

The application overlay also installs the MIT-licensed Resonara Freeverb port
(`org.resonara.freeverb`) and its license notices in `/usr/lib/clap`. Both Gain
and Freeverb must pass ELF audits before staging begins. In Resonara, select
an empty Insert → Resonara Freeverb. Its dedicated ScarletUI editor is shared
with macOS; see [Freeverb](../../../plugins/resonara-freeverb/README.md).
