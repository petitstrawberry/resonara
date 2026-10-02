//! Persistent channel routing, validated without allocating DSP or audio buffers.
//! Structural edits are applied by preparing a new Engine outside the callback.
use crate::{
    ClapInsert, Project, Result,
    graph::{Node, NodeId, Processor, Route, RoutingGraph},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Stable session identity, independent of bus order or display name.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct BusId(pub u64);

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum BusKind {
    Aux,
    Group,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Destination {
    #[default]
    Master,
    Bus(BusId),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum InsertKind {
    Clap {
        plugin: ClapInsert,
    },
    Gain {
        gain: f32,
    },
    OnePole {
        coefficient: f32,
    },
    /// Device-rate frames, matching the runtime graph's delay processor.
    Delay {
        frames: usize,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Insert {
    pub kind: InsertKind,
    #[serde(default)]
    pub bypass: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Send {
    pub target: BusId,
    pub gain: f32,
    #[serde(default)]
    pub pre_fader: bool,
    #[serde(default = "enabled")]
    pub enabled: bool,
}
fn enabled() -> bool {
    true
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ChannelRouting {
    #[serde(default)]
    pub inserts: Vec<Insert>,
    #[serde(default)]
    pub output: Destination,
    #[serde(default)]
    pub sends: Vec<Send>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Bus {
    pub id: BusId,
    pub name: String,
    pub kind: BusKind,
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    #[serde(default)]
    pub routing: ChannelRouting,
}

impl Project {
    /// Index into `Controls::send_gains` for this prepared routing snapshot.
    /// Select exactly one track or bus. Disabled sends retain their slots;
    /// adding, removing, or reordering channels/sends requires new controls.
    pub fn send_control_index(
        &self,
        track: Option<usize>,
        bus: Option<BusId>,
        slot: usize,
    ) -> Option<usize> {
        let channel = match (track, bus) {
            (Some(track), None) if track < self.tracks.len() => track,
            (None, Some(bus)) => self
                .tracks
                .len()
                .checked_add(self.buses.iter().position(|item| item.id == bus)?)?,
            _ => return None,
        };
        let mut offset = 0usize;
        for (index, routing) in self.channel_routings().enumerate() {
            if index == channel {
                return (slot < routing.sends.len())
                    .then(|| offset.checked_add(slot))
                    .flatten();
            }
            offset = offset.checked_add(routing.sends.len())?;
        }
        None
    }
    pub(crate) fn channel_routings(&self) -> impl Iterator<Item = &ChannelRouting> {
        self.tracks
            .iter()
            .map(|track| &track.routing)
            .chain(self.buses.iter().map(|bus| &bus.routing))
    }
    /// IDs remain stable when buses are renamed, reordered, or saved/loaded.
    pub fn add_bus(&mut self, name: impl Into<String>, kind: BusKind) -> BusId {
        let mut candidate = self
            .buses
            .iter()
            .map(|bus| bus.id.0)
            .max()
            .unwrap_or(0)
            .wrapping_add(1);
        // The wraparound case is unlikely, but loading u64::MAX must still work.
        while self.buses.iter().any(|bus| bus.id.0 == candidate) {
            candidate = candidate.wrapping_add(1);
        }
        let id = BusId(candidate);
        self.buses.push(Bus {
            id,
            name: name.into(),
            kind,
            gain: 1.0,
            pan: 0.0,
            mute: false,
            routing: ChannelRouting::default(),
        });
        id
    }
    pub fn bus(&self, id: BusId) -> Option<&Bus> {
        self.buses.iter().find(|bus| bus.id == id)
    }
    pub fn bus_mut(&mut self, id: BusId) -> Option<&mut Bus> {
        self.buses.iter_mut().find(|bus| bus.id == id)
    }
    /// Remove a bus, reroute its dependents to master, and delete sends to it.
    pub fn remove_bus(&mut self, id: BusId) -> bool {
        let before = self.buses.len();
        self.buses.retain(|bus| bus.id != id);
        if before == self.buses.len() {
            return false;
        }
        for routing in self
            .tracks
            .iter_mut()
            .map(|track| &mut track.routing)
            .chain(self.buses.iter_mut().map(|bus| &mut bus.routing))
        {
            if routing.output == Destination::Bus(id) {
                routing.output = Destination::Master;
            }
            routing.sends.retain(|send| send.target != id);
        }
        true
    }
    /// Keep large legacy flat sessions usable without letting new routing data
    /// automatically increase its own admission budget.
    pub(crate) fn routing_limits(&self) -> crate::graph::GraphLimits {
        let defaults = crate::graph::GraphLimits::default();
        let flat_nodes = self.tracks.len().saturating_mul(2).saturating_add(1);
        crate::graph::GraphLimits {
            max_nodes: defaults.max_nodes.max(flat_nodes),
            max_edges: defaults.max_edges.max(self.tracks.len().saturating_mul(2)),
            max_scratch_bytes: defaults.max_scratch_bytes.max(
                flat_nodes
                    .saturating_mul(defaults.quantum)
                    .saturating_mul(8),
            ),
            ..defaults
        }
    }
    /// Check saved topology size before constructing adjacency or graph metadata.
    /// Bypassed inserts/disabled sends count too, so saved edits stay safe to enable.
    fn validate_routing_budgets(&self) -> Result<()> {
        let limits = self.routing_limits();
        let channels = self
            .tracks
            .len()
            .checked_add(self.buses.len())
            .ok_or("Routing size overflow")?;
        let mut edges = channels.checked_mul(2).ok_or("Routing size overflow")?;
        let mut nodes = edges.checked_add(1).ok_or("Routing size overflow")?;
        let mut delay_bytes = 0usize;
        let mut plugin_state_bytes = 0usize;
        for routing in self
            .tracks
            .iter()
            .map(|track| &track.routing)
            .chain(self.buses.iter().map(|bus| &bus.routing))
        {
            let gate = usize::from(routing.sends.iter().any(|send| send.pre_fader));
            nodes = nodes
                .checked_add(routing.inserts.len())
                .and_then(|count| count.checked_add(gate))
                .and_then(|count| count.checked_add(routing.sends.len()))
                .ok_or("Routing size overflow")?;
            edges = edges
                .checked_add(routing.inserts.len())
                .and_then(|count| count.checked_add(gate))
                .and_then(|count| count.checked_add(routing.sends.len()))
                .and_then(|count| count.checked_add(routing.sends.len()))
                .ok_or("Routing size overflow")?;
            for insert in &routing.inserts {
                if let InsertKind::Clap { ref plugin } = insert.kind {
                    plugin_state_bytes = plugin_state_bytes
                        .checked_add(plugin.state.len())
                        .ok_or("CLAP state size overflow")?;
                    if plugin_state_bytes > crate::plugins::MAX_PROJECT_STATE_BYTES {
                        return Err("Project CLAP state storage budget exceeded".into());
                    }
                }
                if let InsertKind::Delay { frames } = insert.kind {
                    delay_bytes = frames
                        .checked_mul(std::mem::size_of::<[f32; 2]>())
                        .and_then(|bytes| delay_bytes.checked_add(bytes))
                        .ok_or("Delay size overflow")?;
                    if delay_bytes > limits.max_delay_bytes {
                        return Err("Delay storage budget exceeded".into());
                    }
                }
            }
        }
        if nodes > limits.max_nodes || edges > limits.max_edges {
            return Err("Routing node/route budget exceeded".into());
        }
        Ok(())
    }
    /// Structural/parameter checks only: no clip scans, graph compilation, or DSP allocation.
    /// Disabled sends and bypassed inserts remain valid editable parts of the session.
    pub fn validate_routing(&self) -> Result<()> {
        self.validate_routing_budgets()?;
        let mut ids = BTreeMap::new();
        for (index, bus) in self.buses.iter().enumerate() {
            if ids.insert(bus.id, index).is_some() {
                return Err(format!("Duplicate bus ID {}", bus.id.0).into());
            }
            if !bus.gain.is_finite()
                || !(0.0..=2.0).contains(&bus.gain)
                || !bus.pan.is_finite()
                || !(-1.0..=1.0).contains(&bus.pan)
            {
                return Err(format!("Invalid mixer value for bus {}", bus.name).into());
            }
        }
        for routing in self
            .tracks
            .iter()
            .map(|track| &track.routing)
            .chain(self.buses.iter().map(|bus| &bus.routing))
        {
            for insert in &routing.inserts {
                match insert.kind {
                    InsertKind::Clap { ref plugin } => plugin.validate()?,
                    InsertKind::Gain { gain } if !gain.is_finite() => {
                        return Err("Insert gain must be finite".into());
                    }
                    InsertKind::OnePole { coefficient }
                        if !coefficient.is_finite() || !(0.0..1.0).contains(&coefficient) =>
                    {
                        return Err("One-pole coefficient must be in [0, 1)".into());
                    }
                    InsertKind::Delay { frames: 0 } => {
                        return Err("Delay must contain at least one frame".into());
                    }
                    InsertKind::Delay { frames } if frames.checked_mul(8).is_none() => {
                        return Err("Delay size overflow".into());
                    }
                    _ => {}
                }
            }
            if let Destination::Bus(id) = routing.output {
                if !ids.contains_key(&id) {
                    return Err(format!("Unknown output bus {}", id.0).into());
                }
            }
            for send in &routing.sends {
                if !ids.contains_key(&send.target) {
                    return Err(format!("Unknown send bus {}", send.target.0).into());
                }
                if !send.gain.is_finite() {
                    return Err("Send gain must be finite".into());
                }
            }
        }
        // All output/send choices participate, even disabled or zero-gain sends.
        // This keeps enabling a send or raising its gain from revealing feedback.
        let mut outgoing = vec![Vec::new(); self.buses.len()];
        let mut indegree = vec![0usize; self.buses.len()];
        for (index, bus) in self.buses.iter().enumerate() {
            let output = match bus.routing.output {
                Destination::Master => None,
                Destination::Bus(id) => Some(id),
            };
            for id in output
                .into_iter()
                .chain(bus.routing.sends.iter().map(|send| send.target))
            {
                let to = ids[&id];
                outgoing[index].push(to);
                indegree[to] += 1;
            }
        }
        let mut ready: Vec<_> = indegree
            .iter()
            .enumerate()
            .filter_map(|(i, &degree)| (degree == 0).then_some(i))
            .collect();
        let mut visited = 0;
        while let Some(index) = ready.pop() {
            visited += 1;
            for &to in &outgoing[index] {
                indegree[to] -= 1;
                if indegree[to] == 0 {
                    ready.push(to);
                }
            }
        }
        if visited != self.buses.len() {
            return Err("Routing cycle: bus feedback, including disabled sends and delayed feedback, is not supported".into());
        }
        Ok(())
    }
    /// Lower the persistent session to one DAG. Inserts execute once regardless
    /// of fan-out; pre-fader sends tap after inserts, post-fader sends after pan.
    /// This only builds graph metadata. Engine setup compiles/allocates it once.
    pub fn routing_graph(&self) -> Result<RoutingGraph> {
        self.lower_routing_graph(false).map(|(graph, _)| graph)
    }
    pub(crate) fn routing_graph_with_insert_controls(
        &self,
    ) -> Result<(RoutingGraph, Vec<(NodeId, usize)>)> {
        self.lower_routing_graph(true)
    }
    fn lower_routing_graph(
        &self,
        live_bypass: bool,
    ) -> Result<(RoutingGraph, Vec<(NodeId, usize)>)> {
        self.validate_routing()?;
        let mut insert_controls = Vec::new();
        let mut insert_offset = 0;
        let mut graph = RoutingGraph {
            nodes: Vec::new(),
            routes: Vec::new(),
            output: NodeId(0),
        };
        // Reserve track source/fader pairs first for compatibility with the old
        // flat graph's IDs; bus source/fader pairs follow, then master/inserts.
        for track in 0..self.tracks.len() {
            graph.nodes.push(Node {
                id: NodeId(graph.nodes.len() as u64),
                processor: Processor::TrackSource { track },
            });
            graph.nodes.push(Node {
                id: NodeId(graph.nodes.len() as u64),
                processor: Processor::TrackFader { track },
            });
        }
        let mut bus_sources = BTreeMap::new();
        for (bus, channel) in self.buses.iter().enumerate() {
            let source = NodeId(graph.nodes.len() as u64);
            bus_sources.insert(channel.id, source);
            graph.nodes.push(Node {
                id: source,
                processor: Processor::BusGate { bus },
            });
            graph.nodes.push(Node {
                id: NodeId(graph.nodes.len() as u64),
                processor: Processor::BusFader { bus },
            });
        }
        graph.output = NodeId(graph.nodes.len() as u64);
        graph.nodes.push(Node {
            id: graph.output,
            processor: Processor::Bus,
        });
        let mut send_offset = 0;
        for (channel, routing) in self.channel_routings().enumerate() {
            let source = NodeId(channel as u64 * 2);
            let fader = NodeId(source.0 + 1);
            let mut tap = source;
            for (slot, insert) in routing.inserts.iter().enumerate() {
                // Live bypass keeps every processor (and its GUI) allocated.
                // Static/offline routing can still omit disabled inserts.
                if insert.bypass && !live_bypass {
                    continue;
                }
                let id = NodeId(graph.nodes.len() as u64);
                insert_controls.push((id, insert_offset + slot));
                let processor = match insert.kind {
                    InsertKind::Clap { ref plugin } => Processor::Clap {
                        plugin: plugin.clone(),
                    },
                    InsertKind::Gain { gain } => Processor::Gain { gain },
                    InsertKind::OnePole { coefficient } => Processor::OnePole { coefficient },
                    InsertKind::Delay { frames } => Processor::Delay { frames },
                };
                graph.nodes.push(Node { id, processor });
                graph.routes.push(Route {
                    from: tap,
                    to: id,
                    gain: 1.0,
                });
                tap = id;
            }
            // A gated pre-fader tap prevents insert tails escaping a muted strip.
            if routing
                .sends
                .iter()
                .any(|send| send.enabled && send.pre_fader)
            {
                let id = NodeId(graph.nodes.len() as u64);
                let processor = if channel < self.tracks.len() {
                    Processor::TrackGate { track: channel }
                } else {
                    Processor::BusGate {
                        bus: channel - self.tracks.len(),
                    }
                };
                graph.nodes.push(Node { id, processor });
                graph.routes.push(Route {
                    from: tap,
                    to: id,
                    gain: 1.0,
                });
                tap = id;
            }
            graph.routes.push(Route {
                from: tap,
                to: fader,
                gain: 1.0,
            });
            let to = match routing.output {
                Destination::Master => graph.output,
                Destination::Bus(id) => bus_sources[&id],
            };
            graph.routes.push(Route {
                from: fader,
                to,
                gain: 1.0,
            });
            for (slot, send) in routing.sends.iter().enumerate() {
                if !send.enabled {
                    continue;
                }
                let id = NodeId(graph.nodes.len() as u64);
                graph.nodes.push(Node {
                    id,
                    processor: Processor::SendGain {
                        send: send_offset + slot,
                    },
                });
                graph.routes.push(Route {
                    from: if send.pre_fader { tap } else { fader },
                    to: id,
                    gain: 1.0,
                });
                graph.routes.push(Route {
                    from: id,
                    to: bus_sources[&send.target],
                    gain: 1.0,
                });
            }
            send_offset += routing.sends.len();
            insert_offset += routing.inserts.len();
        }
        Ok((graph, insert_controls))
    }
}
