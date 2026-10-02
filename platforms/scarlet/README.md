# Native Scarlet bundle

Include Resonara's bundle rather than only a `kind = "cargo"` executable layer:

```toml
[[layers]]
kind = "bundle"
source = { git = "https://github.com/petitstrawberry/resonara", rev = "<published commit>" }
bundle = "platforms/scarlet/bundle.toml"
```

The bundle installs `/bin/resonara` and builds/audits the native Gain and
Freeverb CLAP plugins, including Freeverb's embedded GUI, into `/usr/lib/clap`
with their license notices. Both plugin audits must succeed before either
plugin is installed. The plugin script has no cached `output` field, so source
changes are checked on every image build. A Scarlet Rust toolchain with rust-src
and readelf/llvm-readelf is required, as in the Scarlet development shell.

For a local checkout, a project can instead include this bundle by a relative
`path` in `scarlet.local.toml`. Do not include both the old executable-only
Resonara layer and the new bundle in a published bundle recipe.

Linux and Scarlet search `/usr/lib/clap`, `/usr/local/lib/clap`, `~/.clap` and
`~/.local/lib/clap`, in addition to absolute `CLAP_PATH` entries and the app's
own `plugins` directory. Standard effects prefer the app's own sidecar, then
use the same catalog. Overrides `RESONARA_CLAP_LIBRARY` and
`RESONARA_FREEVERB_LIBRARY` remain authoritative.

The [CLAP specification](https://github.com/free-audio/clap/blob/main/include/clap/entry.h)
names `~/.clap` and `/usr/lib/clap` for Linux; the other two paths are additional
compatibility locations. Each plugin binary must match the host OS and CPU.
Rebuild the guest image to apply packaging changes; running guests are not
updated by building artifacts alone.
