//! Saving and loading graphs.

use std::collections::HashMap;

use ron::ser::PrettyConfig;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::ConstantInputType;
use crate::graph::connection::Connection;
use crate::graph::node::NodeId;
use crate::graph::node::port::{OptionalFloat, OptionalInteger, PortId, port_id};

pub(crate) const SNAPSHOT_VERSION: u32 = 1;
const INDENT: &str = "  ";
const FIRST_SNAPSHOT_NODE_INDEX: u32 = 0;

/// A saved copy of a graph: nodes with their constants and positions,
/// connections and the canvas position.
///
/// Produced by [`Graph::snapshot`](crate::Graph::snapshot) and restored with
/// [`Graph::load`](crate::Graph::load). Node ids in a snapshot are local to it
/// and are replaced with fresh ids when it is loaded.
#[derive(Serialize, Deserialize)]
pub struct GraphSnapshot<K> {
    pub(crate) version: u32,
    pub(crate) nodes: Vec<NodeSnapshot<K>>,
    pub(crate) connections: Vec<ConnectionSnapshot>,
    pub(crate) canvas: CanvasSnapshot,
}

impl<K> GraphSnapshot<K>
where
    K: Serialize + DeserializeOwned,
{
    /// Serializes the snapshot as pretty printed RON.
    ///
    /// # Errors
    ///
    /// Returns an error when a blueprint fails to serialize.
    pub fn to_ron(&self) -> anyhow::Result<String> {
        let config = PrettyConfig::new()
            .struct_names(true)
            .indentor(INDENT.to_string());

        Ok(ron::ser::to_string_pretty(self, config)?)
    }

    /// Parses a snapshot from RON text.
    ///
    /// Parsing only checks the structure. Whether the snapshot fits the current
    /// blueprints is checked by [`Graph::load`](crate::Graph::load).
    ///
    /// # Errors
    ///
    /// Returns an error when `source` is not a valid snapshot, for instance
    /// because a blueprint name is unknown.
    pub fn from_ron(source: &str) -> anyhow::Result<Self> {
        Ok(ron::de::from_str(source)?)
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct CanvasSnapshot {
    pub(crate) pan: (f32, f32),
    pub(crate) zoom: f32,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, Hash, PartialEq)]
#[serde(rename = "NodeId")]
pub(crate) struct SnapshotNodeId {
    id: u32,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, Hash, PartialEq)]
#[serde(rename = "PortId")]
pub(crate) struct SnapshotPortId {
    node_id: SnapshotNodeId,
    id: u32,
}

impl SnapshotPortId {
    pub(crate) fn new(node_id: SnapshotNodeId, index: u32) -> Self {
        Self { node_id, id: index }
    }

    pub(crate) fn index(&self) -> u32 {
        self.id
    }
}

#[derive(Default)]
pub(crate) struct SnapshotIds {
    to_snapshot: HashMap<NodeId, SnapshotNodeId>,
    to_runtime: HashMap<SnapshotNodeId, NodeId>,
}

impl SnapshotIds {
    pub(crate) fn assign(nodes: impl IntoIterator<Item = NodeId>) -> Self {
        let mut ids = Self::default();

        for (index, node_id) in (FIRST_SNAPSHOT_NODE_INDEX..).zip(nodes) {
            ids.bind(SnapshotNodeId { id: index }, node_id);
        }

        ids
    }

    pub(crate) fn bind(&mut self, snapshot_id: SnapshotNodeId, node_id: NodeId) -> bool {
        if self.to_runtime.contains_key(&snapshot_id) {
            return false;
        }

        self.to_runtime.insert(snapshot_id, node_id);
        self.to_snapshot.insert(node_id, snapshot_id);

        true
    }

