# Persistent session routing and compiled audio graph

Session routing is part of the saved project: tracks and named aux/group buses
have ordered inserts, a main destination, and pre/post-fader sends. Playback
and WAV export lower that session to one compiled block schedule. The explicit
runtime graph API remains available for experiments and callers with custom
admission budgets.

## Runtime contract

- `RoutingGraph` names nodes with stable `NodeId` values. `Route` names a source,
  destination and finite signed gain. Duplicate node IDs, missing endpoints,
  invalid processors, source inputs and **all cycles** are rejected
- `CompiledGraph::compile` runs outside the audio thread. It establishes a
  deterministic topological order, fixes incoming-edge summation order, removes
  disconnected acyclic work, computes output-buffer last uses and prepares DSP
  state. Invalid disconnected cycles are still rejected
- Every reachable processor runs exactly once per nonempty internal quantum;
  every incoming route mixes once. A shared source/filter produces one buffer
  read by all consumers. Stateful processing is never repeated for a send
- Buffers remain live until their last read. Slots are reused only after that
  operation finishes. Input/output slices are disjoint, with no unsafe aliasing
- Runtime cost is processor work plus edge mixing, proportional to actual nodes
  and edges rather than the potentially exponential number of signal paths.
  Compilation sorts IDs/inputs off-thread for deterministic floating-point order
- Device callbacks are split into chunks no larger than the prepared quantum
  (128 frames by default). Short/odd callback sizes are supported. This is not
  an extra queued block of output latency
- Scratch audio, delay rings, per-track controls, transport-position scratch and
  the schedule are allocated before rendering. The render loop does not allocate,
  deallocate, lock, log, perform I/O, construct graphs or release ownership
- `GraphLimits` limits declared node/edge counts, quantum, audio-scratch bytes
  and persistent delay bytes. `GraphInfo` reports the actual scheduled graph and
  audio-buffer footprint. Audio budgets exclude shared source assets and plan
  metadata; those scale with the admitted project and graph
- `Engine::try_new` lowers persistent project routing and compiles it once,
  returning a setup error for invalid routing or resource limits. `Engine::new`
  preserves its original infallible API as a convenience wrapper for known-valid
  projects. Its node/edge/scratch admission budget can scale to a legacy flat
  track count; saved routing cannot raise its own budget. Delay storage retains
  the default budget. Explicit graph budgets
  are selected through `Engine::with_graph`, which overrides session routing for
  that engine without changing the saved project

## Persistent session model

`Track.routing: ChannelRouting` and `Project.buses: Vec<Bus>` use serde defaults.
Old version-1 JSON with neither field loads as the unchanged direct-to-master
mix. New saves retain bus identities/names/kinds, gain/pan/mute, inserts, routing
destinations, send levels, pre/post choices, enabled flags, and bypass flags.

`ChannelRouting` contains ordered `inserts`, one `output` (`Master` or
`Bus(BusId)`), and additive `sends`. `InsertKind` is `Gain`, `OnePole`, `Delay`, or the bounded bundled `Clap` effect;
bypass removes the processor from the compiled graph without deleting its saved
parameters. An enabled send has a target bus, finite signed gain, and pre-fader
choice. Disabled sends remain saved but do not contribute audio or graph edges.

`BusId` is a serialized u64 identity, independent of display names and list order.
`Project::add_bus` returns a unique ID; `bus`/`bus_mut` find it. `remove_bus` removes
sends targeting that bus and repairs main outputs targeting it to master, across
both tracks and other buses. Remaining IDs are unchanged. Aux and group kinds
are organizational labels with the same stereo DSP capabilities; a typical aux
receives sends and a group receives main outputs. The native UI follows Logic’s
bus-path / Aux-receiver convention for both use cases. Older saved kinds still
load; they never change processing or restrict eligible routes. Each BusId is
bound to one receiving strip in this initial session model.

