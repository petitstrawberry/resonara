# Compiled audio graph foundation

The desktop application now uses a compiled block schedule for its existing
tracks-to-master playback. The core additionally exposes an opt-in runtime
routing API for buses, pre/post-fader sends and basic inserts. This is a tested
engine foundation, not yet an aux-routing UI or a new project-file format.

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
- `Engine::new` keeps the original API and automatically prepares the ordinary
  track-to-master graph. Its admission budget scales to that existing project;
  explicit graph budgets are selected through `Engine::with_graph`

## Signal semantics

`TrackSource` mixes its clips before fader/pan. It honors the referenced track's
mute and solo gate, so a send directly from that source receives silence when
gated. A send after a stateful insert can still carry the insert's stored tail
even while its source is gated; the initial source gate does not erase DSP state.
`TrackFader` applies the referenced track's gain/pan and measures its post-fader
peak. `Bus` sums its routes; it has no independent mute/solo control in this API.
Thus soloing a source retains its downstream returns instead of muting buses.

A pre-fader send is an edge from the upstream node before `TrackFader`; a
post-fader send is an edge after it. Insert/tap order is explicit in the graph.
A bus can feed another bus through `Gain`, `OnePole` or `Delay` insert nodes.
Multiple routes between two nodes are additive, intentional signal paths.
Intermediate audio is not clipped. Only final device output applies master gain
and the existing output clamp/mono mapping.

`OnePole` uses `y = (1 - coefficient) * input + coefficient * previous`, with
independent stereo state. `Delay` is an explicit feed-forward sample delay with
preallocated storage. It does **not** enable feedback. Even a one-sample delay
inside a cycle is rejected: supporting that correctly requires a causality-aware
sub-block/sample scheduler, not a recursive pull or one device-callback delay.
No automatic delay is added to hide a cycle.

Track controls are sampled once per internal quantum; master/solo are sampled at
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
compatibility. `Controls.master_peak_left` and `master_peak_right` measure the
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
- Project JSON, current save/export, undo and the desktop UI remain on the
  existing tracks-to-master model. Custom routing is currently constructed by a
  core caller with `Engine::with_graph`; it is not persisted or selected in the UI
- Pause outputs silence and freezes both transport and effect state
- EOF remains the existing hard boundary at `Project::duration()`. Delay/reverb
  tails beyond it are not rendered or exported. A caller can supply a longer
  timeline for experiments, but automatic tail handling is not implemented
- No plugin hosting, plugin delay compensation, recording, sidechain policy,
  bus control identity, sample-accurate automation or feedback SCC execution is
  claimed. These require separate session/API/lifecycle work
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
output, hard EOF and nonfinite fail-stop tests.

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
