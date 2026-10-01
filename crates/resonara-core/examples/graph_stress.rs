//! Deterministic graph scaling benchmark, not a physical-device latency test.
//! cargo run --release -p resonara-core --example graph_stress
use resonara_core::{graph::*, *};
use std::{sync::Arc, time::Instant};

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
fn main() -> Result<()> {
    const FRAMES: usize = 32768;
    const QUANTUM: usize = 128;
    let source = Arc::new(vec![[0.125, -0.0625]; FRAMES]);
    let mut results = Vec::new();
    for tracks in [8, 64, 256, 1024] {
        let mut project = Project {
            master: 1.0,
            ..Project::default()
        };
        let aux_start = tracks as u64 * 4;
        let master = aux_start + 8;
        let mut graph = RoutingGraph {
            nodes: vec![],
            routes: vec![],
            output: NodeId(master),
        };
        for track in 0..tracks {
            project.tracks.push(Track {
                name: format!("Track {track}"),
                clips: (0..4)
                    .map(|part| Clip {
                        source_channels: 2,
                        start: (part * FRAMES / 4) as u64,
                        source_offset: part * FRAMES / 4,
                        frames: FRAMES / 4,
                        samples: source.clone(),
                    })
                    .collect(),
                gain: 0.5 / tracks as f32,
                pan: (track % 7) as f32 / 6.0 - 0.5,
                mute: false,
                solo: false,
            });
            let base = track as u64 * 4;
            graph.nodes.extend([
                node(base, Processor::TrackSource { track }),
                node(base + 1, Processor::OnePole { coefficient: 0.25 }),
                node(base + 2, Processor::TrackFader { track }),
            ]);
            graph.routes.extend([
                route(base, base + 1, 1.0),
                route(base + 1, base + 2, 1.0),
                route(base + 2, master, 1.0),
                route(
                    base + 1,
                    aux_start + (track % 4) as u64 * 2,
                    0.25 / tracks as f32,
                ),
                route(base + 2, aux_start + ((track + 1) % 4) as u64 * 2, 0.25),
            ]);
        }
        for aux in 0..4 {
            let id = aux_start + aux * 2;
            graph.nodes.extend([
                node(id, Processor::Bus),
                node(id + 1, Processor::OnePole { coefficient: 0.5 }),
            ]);
            graph
                .routes
                .extend([route(id, id + 1, 1.0), route(id + 1, master, 1.0)]);
        }
        graph.nodes.push(node(master, Processor::Bus));
        let begin = Instant::now();
        let mut engine = Engine::with_graph(
            &project,
            Arc::new(Controls::new(&project)),
            48000,
            0,
            &graph,
            GraphLimits {
                quantum: QUANTUM,
                ..GraphLimits::default()
            },
        )?;
        let compile_us = begin.elapsed().as_secs_f64() * 1e6;
        let mut timings = Vec::with_capacity(FRAMES / QUANTUM);
        let mut block = [0.0f32; QUANTUM * 2];
        let mut first_sample = [0.0; 2];
        let mut peak = 0.0f32;
        for index in 0..FRAMES / QUANTUM {
            let begin = Instant::now();
            engine.render(&mut block, 2);
            timings.push(begin.elapsed().as_secs_f64() * 1e6);
            if index == 0 {
                first_sample.copy_from_slice(&block[..2]);
            }
            for sample in block {
                assert!(sample.is_finite());
                peak = peak.max(sample.abs());
            }
        }
        assert!(peak > 0.0 && peak < 1.0);
        let counts = engine.render_counts();
        assert_eq!(counts.quanta, (FRAMES / QUANTUM) as u64);
        assert_eq!(counts.nodes, counts.quanta * graph.nodes.len() as u64);
        assert_eq!(counts.edges, counts.quanta * graph.routes.len() as u64);
        for node in &graph.nodes {
            assert_eq!(engine.node_process_count(node.id), Some(counts.quanta));
        }
        // Independent first-frame oracle. The source insert has one previous
        // state of zero, then each aux insert also has zero previous state.
        let mut direct = [0.0f32; 2];
        let mut auxes = [[0.0f32; 2]; 4];
        for (track, strip) in project.tracks.iter().enumerate() {
            let pre = [0.125 * 0.75, -0.0625 * 0.75];
            let post = [
                pre[0] * strip.gain * (1.0 - strip.pan.max(0.0)),
                pre[1] * strip.gain * (1.0 + strip.pan.min(0.0)),
            ];
            for ch in 0..2 {
                direct[ch] += post[ch];
                auxes[track % 4][ch] += pre[ch] * (0.25 / tracks as f32);
                auxes[(track + 1) % 4][ch] += post[ch] * 0.25;
            }
        }
        for aux in auxes {
            for ch in 0..2 {
                direct[ch] += aux[ch] * 0.5;
            }
        }
        for ch in 0..2 {
            assert!(
                (direct[ch] - first_sample[ch]).abs() < 2e-6,
                "oracle mismatch"
            );
        }
        timings.sort_by(f64::total_cmp);
        let percentile =
            |fraction: f64| timings[((timings.len() - 1) as f64 * fraction).ceil() as usize];
        let info = engine.graph_info();
        results.push(serde_json::json!({
            "tracks": tracks, "clips_per_track": 4, "source_buffers": 1,
            "aux_buses": 4, "sends_per_track": 2, "stateful_inserts": tracks + 4,
            "scheduled_nodes": info.nodes, "scheduled_edges": info.edges,
            "quantum_frames": info.quantum, "scratch_bytes": info.scratch_bytes,
            "buffer_slots": info.buffer_slots, "compile_and_prepare_us": compile_us,
            "render_p50_us": percentile(0.5), "render_p95_us": percentile(0.95),
            "render_p99_us": percentile(0.99), "render_max_us": timings.last(),
            "quanta_over_deadline": timings.iter().filter(|&&time| time > QUANTUM as f64 / 48000.0 * 1e6).count(),
            "quantum_deadline_us_at_48khz": QUANTUM as f64 / 48000.0 * 1e6,
            "quanta": counts.quanta, "node_calls": counts.nodes, "edge_mixes": counts.edges,
            "first_frame_oracle_passed": true, "every_node_once_per_quantum": true, "peak": peak,
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
            "limits": "Offline synthetic graph benchmark on shared cloud CPU. No device latency, dropout or hard-real-time guarantee. Scratch excludes project assets and plan metadata.",
            "results": results,
        }))?
    );
    Ok(())
}