`Project::validate_routing` checks parameters and topology without scanning clip
samples, compiling the audio graph, or allocating DSP buffers. Structural edits
can use it before acceptance; ordinary repaint and fader changes should not
recompile. It rejects duplicate IDs, dangling outputs/sends, and all bus cycles.
Disabled and zero-gain sends still participate in cycle validation so enabling
or raising one cannot reveal hidden feedback. Bypassed inserts must retain valid
parameters. Saved bypassed inserts and disabled sends count toward the node,
edge, and delay budgets as well. Limits are checked before allocating routing
adjacency, graph metadata, or DSP buffers. `Project::routing_graph` returns validated graph metadata;
`Engine::try_new` performs the actual schedule and storage preparation. Save/load
also validate the complete project. The native playback path and WAV export use
the same persisted routing graph.

## Signal semantics

Each strip is source/sum → ordered enabled inserts → fader/pan → main output.
A pre-fader send taps after all enabled inserts, before gain/pan. A post-fader
send taps after gain/pan. Both tap the same computed buffer rather than invoking
DSP a second time. Multiple sends to the same bus are additive intentional paths.
Bus output/send chains can reach other buses in any acyclic arrangement.

`TrackSource` mixes clips and honors mute/solo; `TrackFader` applies gain/pan,
gates output, and measures its post-fader peak. Project-compiled pre-fader taps
also use `TrackGate`, preventing stored insert tails escaping a muted or
solo-excluded track through its sends. `BusGate` sums incoming routes and honors
live bus mute; a post-insert bus gate protects pre-fader taps in the same way.
`BusFader` applies live bus gain/pan/mute and measures stereo peaks. Solo only
selects track sources; downstream buses remain audible so returns are retained.
A downstream bus's existing effect state can ring while upstream sources are
muted; mute the return bus to gate that bus's own tails.

The plain runtime `Bus` processor remains an uncontrolled sum for compatibility.
Explicit graphs choose their own gates/taps. In particular, old runtime graphs
with a send after an insert and before `TrackFader` may carry stored DSP tails
unless the caller adds `TrackGate`. Intermediate audio is not clipped. Only final
device output applies master gain and the existing output clamp/mono mapping.

`OnePole` uses `y = (1 - coefficient) * input + coefficient * previous`, with
independent stereo state. `Delay` is an explicit feed-forward sample delay with
preallocated storage, measured in device-rate frames. It does **not** enable feedback. Even a one-sample delay
inside a cycle is rejected: supporting that correctly requires a causality-aware
sub-block/sample scheduler, not a recursive pull or one device-callback delay.
No automatic delay is added to hide a cycle.

Track and bus controls are sampled once per internal quantum; master/solo are sampled at
the outer callback boundary. These are bounded-latency live controls, not yet
sample-accurate automation or smoothed gain ramps. Meters are accumulated locally
and published once per quantum. Clip mixing visits each clip's intersection with
the block, retaining original clip summation and sample-position accumulation
order at non-project sample rates.

## Source channel metadata

`Clip.source_channels` records the original imported channel count (1 or 2).
Internal audio still uses stereo frame storage; mono imports duplicate their
single channel exactly. Split/trim/cloning and project save/load retain this
metadata. Validation rejects unsupported channel counts and mono-marked buffers
with unequal channels, so a one-lane waveform cannot hide a second channel.

Older JSON projects omitted channel count and deserialize as stereo (2). Their
original mono/stereo provenance cannot be recovered safely from equal samples:
a stereo file may legitimately contain identical channels. Equal stereo imports
therefore remain stereo. This additive metadata does not change rendered audio.

## Stereo meter telemetry

Each track publishes independent post-fader/pan `Mixer.peak_left` and
`Mixer.peak_right` atomics; the original `Mixer.peak` remains their maximum for
compatibility. `Controls.buses`, indexed in the prepared project's bus order,
provides the same peak fields and live gain/pan/mute via `BusMixer`. Bus reorder,
addition, and removal are structural edits requiring fresh controls and engine;
`BusMixer::set` updates an existing bus's live mix controls. `Controls.master_peak_left` and `master_peak_right` measure the
actual stereo graph output after master gain, **before** final clipping and
mono/multichannel device mapping. Cancellation happens before master measurement;
the master meter is not the sum of track-meter peaks. Values above 1.0 remain
visible as output overrange.

