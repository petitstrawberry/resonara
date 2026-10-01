# Validation record

Date: 2026-10-01. Base commit:
`15b0ced5d49714790c879319034584ceb60d55f7`.
This records the cloud validation and subsequent user-requested Mac handoff.
Publication uses a dedicated branch; no pull request or merge is part of this
work. The cloud sections below retain their original environment and counts.

## Mac handoff and transport corrections

The independent Mac clone checked both remote and checkout commit
`686e4d3fca7883fe2eb65dd81bb2503d6ae17a47`, tree
`38903593101d016fbede1546f1d64f42816d5e11`, on branch
`feat/native-daw-workbench-2026-10-01`. Existing user checkouts and sessions were
preserved. No ScarletUI/SGFX source, revision or lockfile was changed.

- Apple M3 Pro, arm64, macOS 26.6.2; the existing Nix installation successfully
  entered the pinned development shell. Debug and release builds passed.
- `scripts/verify` passed 114 tests: 64 app and 50 core integration, zero failed;
  one optional stress test remained excluded. Formatting, dependency identity
  and shell syntax checks passed. Log: `artifacts/mac-final-verify.log`.
- Native renderer diagnostics selected **Apple M3 Pro / Metal**, rather than
  software Vulkan. Logs: `artifacts/mac-release-native.log` and
  `artifacts/mac-latest-native.log`. The final release was separately relaunched
  and its complete arrangement/mixer and adjacent musical counter/BPM controls
  were observed in the real application. Log: `artifacts/mac-final-native.log`.
- The user operated playback and confirmed audible output. The reported default
  output was MacBook Pro speakers, 48 kHz stereo. No OS volume/device setting was
  changed and no input recording was started. This is audible-output acceptance,
  not a latency, dropout or sound-quality measurement.
- Linux-only ALSA and Xvfb scripts now have platform guards. Mac `verify` reports
  the ALSA file-output check as **SKIP**; it does not silently turn that test into
  real-speaker playback or count it as a CoreAudio pass.

User feedback produced these application changes:

- Hiding the mixer gives the arrangement the full available workspace height.
  Mounted layout tests cover hidden/shown transitions, resize, both inspector
  states, empty and populated projects, and restoration of the split ratio.
- Master gain now publishes its control state during pointer movement. A real
  ElementTree event-dispatch regression verifies successive thumb-state changes
  and one grouped drag history entry, including undo/redo.
- Counter, ruler and waveform grid default to bars/beats/ticks at fixed 4/4,
  960 ticks per quarter note. Adjacent BPM entry accepts 20–400 including
  fractional values. Time and sample-position displays remain selectable.
  Persistence defaults legacy projects to 120 BPM; validation rejects invalid
  tempos. Tests cover beat/bar carry, grid density, input, history, save/load and
  byte-identical WAV export across a tempo change. Tempo edits stop playback;
  they change display conversion, not sample positions or audio stretching.
- The pinned ScarletUI `KeyModifiers::primary()` only checks Control. The app
  translates Command for the focused TextField through its existing input
  boundary. An event-dispatch test covers Command+A, fractional BPM entry and
  Enter without triggering global DAW shortcuts. This is an application
  workaround, not a framework fix. Framework pins remain intact.

