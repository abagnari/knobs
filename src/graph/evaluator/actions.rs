use std::collections::HashMap;
use std::fmt::Debug;

use egui::Context;

use crate::graph::node::port::PortId;
use crate::graph::node::{DataValidityState, EvaluationNode, NodeId};
use crate::{ConstantInputType, GenericStore, NodeBlueprintDefinition, PortTypeInfo};

#[allow(clippy::large_enum_variant)]
pub(crate) enum Command<P, D, K>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
    K: NodeBlueprintDefinition<P, D>,
{
    AddNode {
        node: EvaluationNode<P, D, K>,
    },
    DeleteNode {
        node_id: NodeId,
    },
    AddConnection {
        from: PortId,
        to: PortId,
    },
    DeleteConnection {
        to: PortId,
    },
    FocusNode {
        node_id: NodeId,
    },
    UnfocusNode,
    SetConstant {
        port_id: PortId,
        constant: ConstantInputType,
    },
    AttachContext {
        context: Context,
    },
    AddDefaultEntries {
        entries: GenericStore,
    },
    Reset,
    Shutdown,
}

impl<P, D, K> Command<P, D, K>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
    K: NodeBlueprintDefinition<P, D>,
{
    pub(crate) fn node_id(&self) -> Option<NodeId> {
        match self {
            Command::AddNode { node } => Some(node.id),
            Command::DeleteNode { node_id } => Some(*node_id),
            Command::AddConnection { from, .. } => Some(from.node_id()),
            Command::DeleteConnection { to, .. } => Some(to.node_id()),
            Command::FocusNode { node_id } => Some(*node_id),
            Command::UnfocusNode => None,
            Command::SetConstant { port_id, .. } => Some(port_id.node_id()),
            Command::Reset => None,
            Command::Shutdown => None,
            Command::AttachContext { .. } => None,
            Command::AddDefaultEntries { .. } => None,
        }
    }
}

pub(crate) enum Event {
    ValidityChanged {
        node_id: NodeId,
        validity: DataValidityState,
    },
    ResultReady {
        node_id: NodeId,
        data: GenericStore,
        outputs_text: HashMap<String, Option<String>>,
    },
    Failed {
        node_id: Option<NodeId>,
        error: anyhow::Error,
    },
}
