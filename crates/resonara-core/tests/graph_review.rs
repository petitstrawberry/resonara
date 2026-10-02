//! Independent regression review of the prepared block graph.
use resonara_core::{graph::*, *};
use std::sync::{Arc, atomic::Ordering};

fn node(id: u64, processor: Processor) -> Node {
    Node {
        id: NodeId(id),
        processor,
    }
}
fn route(from: u64, to: u64, gain: f32) -> Route {
    Route {
        from: NodeId(from),
        to: NodeId(to),
        gain,
    }
}
fn project() -> Project {
    Project {
        tracks: (0..2)
            .map(|track| Track {
                name: format!("review {track}"),
                clips: vec![Clip {
                    source_channels: 2,
                    edit: Default::default(),
                    start: 3 * track as u64,
                    source_offset: 2,
                    frames: 143,
                    samples: Arc::new(
                        (0..145)
                            .map(|i| {
                                [
                                    ((i * (17 + track) % 29) as f32 - 14.) / 128.,
                                    ((i * (11 + track) % 23) as f32 - 11.) / 128.,
                                ]
                            })
                            .collect(),
                    ),
                }],
                gain: 0.5 + track as f32 * 0.25,
                pan: if track == 0 { 0.25 } else { -0.5 },
                mute: false,
                solo: false,
                routing: resonara_core::ChannelRouting::default(),
            })
            .collect(),
        master: 0.75,
        ..Project::default()
    }
}
fn engine(p: &Project, graph: &RoutingGraph, rate: u32, quantum: usize) -> Engine {
    Engine::with_graph(
        p,
        Arc::new(Controls::new(p)),
        rate,
        0,
        graph,
        GraphLimits {
            quantum,
            ..GraphLimits::default()
        },
    )
    .unwrap()
}

#[test]
fn finite_graph_parameters_do_not_emit_nan_after_overflow_cancellation() {
    let mut p = project();
    p.tracks.truncate(1);
    p.tracks[0].clips[0].samples = Arc::new(vec![[0.5; 2]; 145]);
    let graph = RoutingGraph {
        nodes: vec![
            node(1, Processor::TrackSource { track: 0 }),
            node(2, Processor::Gain { gain: f32::MAX }),
            node(3, Processor::Bus),
        ],
        routes: vec![
            route(1, 2, 1.),
            route(2, 3, f32::MAX),
            route(2, 3, -f32::MAX),
        ],
        output: NodeId(3),
    };
    p.validate().unwrap();
    let compiled = Engine::with_graph(
        &p,
        Arc::new(Controls::new(&p)),
        48000,
        0,
        &graph,
        GraphLimits::default(),
    );
    // Either reject this graph at admission or keep the device boundary finite.
    if let Ok(mut e) = compiled {
        let mut out = [0.; 2];
        e.render(&mut out, 2);
        assert!(
            out.iter().all(|v| v.is_finite()),
            "accepted finite graph emitted {out:?}"
        );
        assert!(e.controls.error.load(Ordering::Relaxed));
        assert!(!e.controls.playing.load(Ordering::Relaxed));
    }
}