The final transport grouping was informed by the official
[Logic Pro LCD guide](https://support.apple.com/en-euro/guide/logicpro/lgcp127f51bc/mac)
and [Cubase transport sections](https://www.steinberg.help/r/cubase-pro/15.0/en/cubase_nuendo/topics/playback/playback_transport_panel_sections_r.html?contentId=abquVlhKSd8RxOz1CvpAGw).
Region drag, edge trim, Split tool and S behavior were not redesigned. Snap and
the precise locator/inspector fields still explicitly use seconds/milliseconds.
There is no editable meter, tempo map or beat-based audio stretching.

### Mac limitations still open

The user reported heavy scrolling, with some improvement in release. No F9
report was generated, so no native submission rate or ScrollView cost is claimed.
The user was actively operating the windows; control ownership remained
unconfirmed and further agent GUI input was deferred. The new behavior has
automated coverage and final native startup/display acceptance, but its full
native hide/drag/resize/save/stop-restart interaction pass remains incomplete.

An intermediate launch ended with `Error: "UI: RenderError"` after an IMK mach
port diagnostic (`artifacts/mac-latest-native.log`). There was no detailed SGFX
encode/present failure message. The generic error is insufficient to identify
an encoder, damage-region, scale-factor or surface failure. The subsequent final
release launch displayed normally without this error during the observed idle
startup interval; this does **not** establish that the failure is fixed.

## Build and dependencies

- ScarletUI and its eight workspace crates resolve to Git revision
  `91238ed12c3379c0aea7e7ba86e73139cfb1f550`; no adjacent checkout is required
- SGFX's existing pinned sources remain coherent, with one `sgfx-core` identity
- `scripts/check-dependencies`, formatting and shell syntax checks pass
- Debug and optimized release native builds pass on Linux x86_64
- Pinned Nix devShell evaluation passes. `nix develop` launch does **not** pass
  here because the container denies a private mount namespace
- Builds used direct invocation of the same downloaded Scarlet Rust toolchain,
  host GCC and workspace-local official Debian development packages. This is
  explicitly distinct from a successful Nix development shell

## Automated checks actually run

- `scripts/verify`: 108 default tests passed (59 app and 49 core integration), zero failed; one optional stress
  test excluded from the default suite
- The optional release eight-track real-audio model/layout stress test was run
  separately and passed before the final root-invalidation changes; it was not
  rerun during the final fast commit/push handoff
- Core tests include allocation/deallocation auditing, mixer values, WAV formats,
  persistence/export, non-project-rate split equivalence and initial playback
  position publication
- App tests include history/selection, mixer edits, cancellation, asynchronous
  I/O gates and failures, Save As association, viewport preservation, fader input,
  text/shortcut isolation through dialog rebuild, pointer routing, wheel
  direction, offscreen playhead handling and retained mesh identities
- Mixer regressions additionally check Spectrum's independent gain/peak anchor
  tables, interpolation/inversion, actual painted tick/thumb/glyph alignment,
  centered numeric readouts, track/inspector/mixer gain synchronization through
  edits/undo/load, and pan-knob gestures, fine readout and 1×/2× edge coverage
- Graph regressions verify single execution through shared sends/inserts,
  deterministic sums, cycle/resource rejection, callback partition equivalence,
  an independent varied-DAG oracle, and allocation-free latched fault handling
- Stereo-meter regressions verify unequal L/R signals, hard pan, mute/solo,
  actual summed master level after gain and before clipping, one-time polling,
  paired paint geometry, ordered readouts, hold expiry and stop/restart reset
- Source-channel regressions verify mono/stereo metadata through import, edits
  and persistence, conservative legacy defaults, independent signed L/R peaks,
  one versus two waveform lanes, trim/source-offset boundaries and retained
  cache/mesh identity without changing the underlying audio
- CPAL + process-local ALSA file-output smoke passed, with nonzero audio and
  advancing callback position. This does not establish physical-device playback

Latest aggregate log: `artifacts/verify-push-handoff.log`. The core integration count
comprises 13 engine, 6 file, 15 graph, 8 independent-review and 7 meter tests.
The optional release test also passed separately, as recorded in
`artifacts/long-audio-ui-model-stress-packaged.log`.

The final app suite additionally covers bounded default-off frame diagnostics,
paint-only playback updates, static waveform preservation, authoritative resize
coalescing and absence of duplicate scene-root subscriptions. Final native
confirmation of that last root fix is deferred to Mac after cloud UI-control
timeouts; see [native timing and the handoff limit](ui-performance.md).

The separate Xvfb `scripts/gui-smoke` runner could not run in this sandbox because
Unix-domain socket creation is denied. Native interaction checks used the
existing authorized cloud desktop instead. Security settings were not weakened.

## Native checks actually observed

- Real desktop window using ScarletUI / SGFX / WGPU, configured with Mesa
  Lavapipe software Vulkan
- File-browser import of a genuine licensed 5:04 recording; seven duplicates
  produced eight shared-source tracks, with -18 dB gain set using the real fader
- Vertical track scrolling, horizontal mixer scrolling, waveform zoom and
  timeline-arrow panning
- Final-release native horizontal timeline wheel burst: view 0.00–5.00 seconds
  moved to 0.42–5.42, then an equal reverse burst returned to 0.00–5.00; all 16
  native wheel ticks were handled. This additionally covers captured wheel
  transactions, rather than only the first tick
- Live fader changes during playback and corresponding per-track meter changes
- Stop retained the transport position; a second Play resumed and advanced in
  the panned eight-track view after the empty-mesh/initial-position fix
- Native save of the small demo project was inspected after keyboard entry;
  Ctrl+A/Ctrl+S stray-character regressions were fixed and tested
- Inspector resizing survived a bounded native gesture pass; this did not prove
  that the renderer's 32 distinct canvas/size cache limit had been reached

### Compact-mixer acceptance checkpoint, before DAG work

The 11:37 UTC release build was exercised on the native desktop after the
Spectrum scales, alignment, pan antialiasing and unified gain controls landed:

- Changing gain through the track header, inspector and vertical mixer fader
  produced −4.8, −13.0 and −18.0 dB respectively in all three readouts
- Pan drag and Shift fine adjustment produced R39.8; a second channel showed
  L50. Home reset was also checked. The 32-pixel knobs use retained analytic
  edge-coverage buffers, without adding retained SGFX canvas targets
- Eight tracks of the genuine 5:04 recording played, stopped and restarted;
  the restart advanced from 100.516 to 125.705 seconds with live meters and no
  renderer error. The second test-sink stream consumed 55.4 seconds of nonzero
  audio over 55.4039 seconds wall time
- Changing the inspector split kept mixer strips and readouts aligned. The
  attempted native maximize/window-edge actions did **not** establish a reliable
  window-resize test and are not counted as a pass

Native proof: `artifacts/resonara-final-pan-aa-gain-sync-1142.png`,
`artifacts/resonara-pan-aa-closeup-1142.png` (an unscaled crop of native pixels),
and `artifacts/resonara-final-pan-restart-1146.png`. Sink and renderer logs are
retained as `artifacts/stress/ui-checkpoint-restart-audio.json` and
`artifacts/stress/ui-checkpoint-native.log`. This checkpoint predates subsequent
audio-graph changes and does not validate those changes.

### Compiled-graph and genuine stereo-meter integration

A fresh release containing the compiled graph and stereo telemetry imported the
same genuine five-minute recording into three shared-source tracks. Native
controls set gains to −6/−12/−18 dB and pan to hard left / center / hard right.
The paired meters visibly showed left-only / both / right-only signals, with
different actual summed master L/R levels. Track and master numeric summaries
remained live; the expanded L/R hover detail is a snapshot at pointer entry.

Playback stopped at 126.214 seconds, restarted and advanced to 157.400 seconds
with live paired meters. A second Stop held 185.887 seconds, after which the
live bars, hold marks and clip state cleared and summaries returned to −∞.
The second paced test-sink stream consumed 59.7 seconds of nonzero audio over
59.70026 seconds wall time; no renderer error was logged. Existing test sessions
were preserved and left stopped.

Proof: `artifacts/resonara-stereo-track-master-1210.png`,
`artifacts/resonara-stereo-meter-closeup-1210.png`,
`artifacts/resonara-stereo-stopped-clear-1211.png`, and
`artifacts/resonara-stereo-graph-restart-1212.png`. Supporting logs:
`artifacts/stress/final-stereo-native.log` and
`artifacts/stress/final-stereo-restart-audio.json`.

### Source-accurate mono/stereo waveform acceptance

The full-duration, independently verified real-audio derivatives described in
[the fixture report](audio-fixture.md) were imported in a fresh native release.
The stereo clip showed independent L/R lanes, with its attenuated right channel
visibly smaller; the mono-left clip showed one waveform lane. Native selection,
a split at 61.143 seconds and zoom preserved the two stereo lanes.

The final paint-only guard was separately checked in a fresh release with a
saved legacy demo: the selected clip spanned 0…3 seconds and the viewport
0.66…2.96 seconds. Neither offscreen endpoint was incorrectly painted as a trim
handle or vertical selection edge. All test sessions were left stopped and
intact.

Proof: `artifacts/resonara-stereo-mono-waveforms-1229.png`, its unscaled crop
`artifacts/resonara-stereo-mono-waveforms-closeup-1229.png`,
`artifacts/resonara-stereo-waveform-split-zoom-1231.png`, and
`artifacts/resonara-clipped-trim-handles-1237.png`. The split/zoom image predates
the endpoint-only guard; the last image checks the corrected endpoint behavior.

See [licensed fixture and measured results](audio-fixture.md) for source,
license, hashes, exact workload, timings and pre-/post-fix restart evidence.

## Limits and remaining scope

No physical audio-device latency or dropout measurements, GPU timing/FPS
measurements, or production-scale stability claim is made. Mac startup and user
audible-output acceptance are recorded above. The UI-model benchmark measures
CPU work, not presentation timing.

The pinned renderer retains at most 32 canvas/size combinations per window; the
bounded resize pass did not reproduce exhaustion. Larger projects, repeated
project replacement and extended pane-resize workloads require more validation.
JSON project storage repeats embedded source audio per clip and is inefficient
for large sessions. Aux/send/return routing and basic insert scheduling are now
available through the compiled core API; the routing UI, persistence and custom
routing export are not implemented. See [the graph contract](audio-graph.md).
Recording, MIDI, plug-ins, time stretching and higher-quality resampling also
remain outside the implemented audio-editing slice.

File selection currently uses the tested in-app browser. The agreed future
boundary is a common `FileDialog` adapter with macOS, Linux and future Scarlet
implementations choosing the appropriate platform file picker; the in-app
browser remains a fallback. No native file-dialog backend is implemented in
this patch, and OS dialog behavior does not belong in the audio core.

## UI design references

The arrangement/inspector/mixer organization and stable transport were informed
by primary product documentation, with independent native controls wired to
Resonara's own model:

- [Cubase Pro Project Window](https://www.steinberg.help/r/cubase-pro/15.0/en/cubase_nuendo/topics/project_window/project_window_overview_r.html)
- [Logic Pro interface guide](https://support.apple.com/en-euro/guide/logicpro/lgcp2a07a994/10.7/mac/11.0)
- [Logic Pro zoom controls](https://support.apple.com/en-euro/guide/logicpro/lgcpf7c0b924/mac)
- [Pinned Spectrum fader/meter math and attribution](mixer-scale-reference.md)

This does not imply feature parity with either product. An independent Ardour
startup inspection was also performed; a full Ardour edit/import workflow was
not completed in that inspection.