All values are positive `f32` bits stored in `AtomicU32`, initially zero. The
audio thread accumulates maxima in ordinary local arrays and publishes with
`fetch_max` once per internal quantum. A UI consumes each channel with
`f32::from_bits(peak.swap(0, Ordering::Relaxed))`, then applies its own visual
decay. Independent polling resets that channel's accumulation window. Nonfinite
arithmetic faults saturate the affected telemetry at finite `f32::MAX`; the
separate latched audio-fault behavior below still stops playback.

## Lifecycle and explicit limits

- Structural edits retain the existing stop/rebuild/start lifecycle. There is no
  live graph mutation, atomic graph replacement or audio-thread reclamation
- Routing and inserts are persistent session edits. The app's structural
  stop/rebuild/start lifecycle also applies to destination, send, insert,
  bypass, and bus edits. Bus gain/pan/mute can use prepared atomic controls
- Pause outputs silence and freezes both transport and effect state
- EOF remains the existing hard boundary at `Project::duration()`. Delay/reverb
  tails beyond it are not rendered or exported. A caller can supply a longer
  timeline for experiments, but automatic tail handling is not implemented
- No general plugin hosting, plugin delay compensation, recording, sidechain policy,
  sample-accurate automation or feedback SCC execution is claimed. These require separate session/API/lifecycle work
- Finite gains can still overflow floating-point audio. If a final sample is
  nonfinite, the engine silences it and subsequent output, latches a fault, sets
  `Controls.error` and clears `Controls.playing`. That engine stays silent even
  if public control flags are reset. Prepare a new engine to clear potentially
  contaminated DSP state; reducing the gain does not pretend to repair it

## Tests and reproducible scaling workload

```sh
cargo test --locked -p resonara-core
cargo run --locked --release -p resonara-core --example graph_stress
```

The graph tests cover diamonds, shared stateful inserts, twelve fully connected
routing layers, canonical ordering, disconnected work, pre/post sends, signed
unclipped intermediates, validation/budgets, variable callback sizes and delay
continuity. An allocator audit covers the first callback and repeated active
callbacks. The previous default mixer is checked against an independent flat
sample oracle at 44.1/48/96 kHz, alongside existing split/save/export regressions.
Independent review adds varied-DAG sample-oracle checks, pause/resume, partial
output, hard EOF and nonfinite fail-stop tests. Persistent routing tests cover
legacy JSON defaults, save/load/export parity, insert order and bypass, pre/post
send levels and pan, disabled sends, named chained buses, live bus controls and
stereo meters, mute/solo tap gates, repair on bus deletion, invalid/disabled
feedback, and shared stateful processing counts. The allocator audit also covers
a persistent insert/send/bus chain, including first-callback bus controls.

The scale example exercises 8/64/256/1024 tracks, four clips per track, two sends
per track, four aux buses, one stateful insert per track and one per aux, over
256 quanta. It verifies a first-frame oracle and exact node/edge execution counts
and prints preparation time, scratch usage and callback-time percentiles.

Results are synthetic, offline measurements on a shared cloud CPU. Deadline
misses, including workloads slower than their 128-frame/48-kHz budget, must be
reported rather than interpreted as a real-time guarantee. The benchmark does
not establish device latency, dropouts, plugin performance or production scale.

## Observed scaling result, 2026-10-01

Release executables were run serially after the core, UI and review builds had
finished, on the same Linux x86_64 / AMD EPYC 9V74 shared cloud host described in
the [real-audio report](audio-fixture.md). Native UI QA could still be active;
this was not a dedicated real-time machine. The 128-frame period at 48 kHz is
2,666.7 microseconds. Each workload rendered 256 quanta.

| Tracks | Nodes / edges | Scratch bytes | Median µs | p95 µs | p99 µs | Maximum µs | Quanta over period |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 8 | 33 / 48 | 17,408 | 38.6 | 56.0 | 253.6 | 2,089.5 | 0 / 256 |
| 64 | 201 / 328 | 132,096 | 231.0 | 1,758.3 | 5,075.4 | 11,417.2 | 9 / 256 |
| 256 | 777 / 1,288 | 525,312 | 974.9 | 6,240.3 | 10,132.9 | 13,107.1 | 44 / 256 |
| 1,024 | 3,081 / 5,128 | 2,098,176 | 6,733.3 | 14,234.8 | 21,609.3 | 23,561.1 | 256 / 256 |

