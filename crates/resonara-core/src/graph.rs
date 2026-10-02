//! Off-thread compilation and bounded, single-pass stereo graph execution.
//!
//! Project routing lowers to this graph; the explicit runtime API remains supported.
//! Every reachable node executes once per nonempty render quantum. Structural
//! changes require stopping playback and preparing a new Engine. All cycles are
//! rejected, including cycles containing Delay (feedback is not implemented).
use crate::{ClapInsert, Result};
use resonara_clap::{PluginOwner, RealtimePlugin};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u64);

#[derive(Clone, Debug)]
pub enum Processor {
    /// External effect prepared on the control thread. Unknown identities are dry placeholders.
    Clap {
        plugin: ClapInsert,
    },
    /// The clip mix before the track fader. Mute/solo gate this source, including sends.
    TrackSource {
        track: usize,
    },
    /// Applies the referenced track's gain/pan and publishes its post-fader peak.
    TrackFader {
        track: usize,
    },
    /// Gates a post-insert pre-fader tap, including stored DSP tails.
    TrackGate {
        track: usize,
    },
    /// Sums inputs and gates a bus source/tap using its live mute control.
    BusGate {
        bus: usize,
    },
    /// Applies the referenced bus gain/pan and publishes its post-fader peak.
    BusFader {
        bus: usize,
    },
    /// Sums all incoming routes. Aux returns and the master can both use this.
    Bus,
    Gain {
        gain: f32,
    },
    /// Applies a send level sampled from Controls once per render quantum.
    SendGain {
        send: usize,
    },
    /// Feed-forward sample delay. This does not permit a cyclic graph.
    Delay {
        frames: usize,
    },
    /// Stateful test/useful basic insert: y = (1 - coefficient) * x + coefficient * previous.
    OnePole {
        coefficient: f32,
    },
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: NodeId,
    pub processor: Processor,
}
#[derive(Clone, Copy, Debug)]
pub struct Route {
    pub from: NodeId,
    pub to: NodeId,
    pub gain: f32,
}
#[derive(Clone, Debug)]
pub struct RoutingGraph {
    pub nodes: Vec<Node>,
    pub routes: Vec<Route>,
    pub output: NodeId,
}
impl RoutingGraph {
    /// The existing track -> fader -> master signal path, in stable track order.
    pub fn tracks_to_master(tracks: usize) -> Self {
        let output = NodeId(tracks as u64 * 2);
        let mut graph = Self {
            nodes: Vec::with_capacity(tracks * 2 + 1),
            routes: Vec::with_capacity(tracks * 2),
            output,
        };
        for track in 0..tracks {
            let source = NodeId(track as u64 * 2);
            let fader = NodeId(source.0 + 1);
            graph.nodes.push(Node {
                id: source,
                processor: Processor::TrackSource { track },
            });
            graph.nodes.push(Node {
                id: fader,
                processor: Processor::TrackFader { track },
            });
            graph.routes.push(Route {
                from: source,
                to: fader,
                gain: 1.0,
            });
            graph.routes.push(Route {
                from: fader,
                to: output,
                gain: 1.0,
            });
        }
        graph.nodes.push(Node {
            id: output,
            processor: Processor::Bus,
        });
        graph
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GraphLimits {
    /// Maximum internal block size; device callbacks are split without allocation.
    pub quantum: usize,
    pub max_nodes: usize,
    pub max_edges: usize,
    /// Audio arena and planar plug-in host buffers. Model/plan metadata and
    /// shared source assets are separate.
    pub max_scratch_bytes: usize,
    /// Persistent audio delay buffers, separate from the scratch arena.
    pub max_delay_bytes: usize,
}
impl Default for GraphLimits {
    fn default() -> Self {
        Self {
            quantum: 128,
            max_nodes: 4096,
            max_edges: 65536,
            max_scratch_bytes: 64 * 1024 * 1024,
            max_delay_bytes: 64 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Debug)]
pub struct GraphInfo {
    /// Reachable scheduled nodes/edges, after discarding disconnected DAGs.
    pub nodes: usize,
    pub edges: usize,
    pub quantum: usize,
    pub scratch_bytes: usize,
    pub delay_bytes: usize,
    pub buffer_slots: usize,
    /// Active reachable plug-ins that could not be loaded or activated.
    pub unavailable_plugins: u32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderCounts {
    pub quanta: u64,
    pub nodes: u64,
    pub edges: u64,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct BlockMixer {
    pub gain: f32,
    pub pan: f32,
    pub audible: bool,
    pub peak: [f32; 2],
}
struct Input {
    slot: usize,
    gain: f32,
}
enum RuntimeProcessor {
    Clap(RealtimePlugin),
    MissingClap,
    Plain(Processor),
    Delay {
        ring: Vec<[f32; 2]>,
        cursor: usize,
    },
    OnePole {
        coefficient: f32,
        previous: [f32; 2],
    },
}
struct Operation {
    id: NodeId,
    processor: RuntimeProcessor,
    inputs: Vec<Input>,
    slot: usize,
    calls: u64,
}

/// Owns its immutable schedule, scratch storage and audio-thread-exclusive DSP state.
/// Construction and destruction must happen outside the audio callback.
pub struct CompiledGraph {
    operations: Vec<Operation>,
    // Field drop order matters: retire audio handles before owner-thread destroy.
    plugin_owners: Vec<PluginOwner>,
    scratch: Vec<[f32; 2]>,
    output_slot: usize,
    info: GraphInfo,
    counts: RenderCounts,
}
impl CompiledGraph {
    /// Validate and prepare outside the audio thread. No runtime topology traversal.
    pub fn compile(graph: &RoutingGraph, tracks: usize, limits: GraphLimits) -> Result<Self> {
        Self::compile_with_buses(graph, tracks, 0, limits)
    }
    /// Session-aware admission, retaining `compile` for graphs without bus controls.
    pub fn compile_with_buses(
        graph: &RoutingGraph,
        tracks: usize,
        buses: usize,
        limits: GraphLimits,
    ) -> Result<Self> {
        Self::compile_with_buses_at_rate(graph, tracks, buses, limits, 48000)
    }
    /// Prepare external plug-ins for the actual device rate, off the audio thread.
    pub fn compile_with_buses_at_rate(
        graph: &RoutingGraph,
        tracks: usize,
        buses: usize,
        limits: GraphLimits,
        sample_rate: u32,
    ) -> Result<Self> {
        Self::compile_with_sends_at_rate(graph, tracks, buses, 0, limits, sample_rate)
    }
    /// Session-aware preparation with live send controls. The older compile
    /// entry points retain their fixed-route-gain behavior and accept no sends.
    pub fn compile_with_sends_at_rate(
        graph: &RoutingGraph,
        tracks: usize,
        buses: usize,
        sends: usize,
        limits: GraphLimits,
        sample_rate: u32,
    ) -> Result<Self> {
        if sample_rate == 0 {
            return Err("Device sample rate must be nonzero".into());
        }
        if limits.quantum == 0 || limits.quantum > 65536 {
            return Err("Graph quantum must be between 1 and 65536 frames".into());
        }
        if graph.nodes.len() > limits.max_nodes || graph.routes.len() > limits.max_edges {
            return Err("Graph node/route budget exceeded".into());
        }
        let mut ids = BTreeMap::new();
        let mut plugin_state_bytes = 0usize;
        for (index, node) in graph.nodes.iter().enumerate() {
            if ids.insert(node.id, index).is_some() {
                return Err(format!("Duplicate graph node {:?}", node.id).into());
            }
            match node.processor {
                Processor::Clap { ref plugin } => {
                    plugin.validate()?;
                    plugin_state_bytes = plugin_state_bytes
                        .checked_add(plugin.state.len())
                        .ok_or("CLAP state size overflow")?;
                    if plugin_state_bytes > crate::plugins::MAX_PROJECT_STATE_BYTES {
                        return Err("Graph CLAP state storage budget exceeded".into());
                    }
                }
                Processor::TrackSource { track }
                | Processor::TrackFader { track }
                | Processor::TrackGate { track }
                    if track >= tracks =>
                {
                    return Err(format!("Unknown track {track}").into());
                }
                Processor::BusGate { bus } | Processor::BusFader { bus } if bus >= buses => {
                    return Err(format!("Unknown bus {bus}").into());
                }
                Processor::SendGain { send } if send >= sends => {
                    return Err(format!("Unknown send {send}").into());
                }
                Processor::Gain { gain } if !gain.is_finite() => {
                    return Err("Insert gain must be finite".into());
                }
                Processor::Delay { frames: 0 } => {
                    return Err("Delay must contain at least one frame".into());
                }
                Processor::OnePole { coefficient }
                    if !coefficient.is_finite() || !(0.0..1.0).contains(&coefficient) =>
                {
                    return Err("One-pole coefficient must be in [0, 1)".into());
                }
                _ => {}
            }
        }
        let output = *ids.get(&graph.output).ok_or("Missing graph output")?;
        let mut incoming = vec![Vec::new(); graph.nodes.len()];
        let mut outgoing = vec![Vec::new(); graph.nodes.len()];
        let mut indegree = vec![0usize; graph.nodes.len()];
        for route in &graph.routes {
            let from = *ids.get(&route.from).ok_or("Unknown route source")?;
            let to = *ids.get(&route.to).ok_or("Unknown route destination")?;
            if !route.gain.is_finite() {
                return Err("Route gain must be finite".into());
            }
            if matches!(graph.nodes[to].processor, Processor::TrackSource { .. }) {
                return Err("Track sources cannot have input routes".into());
            }
            incoming[to].push((from, route.gain));
            outgoing[from].push(to);
            indegree[to] += 1;
        }
        // Stable NodeId ordering also fixes floating-point summation order,
        // independent of the storage order in nodes/routes.
        for inputs in &mut incoming {
            inputs.sort_by_key(|&(from, gain)| (graph.nodes[from].id, gain.to_bits()));
        }
        let mut ready = BinaryHeap::new();
        for (index, &degree) in indegree.iter().enumerate() {
            if degree == 0 {
                ready.push(Reverse((graph.nodes[index].id, index)));
            }
        }
        let mut order = Vec::with_capacity(graph.nodes.len());
        while let Some(Reverse((_, index))) = ready.pop() {
            order.push(index);
            for &to in &outgoing[index] {
                indegree[to] -= 1;
                if indegree[to] == 0 {
                    ready.push(Reverse((graph.nodes[to].id, to)));
                }
            }
        }
        // Validate even disconnected components so an invalid saved edit cannot
        // become a hidden feedback loop when reconnected later.
        if order.len() != graph.nodes.len() {
            let blocked: Vec<_> = ids
                .iter()
                .filter(|&(_, &index)| indegree[index] > 0)
                .map(|(&id, _)| id)
                .take(8)
                .collect();
            return Err(format!("Routing cycle blocks nodes {blocked:?}; feedback routes, including delayed feedback, are not supported").into());
        }
        let mut reachable = vec![false; graph.nodes.len()];
        let mut stack = vec![output];
        while let Some(index) = stack.pop() {
            if std::mem::replace(&mut reachable[index], true) {
                continue;
            }
            stack.extend(incoming[index].iter().map(|&(from, _)| from));
        }
        order.retain(|&index| reachable[index]);
        let mut last_use = vec![0; graph.nodes.len()];
        for (position, &index) in order.iter().enumerate() {
            last_use[index] = last_use[index].max(position);
            for &(from, _) in &incoming[index] {
                last_use[from] = position;
            }
        }
        last_use[output] = order.len();
        let mut release_after = vec![Vec::new(); order.len()];
        for &index in &order {
            if index != output {
                release_after[last_use[index]].push(index);
            }
        }
        let mut slots = vec![0; graph.nodes.len()];
        let mut free_slots = Vec::new();
        let mut buffer_slots = 0usize;
        for (position, &index) in order.iter().enumerate() {
            slots[index] = free_slots.pop().unwrap_or_else(|| {
                let slot = buffer_slots;
                buffer_slots += 1;
                slot
            });
            // Release only AFTER allocating/writing this operation's output;
            // its input slots must remain intact throughout the operation.
            for &expired in &release_after[position] {
                free_slots.push(slots[expired]);
            }
        }
        let scratch_frames = buffer_slots
            .checked_mul(limits.quantum)
            .ok_or("Scratch size overflow")?;
        let plugin_count = order
            .iter()
            .filter(|&&index| matches!(graph.nodes[index].processor, Processor::Clap { .. }))
            .count();
        let plugin_scratch_bytes = plugin_count
            .checked_mul(limits.quantum)
            .and_then(|frames| frames.checked_mul(4 * std::mem::size_of::<f32>()))
            .ok_or("CLAP scratch size overflow")?;
        let scratch_bytes = scratch_frames
            .checked_mul(std::mem::size_of::<[f32; 2]>())
            .and_then(|bytes| bytes.checked_add(plugin_scratch_bytes))
            .ok_or("Scratch size overflow")?;
        if scratch_bytes > limits.max_scratch_bytes {
            return Err("Graph scratch budget exceeded".into());
        }
        let mut delay_bytes = 0usize;
        for &index in &order {
            if let Processor::Delay { frames } = graph.nodes[index].processor {
                delay_bytes = frames
                    .checked_mul(std::mem::size_of::<[f32; 2]>())
                    .and_then(|bytes| delay_bytes.checked_add(bytes))
                    .ok_or("Delay size overflow")?;
                if delay_bytes > limits.max_delay_bytes {
                    return Err("Graph delay budget exceeded".into());
                }
            }
        }
        let edges = order.iter().map(|&index| incoming[index].len()).sum();
        let mut plugin_owners = Vec::with_capacity(plugin_count);
        let mut unavailable_plugins = 0u32;
        let operations = order
            .iter()
            .map(|&index| {
                let processor = match graph.nodes[index].processor {
                    Processor::Clap { ref plugin } => {
                        match crate::plugins::activate(plugin, sample_rate, limits.quantum) {
                            Ok((owner, processor)) => {
                                plugin_owners.push(owner);
                                RuntimeProcessor::Clap(processor)
                            }
                            Err(_) => {
                                unavailable_plugins = unavailable_plugins.saturating_add(1);
                                RuntimeProcessor::MissingClap
                            }
                        }
                    }
                    Processor::Delay { frames } => RuntimeProcessor::Delay {
                        ring: vec![[0.0; 2]; frames],
                        cursor: 0,
                    },
                    Processor::OnePole { coefficient } => RuntimeProcessor::OnePole {
                        coefficient,
                        previous: [0.0; 2],
                    },
                    ref processor => RuntimeProcessor::Plain(processor.clone()),
                };
                Operation {
                    id: graph.nodes[index].id,
                    processor,
                    inputs: incoming[index]
                        .iter()
                        .map(|&(from, gain)| Input {
                            slot: slots[from],
                            gain,
                        })
                        .collect(),
                    slot: slots[index],
                    calls: 0,
                }
            })
            .collect();
        Ok(Self {
            operations,
            plugin_owners,
            scratch: vec![[0.0; 2]; scratch_frames],
            output_slot: slots[output],
            info: GraphInfo {
                nodes: order.len(),
                edges,
                quantum: limits.quantum,
                scratch_bytes,
                delay_bytes,
                buffer_slots,
                unavailable_plugins,
            },
            counts: RenderCounts::default(),
        })
    }
    /// Keep these on the construction thread while transferring the render
    /// engine. Drop them only after its realtime handles have been retired.
    pub fn take_plugin_owners(&mut self) -> Vec<PluginOwner> {
        std::mem::take(&mut self.plugin_owners)
    }
    pub fn info(&self) -> &GraphInfo {
        &self.info
    }
    pub fn counts(&self) -> RenderCounts {
        self.counts
    }
    pub fn node_process_count(&self, id: NodeId) -> Option<u64> {
        self.operations
            .iter()
            .find(|op| op.id == id)
            .map(|op| op.calls)
    }
    pub(crate) fn process(
        &mut self,
        frames: usize,
        mixers: &mut [BlockMixer],
        buses: &mut [BlockMixer],
        send_gains: &[f32],
        mut source: impl FnMut(usize, &mut [[f32; 2]]),
    ) -> bool {
        debug_assert!(frames > 0 && frames <= self.info.quantum);
        let quantum = self.info.quantum;
        for op in &mut self.operations {
            let start = op.slot * quantum;
            self.scratch[start..start + frames].fill([0.0; 2]);
            for input in &op.inputs {
                let input_start = input.slot * quantum;
                debug_assert_ne!(input.slot, op.slot);
                // Split the arena once per edge, exposing disjoint contiguous
                // slices without unsafe aliasing or per-sample index checks.
                let (output, input_block) = if input_start < start {
                    let (before, after) = self.scratch.split_at_mut(start);
                    (
                        &mut after[..frames],
                        &before[input_start..input_start + frames],
                    )
                } else {
                    let (before, after) = self.scratch.split_at_mut(input_start);
                    (&mut before[start..start + frames], &after[..frames])
                };
                for (output, sample) in output.iter_mut().zip(input_block) {
                    output[0] += sample[0] * input.gain;
                    output[1] += sample[1] * input.gain;
                }
            }
            let block = &mut self.scratch[start..start + frames];
            match &mut op.processor {
                RuntimeProcessor::Clap(plugin) => {
                    if plugin.process(block).is_err() {
                        // No allocation, logging, unloading, or further plug-in
                        // calls after a failed process; the Engine latches it.
                        return false;
                    }
                }
                RuntimeProcessor::MissingClap => {}
                RuntimeProcessor::Plain(Processor::TrackSource { track }) => {
                    if mixers[*track].audible {
                        source(*track, block);
                    }
                }
                RuntimeProcessor::Plain(Processor::TrackGate { track }) => {
                    if !mixers[*track].audible {
                        block.fill([0.0; 2]);
                    }
                }
                RuntimeProcessor::Plain(Processor::BusGate { bus }) => {
                    if !buses[*bus].audible {
                        block.fill([0.0; 2]);
                    }
                }
                RuntimeProcessor::Plain(Processor::TrackFader { track }) => {
                    apply_fader(block, &mut mixers[*track]);
                }
                RuntimeProcessor::Plain(Processor::BusFader { bus }) => {
                    apply_fader(block, &mut buses[*bus]);
                }
                RuntimeProcessor::Plain(Processor::Gain { gain }) => {
                    for sample in block {
                        sample[0] *= *gain;
                        sample[1] *= *gain;
                    }
                }
                RuntimeProcessor::Plain(Processor::SendGain { send }) => {
                    let gain = send_gains[*send];
                    for sample in block {
                        sample[0] *= gain;
                        sample[1] *= gain;
                    }
                }
                RuntimeProcessor::Delay { ring, cursor } => {
                    for sample in block {
                        std::mem::swap(sample, &mut ring[*cursor]);
                        *cursor += 1;
                        if *cursor == ring.len() {
                            *cursor = 0;
                        }
                    }
                }
                RuntimeProcessor::OnePole {
                    coefficient,
                    previous,
                } => {
                    for sample in block {
                        for channel in 0..2 {
                            sample[channel] = (1.0 - *coefficient) * sample[channel]
                                + *coefficient * previous[channel];
                        }
                        *previous = *sample;
                    }
                }
                RuntimeProcessor::Plain(Processor::Bus) => {}
                RuntimeProcessor::Plain(
                    Processor::Delay { .. } | Processor::OnePole { .. } | Processor::Clap { .. },
                ) => {
                    unreachable!("stateful processor compiled as plain")
                }
            }
            op.calls = op.calls.saturating_add(1);
        }
        self.counts.quanta = self.counts.quanta.saturating_add(1);
        self.counts.nodes = self.counts.nodes.saturating_add(self.info.nodes as u64);
        self.counts.edges = self.counts.edges.saturating_add(self.info.edges as u64);
        true
    }
    pub(crate) fn output(&self, frames: usize) -> &[[f32; 2]] {
        let start = self.output_slot * self.info.quantum;
        &self.scratch[start..start + frames]
    }
}

fn apply_fader(block: &mut [[f32; 2]], mix: &mut BlockMixer) {
    if mix.audible {
        let left = mix.gain * (1.0 - mix.pan.max(0.0));
        let right = mix.gain * (1.0 + mix.pan.min(0.0));
        for sample in block {
            sample[0] *= left;
            sample[1] *= right;
            for (channel, value) in sample.iter().enumerate() {
                mix.peak[channel] = mix.peak[channel].max(value.abs().min(f32::MAX));
            }
        }
    } else {
        block.fill([0.0; 2]);
    }
}