// The oracle processes one sample at a time, without the compiled schedule,
// pooled scratch arena, block loop or mutable state from the production engine.
struct ReferenceState {
    delay: Vec<[f32; 2]>,
    cursor: usize,
    previous: [f32; 2],
}
fn oracle(p: &Project, ordered: &RoutingGraph, rate: u32, frames: usize) -> Vec<f32> {
    let send_gains: Vec<_> = p
        .tracks
        .iter()
        .map(|track| &track.routing)
        .chain(p.buses.iter().map(|bus| &bus.routing))
        .flat_map(|routing| routing.sends.iter().map(|send| send.gain))
        .collect();
    let mut state: Vec<_> = ordered
        .nodes
        .iter()
        .map(|node| ReferenceState {
            delay: match node.processor {
                Processor::Delay { frames } => vec![[0.; 2]; frames],
                _ => vec![],
            },
            cursor: 0,
            previous: [0.; 2],
        })
        .collect();
    let inputs: Vec<Vec<(usize, f32)>> = ordered
        .nodes
        .iter()
        .map(|node| {
            let mut incoming: Vec<_> = ordered
                .routes
                .iter()
                .filter(|r| r.to == node.id)
                .map(|r| {
                    (
                        ordered.nodes.iter().position(|n| n.id == r.from).unwrap(),
                        r.gain,
                    )
                })
                .collect();
            incoming.sort_by_key(|&(index, gain)| (ordered.nodes[index].id, gain.to_bits()));
            incoming
        })
        .collect();
    let output = ordered
        .nodes
        .iter()
        .position(|n| n.id == ordered.output)
        .unwrap();
    let mut result = Vec::with_capacity(frames * 2);
    let mut samples = vec![[0.; 2]; ordered.nodes.len()];
    let solo = p.tracks.iter().any(|t| t.solo);
    let audible = |track: usize| !p.tracks[track].mute && (!solo || p.tracks[track].solo);
    let mut position = 0.;
    for _ in 0..frames {
        if position >= p.duration() as f64 {
            result.extend([0.; 2]);
            continue;
        }
        for (index, node) in ordered.nodes.iter().enumerate() {
            let mut sample = [0.; 2];
            for &(from, gain) in &inputs[index] {
                assert!(
                    from < index,
                    "review fixture must be independently topological"
                );
                for channel in 0..2 {
                    sample[channel] += samples[from][channel] * gain;
                }
            }
            match node.processor {
                Processor::Clap { .. } => {
                    panic!("CLAP has its own external-process reference tests")
                }
                Processor::TrackSource { track } => {
                    if audible(track) {
                        for clip in &p.tracks[track].clips {
                            let x = position - clip.start as f64;
                            if x < 0. || x >= clip.frames as f64 {
                                continue;
                            }
                            let a = x as usize;
                            let b = (a + 1).min(clip.frames - 1);
                            let f = x.fract() as f32;
                            for channel in 0..2 {
                                sample[channel] += clip.samples[clip.source_offset + a][channel]
                                    * (1. - f)
                                    + clip.samples[clip.source_offset + b][channel] * f;
                            }
                        }
                    }
                }
                Processor::TrackFader { track } => {
                    let track_model = &p.tracks[track];
                    if audible(track) {
                        sample[0] *= track_model.gain * (1. - track_model.pan.max(0.));
                        sample[1] *= track_model.gain * (1. + track_model.pan.min(0.));
                    } else {
                        sample = [0.; 2];
                    }
                }
                Processor::TrackGate { track } => {
                    if !audible(track) {
                        sample = [0.; 2];
                    }
                }
                Processor::BusGate { bus } => {
                    if p.buses[bus].mute {
                        sample = [0.; 2];
                    }
                }
                Processor::BusFader { bus } => {
                    let bus = &p.buses[bus];
                    if bus.mute {
                        sample = [0.; 2];
                    } else {
                        sample[0] *= bus.gain * (1. - bus.pan.max(0.));
                        sample[1] *= bus.gain * (1. + bus.pan.min(0.));
                    }
                }
                Processor::Gain { gain } => {
                    sample[0] *= gain;
                    sample[1] *= gain;
                }
                Processor::SendGain { send } => {
                    sample[0] *= send_gains[send];
                    sample[1] *= send_gains[send];
                }
                Processor::Bus => {}
                Processor::Delay { .. } => {
                    let s = &mut state[index];
                    let delayed = s.delay[s.cursor];
                    s.delay[s.cursor] = sample;
                    s.cursor = (s.cursor + 1) % s.delay.len();
                    sample = delayed;
                }
                Processor::OnePole { coefficient } => {
                    for channel in 0..2 {
                        sample[channel] = (1. - coefficient) * sample[channel]
                            + coefficient * state[index].previous[channel];
                    }
                    state[index].previous = sample;
                }
            }
            samples[index] = sample;
        }
        result.extend(samples[output].map(|v| (v * p.master).clamp(-1., 1.)));
        position += p.sample_rate as f64 / rate as f64;
    }
    result
}

