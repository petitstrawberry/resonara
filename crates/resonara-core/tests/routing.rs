use resonara_core::{graph::Processor, *};
use std::sync::{Arc, atomic::Ordering};

fn project(samples: Vec<[f32; 2]>) -> Project {
    Project {
        tracks: vec![Track {
            name: "Source".into(),
            clips: vec![Clip {
                source_channels: 2,
                start: 0,
                source_offset: 0,
                frames: samples.len(),
                samples: Arc::new(samples),
            }],
            gain: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
            routing: ChannelRouting::default(),
        }],
        ..Project::default()
    }
}
fn insert(kind: InsertKind) -> Insert {
    Insert {
        kind,
        bypass: false,
    }
}
fn send(target: BusId, gain: f32, pre_fader: bool) -> Send {
    Send {
        target,
        gain,
        pre_fader,
        enabled: true,
    }
}
fn render(p: &Project, frames: usize) -> Vec<f32> {
    let mut engine = Engine::try_new(p, Arc::new(Controls::new(p)), p.sample_rate, 0).unwrap();
    let mut out = vec![0.; frames * 2];
    engine.render(&mut out, 2);
    out
}
fn close(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert!(
            (actual - expected).abs() < 1e-6,
            "sample {index}: {actual} != {expected}"
        );
    }
}
#[test]
fn default_routing_preserves_flat_graph_and_audio() {
    let p = project(vec![[0.2, -0.3]; 4]);
    let graph = p.routing_graph().unwrap();
    assert_eq!(graph.nodes.len(), 3);
    assert_eq!(graph.routes.len(), 2);
    assert_eq!(graph.output, graph::NodeId(2));
    close(
        &render(&p, 4),
        &[0.2, -0.3, 0.2, -0.3, 0.2, -0.3, 0.2, -0.3],
    );
}
#[test]
fn inserts_are_persistent_ordered_and_bypassed_without_dsp_state() {
    let mut p = project(vec![[0.4; 2], [0.; 2], [0.; 2], [0.; 2]]);
    p.tracks[0].routing.inserts = vec![
        insert(InsertKind::Gain { gain: 0.5 }),
        insert(InsertKind::OnePole { coefficient: 0.5 }),
        insert(InsertKind::Delay { frames: 1 }),
        Insert {
            kind: InsertKind::Gain { gain: 100. },
            bypass: true,
        },
    ];
    let graph = p.routing_graph().unwrap();
    let kinds: Vec<_> = graph
        .nodes
        .iter()
        .filter_map(|node| match node.processor {
            Processor::Gain { .. } => Some("gain"),
            Processor::OnePole { .. } => Some("filter"),
            Processor::Delay { .. } => Some("delay"),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, ["gain", "filter", "delay"]);
    close(
        &render(&p, 4),
        &[0., 0., 0.1, 0.1, 0.05, 0.05, 0.025, 0.025],
    );
    p.tracks[0].routing.inserts[2].bypass = true;
    close(
        &render(&p, 4),
        &[0.1, 0.1, 0.05, 0.05, 0.025, 0.025, 0.0125, 0.0125],
    );
    p.tracks[0].routing.inserts.swap(0, 1);
    let graph = p.routing_graph().unwrap();
    assert!(matches!(
        graph.nodes[3].processor,
        Processor::OnePole { .. }
    ));
    assert!(matches!(graph.nodes[4].processor, Processor::Gain { .. }));
}
#[test]
fn pre_and_post_sends_tap_after_inserts_and_respect_fader_pan() {
    let mut p = project(vec![[0.2; 2]; 4]);
    let aux = p.add_bus("Return", BusKind::Aux);
    p.bus_mut(aux).unwrap().gain = 0.5;
    p.tracks[0].gain = 0.5;
    p.tracks[0].pan = 0.5;
    p.tracks[0]
        .routing
        .inserts
        .push(insert(InsertKind::Gain { gain: 2.0 }));
    p.tracks[0].routing.sends.push(send(aux, 0.25, true));
    close(&render(&p, 1), &[0.15, 0.25]);
    p.tracks[0].routing.sends[0].pre_fader = false;
    close(&render(&p, 1), &[0.1125, 0.225]);
    p.tracks[0].routing.sends[0].enabled = false;
    close(&render(&p, 1), &[0.1, 0.2]);
}
#[test]
fn send_control_slots_include_disabled_sends_in_track_then_bus_order() {
    let mut p = project(vec![[0.2; 2]; 4]);
    p.tracks.push(p.tracks[0].clone());
    let a = p.add_bus("A", BusKind::Aux);
    let b = p.add_bus("B", BusKind::Aux);
    let sink = p.add_bus("Sink", BusKind::Aux);
    p.tracks[0].routing.sends = vec![send(a, 0.1, true), send(b, 0.2, false)];
    p.tracks[0].routing.sends[0].enabled = false;
    p.tracks[1].routing.sends = vec![send(a, 0.3, false)];
    p.bus_mut(a).unwrap().routing.sends = vec![send(sink, 0.4, true), send(sink, 0.5, false)];
    p.bus_mut(a).unwrap().routing.sends[0].enabled = false;
    p.bus_mut(b).unwrap().routing.sends = vec![send(sink, 0.6, false)];
    for (track, bus, slot, expected) in [
        (Some(0), None, 0, 0),
        (Some(0), None, 1, 1),
        (Some(1), None, 0, 2),
        (None, Some(a), 0, 3),
        (None, Some(a), 1, 4),
        (None, Some(b), 0, 5),
    ] {
        assert_eq!(p.send_control_index(track, bus, slot), Some(expected));
    }
    for (track, bus, slot) in [
        (None, None, 0),
        (Some(0), Some(a), 0),
        (Some(usize::MAX), None, 0),
        (None, Some(BusId(u64::MAX)), 0),
        (Some(0), None, 2),
        (None, Some(sink), 0),
    ] {
        assert_eq!(p.send_control_index(track, bus, slot), None);
    }
    let controls = Controls::new(&p);
    let gains: Vec<_> = controls
        .send_gains
        .iter()
        .map(|gain| f32::from_bits(gain.load(Ordering::Relaxed)))
        .collect();
    assert_eq!(gains, [0.1, 0.2, 0.3, 0.4, 0.5, 0.6]);
    let graph = p.routing_graph().unwrap();
    let slots: Vec<_> = graph
        .nodes
        .iter()
        .filter_map(|node| match node.processor {
            Processor::SendGain { send } => Some(send),
            _ => None,
        })
        .collect();
    assert_eq!(slots, [1, 2, 4, 5]);
    assert!(graph.routes.iter().all(|route| route.gain == 1.0));
    p.tracks[0].routing.sends[0].enabled = true;
    assert_eq!(p.send_control_index(Some(1), None, 0), Some(2));
    p.buses.swap(0, 1);
    assert_eq!(p.send_control_index(None, Some(b), 0), Some(3));
    assert_eq!(p.send_control_index(None, Some(a), 1), Some(5));
}

#[test]
fn live_pre_and_post_send_levels_update_tracks_and_buses_without_repreparing() {
    for bus_sender in [false, true] {
        let mut p = project(vec![[0.2, 0.4]; 32]);
        let mut silent = p.tracks[0].clone();
        silent.clips.clear();
        p.tracks.push(silent);
        let source_bus = p.add_bus("Source bus", BusKind::Group);
        let aux = p.add_bus("Return", BusKind::Aux);
        p.bus_mut(aux).unwrap().gain = 0.5;
        let (track, bus, routing) = if bus_sender {
            p.tracks[0].routing.output = Destination::Bus(source_bus);
            let source = p.bus_mut(source_bus).unwrap();
            source.gain = 0.5;
            source.pan = 0.5;
            (None, Some(source_bus), &mut source.routing)
        } else {
            p.tracks[0].gain = 0.5;
            p.tracks[0].pan = 0.5;
            (Some(0), None, &mut p.tracks[0].routing)
        };
        routing.inserts.push(insert(InsertKind::Gain { gain: 2.0 }));
        routing.sends = vec![
            send(aux, 10., true),
            send(aux, 0.25, true),
            send(aux, 0.5, false),
        ];
        routing.sends[0].enabled = false;
        let pre = p.send_control_index(track, bus, 1).unwrap();
        let post = p.send_control_index(track, bus, 2).unwrap();
        let controls = Arc::new(Controls::new(&p));
        let mut e = Engine::try_new(&p, controls.clone(), p.sample_rate, 0).unwrap();
        let mut out = [0.; 2];
        e.render(&mut out, 2);
        close(&out, &[0.175, 0.6]);
        controls.send_gains[pre].store(0.5f32.to_bits(), Ordering::Relaxed);
        controls.send_gains[post].store(0.25f32.to_bits(), Ordering::Relaxed);
        e.render(&mut out, 2);
        close(&out, &[0.2125, 0.65]);
        let gain = if bus_sender {
            &controls.buses[0].gain
        } else {
            &controls.tracks[0].gain
        };
        gain.store(0.0f32.to_bits(), Ordering::Relaxed);
        e.render(&mut out, 2);
        close(&out, &[0.1, 0.2]);
        controls.tracks[0].mute.store(true, Ordering::Relaxed);
        e.render(&mut out, 2);
        close(&out, &[0., 0.]);
        controls.tracks[0].mute.store(false, Ordering::Relaxed);
        controls.tracks[1].solo.store(true, Ordering::Relaxed);
        e.render(&mut out, 2);
        close(&out, &[0., 0.]);
        controls.tracks[1].solo.store(false, Ordering::Relaxed);
        controls.buses[1].mute.store(true, Ordering::Relaxed);
        e.render(&mut out, 2);
        close(&out, &[0., 0.]);
        controls.buses[1].mute.store(false, Ordering::Relaxed);
        gain.store(0.5f32.to_bits(), Ordering::Relaxed);
        e.render(&mut out, 2);
        close(&out, &[0.2125, 0.65]);
        assert_eq!(e.render_counts().quanta, 7);
        assert_eq!(e.render_counts().nodes, e.graph_info().nodes as u64 * 7);
        assert_eq!(e.render_counts().edges, e.graph_info().edges as u64 * 7);
        assert_eq!(controls.position.load(Ordering::Relaxed), 7);
        assert!(controls.playing.load(Ordering::Relaxed));
        assert!(!controls.error.load(Ordering::Relaxed));
    }
}

#[test]
fn live_send_compilation_validates_indices_and_matching_controls() {
    let mut p = project(vec![[0.2; 2]; 4]);
    let aux = p.add_bus("Return", BusKind::Aux);
    p.tracks[0].routing.sends = vec![send(aux, 0.5, true)];
    let mut graph = p.routing_graph().unwrap();
    let limits = graph::GraphLimits::default();
    assert!(graph::CompiledGraph::compile_with_buses(&graph, 1, 1, limits).is_err());
    assert!(
        graph::CompiledGraph::compile_with_sends_at_rate(&graph, 1, 1, 1, limits, 48000).is_ok()
    );
    graph
        .nodes
        .iter_mut()
        .find(|node| matches!(node.processor, Processor::SendGain { .. }))
        .unwrap()
        .processor = Processor::SendGain { send: 1 };
    let error = Engine::with_graph(&p, Arc::new(Controls::new(&p)), 48000, 0, &graph, limits)
        .err()
        .unwrap();
    assert!(error.to_string().contains("Unknown send 1"));
    let mut controls = Controls::new(&p);
    controls.send_gains.clear();
    let error = Engine::try_new(&p, Arc::new(controls), 48000, 0)
        .err()
        .unwrap();
    assert!(error.to_string().contains("Send controls"));
}
#[test]
fn named_aux_and_group_buses_chain_and_keep_ids_when_reordered() {
    let mut p = project(vec![[0.4; 2]; 4]);
    let aux = p.add_bus("Aux", BusKind::Aux);
    let group = p.add_bus("Group", BusKind::Group);
    p.tracks[0].routing.output = Destination::Bus(aux);
    let a = p.bus_mut(aux).unwrap();
    a.routing
        .inserts
        .push(insert(InsertKind::Gain { gain: 0.5 }));
    a.gain = 0.5;
    a.routing.output = Destination::Bus(group);
    let g = p.bus_mut(group).unwrap();
    g.gain = 0.5;
    g.pan = -0.5;
    close(&render(&p, 1), &[0.05, 0.025]);
    p.buses.reverse();
    p.bus_mut(aux).unwrap().name = "Renamed".into();
    close(&render(&p, 1), &[0.05, 0.025]);
    assert_eq!(p.tracks[0].routing.output, Destination::Bus(aux));
}
#[test]
fn live_bus_controls_publish_independent_post_fader_meters() {
    let mut p = project(vec![[0.4, 0.2]; 16]);
    let id = p.add_bus("Group", BusKind::Group);
    p.tracks[0].routing.output = Destination::Bus(id);
    let controls = Arc::new(Controls::new(&p));
    let mut e = Engine::try_new(&p, controls.clone(), p.sample_rate, 0).unwrap();
    let bus = p.bus_mut(id).unwrap();
    bus.gain = 0.5;
    bus.pan = -0.5;
    controls.buses[0].set(bus);
    let mut out = [0.; 2];
    e.render(&mut out, 2);
    close(&out, &[0.2, 0.05]);
    assert_eq!(
        f32::from_bits(controls.buses[0].peak_left.swap(0, Ordering::Relaxed)),
        0.2
    );
    assert_eq!(
        f32::from_bits(controls.buses[0].peak_right.swap(0, Ordering::Relaxed)),
        0.05
    );
    assert_eq!(
        f32::from_bits(controls.buses[0].peak.load(Ordering::Relaxed)),
        0.2
    );
    controls.buses[0].mute.store(true, Ordering::Relaxed);
    e.render(&mut out, 2);
    assert_eq!(out, [0.; 2]);
    assert_eq!(controls.buses[0].peak_left.load(Ordering::Relaxed), 0);
    assert_eq!(controls.buses[0].peak_right.load(Ordering::Relaxed), 0);
}
#[test]
fn pre_fader_taps_gate_track_and_bus_tails_on_mute_and_track_solo() {
    let mut p = project(vec![[0.4; 2]; 16]);
    let aux = p.add_bus("Aux", BusKind::Aux);
    let sink = p.add_bus("Sink", BusKind::Aux);
    p.tracks[0].gain = 0.0;
    p.tracks[0]
        .routing
        .inserts
        .push(insert(InsertKind::OnePole { coefficient: 0.5 }));
    p.tracks[0].routing.sends.push(send(aux, 1., true));
    p.bus_mut(aux).unwrap().gain = 0.0;
    p.bus_mut(aux)
        .unwrap()
        .routing
        .inserts
        .push(insert(InsertKind::OnePole { coefficient: 0.5 }));
    p.bus_mut(aux)
        .unwrap()
        .routing
        .sends
        .push(send(sink, 1., true));
    let controls = Arc::new(Controls::new(&p));
    let mut e = Engine::try_new(&p, controls.clone(), p.sample_rate, 0).unwrap();
    let mut out = [0.; 2];
    e.render(&mut out, 2);
    close(&out, &[0.1, 0.1]);
    controls.buses[0].mute.store(true, Ordering::Relaxed);
    e.render(&mut out, 2);
    assert_eq!(out, [0.; 2]);
    // With the bus insert bypassed, a muted track's filter tail is also silent.
    p.bus_mut(aux).unwrap().routing.inserts.clear();
    let controls = Arc::new(Controls::new(&p));
    let mut e = Engine::try_new(&p, controls.clone(), p.sample_rate, 0).unwrap();
    e.render(&mut out, 2);
    controls.tracks[0].mute.store(true, Ordering::Relaxed);
    e.render(&mut out, 2);
    assert_eq!(out, [0.; 2]);
    // Soloing another track gates the original source and both of its taps.
    let mut silent = p.tracks[0].clone();
    silent.clips.clear();
    silent.solo = true;
    p.tracks.push(silent);
    assert_eq!(render(&p, 2), [0.; 4]);
}
#[test]
fn shared_stateful_insert_is_processed_once_for_all_outputs_and_sends() {
    let mut p = project(vec![[0.2; 2]; 512]);
    let a = p.add_bus("A", BusKind::Aux);
    let b = p.add_bus("B", BusKind::Aux);
    p.tracks[0]
        .routing
        .inserts
        .push(insert(InsertKind::OnePole { coefficient: 0.5 }));
    p.tracks[0].routing.sends = vec![send(a, 0.5, true), send(b, 0.25, false)];
    let graph = p.routing_graph().unwrap();
    let id = graph
        .nodes
        .iter()
        .find(|node| matches!(node.processor, Processor::OnePole { .. }))
        .unwrap()
        .id;
    let controls = Arc::new(Controls::new(&p));
    let mut e = Engine::try_new(&p, controls.clone(), p.sample_rate, 0).unwrap();
    let mut out = [0.; 512];
    e.render(&mut out, 2);
    close(&out[..4], &[0.175, 0.175, 0.2625, 0.2625]);
    assert_eq!(e.node_process_count(id), Some(2));
    assert_eq!(e.render_counts().nodes, e.graph_info().nodes as u64 * 2);
    assert_eq!(e.render_counts().edges, e.graph_info().edges as u64 * 2);
    controls.send_gains[0].store(0.25f32.to_bits(), Ordering::Relaxed);
    controls.send_gains[1].store(0.5f32.to_bits(), Ordering::Relaxed);
    e.render(&mut out, 2);
    close(&out[..2], &[0.35, 0.35]);
    assert_eq!(e.node_process_count(id), Some(4));
    assert_eq!(e.render_counts().nodes, e.graph_info().nodes as u64 * 4);
    assert_eq!(e.render_counts().edges, e.graph_info().edges as u64 * 4);
}
#[test]
fn delete_bus_repairs_outputs_and_enabled_or_disabled_sends_everywhere() {
    let mut p = project(vec![[0.2; 2]; 4]);
    let a = p.add_bus("A", BusKind::Group);
    let b = p.add_bus("B", BusKind::Aux);
    p.tracks[0].routing.output = Destination::Bus(a);
    p.tracks[0].routing.sends.push(send(a, 0.5, false));
    p.tracks[0].routing.sends[0].enabled = false;
    p.bus_mut(b).unwrap().routing.output = Destination::Bus(a);
    p.bus_mut(b).unwrap().routing.sends.push(send(a, 0.1, true));
    assert!(p.remove_bus(a));
    assert!(!p.remove_bus(a));
    assert_eq!(p.tracks[0].routing.output, Destination::Master);
    assert!(p.tracks[0].routing.sends.is_empty());
    assert_eq!(p.bus(b).unwrap().routing.output, Destination::Master);
    assert!(p.bus(b).unwrap().routing.sends.is_empty());
    p.validate().unwrap();
    close(&render(&p, 1), &[0.2, 0.2]);
}
#[test]
fn validation_rejects_dangling_duplicate_self_and_disabled_feedback() {
    let mut p = project(vec![[0.2; 2]; 4]);
    p.tracks[0].routing.output = Destination::Bus(BusId(42));
    assert!(p.validate().is_err());
    assert!(Engine::try_new(&p, Arc::new(Controls::new(&p)), p.sample_rate, 0).is_err());
    p.tracks[0].routing.output = Destination::Master;
    p.tracks[0].routing.sends.push(send(BusId(42), 0.0, true));
    p.tracks[0].routing.sends[0].enabled = false;
    assert!(p.validate_routing().is_err());
    p.tracks[0].routing.sends.clear();
    let a = p.add_bus("A", BusKind::Aux);
    let b = p.add_bus("B", BusKind::Group);
    p.bus_mut(a).unwrap().routing.output = Destination::Bus(b);
    p.bus_mut(b)
        .unwrap()
        .routing
        .sends
        .push(send(a, 0.0, false));
    p.bus_mut(b).unwrap().routing.sends[0].enabled = false;
    p.bus_mut(b)
        .unwrap()
        .routing
        .inserts
        .push(insert(InsertKind::Delay { frames: 1 }));
    assert!(
        p.validate_routing()
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
    p.bus_mut(b).unwrap().routing.sends.clear();
    p.bus_mut(a).unwrap().routing.output = Destination::Bus(a);
    assert!(p.validate_routing().is_err());
    p.bus_mut(a).unwrap().routing.output = Destination::Master;
    p.buses[1].id = a;
    assert!(
        p.validate_routing()
            .unwrap_err()
            .to_string()
            .contains("Duplicate")
    );
}
#[test]
fn bypassed_parameters_and_bus_mixer_values_still_validate() {
    let mut p = project(vec![[0.2; 2]; 4]);
    for kind in [
        InsertKind::Gain { gain: f32::NAN },
        InsertKind::OnePole { coefficient: 1.0 },
        InsertKind::Delay { frames: 0 },
        InsertKind::Delay { frames: usize::MAX },
    ] {
        p.tracks[0].routing.inserts = vec![Insert { kind, bypass: true }];
        assert!(p.validate_routing().is_err());
    }
    p.tracks[0].routing.inserts.clear();
    let id = p.add_bus("Invalid", BusKind::Aux);
    p.bus_mut(id).unwrap().gain = 3.0;
    assert!(p.validate_routing().is_err());
    p.bus_mut(id).unwrap().gain = 1.0;
    p.bus_mut(id).unwrap().pan = f32::NAN;
    assert!(p.validate_routing().is_err());
}
#[test]
fn routing_roundtrip_legacy_defaults_and_export_use_identical_audio() {
    let mut p = project(vec![[0.4; 2], [0.; 2], [0.; 2], [0.; 2]]);
    let id = p.add_bus("Return", BusKind::Aux);
    p.tracks[0].routing.output = Destination::Bus(id);
    p.tracks[0]
        .routing
        .inserts
        .push(insert(InsertKind::Gain { gain: 0.5 }));
    p.bus_mut(id)
        .unwrap()
        .routing
        .inserts
        .push(insert(InsertKind::OnePole { coefficient: 0.5 }));
    let group = p.add_bus("Group", BusKind::Group);
    p.tracks[0].routing.sends.push(Send {
        target: group,
        gain: 0.25,
        pre_fader: true,
        enabled: false,
    });
    p.tracks[0].routing.inserts.push(Insert {
        kind: InsertKind::Gain { gain: 100. },
        bypass: true,
    });
    p.bus_mut(id)
        .unwrap()
        .routing
        .sends
        .push(send(group, 0.125, false));
    let path = std::env::temp_dir().join(format!(
        "resonara-routing-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    p.save(&path.join("session.json")).unwrap();
    let restored = Project::load(&path.join("session.json")).unwrap();
    assert_eq!(restored.buses, p.buses);
    assert_eq!(restored.tracks[0].routing, p.tracks[0].routing);
    close(&render(&restored, 4), &render(&p, 4));
    restored.export_wav(&path.join("mix.wav")).unwrap();
    let exported: Vec<f32> = hound::WavReader::open(path.join("mix.wav"))
        .unwrap()
        .samples::<f32>()
        .map(|sample| sample.unwrap())
        .collect();
    close(&exported, &render(&p, 4));
    let mut legacy = serde_json::to_value(&p).unwrap();
    legacy.as_object_mut().unwrap().remove("buses");
    for track in legacy["tracks"].as_array_mut().unwrap() {
        track.as_object_mut().unwrap().remove("routing");
    }
    let legacy: Project = serde_json::from_value(legacy).unwrap();
    legacy.validate().unwrap();
    assert!(legacy.buses.is_empty());
    assert_eq!(legacy.tracks[0].routing, ChannelRouting::default());
    close(&render(&legacy, 4), &[0.4, 0.4, 0., 0., 0., 0., 0., 0.]);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn shared_bus_insert_runs_once_and_legacy_runtime_override_remains_available() {
    let mut p = project(vec![[0.2; 2]; 256]);
    let a = p.add_bus("Shared", BusKind::Group);
    let b = p.add_bus("Return", BusKind::Aux);
    p.tracks[0].routing.output = Destination::Bus(a);
    p.bus_mut(a)
        .unwrap()
        .routing
        .inserts
        .push(insert(InsertKind::OnePole { coefficient: 0.5 }));
    p.bus_mut(a).unwrap().routing.sends.push(send(b, 0.5, true));
    let graph = p.routing_graph().unwrap();
    let id = graph
        .nodes
        .iter()
        .find(|node| matches!(node.processor, Processor::OnePole { .. }))
        .unwrap()
        .id;
    let mut engine = Engine::try_new(&p, Arc::new(Controls::new(&p)), p.sample_rate, 0).unwrap();
    let mut out = [0.; 512];
    engine.render(&mut out, 2);
    close(&out[..4], &[0.15, 0.15, 0.225, 0.225]);
    assert_eq!(engine.node_process_count(id), Some(2));
    let legacy = graph::RoutingGraph::tracks_to_master(p.tracks.len());
    let mut engine = Engine::with_graph(
        &p,
        Arc::new(Controls::new(&p)),
        p.sample_rate,
        0,
        &legacy,
        graph::GraphLimits::default(),
    )
    .unwrap();
    engine.render(&mut out[..2], 2);
    close(&out[..2], &[0.2, 0.2]);
}

#[test]
fn invalid_preparation_reports_delay_budget_without_allocating_the_delay() {
    let mut p = project(vec![[0.2; 2]; 2]);
    p.tracks[0].routing.inserts.push(insert(InsertKind::Delay {
        frames: graph::GraphLimits::default().max_delay_bytes / 8 + 1,
    }));
    assert!(p.validate_routing().is_err());
    let error = Engine::try_new(&p, Arc::new(Controls::new(&p)), p.sample_rate, 0)
        .err()
        .unwrap();
    assert!(error.to_string().contains("Delay"));
    p.tracks[0].routing.inserts[0].bypass = true;
    assert!(Engine::try_new(&p, Arc::new(Controls::new(&p)), p.sample_rate, 0).is_err());
}

#[test]
fn saved_routing_size_is_bounded_even_when_inserts_are_bypassed() {
    let mut p = project(vec![[0.2; 2]; 2]);
    p.tracks[0].routing.inserts = vec![
        Insert {
            kind: InsertKind::Gain { gain: 1. },
            bypass: true
        };
        graph::GraphLimits::default().max_nodes
    ];
    assert!(
        p.validate_routing()
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
    assert!(p.routing_graph().is_err());
    p.tracks[0].routing.inserts.clear();
    // Scalar/mixer validation does not need a DSP buffer for large valid delays.
    p.tracks[0].routing.inserts.push(insert(InsertKind::Delay {
        frames: graph::GraphLimits::default().max_delay_bytes / 8,
    }));
    p.validate_routing().unwrap();
}

#[test]
fn saved_routing_budget_counts_enabled_and_disabled_send_gain_nodes() {
    let mut p = project(vec![[0.2; 2]; 2]);
    let aux = p.add_bus("Return", BusKind::Aux);
    // One track and one bus need five base nodes. Every send needs its own
    // gain operation, including currently disabled slots in the saved budget.
    let count = graph::GraphLimits::default().max_nodes - 5;
    p.tracks[0].routing.sends = vec![send(aux, 0.5, false); count];
    p.validate_routing().unwrap();
    let graph = p.routing_graph().unwrap();
    assert_eq!(graph.nodes.len(), graph::GraphLimits::default().max_nodes);
    assert_eq!(graph.routes.len(), 4 + count * 2);
    p.tracks[0].routing.sends.push(send(aux, 0.5, false));
    for enabled in [true, false] {
        for send in &mut p.tracks[0].routing.sends {
            send.enabled = enabled;
        }
        assert!(
            p.validate_routing()
                .unwrap_err()
                .to_string()
                .contains("budget")
        );
    }
}
