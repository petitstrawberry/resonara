# Validation record

## 2026-10-06 — Native resize notification handling

ScarletUI is pinned to `2ae7c2d0c90d5beafdcaddd39e36608803f34f75`.
Platform resize notifications now update layout without asking winit to resize
the native window back to the reported size. Explicit application resize requests
remain separate; SWS keeps its backing-allocation handshake. Release workspace
tests passed: 407 tests, 21 existing opt-in tests ignored. Log:
`artifacts/resize-git-tests.log`. Native interaction is checked in Cadence separately.

## 2026-10-06 — Backend decoration and content geometry

ScarletUI is pinned to `339f2550e726e6bb70388f3f7a9940f6c07ea26f`.
The winit backend now uses system decorations by default; SWS retains ScarletUI chrome.
Resonara expands its content viewport and uses the existing `on_geometry_change`
notification instead of subtracting 32 pixels from the platform surface height.
Resize tests exercise actual pipeline layout, coalesced notifications, and unchanged
geometry without repeated root rebuilds. Release workspace tests from the normal
Git pins passed: 407 tests, 21 existing opt-in tests ignored. Logs are
`artifacts/system-decoration-git-tests.log`. Native window interaction and Scarlet
runtime are not established by these tests.

Date: 2026-10-01. Base commit:
`15b0ced5d49714790c879319034584ceb60d55f7`.
This records the cloud validation and subsequent user-requested Mac handoff.
Publication uses a dedicated branch; no pull request or merge is part of this
work. The cloud sections below retain their original environment and counts.

## 2026-10-02 Scarlet port/routing checkpoint (before CLAP extension)

Source baseline: Resonara `7564ddfbeed6f3700579398883bd5dad9d3ce4bf`.
The inspected Scarlet `dev` checkout is
`0639a916dfd652e9b2c1ea740cacc1c09743d9eb`. No push, PR or merge is part
of this validation pass.

- The host aggregate check at this checkpoint passed **247 tests**, zero failures, with one
  existing opt-in stress test ignored. Dependency pins, formatting and build
  checks passed. Both flat and routed CPAL/ALSA file-output smokes passed with
  nonzero PCM. Routed smoke includes three tracks, two buses, inserts, a
  pre-fader send and project save/load/export. Logs are
  `artifacts/routing/aggregate-verify.log` and
  `artifacts/routing/audio-smoke-routed.log`.
- Native **AArch64** and **RISC-V 64** release builds passed after the EOF fix
  for this pre-CLAP checkpoint. These binaries do not yet validate CLAP imports.
  The compiler is Scarlet Rust commit
  `a5a166ab0ba10eaad36eb90d1e4af26eadfdec0c` (LLVM 21.1.8), matching Scarlet's
  pinned Nix toolchain. The compiler and linker were materialized from the
  configured signed Nix caches into the workspace; signatures, compressed
  SHA-256 and NAR SHA-256 were checked. Direct toolchain execution is separate
  from a complete `nix develop` run.
- ELF inspection confirms the intended ARM64/RISC-V64 architectures and
  `/bin/scarlet-ld` interpreter, without external `NEEDED` libraries. The
  AArch64 SHA-256 is
  `1c43f815fd7c4232906bedf2cc1a7de601a3446364227922e28b64b221049da9`;
  RISC-V64 is
  `990c86f76a4ff9d7b5bfac4941153e4e7d1c47db4abb16cd7a91e08817a0319d`.
  Archived logs are `artifacts/scarlet/pre-clap-build-aarch64-unknown-scarlet.log`
  and `artifacts/scarlet/pre-clap-build-riscv64gc-unknown-scarlet.log`.
- Six host-tested SAS pump cases cover partial/zero writes, ring backpressure,
  final staged PCM, drain progress versus completion, cancellation, and closed
  rings. Natural completion is separate from render completion. The app
  retains the drained output until an explicit transport/edit action so SAS's
  device queue can finish; it does not claim a hardware-drain acknowledgement.
- Isolated wrapper tests pass for locked native build arguments, additive
  image overlays, user-overlay preservation, persistent-disk reuse, explicit
  image-replacement opt-in, advisory locking, and GL/display guards. The QMP
  presence guard uses a marked mock in these tests. No mock test is counted
  as an actual image, QEMU or guest runtime pass.

## 2026-10-02 CLAP-enabled native artifacts (03:17 UTC snapshot)

At this snapshot the host workspace check passed **300 tests** with ten opt-in cases
ignored by default. Explicit real-library runs also passed (five host, nine
core and one app test), including the nine fixture-dependent ignored cases;
the stress case remains opt-in. The standalone gain effect's ten tests and
the native ELF auditor's nine tests passed. Flat, routed and routed-plus-CLAP
CPAL/ALSA smokes passed; both routed captures contained 493,920 bytes of
nonzero PCM. These are host results, not SAS guest results. Evidence is in
`artifacts/routing/final-compact-routing-verify.log` and
`artifacts/routing/final-compact-clap-routed.log`.

