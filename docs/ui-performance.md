# Native UI update diagnostics

The opt-in application profiler uses ScarletUI's public
`Application::on_window_sync`, `on_frame_presented` and `on_render_error` hooks.
No pinned ScarletUI or SGFX implementation was modified.

## What is measured

- Accepted-frame intervals are elapsed wall time between successful renderer /
  platform submission callbacks
- Sync-to-accepted time starts after application input dispatch and `on_idle`.
  It includes subsequent synchronization, layout/build/paint, SGFX encoding,
  submission, surface acquisition and the present call returning. It may include
  driver/backpressure waits; it is not a pure CPU-work measurement
- Failed frames are counted separately and never counted as accepted frames
- Body builds, waveform refresh requests, playhead updates and meter updates
  are application counters, not GPU execution counters

The hook explicitly does not provide GPU-completion or scanout timestamps.
SGFX schedules presentation and polls without waiting for GPU completion. Thus
these results describe accepted native submissions, not measured display FPS.
The independent model/layout benchmark in [audio-fixture.md](audio-fixture.md)
does not measure this native pipeline.

The intended current cadence is a 30 Hz playhead and 20 Hz meters, using retained
paint-only leaves. These are update targets, not a claim that a particular
machine sustains them. Static waveform geometry and frame state must remain
unchanged during playback-only updates; scrolling/zooming appropriately refresh
the visible waveform projection.

## Actual cloud renderer

The application's WGPU selected-adapter log confirms:

- Backend: Vulkan via SGFX/WGPU
- Device type: CPU
- Device: `llvmpipe (LLVM 19.1.7, 256 bits)`
- Driver: Mesa `25.0.7-2+deb13u1`, configured through the Lavapipe ICD

This is evidence from the actual application window, rather than merely a
standalone adapter inventory. The selected-adapter record is retained in
`artifacts/ui-profile-adapter.log`.

## Baseline and identified rebuild defect

An eight-track real-audio session, with a 1188×816 logical content area, was
measured for approximately 15 seconds per phase:

| Baseline phase | Accepted frames / duration | Median interval | p95 interval | Median sync-to-accepted | Body builds | Waveform refreshes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Playback | 62 / 15.276 s | 248.20 ms | 316.72 ms | 191.53 ms | 124 | 0 |
| Playback and scroll | 63 / 15.100 s | 238.82 ms | 301.95 ms | 188.65 ms | 126 | 6 |

No render failures occurred, but those approximately four accepted submissions
per second plainly did not meet a 30 Hz update budget in this environment.
The zero waveform-refresh count in playback proves that waveform peak/mesh
regeneration was not occurring on every tick.

A native backtrace and root-listener diagnostic instead identified repeated
size notifications feeding `SceneWindowRootElement` → `Window` → `Daw` rebuilds.
The application now coalesces size publication from the authoritative native
window during sync and leaves the fixed scene descriptor unsubscribed from
content state. Mounted-pipeline regressions additionally verify that dynamic
playhead, clock and meter changes do not rebuild the whole body or replace static
waveform meshes/frames.

Raw baseline reports: `artifacts/ui-profile-baseline.1.json` and `.2.json`.
The diagnostic trace is `artifacts/ui-profile-resize-trace.log`. Multiple older
test instances were preserved during the baseline; closing them later changes
background load, so timing improvements must not all be attributed to code.

## Handoff status

The final mounted-pipeline regressions pass: playback leaves the whole body,
static frame state and waveform meshes unchanged; stale resize notifications
are coalesced and the fixed scene descriptor has no duplicate content-state
subscription. Native confirmation of this final root fix was not completed
before the user requested commit/push and migration to Mac. Computer-control
calls had timed out for several minutes, so further cloud timing was stopped.
No final native 30 Hz result or hardware-GPU frame-rate claim is made. Complete
that acceptance on the Mac environment, including actual window resize.

## Reproduce a bounded measurement

The diagnostic is disabled unless `RESONARA_UI_PROFILE` is set. A session ends
after 15 seconds or 4096 accepted frames, and writes its aggregate report after
measurement. There is no per-frame log output or audio-callback instrumentation.

```sh
mkdir -p artifacts
RESONARA_UI_PROFILE="$PWD/artifacts/ui-profile" \
  ./scripts/dev cargo run --locked --release -p resonara
```

Start playback, press F9 and exercise one declared workload, such as playback
alone or playback with timeline scrolling. Press F9 again to finish early, or
allow the bounded window to finish automatically. Reports are written as
`artifacts/ui-profile.1.json`, `.2.json`, and so on. Keep intentional idle time
separate from an active workload.

For the genuine eight-track source workload, also set `RESONARA_PROFILE_WAV` to
the licensed WAV described in [audio-fixture.md](audio-fixture.md). The app imports
it once, clones eight shared-source tracks and initializes each to −18 dB. This
optional setup is used only when profiling is enabled; it does not replace the
normal project-opening path.

## Mac handoff observations

The Mac's existing Nix development shell built both profiles successfully.
Native diagnostics selected Apple M3 Pro / Metal. The user confirmed audible
CoreAudio output and described scrolling as heavy, with some improvement after
switching from debug to release. This is qualitative feedback; it does not
isolate ScrollView, layout, paint, encoder or presentation cost.

The mounted playback/resize regressions pass on Mac, as do the new full-height
mixer-hidden layout and Master pointer-state regressions. The final release's
initial native screen displays the bars/beats counter and integrated BPM/meter fields
with the complete arrangement and mixer. No F9 report was generated and agent
GUI input was deferred while the user operated the session. The 30/20 Hz targets
and final native resize/scroll/stop-restart acceptance remain unmeasured.

One intermediate launch logged `Error: "UI: RenderError"`, without the backend's
detailed SGFX render/present error output. The next final-release startup was
observed without that error, but no reproduction or fix is established. Keep
`artifacts/mac-latest-native.log`, `artifacts/mac-final-native.log` and
`artifacts/mac-final-verify.log` separate from cloud timing comparisons. Framework
pins and dependency implementation files were left unchanged.

The Follow/Fixed and ruler-capture regressions exercise the mounted pipeline:
playback within a Follow page keeps body rebuild and static-wave refresh at zero;
only a viewport shift refreshes the wave grid. Ruler moves preview the retained
playhead without waveform generation. These are model/pipeline observations,
not native GPU or ScrollView performance measurements. Latest check logs are
`artifacts/mac-workbench-final-verify.log` and
`artifacts/mac-workbench-final-build.log`.