    pub(crate) fn snapshot_node(&self, node_id: NodeId) -> anyhow::Result<SnapshotNodeId> {
        self.to_snapshot
            .get(&node_id)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("Node {node_id:?} has no snapshot id"))
    }

    pub(crate) fn snapshot_port(&self, port: PortId) -> anyhow::Result<SnapshotPortId> {
        Ok(SnapshotPortId::new(
            self.snapshot_node(port.node_id())?,
            port.index(),
        ))
    }

    pub(crate) fn runtime_node(&self, snapshot_id: SnapshotNodeId) -> anyhow::Result<NodeId> {
        self.to_runtime
            .get(&snapshot_id)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("Unknown node in snapshot: {snapshot_id:?}"))
    }

    pub(crate) fn runtime_port(&self, snapshot_port: SnapshotPortId) -> anyhow::Result<PortId> {
        Ok(port_id(
            self.runtime_node(snapshot_port.node_id)?,
            snapshot_port.id,
        ))
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct ConnectionSnapshot {
    pub(crate) from: SnapshotPortId,
    pub(crate) to: SnapshotPortId,
}

impl ConnectionSnapshot {
    pub(crate) fn new(connection: &Connection, ids: &SnapshotIds) -> anyhow::Result<Self> {
        Ok(Self {
            from: ids.snapshot_port(connection.from_port)?,
            to: ids.snapshot_port(connection.to_port)?,
        })
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct NodeSnapshot<K> {
    pub(crate) id: SnapshotNodeId,
    pub(crate) blueprint: K,
    pub(crate) name: String,
    pub(crate) position: (f32, f32),
    pub(crate) ports: Vec<PortSnapshot>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct PortSnapshot {
    pub(crate) id: SnapshotPortId,
    pub(crate) name: String,
    pub(crate) constant: Option<ConstantValue>,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) enum ConstantValue {
    Float(f32),
    Integer(i32),
    UnsignedInteger(u32),
    Path(String),
    String(String),
    Selection(usize),
    OptionalFloat(Option<f32>),
    OptionalInteger(Option<i32>),
    Boolean(bool),
}

impl ConstantValue {
    pub(crate) fn apply_to(&self, target: &mut ConstantInputType) -> anyhow::Result<()> {
        match (target, self) {
            (
                ConstantInputType::Float {
                    value,
                    min_bound,
                    max_bound,
                    ..
                },
                Self::Float(saved),
            ) => {
                *value = saved.clamp(*min_bound, *max_bound);
            }
            (
                ConstantInputType::Integer {
                    value,
                    min_bound,
                    max_bound,
                    ..
                },
                Self::Integer(saved),
            ) => {
                *value = (*saved).clamp(*min_bound, *max_bound);
            }
            (ConstantInputType::Path(value), Self::Path(saved)) => {
                *value = saved.clone();
            }
            (ConstantInputType::String(value), Self::String(saved)) => {
                *value = saved.clone();
            }
            (ConstantInputType::Selection { options, selected }, Self::Selection(saved)) => {
                if *saved >= options.len() {
                    anyhow::bail!("Selection index {saved} out of range for port constant");
                }
                *selected = *saved;
            }
            (
                ConstantInputType::OptionalFloat {
                    value_option,
                    min_bound,
                    max_bound,
                },
                Self::OptionalFloat(saved),
            ) => {
                *value_option = OptionalFloat {
                    value: saved.unwrap_or(0.0).clamp(*min_bound, *max_bound),
                    enabled: saved.is_some(),
                };
            }
            (
                ConstantInputType::RandomizableUnsignedInteger {
                    value,
                    min_bound,
                    max_bound,
                },
                Self::UnsignedInteger(saved),
            ) => {
                *value = (*saved).clamp(*min_bound, *max_bound);
            }
            (
                ConstantInputType::OptionalInteger {
                    value_option,
                    min_bound,
                    max_bound,
                },
                Self::OptionalInteger(saved),
            ) => {
                *value_option = OptionalInteger {
                    value: saved.unwrap_or(0).clamp(*min_bound, *max_bound),
                    enabled: saved.is_some(),
                };
            }
            (ConstantInputType::Boolean { value, .. }, Self::Boolean(saved)) => {
                *value = *saved;
            }
            (target, saved) => {
                anyhow::bail!("Constant kind mismatch: {target:?} does not accept {saved:?}")
            }
        }

        Ok(())
    }
}

impl From<&ConstantInputType> for ConstantValue {
    fn from(value: &ConstantInputType) -> Self {
        match value {
            ConstantInputType::Float { value, .. } => Self::Float(*value),
            ConstantInputType::Integer { value, .. } => Self::Integer(*value),
            ConstantInputType::Path(value) => Self::Path(value.clone()),
            ConstantInputType::String(value) => Self::String(value.clone()),
            ConstantInputType::Selection { selected, .. } => Self::Selection(*selected),
            ConstantInputType::OptionalFloat {
                value_option: value,
                ..
            } => Self::OptionalFloat(value.get()),
            ConstantInputType::OptionalInteger {
                value_option: value,
                ..
            } => Self::OptionalInteger(value.get()),
            ConstantInputType::RandomizableUnsignedInteger { value, .. } => {
                Self::UnsignedInteger(*value)
            }
            ConstantInputType::Boolean { value, .. } => Self::Boolean(*value),
        }
    }
}