The CLAP-enabled native release app built and passed the mandatory ELF
audit for both targets. Each artifact has exactly `dlopen`, `dlsym`, `dlclose`
and `dlerror` as dynamic imports, no startup `DT_NEEDED` libraries or TLS, and
only the architecture's `JUMP_SLOT` and `RELATIVE` dynamic relocations.

- AArch64 SHA-256:
  `dec6dd1c80631d8a6081c2a51958003a5a6ecde393b96b10a73489922717bb02`
- RISC-V64 SHA-256:
  `92b05746599b6f2ae8350900329e274e12497b42cc394535296e0f25ec0270b0`
- Audits: `artifacts/audit-aarch64-unknown-scarlet.json` and
  `artifacts/audit-riscv64gc-unknown-scarlet.json`
- Combined build log: `artifacts/scarlet/final-native-clap-build.log`

These supersede the pre-CLAP binaries above. They have not yet been executed
inside Scarlet. The optional validation overlay includes an independently
audited native CLAP loader/DSP/state/teardown smoke executable, a WAV import
fixture and a routed CLAP project; their presence is not a runtime pass.
The later M/S-control centering refinement is not included in these hashes;
the image wrapper rebuilds and audits the current app before staging it.

## 2026-10-02 desktop Nix/QEMU setup

The actual cloud desktop successfully executed the official Nix 2.24.12
rootless-store probe without security-setting changes. Full Scarlet Nix
environment evaluation was killed under its 8 GiB memory limit. The exact
already-evaluated QEMU derivation subsequently built successfully with one
job/core, without changing its source revision or isolation settings.

- QEMU source: `d94a1407ab9ccd60559bfd80182a81bb4261fb84`
- Derivation: `wd90v1y5sl6xli19f6s7fi0fja5hg3m7-qemu-11.1.0-scarlet.drv`
- Output: `z57b2753zj6f0rr7bkavlv0rip4in4vn-qemu-11.1.0-scarlet`
- The actual desktop rootless-store execution of
  `qemu-system-aarch64 --version` returned **11.1.0**, exit zero
- Build evidence: `artifacts/scarlet/qemu-nix-build.log` and `.json`

The selected runtime-image recipe is the separate `native-desktop` profile,
retaining Scarlet's exact desktop bundle, SWS/SAS and native apps while leaving
the full Debian/Wine project unchanged. Its rootfs starts at 2 GiB and grows
under the SDK's normal sizing rules. No fitted-image claim is implied.
Scarlet image construction, QEMU boot, SWS interaction, SAS guest audio and
guest file I/O have **not yet been verified**. Record those stages separately
after observation; compilation and the version check do not imply guest success.

## 2026-10-02 audio import, musical grid, metronome and saving

`cargo test --locked --workspace` passed 208 tests: 91 app, 59 core and 58
vendored renderer tests; the opt-in long-audio stress test remained ignored.
Formatting, dependency-pin verification and the release build passed. The
release application started with the winit/SGFX/wgpu backend. Native GUI
automation was unavailable (`Sky Computer Use native pipe startup failed`),
so the save panel was not manually exercised in this pass. Async save success,
failure, retry, history and file-dialog result delivery were covered by tests.

New coverage checks zoom-dependent 1/4, 1/8 and 1/16 ruler lines and their painted
lengths; snap units, odd-bar boundaries, pointer seeking, clip move/trim/split
and undo; empty-session clicks, live switching, tempo/meter/device-rate and seek
synchronization, callback partitioning and click-free exports. The callback
allocator audit passed with the metronome enabled. Generated checked-in fixtures
exercise FLAC, AIFF, CAF, ALAC/M4A, AAC/M4A, ADTS/AAC, MP3 and OGG/Vorbis;
lossless round trips match WAV samples, and failed imports preserve the project.

Saving previously gave each tiny JSON write directly to `File`; embedded audio
therefore generated millions of writes. Save/load now use 256 KiB buffers, with
save flush and synchronization before replacement. On the Mac release build,
576,000 stereo frames (12 seconds, 14,372,889 JSON bytes) saved in 3.884 seconds
with the old direct writer and 0.0303 seconds with buffering (128.2×); loading
took 0.0436 seconds. Both writers produced identical files and audio round trips.
The user-provided 10.411-second mono WAV also saved/reloaded successfully:
13,077,504 JSON bytes, save 0.0335 seconds, load 0.0384 seconds, identical samples.
These are local file-I/O timings, not GUI latency measurements or a general
performance guarantee. The existing embedded-per-clip JSON size limitation
remains. Evidence is in `artifacts/audio-grid-metronome-tests.log`,
`artifacts/audio-grid-metronome-release.log`, `artifacts/save-buffering-benchmark.rs`,
`artifacts/save-buffering-benchmark.log` and `artifacts/user-audio-save-roundtrip.log`.

### Native audio-selection follow-up

