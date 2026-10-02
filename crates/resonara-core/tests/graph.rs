use resonara_core::{graph::*, *};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::{Arc, atomic::Ordering},
};

// Integration tests run in their own executable, so this allocator audits the
// actual first callback without affecting the other test binaries or threads.
thread_local! {
    static AUDITING: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
    static FREES: Cell<usize> = const { Cell::new(0) };
}
struct Audit;
fn allocation() {
    AUDITING.with(|guard| {
        if guard.get() {
            ALLOCS.with(|count| count.set(count.get() + 1));
        }
    });
}
fn deallocation() {
    AUDITING.with(|guard| {
        if guard.get() {
            FREES.with(|count| count.set(count.get() + 1));
        }
    });
}
unsafe impl GlobalAlloc for Audit {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        deallocation();
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        allocation();
        deallocation();
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Audit = Audit;

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
fn project(samples: Vec<[f32; 2]>) -> Project {
    Project {
        tracks: vec![Track {
            name: "graph source".into(),
            clips: vec![Clip {
                source_channels: 2,
                start: 0,
                source_offset: 0,
                frames: samples.len(),
                samples: Arc::new(samples),
            }],
            gain: 1.,
            pan: 0.,
            mute: false,
            solo: false,
            routing: resonara_core::ChannelRouting::default(),
        }],
        master: 1.,
        ..Project::default()
    }
}
fn constant(sample: [f32; 2], frames: usize) -> Project {
    project(vec![sample; frames])
}
fn limits(quantum: usize) -> GraphLimits {
    GraphLimits {
        quantum,
        ..GraphLimits::default()
    }
}
fn engine(p: &Project, graph: &RoutingGraph, quantum: usize) -> Engine {
    Engine::with_graph(
        p,
        Arc::new(Controls::new(p)),
        p.sample_rate,
        0,
        graph,
        limits(quantum),
    )
    .unwrap()
}
fn direct_graph() -> RoutingGraph {
    RoutingGraph {
        nodes: vec![
            node(1, Processor::TrackSource { track: 0 }),
            node(2, Processor::Bus),
        ],
        routes: vec![route(1, 2, 1.)],
        output: NodeId(2),
    }
}
fn render(engine: &mut Engine, frames: usize) -> Vec<f32> {
    let mut out = vec![0.; frames * 2];
    engine.render(&mut out, 2);
    out
}
fn assert_counts(engine: &Engine, quanta: u64, nodes: u64, edges: u64) {
    let counts = engine.render_counts();
    assert_eq!(counts.quanta, quanta);
    assert_eq!(counts.nodes, quanta * nodes);
    assert_eq!(counts.edges, quanta * edges);
}

#[test]
fn diamond_sums_both_paths_but_processes_each_node_once_per_quantum() {
    let p = constant([0.0625, -0.125], 100);
    let graph = RoutingGraph {
        nodes: vec![
            node(4, Processor::Bus),
            node(2, Processor::Gain { gain: 2. }),
            node(1, Processor::TrackSource { track: 0 }),
            node(3, Processor::Gain { gain: 3. }),
        ],
        routes: vec![
            route(3, 4, 1.),
            route(1, 3, 1.),
            route(2, 4, 1.),
            route(1, 2, 1.),
        ],
        output: NodeId(4),
    };
    let mut e = engine(&p, &graph, 7);
    assert_eq!(render(&mut e, 20), [0.3125, -0.625].repeat(20));
    assert_counts(&e, 3, 4, 4);
    for id in 1..=4 {
        assert_eq!(e.node_process_count(NodeId(id)), Some(3));
    }
    assert_eq!(render(&mut e, 2), [0.3125, -0.625].repeat(2));
    assert_counts(&e, 4, 4, 4);
}

#[test]
fn shared_stateful_processor_advances_once_before_fanout() {
    let mut samples = vec![[0.; 2]; 64];
    samples[0] = [0.25, -0.5];
    let p = project(samples);
    let graph = RoutingGraph {
        nodes: vec![
            node(1, Processor::TrackSource { track: 0 }),
            node(2, Processor::OnePole { coefficient: 0.5 }),
            node(3, Processor::Bus),
            node(4, Processor::Bus),
            node(5, Processor::Bus),
        ],
        routes: vec![
            route(1, 2, 1.),
            route(2, 3, 0.5),
            route(2, 4, 1.5),
            route(3, 5, 1.),
            route(4, 5, 1.),
        ],
        output: NodeId(5),
    };
    let mut e = engine(&p, &graph, 7);
    let out = render(&mut e, 32);
    let mut previous = [0.; 2];
    for (index, frame) in out.chunks_exact(2).enumerate() {
        let input = if index == 0 { [0.25, -0.5] } else { [0.; 2] };
        for channel in 0..2 {
            previous[channel] = 0.5 * input[channel] + 0.5 * previous[channel];
            assert_eq!(
                frame[channel],
                2. * previous[channel],
                "frame {index}, channel {channel}"
            );
        }
    }
    assert_eq!(e.node_process_count(NodeId(2)), Some(5));
    assert_counts(&e, 5, 5, 5);
}

#[test]
fn deep_layered_fanout_work_is_linear_in_nodes_and_edges() {
    const WIDTH: u64 = 4;
    const LAYERS: u64 = 12;
    let p = constant([0.125, -0.0625], 64);
    let mut graph = RoutingGraph {
        nodes: vec![node(0, Processor::TrackSource { track: 0 })],
        routes: vec![],
        output: NodeId(1 + WIDTH * LAYERS),
    };
    let mut previous = vec![0];
    for layer in 0..LAYERS {
        let current: Vec<_> = (0..WIDTH).map(|i| 1 + layer * WIDTH + i).collect();
        for &id in &current {
            graph.nodes.push(node(id, Processor::Bus));
            for &from in &previous {
                graph
                    .routes
                    .push(route(from, id, 1. / previous.len() as f32));
            }
        }
        previous = current;
    }
    graph.nodes.push(node(graph.output.0, Processor::Bus));
    for from in previous {
        graph
            .routes
            .push(route(from, graph.output.0, 1. / WIDTH as f32));
    }
    let mut e = engine(&p, &graph, 7);
    assert_eq!(render(&mut e, 23), [0.125, -0.0625].repeat(23));
    assert_counts(&e, 4, graph.nodes.len() as u64, graph.routes.len() as u64);
    for node in &graph.nodes {
        assert_eq!(e.node_process_count(node.id), Some(4));
    }
}

fn stateful_graph() -> RoutingGraph {
    RoutingGraph {
        nodes: vec![
            node(1, Processor::TrackSource { track: 0 }),
            node(2, Processor::Delay { frames: 19 }),
            node(3, Processor::OnePole { coefficient: 0.625 }),
            node(4, Processor::Gain { gain: -0.5 }),
            node(5, Processor::Bus),
        ],
        routes: vec![
            route(1, 2, 1.),
            route(2, 3, 1.),
            route(3, 4, 1.),
            route(4, 5, 1.),
            route(1, 5, 0.25),
        ],
        output: NodeId(5),
    }
}

#[test]
fn stateful_render_is_bitwise_equal_across_variable_callback_partitions() {
    let p = project(
        (0..2048)
            .map(|i| {
                [
                    ((i * 17 % 29) as f32 - 14.) / 32.,
                    ((i * 11 % 23) as f32 - 11.) / 32.,
                ]
            })
            .collect(),
    );
    let graph = stateful_graph();
    let partitions = [1, 2, 3, 17, 31, 127, 128, 129, 251];
    let frames: usize = partitions.iter().sum();
    for rate in [44100, 48000, 96000] {
        for quantum in [1, 17, 128] {
            let build = || {
                Engine::with_graph(
                    &p,
                    Arc::new(Controls::new(&p)),
                    rate,
                    0,
                    &graph,
                    limits(quantum),
                )
                .unwrap()
            };
            let expected = render(&mut build(), frames);
            let mut e = build();
            let mut actual = vec![0.; frames * 2];
            let mut offset = 0;
            for size in partitions {
                e.render(&mut actual[offset * 2..(offset + size) * 2], 2);
                offset += size;
            }
            assert_eq!(actual, expected, "device rate {rate}, quantum {quantum}");
            let quanta = partitions.iter().map(|n| n.div_ceil(quantum) as u64).sum();
            assert_counts(&e, quanta, 5, 5);
        }
    }
}

#[test]
fn delay_preserves_independent_stereo_state_across_quantum_boundaries() {
    let mut samples = vec![[0.; 2]; 64];
    samples[0] = [0.25, -0.5];
    samples[7] = [-0.125, 0.75];
    let p = project(samples.clone());
    let graph = RoutingGraph {
        nodes: vec![
            node(1, Processor::TrackSource { track: 0 }),
            node(2, Processor::Delay { frames: 11 }),
        ],
        routes: vec![route(1, 2, 1.)],
        output: NodeId(2),
    };
    let mut e = engine(&p, &graph, 5);
    assert_eq!(
        e.graph_info().delay_bytes,
        11 * std::mem::size_of::<[f32; 2]>()
    );
    let out = render(&mut e, 32);
    for (i, frame) in out.chunks_exact(2).enumerate() {
        assert_eq!(frame, if i < 11 { [0.; 2] } else { samples[i - 11] });
    }
}

#[test]
fn sparse_ids_and_disconnected_acyclic_nodes_do_not_add_runtime_work() {
    let p = constant([0.125; 2], 64);
    let mut graph = direct_graph();
    graph.nodes[0].id = NodeId(9_000_000_001);
    graph.nodes[1].id = NodeId(u64::MAX);
    graph.routes[0] = route(9_000_000_001, u64::MAX, 1.);
    graph.output = NodeId(u64::MAX);
    graph.nodes.push(node(2, Processor::Gain { gain: 9. }));
    graph
        .nodes
        .push(node(3, Processor::OnePole { coefficient: 0.5 }));
    graph.routes.push(route(2, 3, 1.));
    let mut e = engine(&p, &graph, 8);
    assert_eq!(render(&mut e, 17), [0.125; 2].repeat(17));
    assert_counts(&e, 3, 2, 1);
    assert_eq!(e.graph_info().nodes, 2);
    assert_eq!(e.graph_info().edges, 1);
    assert_eq!(e.node_process_count(NodeId(9_000_000_001)), Some(3));
    assert_eq!(e.node_process_count(NodeId(u64::MAX)), Some(3));
    assert_eq!(e.node_process_count(NodeId(2)), None);
    assert_eq!(e.node_process_count(NodeId(3)), None);
    assert_eq!(e.node_process_count(NodeId(555)), None);
}

#[test]
fn declaration_order_does_not_change_floating_point_accumulation() {
    let p = constant([0.25, -0.125], 64);
    let mut graph = RoutingGraph {
        nodes: vec![
            node(1, Processor::TrackSource { track: 0 }),
            node(2, Processor::Gain { gain: 4.0e20 }),
            node(3, Processor::Gain { gain: -4.0e20 }),
            node(4, Processor::Bus),
            node(5, Processor::Bus),
        ],
        routes: vec![
            route(1, 2, 1.),
            route(1, 3, 1.),
            route(1, 4, 1.),
            route(2, 5, 1.),
            route(3, 5, 1.),
            route(4, 5, 1.),
        ],
        output: NodeId(5),
    };
    let expected = render(&mut engine(&p, &graph, 8), 32);
    graph.nodes.reverse();
    graph.routes.reverse();
    assert_eq!(render(&mut engine(&p, &graph, 8), 32), expected);
    graph.nodes.rotate_left(2);
    graph.routes.rotate_left(3);
    assert_eq!(render(&mut engine(&p, &graph, 8), 32), expected);
}

#[test]
fn pre_and_post_fader_sends_honor_live_gain_pan_mute_solo_and_master() {
    let mut p = constant([0.25, 0.5], 64);
    p.tracks[0].gain = 0.5;
    p.tracks[0].pan = 0.5;
    p.tracks.push(constant([0.; 2], 64).tracks.remove(0));
    let graph = RoutingGraph {
        nodes: vec![
            node(1, Processor::TrackSource { track: 0 }),
            node(2, Processor::TrackFader { track: 0 }),
            node(3, Processor::Bus),
        ],
        routes: vec![route(1, 2, 1.), route(1, 3, 0.5), route(2, 3, 1.)],
        output: NodeId(3),
    };
    let controls = Arc::new(Controls::new(&p));
    let mut e = Engine::with_graph(&p, controls.clone(), 48000, 0, &graph, limits(8)).unwrap();
    assert_eq!(render(&mut e, 1), [0.1875, 0.5]);
    controls.tracks[0]
        .gain
        .store(2f32.to_bits(), Ordering::Relaxed);
    controls.tracks[0]
        .pan
        .store((-1f32).to_bits(), Ordering::Relaxed);
    assert_eq!(render(&mut e, 1), [0.625, 0.25]);
    controls.tracks[0].mute.store(true, Ordering::Relaxed);
    assert_eq!(render(&mut e, 1), [0.; 2]);
    controls.tracks[0].mute.store(false, Ordering::Relaxed);
    controls.tracks[1].solo.store(true, Ordering::Relaxed);
    assert_eq!(render(&mut e, 1), [0.; 2]);
    controls.tracks[0].solo.store(true, Ordering::Relaxed);
    controls.master.store(0.5f32.to_bits(), Ordering::Relaxed);
    assert_eq!(render(&mut e, 1), [0.3125, 0.125]);
}

#[test]
fn intermediates_are_unclipped_and_signed_route_gain_preserves_polarity() {
    let mut p = constant([0.75, -0.75], 32);
    p.master = 2.;
    let graph = RoutingGraph {
        nodes: vec![
            node(1, Processor::TrackSource { track: 0 }),
            node(2, Processor::Gain { gain: 4. }),
            node(3, Processor::Bus),
        ],
        routes: vec![route(1, 2, 1.), route(2, 3, -0.125)],
        output: NodeId(3),
    };
    assert_eq!(
        render(&mut engine(&p, &graph, 8), 4),
        [-0.75, 0.75].repeat(4)
    );
}

fn rejected(graph: RoutingGraph, limits: GraphLimits) {
    let p = constant([0.125; 2], 64);
    assert!(Engine::with_graph(&p, Arc::new(Controls::new(&p)), 48000, 0, &graph, limits).is_err());
}

#[test]
fn compilation_rejects_cycles_even_when_disconnected_from_output() {
    rejected(
        RoutingGraph {
            nodes: vec![node(1, Processor::Bus)],
            routes: vec![route(1, 1, 1.)],
            output: NodeId(1),
        },
        limits(8),
    );
    rejected(
        RoutingGraph {
            nodes: vec![node(1, Processor::Bus), node(2, Processor::Bus)],
            routes: vec![route(1, 2, 1.), route(2, 1, 1.)],
            output: NodeId(2),
        },
        limits(8),
    );
    let mut graph = direct_graph();
    graph.nodes.extend([
        node(3, Processor::Bus),
        node(4, Processor::Delay { frames: 1 }),
    ]);
    graph.routes.extend([route(3, 4, 1.), route(4, 3, 1.)]);
    rejected(graph, limits(8));
}

#[test]
fn compilation_rejects_missing_references_duplicate_ids_and_source_inputs() {
    let mut graph = direct_graph();
    graph.output = NodeId(99);
    rejected(graph, limits(8));
    for from_missing in [false, true] {
        let mut graph = direct_graph();
        if from_missing {
            graph.routes[0].from = NodeId(99);
        } else {
            graph.routes[0].to = NodeId(99);
        }
        rejected(graph, limits(8));
    }
    let mut graph = direct_graph();
    graph.nodes.push(node(1, Processor::Bus));
    rejected(graph, limits(8));
    let mut graph = direct_graph();
    graph.nodes.push(node(3, Processor::Bus));
    graph.routes.push(route(3, 1, 1.));
    rejected(graph, limits(8));
    for processor in [
        Processor::TrackSource { track: 1 },
        Processor::TrackFader { track: 1 },
    ] {
        let mut graph = direct_graph();
        graph.nodes[0].processor = processor;
        rejected(graph, limits(8));
    }
}

#[test]
fn compilation_rejects_invalid_processor_values_and_nonfinite_route_gains() {
    for gain in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut graph = direct_graph();
        graph.routes[0].gain = gain;
        rejected(graph, limits(8));
        let mut graph = direct_graph();
        graph.nodes[1].processor = Processor::Gain { gain };
        rejected(graph, limits(8));
    }
    for coefficient in [-0.01, 1., f32::NAN, f32::INFINITY] {
        let mut graph = direct_graph();
        graph.nodes[1].processor = Processor::OnePole { coefficient };
        rejected(graph, limits(8));
    }
    for frames in [0, usize::MAX] {
        let mut graph = direct_graph();
        graph.nodes[1].processor = Processor::Delay { frames };
        rejected(graph, limits(8));
    }
}

