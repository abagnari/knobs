pub(crate) mod actions;

use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvError, RecvTimeoutError, Sender};

use egui::Context;
use slotmap::SecondaryMap;

use crate::graph::GraphEventHandler;
use crate::graph::connection::Connections;
use crate::graph::evaluator::actions::{Command, Event};
use crate::graph::node::port::{PortDirection, PortId};
use crate::graph::node::threading::POLL_INTERVAL;
use crate::graph::node::{DataValidityState, EvaluationNode, InvalidationReason, NodeId};
use crate::helpers::compute_strongly_connected_nodes;
use crate::options::Options;
use crate::{
    ComputeState, ConstantInputType, GenericStore, NodeBlueprintDefinition, PortInputType,
    PortTypeInfo,
};

pub(super) struct Evaluator<P, D, K, A>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
    K: NodeBlueprintDefinition<P, D>,
    A: GraphEventHandler<D>,
{
    #[allow(dead_code)]
    options: Options,
    egui_context: Option<Context>,
    nodes: SecondaryMap<NodeId, EvaluationNode<P, D, K>>,
    reported_validity: SecondaryMap<NodeId, DataValidityState>,

    evaluation_stack: Vec<NodeId>,

    connections: Connections,
    focused_node: Option<NodeId>,

    default_compute_store: GenericStore,
    looping_nodes: Vec<Vec<NodeId>>,
    event_handler: A,
    receiver: Receiver<Command<P, D, K>>,
    sender: Sender<Event>,
    events_to_send: Vec<Event>,
}