The initial format tests exercised the decoder and fallback browser, but native
dialog result validation still accepted only `.wav` for Import. This rejected
MP3 and other newly supported files before their decoding worker could start.
Import validation now shares `audio::supported_path` with the browser and dialog
filter; project and WAV export filters retain their respective extensions.
Two regression tests check native selected-result delivery through actual async
decoding and undo for all ten audio fixtures (including uppercase `.MP3` and a
Unicode filename), and invalid paths/extensions for all four file actions.
The updated app suite passed 93 tests with one optional stress test ignored;
the release build also passed. Logs are `artifacts/native-audio-import-tests.log`
and `artifacts/native-audio-import-release.log`. OS-panel interactions remain
unverified due to the native automation startup failure above.

## Mac handoff and transport corrections

The independent Mac clone checked both remote and checkout commit
`686e4d3fca7883fe2eb65dd81bb2503d6ae17a47`, tree
`38903593101d016fbede1546f1d64f42816d5e11`, on branch
`feat/native-daw-workbench-2026-10-01`. Existing user checkouts and sessions were
preserved. No ScarletUI/SGFX source, revision or lockfile was changed.

- Apple M3 Pro, arm64, macOS 26.6.2; the existing Nix installation successfully
  entered the pinned development shell. Debug and release builds passed.
- `scripts/verify` passed 125 tests: 74 app and 51 core integration, zero failed;
  one optional stress test remained excluded. Formatting, dependency identity
  and shell syntax checks passed. Logs: `artifacts/mac-workbench-final-verify.log`
  and `artifacts/mac-workbench-final-build.log`.
- Native renderer diagnostics selected **Apple M3 Pro / Metal**, rather than
  software Vulkan. Logs: `artifacts/mac-release-native.log` and
  `artifacts/mac-latest-native.log`. The final release was separately relaunched
  and its complete arrangement/mixer and integrated musical counter/BPM/meter
  controls were observed in the real application, including rounded LCD border,
  inset labels and borderless inputs. Logs: `artifacts/mac-final-native.log` and
  `artifacts/mac-workbench-final-native.log`. Initial screenshots on two launches
  showed a narrow partial window; subsequent read-only snapshots showed the full
  window without agent input. The resize cause was not isolated.
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
- Counter, ruler and waveform grid default to bars/beats/ticks,
  960 ticks per quarter note. BPM entry in a shared rounded LCD accepts 20–400 including
  fractional values. Time and sample-position displays remain selectable.
  Persistence defaults legacy projects to 120 BPM and 4/4; validation rejects invalid
  tempos and meters. Meter numerator accepts 1–32 and denominator 1, 2, 4, 8, 16
  or 32; BPM remains quarter notes per minute. Tests cover beat/bar carry, grid
  density, input, history, save/load and byte-identical WAV export across tempo
  and meter changes. Tempo/meter edits stop playback;
  they change display conversion, not sample positions or audio stretching.
- The pinned ScarletUI `KeyModifiers::primary()` only checks Control. The app
  translates Command for the focused TextField through its existing input
  boundary. An event-dispatch test covers Command+A, fractional BPM entry and
  Enter without triggering global DAW shortcuts. This is an application
  workaround, not a framework fix. Framework pins remain intact.
- Gain scale clicks on both track and Master set exact +6, 0, −6, −18,
  −48 dB or silence. The separate dBFS meter remains read-only. Tests check
  state/model/audio agreement, neighboring controls, cancel and grouped history.
- Track header context menus select the clicked track before Add/Duplicate/Delete;
  blank track-column context and the ruler's + add empty tracks. Inspector
  contains properties. Event-dispatch tests cover menu actions, Escape/outside
  dismissal, empty projects, shortcuts, selection, Undo/Redo and persistence.
  Pinned MouseEvent has no modifier snapshot, so Mac Control-click reads
  CoreGraphics combined-session flags on button press. This narrow compatibility
  path has semantic tests but awaits a native manual check.
- Follow/Fixed toggles paged follow at 90% of the viewport. Mounted pipeline tests
  show no body rebuild/static-wave refresh during playback inside a page; a
  viewport page shift refreshes the grid. Ruler capture previews seeks through
  pointer moves/outside bounds, stops the audio stream once on press, and resumes
  once on release if previously playing; Escape restores the previous position.
  This implements seek dragging, not audible scrubbing.
- Header/transport controls use common height/type/padding. Counter, borderless
  BPM and meter fields share one rounded LCD; label inset and vertical padding
  keep text away from its border. All formats fit the minimum 1000-pixel window
  in layout tests. Non-4/4 tests cover carry, odd-bar grid alignment and actual
  meter-field Command+A/type/Enter routing.

The final transport grouping was informed by the official
[Logic Pro LCD guide](https://support.apple.com/en-euro/guide/logicpro/lgcp127f51bc/mac)
and [Cubase transport sections](https://www.steinberg.help/r/cubase-pro/15.0/en/cubase_nuendo/topics/playback/playback_transport_panel_sections_r.html?contentId=abquVlhKSd8RxOz1CvpAGw).
Region drag, edge trim, Split tool and S behavior were not redesigned. Snap and
the inspector fields still explicitly use seconds/milliseconds. The redundant
GO TO seconds entry was removed from the transport; ruler click/drag seeks remain.
There is one editable project meter; tempo/meter maps and beat-based audio
stretching remain unimplemented.

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