#[test]
fn compilation_enforces_each_explicit_resource_limit() {
    rejected(direct_graph(), limits(0));
    rejected(
        direct_graph(),
        GraphLimits {
            max_nodes: 1,
            ..limits(8)
        },
    );
    rejected(
        direct_graph(),
        GraphLimits {
            max_edges: 0,
            ..limits(8)
        },
    );
    rejected(
        direct_graph(),
        GraphLimits {
            max_scratch_bytes: 0,
            ..limits(8)
        },
    );
    rejected(direct_graph(), limits(usize::MAX));
    let mut graph = direct_graph();
    graph.nodes[1].processor = Processor::Delay { frames: 32 };
    rejected(
        graph,
        GraphLimits {
            max_delay_bytes: 31 * 8,
            ..limits(8)
        },
    );
}

#[test]
fn graph_reports_bounded_preallocated_resources() {
    let p = constant([0.125; 2], 64);
    let e = engine(&p, &stateful_graph(), 17);
    let info = e.graph_info();
    assert_eq!(info.nodes, 5);
    assert_eq!(info.edges, 5);
    assert_eq!(info.quantum, 17);
    assert!(info.buffer_slots > 0 && info.buffer_slots <= info.nodes);
    assert_eq!(
        info.scratch_bytes,
        info.buffer_slots * 17 * std::mem::size_of::<[f32; 2]>()
    );
    assert_eq!(info.delay_bytes, 19 * std::mem::size_of::<[f32; 2]>());
    assert_counts(&e, 0, 5, 5);
}