fn varied_graph(seed: u64) -> RoutingGraph {
    let id = |index: usize| 10_000 - index as u64 * 31;
    let mut graph = RoutingGraph {
        nodes: vec![
            node(id(0), Processor::TrackSource { track: 0 }),
            node(id(1), Processor::TrackSource { track: 1 }),
            node(id(2), Processor::TrackFader { track: 0 }),
            node(id(3), Processor::TrackFader { track: 1 }),
        ],
        routes: vec![route(id(0), id(2), 1.), route(id(1), id(3), 1.)],
        output: NodeId(id(29)),
    };
    let mut random = seed + 1;
    for index in 4..30 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let processor = match (index as u64 + seed) % 5 {
            0 => Processor::Gain { gain: -0.5 },
            1 => Processor::OnePole { coefficient: 0.625 },
            2 => Processor::Delay {
                frames: 1 + random as usize % 19,
            },
            _ => Processor::Bus,
        };
        graph.nodes.push(node(id(index), processor));
        graph.routes.push(route(id(index - 1), id(index), 0.5));
        graph
            .routes
            .push(route(id(random as usize % index), id(index), 0.25));
        if index == 4 {
            graph.routes.push(route(id(2), id(index), 0.5));
        }
        if index % 3 == 0 {
            // Repeated inputs are intentional routes, not duplicate node execution.
            graph.routes.push(route(id(index - 1), id(index), -0.125));
        }
    }
    graph
}

#[test]
fn pooled_graph_matches_independent_sample_oracle_for_varied_dags_and_rates() {
    for seed in 0..12 {
        let mut p = project();
        match seed % 4 {
            1 => p.tracks[0].mute = true,
            2 => p.tracks[1].solo = true,
            3 => {
                p.tracks[0].solo = true;
                p.tracks[1].solo = true;
            }
            _ => {}
        }
        let ordered = varied_graph(seed);
        let mut shuffled = ordered.clone();
        shuffled.nodes.reverse();
        shuffled.routes.rotate_left(seed as usize);
        if seed % 2 == 0 {
            shuffled.routes.reverse();
        }
        for rate in [44100, 96000] {
            let expected = oracle(&p, &ordered, rate, 340);
            for quantum in [1, 7, 17, 128] {
                let mut e = engine(&p, &shuffled, rate, quantum);
                let mut actual = vec![0.; expected.len()];
                let mut offset = 0;
                for size in [1, 2, 3, 127, 17, 129, 61] {
                    e.render(&mut actual[offset * 2..(offset + size) * 2], 2);
                    offset += size;
                }
                assert_eq!(
                    actual, expected,
                    "seed={seed}, rate={rate}, quantum={quantum}"
                );
                let counts = e.render_counts();
                assert_eq!(counts.nodes, counts.quanta * ordered.nodes.len() as u64);
                assert_eq!(counts.edges, counts.quanta * ordered.routes.len() as u64);
                for node in &ordered.nodes {
                    assert_eq!(e.node_process_count(node.id), Some(counts.quanta));
                }
                assert_eq!(e.controls.position.load(Ordering::Relaxed), p.duration());
                assert!(!e.controls.playing.load(Ordering::Relaxed));
            }
        }
    }
}

#[test]
fn paused_graph_freezes_delay_filter_source_and_node_counts_until_resume() {
    let p = project();
    let graph = varied_graph(4);
    let mut continuous = engine(&p, &graph, 96000, 7);
    let mut expected = [0.; 180];
    continuous.render(&mut expected, 2);
    let mut paused = engine(&p, &graph, 96000, 7);
    let mut actual = [0.; 180];
    paused.render(&mut actual[..62], 2);
    let position = paused.controls.position.load(Ordering::Relaxed);
    let counts = paused.render_counts();
    paused.controls.playing.store(false, Ordering::Relaxed);
    let mut silence = [99.; 82];
    for _ in 0..4 {
        paused.render(&mut silence, 2);
        assert_eq!(silence, [0.; 82]);
    }
    assert_eq!(paused.controls.position.load(Ordering::Relaxed), position);
    assert_eq!(paused.render_counts(), counts);
    paused.controls.playing.store(true, Ordering::Relaxed);
    paused.render(&mut actual[62..], 2);
    assert_eq!(actual, expected);
}

