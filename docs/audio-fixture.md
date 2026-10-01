# Licensed real-audio stress-test fixture

Downloaded and verified 2026-10-01. These local fixtures are not committed.

## Attribution and rights

**Clair de lune**, from *Suite bergamasque* by Claude Debussy; piano performance by **Laurens Goedhart**, 2011 (source date: 20 August 2011).

- Recording license: **Creative Commons Attribution 3.0 Unported (CC BY 3.0)**, https://creativecommons.org/licenses/by/3.0/
- Verified Wikimedia file/recording-license page: https://commons.wikimedia.org/wiki/File:Clair_de_lune_(Claude_Debussy)_Suite_bergamasque.ogg
- License-page revision observed: https://commons.wikimedia.org/w/index.php?title=File:Clair_de_lune_(Claude_Debussy)_Suite_bergamasque.ogg&oldid=1264119050
- Original artist source credited by Wikimedia: https://soundcloud.com/laurensgoedhart/claude-debussys-clair-de-lune
- Exact downloaded URL: https://upload.wikimedia.org/wikipedia/commons/b/be/Clair_de_lune_%28Claude_Debussy%29_Suite_bergamasque.ogg
- Wikimedia identifies the musical composition separately as public domain; the recording is CC BY 3.0 and requires attribution. Do not describe the recording itself as public domain.
- Wikimedia credits the OGG encoding to user Akaniji. The local WAV is a format conversion of this OGG using ffmpeg, with no looping, trimming, or synthetic material added. Preserve this credit, source and license with any redistributed fixture or adapted output. No endorsement is implied.