All first-frame oracles and exact single-execution node/edge counts passed.
The 1,024-track workload was slower than the audio period on every quantum;
even the smaller workloads had scheduling outliers. Linear graph work is not
a claim of dropout-free real-time operation. Raw results are in
`artifacts/stress/graph-benchmark-final.json`.

Earlier runs that overlapped or may have overlapped local compilation remain in
`graph-benchmark.json` and `graph-benchmark-stereo-pre-confirmation.json` with
their measurement conditions. They are retained rather than discarded to select
a favorable result. No hardware-device latency or GUI frame-rate was measured.

## Native inspector and mixer integration

The track and aux inspectors expose OUTPUT, INSERTS, and SENDS racks using
ScarletUI and the existing application colors. Inserts use a dedicated retained
190×26 native slot, rather than an always-visible collection of buttons. The
slot name opens a floating editor, its power hit area bypasses, its empty state
opens the picker, and its right-click/chevron menu exposes move/remove actions.
Enter, Space and Shift-F10 operate the focused slot. The Inspector owns the
full left work-area height, with a single editable name, independently scrolling upper content, and a
fixed selected-channel strip below. Selected-region metadata is a separate
collapsible section above the channel name and disappears when no region is
selected. Aux input metadata also sits above the signal path, keeping
Output → Inserts → Sends → fader uninterrupted. The Inspector fader has no
separate card border. That strip reuses the mixer’s exact
M/S, pan, fader, scale, meter and readout component. Its fader travel follows the
mixer height through resizing; the content above scrolls rather than shrinking
the controls. Insert rows have zero gaps and thin separators, including the
empty add slot. The small bypass power glyph uses cached, DPI-aware analytic
antialiasing instead of segmented backend strokes; the hit area stays unchanged.
Only the right-hand arrangement and mixer share the vertical split. Track and
Aux Inspector controls reuse the same gain/pan/peak states as their mixer strip;
focus is separate so a keystroke does not apply a gain change twice. The mixer has one Add Aux
control; every receiver accepts both main outputs and sends. Bus paths appear
as `Bus N → receiver name` in destination lists and `Input Bus N` on the receiver.
Choosing `New Bus → Aux` from Output or Sends creates and connects both in one
undoable transaction. Aux strips reuse the same vertical gain fader, pan
knob, stereo meter, and peak readout as audio tracks. Selecting a mixer strip
opens its inspector. Audio clips remain on the arrangement, while buses appear
only as mixer channels.

- Inserts can be added, reordered, bypassed, removed, and edited numerically
- Built-in Gain uses dB input, OnePole uses a coefficient, and Delay uses an
  integer sample count; delay is feed-forward and fully wet
- Output and send destination lists identify saved buses; sends expose enabled,
  pre-fader/post-pan, and an antialiased gain knob with dB readout. The native
  send slot is one 26px row with an 18px visible dial in a 24px hit area; mode,
  bypass, removal and exact value entry live in its context menu. A matching
  empty destination slot replaces a persistent Add button. The menu can open
  the receiving Aux, whose Inspector lists upstream output/send connections
- Routing candidates are validated before touching the live stream. Rejected
  cycles, resource limits, stale targets, and unchanged edits do not create
  history entries or reset running DSP. Successful routing edits stop playback;
  playback preparation compiles the new graph, rather than ordinary repaint,
  selection, or mixer-level changes doing so
- Track and bus levels/pan/mute and send gains remain live atomic controls. Their drag gestures
  coalesce into one undo entry. Undo/Redo retain bus identities, routing, insert
  parameters and bypass, sends, and the selected channel
- Delete-bus repairs references transactionally and is undoable. Selecting an
  audio clip clears bus selection, preventing shortcuts acting on a hidden track

The first native CLAP effect is integrated through a separate bounded host; see
[CLAP hosting](clap-host.md). Master inserts, bus solo, PDC, feedback, recording,
automation, effect-tail extension, and click-free live graph replacement remain
outside this implementation.