#[test]
fn zero_channel_empty_and_partial_output_preserve_existing_frame_semantics() {
    let p = project();
    let graph = RoutingGraph::tracks_to_master(p.tracks.len());
    let mut e = engine(&p, &graph, 48000, 1);
    let mut untouched = [123.; 9];
    e.render(&mut untouched, 0);
    e.render::<f32>(&mut [], 2);
    assert_eq!(untouched, [123.; 9]);
    assert_eq!(e.controls.position.load(Ordering::Relaxed), 0);
    assert_eq!(e.render_counts(), RenderCounts::default());
    let mut expected_engine = engine(&p, &graph, 48000, 1);
    let mut stereo = [0.; 6];
    expected_engine.render(&mut stereo, 2);
    let mut partial = [0.; 5];
    e.render(&mut partial, 2);
    assert_eq!(partial, stereo[..5]);
    assert_eq!(e.controls.position.load(Ordering::Relaxed), 3);
    let mut huge_channels = engine(&p, &graph, 48000, 65536);
    let mut short = [0.; 3];
    huge_channels.render(&mut short, usize::MAX);
    assert_eq!(short, [stereo[0], stereo[1], 0.]);
    assert_eq!(huge_channels.controls.position.load(Ordering::Relaxed), 1);
}

#[test]
fn hard_project_eof_is_explicit_even_when_delay_contains_unplayed_audio() {
    let mut p = project();
    p.tracks.truncate(1);
    let graph = RoutingGraph {
        nodes: vec![
            node(1, Processor::TrackSource { track: 0 }),
            node(2, Processor::Delay { frames: 200 }),
        ],
        routes: vec![route(1, 2, 1.)],
        output: NodeId(2),
    };
    let mut e = engine(&p, &graph, 48000, 17);
    let mut out = [99.; 700];
    e.render(&mut out, 2);
    assert_eq!(out, [0.; 700]);
    assert_eq!(e.controls.position.load(Ordering::Relaxed), p.duration());
    assert!(!e.controls.playing.load(Ordering::Relaxed));
    assert_eq!(e.render_counts().quanta, p.duration().div_ceil(17));
    let counts = e.render_counts();
    e.controls.playing.store(true, Ordering::Relaxed);
    e.render(&mut out, 2);
    assert_eq!(e.render_counts(), counts);
    assert_eq!(out, [0.; 700]);
}

#[test]
fn nonfinite_state_requires_explicit_fail_stop_or_recovers_after_gain_returns() {
    let mut p = project();
    p.tracks.truncate(1);
    p.tracks[0].pan = 0.;
    p.tracks[0].gain = 2.;
    p.tracks[0].clips[0].samples = Arc::new(vec![[1., -1.]; 145]);
    let graph = RoutingGraph {
        nodes: vec![
            node(1, Processor::TrackSource { track: 0 }),
            node(2, Processor::Gain { gain: f32::MAX }),
            node(3, Processor::TrackFader { track: 0 }),
            node(4, Processor::Delay { frames: 2 }),
            node(5, Processor::OnePole { coefficient: 0.5 }),
        ],
        routes: vec![
            route(1, 2, 1.),
            route(2, 3, 1.),
            route(3, 4, 1.),
            route(4, 5, 1.),
        ],
        output: NodeId(5),
    };
    p.validate().unwrap();
    let mut e = engine(&p, &graph, 48000, 7);
    let mut out = [0.; 20];
    e.render(&mut out, 2);
    assert!(out.iter().all(|v| v.is_finite()));
    assert!(e.controls.error.load(Ordering::Relaxed));
    e.controls.tracks[0]
        .gain
        .store(0.5f32.to_bits(), Ordering::Relaxed);
    e.render(&mut out, 2);
    assert!(out.iter().all(|v| v.is_finite()));
    // Output-only sanitization hides poisoned OnePole state forever. Either
    // stop explicitly or recover finite state; don't silently play permanent zero.
    assert!(
        !e.controls.playing.load(Ordering::Relaxed) || out.iter().any(|v| *v != 0.),
        "finite gain returned, but transport still plays while poisoned state emits only silence"
    );
    // The selected policy is a permanent engine fault, not a filter-state repair.
    // Even clearing the public status atomics must not resume contaminated DSP.
    let counts = e.render_counts();
    let position = e.controls.position.load(Ordering::Relaxed);
    e.controls.error.store(false, Ordering::Relaxed);
    e.controls.playing.store(true, Ordering::Relaxed);
    e.render(&mut out, 2);
    assert_eq!(out, [0.; 20]);
    assert_eq!(e.render_counts(), counts);
    assert_eq!(e.controls.position.load(Ordering::Relaxed), position);
    assert!(e.controls.error.load(Ordering::Relaxed));
    assert!(!e.controls.playing.load(Ordering::Relaxed));
    // Repreparing outside the callback restores clean state with normal controls.
    p.tracks[0].gain = 0.5;
    let mut recovered = engine(&p, &graph, 48000, 7);
    recovered.render(&mut out, 2);
    assert!(out.iter().all(|v| v.is_finite()));
    assert!(out.iter().any(|v| *v != 0.));
    assert!(!recovered.controls.error.load(Ordering::Relaxed));
}