Suggested credit: “Clair de lune” by Claude Debussy, performed by Laurens Goedhart (2011), recording licensed CC BY 3.0 (https://creativecommons.org/licenses/by/3.0/), via Wikimedia Commons; OGG encoding by Akaniji. WAV converted from the OGG for testing.

## Files and measured properties

Both files contain the full, genuine recording. Duration measured by ffprobe: **304.088526 seconds (5 minutes 4.089 seconds)**. Both are stereo, 44,100 Hz.

- `clair-de-lune-laurens-goedhart-2011.ogg`: Vorbis; 15,740,765 bytes
  - SHA-256: `2f7a1e833f4e2408d9d1b893131863d790ba75cd5fbcf81292dc5ed6f25918fe`
  - SHA-1: `50d152ad99294c8dd3783b6ddde30b1dfaef19c3` (matches the Wikimedia file's published checksum)
- `clair-de-lune-laurens-goedhart-2011.wav`: signed 16-bit little-endian PCM WAV; 53,641,294 bytes
  - SHA-256: `1ce9d05f8725aa6f93adad59a601d47e2bdfd0ce5cd8eb5c3835a834a38444ab`

Conversion used:

```sh
ffmpeg -v error -nostdin \
  -i artifacts/fixtures/clair-de-lune-laurens-goedhart-2011.ogg \
  -c:a pcm_s16le artifacts/fixtures/clair-de-lune-laurens-goedhart-2011.wav
```

The decoded WAV is approximately 51.16 MiB. Both files together occupy approximately 66.17 MiB.

## Mono/stereo waveform derivatives

Two additional uncommitted test fixtures were derived from the verified PCM16
recording to make channel-independent waveform rendering visually testable.
Both preserve all 13,410,304 frames at 44,100 Hz (304.088526 seconds).

- `artifacts/fixtures/mono-left-source.wav`: one channel containing the original
  left channel exactly; 26,820,686 bytes; SHA-256
  `162a2b7d832ba30545a72a1785349c89c7fb97c4474931b7bf32ae231aee048b`
- `artifacts/fixtures/stereo-right-minus-12db.wav`: two channels; original left
  unchanged and original right multiplied by `10^(-12/20)`, then quantized to
  PCM16; 53,641,294 bytes; SHA-256
  `4c8ec6406c71a00f37f3210b6cfba58fe6ada75717d0cc755ef72005de1fa10b`

All frames were independently checked: both preserved left channels match
exactly, and the attenuated right channel differs from the mathematical target
by at most 0.535 integer PCM16 units. Validation is saved in
`artifacts/fixtures/waveform-fixture-validation.json`.

```sh
ffmpeg -v error -nostdin -n \
  -i artifacts/fixtures/clair-de-lune-laurens-goedhart-2011.wav \
  -af 'pan=mono|c0=c0' -c:a pcm_s16le artifacts/fixtures/mono-left-source.wav
ffmpeg -v error -nostdin -n \
  -i artifacts/fixtures/clair-de-lune-laurens-goedhart-2011.wav \
  -af 'pan=stereo|c0=c0|c1=0.251188643150958*c1' -c:a pcm_s16le \
  artifacts/fixtures/stereo-right-minus-12db.wav
```

These are declared channel-selection/level adaptations of the genuine
Goedhart recording, not synthetic or looped audio. The recording's CC BY 3.0
credit and source above apply to both derivatives; preserve the attribution and
describe these changes if redistributing either file. The files remain ignored
test data, outside the source patch.

## Reproduce the eight-track offline workload

After placing the licensed WAV at the path above:

```sh
mkdir -p artifacts/stress
./scripts/dev cargo run --locked --release -p resonara-core --example stress -- \
  artifacts/fixtures/clair-de-lune-laurens-goedhart-2011.wav \
  artifacts/stress/real-eight-track-mix.wav 8 \
  > artifacts/stress/core-benchmark.json
```

The workload imports the complete recording at the project's 48 kHz rate, uses
one immutable source buffer shared by eight tracks, and offsets each track by
250 ms. The individual track gains and pans vary deterministically. It renders
the entire arrangement in 512-frame blocks, checks nonzero/finite audio, checks
selected blocks against an independent sample-index mix oracle, verifies silence
after EOF, then exports and checks the stereo float WAV's frame count and format.

Timings isolate each `Engine::render` call. Total render wall time additionally
includes the sample checks. Report the build profile, host CPU, shared-machine
conditions and render backend alongside results. These measurements are offline
core throughput, not audio-device latency, dropout-free hardware playback,
GUI frame rate, or a hard real-time guarantee.

For the native UI workload, import this WAV once and duplicate the track to eight
tracks. Test timeline scrolling, zoom, track selection, mixer changes and playback
on the actual native window. Record which input actions completed and any stalls;
do not infer frame rate from screenshots. Keeping shared clips in memory avoids
unnecessarily writing repeated embedded audio into the current JSON project
format. Media, exported mixes, screenshots and logs remain under ignored
`artifacts/`; only attribution and test code are tracked.

## Earlier offline result before block-DAG integration (2026-10-01)

On this run: Linux x86_64 / Debian 13, AMD EPYC 9V74 host, nine logical workers
reported to the process, shared cloud CPU, Scarlet Rust 1.94.0-nightly / LLVM
21.1.8, optimized release build. This is one observed run, not a worst-case bound.

- Eight stereo tracks, one shared 304.089-second source buffer
- Arrangement duration: 305.839 seconds at 48 kHz
- Import/resample: 0.755 seconds
- Full render with correctness checks: 1.894 seconds
- Per 512-frame render call: median 49.5 µs, p95 94.1 µs, p99 131.3 µs,
  largest observed 1,336.8 µs
- The corresponding 512-frame / 48 kHz block period is 10,666.7 µs;
  this comparison does not establish real-time scheduler or driver behavior
- Independent oracle checked 146,802 sample values; finite/nonzero checks and
  EOF silence passed
- Export: 2.030 seconds, 117,442,060-byte stereo 32-bit float WAV; format and
  exact frame count checked

Raw output is in `artifacts/stress/core-benchmark.json`. Native GUI testing uses
SGFX/WGPU with Mesa Lavapipe software Vulkan in this environment. The timings
above are CPU engine timings and must not be described as GUI frame timings or
hardware-audio latency.

## Eight-track UI-model / CPU-layout workload

The ignored test reuses the same real recording and shared-source tracks:

```sh
RESONARA_LONG_AUDIO_WAV="$PWD/artifacts/fixtures/clair-de-lune-laurens-goedhart-2011.wav" \
  ./scripts/dev cargo test --locked --release -p resonara \
  long_audio_eight_track_ui_model_stress -- --ignored --nocapture
```

Rerun after the compact mixer, unified gain controls, pan-knob, block graph,
stereo-meter and independent stereo-waveform changes on
the same cloud CPU with a writable font cache:

- Import/resample: 1,073.5 ms
- Peak-cache and initial model setup: 33.2 ms
- 48 select + zoom + horizontal-scroll model cycles: median 8.05 ms,
  p95 18.47 ms, largest observed 19.45 ms
- First CPU tree layout: 69.4 ms (with an already populated font cache)
- Eight subsequent select + rebuild + CPU-layout cycles: median 7.89 ms,
  largest observed 23.76 ms
- Shared audio-buffer identities and unchanged session audio were verified

The test constructs and lays out ScarletUI elements without presenting a GPU
surface. It therefore measures application model and CPU layout cost only.
Actual scrolling animation, presentation latency and input responsiveness must
also be checked in the native application. Raw output is retained in
`artifacts/long-audio-ui-model-stress-packaged.log`. Earlier runs remain in
`artifacts/long-audio-ui-model-stress.log` and
`artifacts/long-audio-ui-model-stress-current-ui.log` and
`artifacts/long-audio-ui-model-stress-final.log` and
`artifacts/long-audio-ui-model-stress-final-waveforms.log`; these are individual shared-host
observations rather than a statistically controlled before/after comparison.
The final UI renders independent signed L/R envelopes. Its measured navigation
cost is higher than the earlier single-waveform workload and is recorded here
rather than reusing the older UI timings. This is separate from the matched
audio-engine CPU comparison below.

## Native stress observations before the restart fix

The optimized native application imported the complete WAV through its file
browser, then duplicated it into eight shared-source tracks. A real vertical
fader set the first track to -18 dB before duplication. Native checks confirmed
vertical scrolling to tracks 5–8, horizontal mixer scrolling, waveform zoom,
timeline-arrow panning, an advancing playhead, and live per-track meters.
Changing track 2 to -22.5 dB produced the corresponding 4.5 dB meter difference
relative to the other -18 dB tracks. Stop retained position 165.664 seconds.

The temporary test sink was a process-local ALSA file PCM into a paced FIFO
consumer at 48 kHz / stereo PCM16. One measurement consumed 12.821 seconds of
nonzero audio over 12.821 seconds wall time. ALSA/pipe buffering can put the UI
position ahead of that consumer; this is not a physical audio device.

A second Play exposed a native render crash: “ScarletUI frame exceeds SGFX IR
limits”. The reader had reopened successfully and consumed nonzero samples on
stream 2, separating the render failure from a FIFO-reconnect failure. Inspection
found that an offscreen playhead generated an empty SGFX mesh, which the pinned
renderer rejects with this generic error. Restart could briefly publish frame 0
before its first callback even when starting in a panned viewport. The core now
publishes its start position immediately; app-level render fixes and a repeat
native verification are required before this sequence can be marked passed.

Screenshots and the pre-fix log remain in ignored `artifacts/`, including
`resonara-long-eight-tracks.png`, `resonara-long-playing.png`,
`resonara-long-stopped.png` and `stress/restart-before-fix.log`.

## Native restart verification after the fix

The failing sequence was repeated on the optimized build with all eight tracks
of the same real recording. With the viewport panned to 87.43–262.28 seconds,
playback stopped at 176.343 seconds, restarted from that position, and advanced
to 210.927 seconds with live meters and no renderer error. The second FIFO stream
contained nonzero audio and consumed 16.4053 seconds of audio over 16.4055 seconds
wall time. The application then stopped cleanly.

The fix publishes the engine start position before the first callback, omits an
empty offscreen playhead draw, and retains waveform/playhead mesh handles with
revision updates. Regression tests cover these cases. Proof is retained in
`artifacts/resonara-long-restart-passed.png`,
`artifacts/stress/restart-after-fix.log` and
`artifacts/stress/restart-after-fix-audio.json`.

A bounded native inspector-resize attempt also stayed alive. Source inspection
shows a pinned-renderer cache limit of 32 distinct canvas/size combinations per
window. The resize gestures did not establish that 32 distinct retained targets
were reached, so this remains a scalability caveat, not a reproduced crash in
that pass. Normal window resizing resets the renderer cache. No arbitrary
renderer limit was raised and no dependency checkout was patched.

These native checks establish the exercised workflows and visible progression.
They do not measure GPU frame rate or prove all-session stability. Screen-capture
round trips are not a valid input-latency timer.

The final horizontal-wheel fix was also tested separately in a fresh native
release window: a rightward burst moved the view from 0.00–5.00 seconds to
0.42–5.42, and the same leftward burst returned it to 0.00–5.00. All 16 native
wheel ticks were handled. Evidence: `artifacts/native-wheel-burst-passed.log`,
`artifacts/resonara-wheel-right-passed.png` and
`artifacts/resonara-wheel-return-passed.png`.

## Final offline rerun after block-DAG and stereo telemetry

The same complete eight-track recording workload was rerun with the final
compiled engine, after local build jobs had completed. Native UI QA could still
be active on the shared host. All 146,802 independently selected sample values,
finite/nonzero checks, EOF silence and export shape passed again.

- Import/resample: 1.236 seconds
- Full 305.839-second arrangement render, including checks: 2.599 seconds
- 512-frame render call: median 54.8 µs, p95 93.1 µs, p99 617.1 µs,
  largest observed 16,629.2 µs
- The maximum exceeded the 10,666.7 µs block period; this run does not establish
  dropout-free realtime playback despite fast median throughput
- 28,673 render blocks were measured. At least one exceeded the period; this
  benchmark version does not retain the exact miss count, so none is inferred
  from percentiles. The separate graph benchmark does report exact miss counts
- Export: 2.361 seconds, 117,442,060-byte stereo float WAV; exact format/frame
  count checked

Raw final output: `artifacts/stress/core-benchmark-final.json`. Earlier
integration timing runs remain in `core-benchmark-with-graph.json` and
`core-benchmark-stereo-pre-confirmation.json`, explicitly marked as overlapping
or potentially overlapping compilation. Those timing runs also passed their
correctness checks. The [synthetic routing-scale benchmark](audio-graph.md)
separately measures nodes, sends, buses and stateful inserts.

## Matched pre-DAG / compiled-engine comparison

The isolated earlier maxima were not a controlled performance comparison. A
separate A/B restored base commit `15b0ced5d49714790c879319034584ceb60d55f7`
plus the verified pre-DAG UI checkpoint patch, then copied the identical
`compare_render.rs` harness into both checkouts. Both used the same compiler,
release profile, full real recording, eight tracks, gains/pans/250 ms offsets,
48 kHz output and 512-frame callbacks. Each run warmed 128 callbacks before
preparing a fresh measured engine. Local compilation had finished and all
operated native test sessions were stopped for the measurement window.

Four runs of each version were interleaved in order old/new/new/old/old/new/new/old.
Every run rendered 28,673 callbacks, checked 146,802 oracle sample values and
produced the same full-output sample-bit hash `e77cafc782fb63e3`. The benchmark
records both monotonic wall time and Linux render-thread CPU time; clock calls
are outside `render` and add small common instrumentation overhead.

| Measure | Pre-DAG | Compiled engine |
| --- | ---: | ---: |
| Total render-thread CPU, four runs | 7.409 s | 6.211 s |
| Median of per-run CPU p50 | 55.157 µs | 47.711 µs |
| Median of per-run CPU p95 | 99.838 µs | 80.790 µs |
| Median of per-run CPU p99 | 136.813 µs | 117.991 µs |
| Median of per-run wall p50 | 55.642 µs | 47.790 µs |
| Median of per-run wall p95 | 103.934 µs | 94.054 µs |
| Median of per-run wall p99 | 276.900 µs | 288.532 µs |
| Largest wall observation | 15,621.203 µs | 8,178.994 µs |
| Wall callbacks over 10,666.7 µs | 2 / 114,692 | 0 / 114,692 |

The new implementation used approximately **16.2% less render-thread CPU** in
this matched workload. Wall p99 remained noisy rather than uniformly better.
This supports no measured DSP regression for this workload; it does not prove
hardware real-time behavior or extend to arbitrary routing/plugin loads. No
functional optimization was applied just to improve the reported result.

Raw per-run files, combined results and summary are retained under
`artifacts/stress/ab/`. Reproduce each side with the same fixture and unchanged
harness:

```sh
cargo run --locked --release -p resonara-core --example compare_render -- \
  artifacts/fixtures/clair-de-lune-laurens-goedhart-2011.wav descriptive-label
```

CPU-clock results from this example are meaningful only on Linux. The earlier
non-paired observations above remain recorded with their original conditions;
they are not replaced by a favorable maximum from this comparison.