### Host verification, 2026-10-02

The final handoff `scripts/verify` run, including the latest M/S alignment, passed
dependency-pin checks, formatting, all 301 ordinary workspace tests, documentation
tests, the application build, and flat CPAL/ALSA file-sink playback. The official nightly-2025-12-31 x86_64 Linux
toolchain was used (Rust 1.94.0-nightly). The nine real-CLAP fixture-dependent
cases also passed when enabled explicitly; only the pre-existing long-audio
profile workload remained unrun. The separate gain effect passed its ten tests,
and the Scarlet executable auditor passed nine rejection/acceptance tests.

Coverage includes 18 persisted-routing tests, 153 ordinary application tests,
real library/state/parameter/export roundtrips, missing-plugin placeholders,
transaction and gesture Undo, callback allocation/deallocation audits, native
slot input and paint behavior, retained focus, modal keyboard capture, and
Inspector/Mixer fader equality across window sizes and split positions. The ten
small synthetic codec fixtures are included with their regeneration script.
The M/S geometry regression checks both Inspector and Mixer widths, including
the Aux-only M control; the separate AUX label does not shift its interactive
center.

Both `RESONARA_SMOKE_ROUTING=1 bash scripts/audio-smoke` and the same command
with `RESONARA_SMOKE_CLAP=1` and the built native Linux effect succeeded. Each
routed smoke prepared three tracks and two buses, saved and reloaded the session,
exported its graph, and produced 458,640 bytes of nonzero file-sink PCM. The CLAP
variant included the real loaded gain effect. These are host-toolchain checks,
not Scarlet guest execution or real sound-device latency/dropout measurements.

### Routing workflow references

The naming and creation flow were checked against the official Logic Pro and
Cubase guides on 2026-10-02. Logic’s [Aux channel overview](https://support.apple.com/guide/logicpro/aux-channel-strips-overview-lgcp8e7db552/mac)
and [mix subgroup workflow](https://support.apple.com/guide/logicpro/create-mix-subgroups-lgcp8e8310ed/mac)
use the same receiver strip for sends and subgroups, with a new receiver created
when an unused bus is assigned. Its [send routing guide](https://support.apple.com/guide/logicpro/route-audio-via-send-effects-lgcp8ea0091c/mac)
and destination-menu screenshot distinguish a numbered bus from its named
receiver. Resonara follows that distinction, with explicit new-bus creation
instead of listing a fixed bank of unused bus numbers.

Cubase instead provides [Group-channel creation and output routing](https://www.steinberg.help/r/cubase-pro/15.0/en/cubase_nuendo/topics/audio_effects/audio_effects_insert_effects_adding_to_group_channels_t.html)
and [effect-channel send destinations](https://www.steinberg.help/r/cubase-pro/15.0/en/cubase_nuendo/topics/audio_effects/audio_effects_send_effects_fx_channels_routing_audio_channels_to_t.html).
Those are useful workflow roles, not a reason to duplicate Resonara’s summing or
insert DSP. This implementation keeps one shared processor model.

Logic also offers a post-fader/pre-pan send mode. Resonara currently offers only
Pre and Post Pan, and labels them accordingly rather than implying that the
third position or independent send pan is available.

### Live send gain control

`Controls.send_gains` stores prepared send levels in deterministic track-then-bus,
send-slot order, including disabled slots. `Project::send_control_index` resolves
that stable prepared index outside rendering. Each active send uses a small
`SendGain` processor with unit-gain connecting edges; the shared upstream insert
still processes only once. Gains are sampled into preallocated storage once per
quantum. Changing a send knob updates its atomic and project value without
stopping playback, rebuilding the graph, or resetting effect state. The UI shares
one undo snapshot across a drag and restores exact values on cancellation.

The send knob is a unipolar nonlinear gain control, not a bipolar pan dial. It
supports silence, unity reset, finite upper gain, captured relative drag, Escape
and pointer cancellation, and 1/0.1-dB keyboard steps. Analytic-AA rasterization
shares the existing pan dial’s rendering approach. Shift-drag depends on focused
keyboard modifier delivery because current ScarletUI mouse events lack modifiers.