#[test]
fn explicit_graph_preserves_split_and_reloaded_sources_at_device_rates() {
    let p = project();
    let graph = varied_graph(5);
    let mut split = p.clone();
    for (track, at) in [(0, 1), (0, 71), (0, 142), (1, 4), (1, 88)] {
        split.split(track, at).unwrap();
    }
    for track in &mut split.tracks {
        track.clips.reverse();
    }
    let bytes = serde_json::to_vec(&split).unwrap();
    let reloaded: Project = serde_json::from_slice(&bytes).unwrap();
    reloaded.validate().unwrap();
    assert!(!Arc::ptr_eq(
        &reloaded.tracks[0].clips[0].samples,
        &reloaded.tracks[0].clips[1].samples
    ));
    for rate in [44100, 96000] {
        let mut expected = [0.; 680];
        engine(&p, &graph, rate, 7).render(&mut expected, 2);
        for (name, project) in [("split", &split), ("reloaded", &reloaded)] {
            for quantum in [1, 17, 128] {
                let mut actual = [0.; 680];
                engine(project, &graph, rate, quantum).render(&mut actual, 2);
                assert_eq!(actual, expected, "{name}, rate={rate}, quantum={quantum}");
            }
        }
    }
}

thread_local! {
    static AUDIT_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static AUDIT_ALLOCS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static AUDIT_FREES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
struct AllocationAudit;
unsafe impl std::alloc::GlobalAlloc for AllocationAudit {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        AUDIT_ACTIVE.with(|active| {
            if active.get() {
                AUDIT_ALLOCS.with(|n| n.set(n.get() + 1));
            }
        });
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        AUDIT_ACTIVE.with(|active| {
            if active.get() {
                AUDIT_ALLOCS.with(|n| n.set(n.get() + 1));
            }
        });
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        AUDIT_ACTIVE.with(|active| {
            if active.get() {
                AUDIT_FREES.with(|n| n.set(n.get() + 1));
            }
        });
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        AUDIT_ACTIVE.with(|active| {
            if active.get() {
                AUDIT_ALLOCS.with(|n| n.set(n.get() + 1));
                AUDIT_FREES.with(|n| n.set(n.get() + 1));
            }
        });
        unsafe { std::alloc::System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: AllocationAudit = AllocationAudit;

#[test]
fn render_fault_and_repeated_latched_callbacks_allocate_and_free_nothing() {
    let p = project();
    let mut e = engine(&p, &varied_graph(4), 44100, 7);
    let mut output = [0.; 512];
    // Exercise the new error branch from the first callback, followed by a
    // thousand attempts to clear public flags and resume the latched engine.
    e.controls
        .master
        .store(f32::NAN.to_bits(), Ordering::Relaxed);
    AUDIT_ALLOCS.with(|n| n.set(0));
    AUDIT_FREES.with(|n| n.set(0));
    AUDIT_ACTIVE.with(|active| active.set(true));
    e.render(&mut output, 2);
    for _ in 0..1000 {
        e.controls.master.store(1f32.to_bits(), Ordering::Relaxed);
        e.controls.playing.store(true, Ordering::Relaxed);
        e.controls.error.store(false, Ordering::Relaxed);
        e.render(&mut output, 2);
    }
    AUDIT_ACTIVE.with(|active| active.set(false));
    assert_eq!(AUDIT_ALLOCS.with(std::cell::Cell::get), 0);
    assert_eq!(AUDIT_FREES.with(std::cell::Cell::get), 0);
    assert!(e.controls.error.load(Ordering::Relaxed));
    assert!(!e.controls.playing.load(Ordering::Relaxed));
    assert_eq!(output, [0.; 512]);
    assert_eq!(e.render_counts().quanta, 1);
}