#[test]
fn first_and_repeated_active_callbacks_neither_allocate_nor_deallocate() {
    let p = constant([0.125, -0.25], 100_000);
    let mut e = engine(&p, &stateful_graph(), 17);
    let mut out = [0.; 514];
    ALLOCS.with(|count| count.set(0));
    FREES.with(|count| count.set(0));
    AUDITING.with(|guard| guard.set(true));
    // No warm-up: the first callback is inside the audit, as are varying
    // lengths and repeated callbacks while transport is still active.
    for i in 0..300 {
        let frames = [1, 17, 128, 257][i % 4];
        e.render(&mut out[..frames * 2], 2);
    }
    AUDITING.with(|guard| guard.set(false));
    assert_eq!(ALLOCS.with(Cell::get), 0, "render allocated");
    assert_eq!(FREES.with(Cell::get), 0, "render deallocated");
    assert!(e.controls.playing.load(Ordering::Relaxed));
    assert!(out.iter().any(|sample| *sample != 0.));
}

#[test]
fn persistent_routing_bus_controls_and_sends_do_not_allocate_in_callbacks() {
    let mut p = constant([0.125, -0.25], 100_000);
    let a = p.add_bus("Aux", BusKind::Aux);
    let b = p.add_bus("Group", BusKind::Group);
    p.tracks[0].routing.inserts = vec![Insert {
        kind: InsertKind::OnePole { coefficient: 0.75 },
        bypass: false,
    }];
    p.tracks[0].routing.sends = vec![
        Send {
            target: a,
            gain: 0.5,
            pre_fader: true,
            enabled: true,
        },
        Send {
            target: b,
            gain: 0.25,
            pre_fader: false,
            enabled: true,
        },
        Send {
            target: b,
            gain: 10.,
            pre_fader: false,
            enabled: false,
        },
    ];
    p.bus_mut(a).unwrap().routing.inserts = vec![Insert {
        kind: InsertKind::Delay { frames: 19 },
        bypass: false,
    }];
    p.bus_mut(a).unwrap().routing.output = Destination::Bus(b);
    p.bus_mut(a).unwrap().routing.sends.push(Send {
        target: b,
        gain: 0.25,
        pre_fader: true,
        enabled: true,
    });
    let mut e = Engine::try_new(&p, Arc::new(Controls::new(&p)), p.sample_rate, 0).unwrap();
    let mut out = [0.; 514];
    ALLOCS.with(|count| count.set(0));
    FREES.with(|count| count.set(0));
    AUDITING.with(|guard| guard.set(true));
    for i in 0..300 {
        let frames = [1, 17, 128, 257][i % 4];
        e.controls.buses[0]
            .gain
            .store((0.1 * (i % 8) as f32).to_bits(), Ordering::Relaxed);
        for (slot, gain) in e.controls.send_gains.iter().enumerate() {
            gain.store(
                (0.05 * ((i + slot) % 8) as f32).to_bits(),
                Ordering::Relaxed,
            );
        }
        e.render(&mut out[..frames * 2], 2);
    }
    AUDITING.with(|guard| guard.set(false));
    assert_eq!(ALLOCS.with(Cell::get), 0, "render allocated");
    assert_eq!(FREES.with(Cell::get), 0, "render deallocated");
    assert!(e.controls.playing.load(Ordering::Relaxed));
    assert!(out.iter().any(|sample| *sample != 0.));
}