impl<P, D, K, A> Evaluator<P, D, K, A>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
    K: NodeBlueprintDefinition<P, D>,
    A: GraphEventHandler<D>,
{
    pub(super) fn new(
        options: Options,
        nodes: SecondaryMap<NodeId, EvaluationNode<P, D, K>>,
        connections: Connections,
        focused_node: Option<NodeId>,
        default_compute_store: GenericStore,
        event_handler: A,
        receiver: Receiver<Command<P, D, K>>,
        sender: Sender<Event>,
    ) -> Self {
        Self {
            options,
            egui_context: None,
            nodes,
            reported_validity: SecondaryMap::new(),
            evaluation_stack: Vec::new(),
            connections,
            focused_node,
            default_compute_store,
            looping_nodes: Vec::new(),
            event_handler,
            receiver,
            sender,
            events_to_send: Vec::new(),
        }
    }

    pub(super) fn run_thread(&mut self) {
        loop {
            let commands = self.drain_commands();
            let mut events: Vec<Event> = Vec::new();
            let mut shutdown = false;

            for command in commands {
                let node_id = command.node_id();

                match self.apply(command) {
                    Ok(shutdown_received) => shutdown = shutdown || shutdown_received,
                    Err(error) => events.push(Event::Failed { node_id, error }),
                }
            }

            if shutdown {
                return;
            }

            if !self.evaluation_stack.is_empty() {
                if self.looping_nodes.is_empty() {
                    match self.evaluate() {
                        Ok(_) => {}
                        Err(error) => events.push(Event::Failed {
                            error,
                            node_id: None,
                        }),
                    }
                } else {
                    log::debug!("[Graph] Looping nodes: {:#?}", self.looping_nodes);
                    for nodes_group in &self.looping_nodes {
                        for node_id in nodes_group {
                            match self
                                .nodes
                                .get_mut(*node_id)
                                .ok_or_else(|| anyhow::anyhow!("Could not find node"))
                                .unwrap()
                                .invalidate(InvalidationReason::EvaluationError)
                            {
                                Ok(_) => {}
                                Err(error) => events.push(Event::Failed {
                                    error,
                                    node_id: Some(*node_id),
                                }),
                            }
                        }
                    }
                }
            }

            self.collect_node_events();

            self.events_to_send.extend(events);
            let repaint_after_send = !self.events_to_send.is_empty();

            if !self.send_events() {
                if let Err(error) = self.shutdown() {
                    log::error!("[Graph] Error while shutting down: {:?}", error);
                }
                return;
            } else if repaint_after_send {
                if let Some(egui_context) = &self.egui_context {
                    egui_context.request_repaint();
                } else {
                    log::warn!("Need to request repaint but context missing!");
                }
            }
        }
    }

    fn apply(&mut self, command: Command<P, D, K>) -> anyhow::Result<bool> {
        match command {
            Command::AddNode { node } => self.insert_node(node)?,
            Command::DeleteNode { node_id } => self.delete_node(node_id)?,
            Command::AddConnection { from, to } => self.insert_connection(from, to)?,
            Command::DeleteConnection { to } => self.delete_connection_to_port(to, true, true)?,
            Command::FocusNode { node_id } => self.focus_node(node_id)?,
            Command::UnfocusNode => {
                self.focused_node = None;
            }
            Command::SetConstant { port_id, constant } => self.set_constant(port_id, constant)?,
            Command::AttachContext { context } => self.egui_context = Some(context),
            Command::AddDefaultEntries { entries } => self.add_default_compute_entries(entries),
            Command::Reset => self.reset()?,
            Command::Shutdown => {
                self.shutdown()?;
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn drain_commands(&mut self) -> Vec<Command<P, D, K>> {
        let mut commands: Vec<Command<P, D, K>> = Vec::new();

        if self.evaluation_stack.is_empty() {
            match self.receiver.recv() {
                Ok(command) => commands.push(command),
                Err(RecvError) => commands.push(Command::Shutdown),
            }
        } else {
            match self.receiver.recv_timeout(POLL_INTERVAL) {
                Ok(command) => commands.push(command),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => commands.push(Command::Shutdown),
            }
        }

        commands.extend(self.receiver.try_iter());

        commands
    }

    fn send_events(&mut self) -> bool {
        self.events_to_send
            .drain(..)
            .all(|event| self.sender.send(event).is_ok())
    }

    /// Works through the evaluation stack from the top until it is empty or a
    /// node is still computing.
    ///
    /// An invalid node on top expands the stack with its dependencies. Valid nodes
    /// are popped. An error propagates downstream and clears the stack, and a
    /// stopped computation invalidates downstream and clears the stack.
    fn evaluate(&mut self) -> anyhow::Result<()> {
        log::debug!("[Graph] Evaluating nodes");

        while let Some(node_id) = self.evaluation_stack.last().copied() {
            let validity = self
                .nodes
                .get(node_id)
                .ok_or_else(|| anyhow::anyhow!("Could not find node"))?
                .validity;

            let validity = match validity {
                DataValidityState::Invalid => {
                    self.fill_evaluation_stack()?;
                    DataValidityState::PendingEvaluation
                }
                DataValidityState::PendingEvaluation => self.evaluate_node(node_id)?,
                DataValidityState::PendingComputation => self.evaluate_node(node_id)?,
                DataValidityState::Valid => validity,
                DataValidityState::Error { .. } => validity,
            };

            match validity {
                DataValidityState::Invalid => {
                    self.propagate_state(node_id, InvalidationReason::ObsoleteData)?;

                    log::debug!(
                        "[Graph] Clearing evaluation stack after assuming evaluation stopped"
                    );
                    self.evaluation_stack.clear();
                }
                DataValidityState::PendingEvaluation => {}
                DataValidityState::PendingComputation => {
                    break;
                }
                DataValidityState::Valid => {
                    self.evaluation_stack
                        .remove(self.evaluation_stack.len() - 1);
                }
                DataValidityState::Error { .. } => {
                    self.propagate_state(node_id, InvalidationReason::EvaluationError)?;

                    log::debug!("[Graph] Clearing evaluation stack after propagating error");
                    self.evaluation_stack.clear();
                }
            }
        }

        Ok(())
    }

    /// Empties the evaluation stack and invalidates the nodes it held with
    /// `reason`. Currently unused.
    #[allow(dead_code)]
    fn clear_evaluation_stack(&mut self, reason: InvalidationReason) -> anyhow::Result<()> {
        for node_id in &self.evaluation_stack {
            let node = self
                .nodes
                .get_mut(*node_id)
                .ok_or_else(|| anyhow::anyhow!("Could not find node"))?;

            match node.validity {
                DataValidityState::PendingEvaluation
                | DataValidityState::PendingComputation => {
                    node.invalidate(reason)?;
                }
                DataValidityState::Invalid => {
                    if matches!(reason, InvalidationReason::EvaluationError) {
                        node.invalidate(reason)?;
                    }
                }
                // We don't do anything for Valid nodes. It's probably fine.
                _ => {}
            }
        }

        self.evaluation_stack.clear();

        Ok(())
    }

    /// Expands the evaluation stack with the transitive dependencies of its top
    /// node, then keeps only the invalid entries and marks them pending
    /// evaluation.
    ///
    /// Dependencies end up above the nodes that read them, so popping the stack
    /// computes them first. The graph must be acyclic, which
    /// `run_thread` guarantees by not evaluating while a cycle exists.
    fn fill_evaluation_stack(&mut self) -> anyhow::Result<()> {
        log::debug!("[Graph] Trying to fill the evaluation stack");
        if !self.evaluation_stack.is_empty() {
            let mut i = self.evaluation_stack.len() - 1;

            while i < self.evaluation_stack.len() {
                let node_id = self.evaluation_stack[i];

                for input_node_id in self.connections.upstream_nodes(node_id) {
                    log::debug!("[Graph] Added {input_node_id:?} to the evaluation stack");
                    self.evaluation_stack.push(input_node_id);
                }

                i += 1;
            }

            i = self.evaluation_stack.len();

            while i > 0 {
                i -= 1;
                let node = self.nodes.get_mut(self.evaluation_stack[i]).unwrap();

                if node.is_invalid() {
                    node.validity = DataValidityState::PendingEvaluation;
                } else {
                    self.evaluation_stack.remove(i);
                }
            }

            log::info!("[Graph] Evaluation stack: {:?}", self.evaluation_stack);
        }

        Ok(())
    }

    /// Invalidates `node_id` and every node downstream of it with `reason`.
    ///
    /// Valid and pending nodes are always invalidated. Invalid nodes only move to
    /// the error state for [`InvalidationReason::EvaluationError`], and nodes in
    /// the error state are only reset for [`InvalidationReason::ObsoleteData`].
    /// Returns whether the focused node was invalidated, in which case the caller
    /// has to schedule it again.
    pub(super) fn propagate_state(
        &mut self,
        node_id: NodeId,
        reason: InvalidationReason,
    ) -> anyhow::Result<bool> {
        log::debug!("[Graph] Propagating state from {node_id:?}, reason: {reason:?}");
        let mut nodes_queue: Vec<NodeId> = vec![node_id];
        let mut i = 0;
        let mut focused_node_was_invalidated: bool = false;
        let mut visited: HashSet<NodeId> = HashSet::new();

        while i < nodes_queue.len() {
            let node_id = nodes_queue[i];
            let node = self.nodes.get_mut(node_id).unwrap();

            // No check for is_error just in case the error needs to be overwritten
            match node.validity {
                DataValidityState::Valid
                | DataValidityState::PendingEvaluation
                | DataValidityState::PendingComputation => {
                    node.invalidate(reason)?;
                    if self.focused_node == Some(node_id) {
                        focused_node_was_invalidated = true;
                    }
                }
                DataValidityState::Invalid => {
                    if matches!(reason, InvalidationReason::EvaluationError) {
                        node.invalidate(reason)?;
                        if self.focused_node == Some(node_id) {
                            focused_node_was_invalidated = true;
                        }
                    }
                }
                DataValidityState::Error { .. } => {
                    if matches!(reason, InvalidationReason::ObsoleteData) {
                        node.invalidate(reason)?;
                        if self.focused_node == Some(node_id) {
                            focused_node_was_invalidated = true;
                        }
                    }
                }
            }
            if !visited.contains(&node_id) {
                nodes_queue.extend(self.connections.downstream_nodes(node_id));
            }
            visited.insert(node_id);

            i += 1;
        }

        Ok(focused_node_was_invalidated)
    }

    /// Advances a single node by one step.
    ///
    /// A node pending evaluation gets its inputs gathered and the default compute
    /// entries copied in before `EvaluationNode::compute` runs. A node pending computation
    /// is polled again. The resulting state is `Valid` when the computation is
    /// done, `PendingComputation` while it is running, `Error` when gathering the
    /// inputs or the computation failed, and `Invalid` when it was stopped. `Valid`
    /// and `Error` nodes are returned unchanged, and an `Invalid` node is reported
    /// as an error because it should have been scheduled first.
    fn evaluate_node(&mut self, node_id: NodeId) -> anyhow::Result<DataValidityState> {
        log::debug!("[{node_id:?}] Evaluating node");
        let inputs: anyhow::Result<Option<HashMap<String, Arc<D>>>> = {
            let node = &self.nodes[node_id];

            match node.validity {
                DataValidityState::PendingEvaluation => {
                    match self.extract_node_inputs(node) {
                        Ok(inputs) => Ok(Some(inputs)),
                        Err(e) => {
                            log::error!(
                                "[{node_id:?}] Could not extract node inputs: {:?}",
                                e.to_string()
                            );
                            Err(e)
                        }
                    }
                }
                DataValidityState::PendingComputation => {
                    log::debug!("[{node_id:?}] Waiting for pending computation");
                    Ok(None)
                }
                DataValidityState::Invalid => {
                    return Err(anyhow::anyhow!(
                        "Inconsistent Invalid state for {:?}",
                        node_id
                    ));
                }
                DataValidityState::Valid | DataValidityState::Error { .. } => {
                    return Ok(node.validity);
                }
            }
        };

        let node = self.nodes.get_mut(node_id).unwrap();

        match inputs {
            Ok(inputs) => {
                if let Some(inputs) = inputs {
                    node.set_inputs_map(inputs);
                    node.extend_store(&self.default_compute_store);
                }
            }
            Err(_) => {
                node.invalidate(InvalidationReason::EvaluationError)?;
                return Ok(node.validity);
            }
        }

        match node.compute() {
            Ok(ComputeState::Done { outputs }) => {
                if self.focused_node == Some(node.id) {
                    match self
                        .event_handler
                        .on_focused_output_change(node.data_view_from_outputs(&outputs))
                    {
                        Ok(_) => {}
                        Err(error) => {
                            log::error!(
                                "[{node_id:?}] on_focused_output_change returned an error: {:?}",
                                error
                            );
                            self.events_to_send.push(Event::Failed {
                                node_id: Some(node_id),
                                error,
                            });
                        }
                    }
                }
                node.validity = DataValidityState::Valid;
            }
            Ok(ComputeState::Running) => {
                node.validity = DataValidityState::PendingComputation;
            }
            Ok(ComputeState::Error) => {
                node.invalidate(InvalidationReason::EvaluationError)?;
            }
            Ok(ComputeState::Stopped) => {
                node.validity = DataValidityState::Invalid;
            }
            Err(e) => {
                log::error!("[{node_id:?}] Compute failed: {:?}", e.to_string());
                node.invalidate(InvalidationReason::EvaluationError)?;
            }
        }

        Ok(node.validity)
    }

    /// Gathers the values of a node's input ports, keyed by port name.
    ///
    /// A connected port takes the output buffer of its source, a constant port
    /// takes its converted constant, and a port accepting both prefers the
    /// connection. An unconnected connection-only port is left out, or reported as
    /// an error when it is required. A connected source without a result is also
    /// an error, since sources are evaluated first.
    fn extract_node_inputs(
        &self,
        node: &EvaluationNode<P, D, K>,
    ) -> anyhow::Result<HashMap<String, Arc<D>>> {
        let mut inputs: HashMap<String, Arc<D>> = HashMap::new();

        fn extract_source_output<P, D, K>(
            node: &EvaluationNode<P, D, K>,
            source_node: &EvaluationNode<P, D, K>,
            data_source: &PortId,
        ) -> anyhow::Result<Arc<D>>
        where
            P: PortTypeInfo + Eq + Copy + Clone + Debug,
            D: PartialEq
            + Clone
            + Debug
            + for<'a> From<&'a ConstantInputType>
            + Send
            + Sync
            + 'static,
            for<'a> &'a D: TryInto<String>,
            K: NodeBlueprintDefinition<P, D>,
        {
            if source_node.ports()[data_source].direction == PortDirection::Input {
                anyhow::bail!(
                    "Inconsistent input direction from {:?} into {:?}",
                    data_source.node_id(),
                    node.id
                );
            }

            source_node
                .output_value(data_source)
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Inconsistent input value (None) from {:?} into {:?}",
                        data_source.node_id(),
                        node.id
                    )
                })
        }

        for port in node.input_ports_and_parameters() {
            match port.input_type.as_ref().unwrap() {
                PortInputType::ConnectionOnly { data_source, .. } => {
                    if let Some(data_source) = data_source {
                        let source_node = &self.nodes[data_source.node_id()];

                        inputs.insert(
                            port.name.clone(),
                            extract_source_output(node, source_node, data_source)?,
                        );
                    } else {
                        if port.required {
                            anyhow::bail!("No input for a required port: {:?}", port.id);
                        }
                    }
                }
                PortInputType::ConstantOnly { constant_type, .. } => {
                    // TODO: Case in which ::from fails / invalid constant
                    inputs.insert(port.name.clone(), Arc::new(D::from(constant_type)));
                }
                PortInputType::ConnectionAndConstant {
                    constant_type,
                    data_source,
                    ..
                } => {
                    if let Some(data_source) = data_source {
                        let source_node = &self.nodes[data_source.node_id()];

                        inputs.insert(
                            port.name.clone(),
                            extract_source_output(node, source_node, data_source)?,
                        );
                    } else {
                        // TODO: Case in which ::from fails / invalid constant
                        inputs.insert(port.name.clone(), Arc::new(D::from(constant_type)));
                    }
                }
            }
        }

        Ok(inputs)
    }

    /// Focuses a node. An invalid or erroneous node is scheduled for evaluation, a
    /// pending node is left alone, and a valid node reports its current output to
    /// the event handler right away.
    pub(super) fn focus_node(&mut self, node_id: NodeId) -> anyhow::Result<()> {
        log::debug!("[{node_id:?}] Setting focus");
        let node = self
            .nodes
            .get_mut(node_id)
            .ok_or_else(|| anyhow::anyhow!("Focused node does not exist"))?;
        self.focused_node = Some(node_id);

        match node.validity {
            DataValidityState::Invalid | DataValidityState::Error { .. } => {
                self.evaluation_stack.push(node_id);
                node.invalidate(InvalidationReason::ObsoleteData)?;
            }
            DataValidityState::PendingEvaluation | DataValidityState::PendingComputation => {}
            DataValidityState::Valid => {
                self.event_handler
                    .on_focused_output_change(node.data_view())?;
            }
        }

        Ok(())
    }

    /// Clears the focused node without touching any node state.
    #[allow(dead_code)]
    pub(super) fn clear_focused_node(&mut self) {
        self.focused_node = None;
    }

    pub(super) fn insert_connection(
        &mut self,
        from_port: PortId,
        to_port: PortId,
    ) -> anyhow::Result<()> {
        self.connections.insert(from_port, to_port);

        match self
            .nodes
            .get_mut(to_port.node_id())
            .ok_or_else(|| anyhow::anyhow!("Could not find node"))?
            .ports_get_mut(&to_port)
            .ok_or_else(|| anyhow::anyhow!("Could not find port"))?
            .input_type
        {
            Some(PortInputType::ConnectionAndConstant {
                     ref mut data_source,
                     ..
                 })
            | Some(PortInputType::ConnectionOnly {
                       ref mut data_source,
                       ..
                   }) => {
                *data_source = Some(from_port);
            }
            _ => {}
        }

        let focused_node_was_invalidated =
            self.propagate_state(to_port.node_id(), InvalidationReason::ObsoleteData)?;

        if focused_node_was_invalidated {
            log::debug!(
                "[Graph] Putting {:?} back on the evaluation stack because its \
                data is obsolete",
                self.focused_node
            );

            self.requeue_focused_node();
        }

        log::debug!("[Graph] Recalculating strongly connected (looping) nodes");
        self.recompute_looping_nodes();

        Ok(())
    }

    pub(super) fn delete_connection_to_port(
        &mut self,
        to_port_id: PortId,
        compute_looping_nodes: bool,
        propagate_state: bool,
    ) -> anyhow::Result<()> {
        log::debug!("[Graph] Deleting all connections to {to_port_id:?}");
        // TODO: In the future it's probably best to get rid of the bool

        if let Some(connection) = self.connections.remove(to_port_id) {
            log::debug!("[Graph] Deleting data_source in {to_port_id:?}");
            match self
                .nodes
                .get_mut(connection.to_port.node_id())
                .ok_or_else(|| anyhow::anyhow!("Could not find node"))?
                .ports_get_mut(&connection.to_port)
                .ok_or_else(|| anyhow::anyhow!("Could not find port"))?
                .input_type
            {
                Some(PortInputType::ConnectionAndConstant {
                         ref mut data_source,
                         ..
                     })
                | Some(PortInputType::ConnectionOnly {
                           ref mut data_source,
                           ..
                       }) => {
                    *data_source = None;
                }
                _ => {}
            }

            if propagate_state {
                let focused_node_was_invalidated =
                    self.propagate_state(to_port_id.node_id(), InvalidationReason::ObsoleteData)?;

                if focused_node_was_invalidated {
                    log::debug!(
                        "[Graph] Putting {:?} back on the evaluation stack because its \
                        data is obsolete",
                        self.focused_node
                    );

                    self.requeue_focused_node();
                }
            }
        }

        if compute_looping_nodes {
            log::debug!("[Graph] Recalculating strongly connected (looping) nodes");
            self.recompute_looping_nodes();
        }

        Ok(())
    }

    pub(super) fn insert_node(&mut self, node: EvaluationNode<P, D, K>) -> anyhow::Result<()> {
        let node_id = node.id;

        self.nodes.insert(node_id, node);

        if !self.nodes.contains_key(node_id) {
            anyhow::bail!("Could not insert node {:?}", node_id);
        }

        log::debug!("[Evaluator] Inserted node {:?}", node_id);

        Ok(())
    }

    pub(super) fn set_constant(
        &mut self,
        port_id: PortId,
        constant: ConstantInputType,
    ) -> anyhow::Result<()> {
        self.nodes
            .get_mut(port_id.node_id())
            .ok_or_else(|| anyhow::anyhow!("Could not find node {:?}", port_id.node_id()))?
            .set_constant(&port_id, constant)?;

        let focused_node_was_invalidated =
            self.propagate_state(port_id.node_id(), InvalidationReason::ObsoleteData)?;

        if focused_node_was_invalidated {
            self.requeue_focused_node();
        }

        Ok(())
    }

    pub(super) fn delete_node(&mut self, node_id: NodeId) -> anyhow::Result<()> {
        {
            let node = self.nodes.get_mut(node_id).unwrap();
            if node.validity.is_pending_computation() {
                node.drop_compute()?;
            }
        }

        if self
            .focused_node
            .is_some_and(|focused_node_id| focused_node_id == node_id)
        {
            self.focused_node = None;
        }

        let outbound_connections: Vec<PortId> = self
            .connections
            .touching(node_id)
            .into_iter()
            .map(|connection| connection.to_port)
            .collect();

        for (i, port_id) in outbound_connections.iter().enumerate() {
            if i == outbound_connections.len() - 1 {
                self.delete_connection_to_port(*port_id, true, true)?;
            } else {
                self.delete_connection_to_port(*port_id, false, true)?;
            }
        }

        self.nodes.remove(node_id);
        self.reported_validity.remove(node_id);

        self.evaluation_stack.retain(|id| *id != node_id);
        Ok(())
    }

    fn collect_node_events(&mut self) {
        for (node_id, node) in &self.nodes {
            let previous = self.reported_validity.insert(node_id, node.validity);

            if previous == Some(node.validity) {
                continue;
            }

            if node.validity.is_valid() {
                let outputs_text: HashMap<String, Option<String>> = node
                    .outputs
                    .iter()
                    .map(|(name, value)| (name.clone(), value.as_ref().try_into().ok()))
                    .collect();

                self.events_to_send.push(Event::ResultReady {
                    node_id,
                    data: node.custom_ui_data(),
                    outputs_text,
                });
            }

            self.events_to_send.push(Event::ValidityChanged {
                node_id,
                validity: node.validity,
            });
        }
    }

    pub(super) fn recompute_looping_nodes(&mut self) {
        log::debug!("[Graph] Recalculating strongly connected (looping) nodes");
        self.looping_nodes = compute_strongly_connected_nodes(&self.connections.dependency_graph());
    }

    pub(super) fn add_default_compute_entries(&mut self, entries: GenericStore) {
        self.default_compute_store.extend(entries);
    }

    pub(super) fn requeue_focused_node(&mut self) {
        if let Some(focused) = self.focused_node
            && !self.evaluation_stack.contains(&focused)
        {
            self.evaluation_stack.push(focused);
        }
    }

    pub(super) fn reset(&mut self) -> anyhow::Result<()> {
        for node in self.nodes.values_mut() {
            if node.is_pending_computation() {
                node.drop_compute()?;
            }
        }

        self.looping_nodes.clear();
        self.evaluation_stack.clear();
        self.connections.clear();
        self.nodes.clear();
        self.reported_validity.clear();
        self.focused_node = None;

        Ok(())
    }

    fn shutdown(&mut self) -> anyhow::Result<()> {
        if !self.evaluation_stack.is_empty()
            && let Some(node) = self.nodes.get_mut(*self.evaluation_stack.last().unwrap())
        {
            node.drop_compute()?;
        }

        Ok(())
    }

    #[allow(dead_code)]
    fn clear(&mut self) {
        self.looping_nodes.clear();
        self.evaluation_stack.clear();
    }
}

#[cfg(test)]
mod tests;
