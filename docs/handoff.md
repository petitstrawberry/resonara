# Resonara source handoff

This source snapshot continues base commit
`15b0ced5d49714790c879319034584ceb60d55f7`. The user requested commit/push and
continuation on Mac. The dedicated handoff branch is
`feat/native-daw-workbench-2026-10-01`; no pull request or merge is included.
A separate patch applies to the exact base above. The portable source archive contains the complete project,
without `.git`, build output, logs, screenshots or large licensed audio fixtures.

## Run the extracted source

In a Linux/macOS environment with Nix available:

```sh
tar -xzf resonara-source-2026-10-01.tar.gz
cd resonara
./scripts/dev cargo run --locked -p resonara
# Start without the generated demo
./scripts/dev cargo run --locked -p resonara -- --empty
# Reopen a saved project
./scripts/dev cargo run --locked -p resonara -- --project session.resonara.json
```

ScarletUI is fetched from its pinned Git revision; no adjacent ScarletUI/SGFX
checkout or machine-specific source path is needed. Nix and Cargo may need
network access to download the pinned dependencies. The generated demo runs
without the optional real-audio test files.

For checks:

```sh
./scripts/dev cargo test --locked --workspace
# Linux: dependencies, formatting, tests, build and ALSA file-output smoke
./scripts/dev bash scripts/verify
```

The pinned Nix flake was evaluated successfully in the cloud validation
environment. Launching `nix develop` there was blocked by its private-mount-
namespace restriction. Linux builds/tests/native interaction instead used the
same downloaded Scarlet Rust toolchain directly with local development
libraries. A successful cloud Nix development-shell launch is not claimed.
macOS compilation and physical audio-device playback remain unverified.

The final cloud native check of the resize/root-invalidation fix was interrupted
by long computer-control timeouts. Its mounted-pipeline regressions pass, but
native confirmation and hardware-backed UI timing must continue on Mac. Do not
interpret the 30 Hz playhead / 20 Hz meter targets as achieved frame-rate proof.

## What is in this snapshot

- Native ScarletUI arrangement, inspector, compact mixer, vertical gain faders,
  antialiased pan knobs, real stereo track/master meters and source-accurate
  mono/stereo waveform lanes
- WAV import, non-destructive move/split/trim, transport, mute/solo, undo/redo,
  project JSON persistence and stereo WAV export
- Consistent Spectrum-referenced gain controls and separate peak-scale geometry;
  see [the source math](mixer-scale-reference.md)
- Compiled block-DAG audio scheduling with preallocated scratch, shared
  node-output reuse and core APIs for buses, pre/post sends and basic inserts
- Automated correctness/real-time allocation regressions, genuine long-audio
  tests, native interaction evidence and a matched pre-DAG/current CPU comparison

Read [validation](validation.md), [audio graph](audio-graph.md) and
[licensed audio/performance](audio-fixture.md) for the actual tests and limits.

## Important remaining scope

Custom bus/send/insert routing is a core API foundation; routing UI,
custom-routing persistence/export, plugin hosting, effect-tail extension, PDC
and live graph swapping are not implemented. Project JSON embeds source audio
per clip and is inefficient for large sessions. Native platform file-dialog
adapters are planned; the current tested picker is the in-app browser. Recording,
MIDI, time stretching and higher-quality resampling remain future work.

The real-time callback tests establish no application render allocation,
deallocation or locks in the exercised cases. Shared-cloud timings and a paced
file sink do not establish device latency, dropout-free playback or hardware-GPU
frame rate. This is a functional native audio-editing foundation, not a claim
of complete production-DAW feature parity.
