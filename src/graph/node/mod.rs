//! Nodes, the blueprints describing them and the values they produce.

pub mod port;
pub mod state;
pub(crate) mod threading;

use std::cmp::PartialEq;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;
use std::thread::JoinHandle;

use egui::{Pos2, Ui, Vec2, pos2, vec2};
use port::{ConstantInputType, PortDefinition, PortTypeInfo};
use serde::Serialize;
use serde::de::DeserializeOwned;
use state::{InputsMap, RuntimeState};

use crate::PortInputType;
use crate::graph::canvas::Canvas;
use crate::graph::node::port::{
    EvaluationPort, Port, PortDirection, PortId, PortLinkageType, PortOutputType, port_id,
    resolve_signal, validate_enablers,
};
use crate::graph::node::threading::{CancelSignal, POLL_INTERVAL};
use crate::graph::snapshot::{NodeSnapshot, PortSnapshot, SnapshotNodeId};
use crate::options::Options;
use crate::store::GenericStore;

const NODE_DEFAULT_WIDTH: f32 = 200.0;
const PORT_DEFAULT_HEIGHT: f32 = 20.0;
const FIRST_PORT_INDEX: u32 = 0;

/// Smallest size a node is drawn at, in editor units.
pub const NODE_MINIMUM_SIZE: Vec2 = vec2(150.0, 0.0);

/// Outcome of a single call to [`NodeBlueprintDefinition::compute`].
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ComputeState<D>
where
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
{
    /// Evaluation succeeded and produced `outputs`, keyed by output port name.
    Done {
        /// Result of each output port, keyed by port name. Outputs left out of
        /// the map are left without a value. A name that matches no port of the
        /// node fails the evaluation, and names of input ports are ignored.
        outputs: HashMap<String, Arc<D>>,
    },
    /// Work is in flight and the node has to be polled again.
    Running,
    /// Evaluation failed without a message to report, for instance because an
    /// input held an unusable variant.
    Error,
    /// Evaluation was interrupted before completion, for instance because the
    /// user cancelled the run.
    Stopped,
}

impl<D> ComputeState<D>
where
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
{
    /// Builds a [`ComputeState::Done`] from `(port name, value)` pairs.
    ///
    /// When a name appears more than once, the last value wins.
    pub fn done(outputs: Vec<(String, Arc<D>)>) -> Self {
        ComputeState::Done {
            outputs: HashMap::from_iter(outputs),
        }
    }
}

/// Describes one kind of node that can be placed in the graph editor.
///
/// A blueprint carries both the static description of a node (its display name,
/// its ports, its size and the menu categories it appears under) and the
/// behavior that runs when the graph is evaluated.
///
/// Implementors are typically enums whose variants each stand for one node kind,
/// so a single `impl` covers the whole node library and every method dispatches
/// on `self`.
///
/// # Type parameters
///
/// * `P` - the port type of the concrete editor, i.e. the set of data kinds a
///   port can carry (scalar field, string, ...). It decides which connections
///   are legal and how a port is colored.
/// * `D` - the value type that actually travels along connections at runtime.
///   A `D` must be constructible from a [`ConstantInputType`] so that constants
///   typed into a port can be fed into `compute` like any other input. `&D`
///   must also implement `TryInto<String>`, which provides the inline text of
///   output ports declared with a [`PortOutputType`] that has a constant part;
///   a failed conversion shows no text.
pub trait NodeBlueprintDefinition<P, D>:
Serialize + DeserializeOwned + Debug + Eq + Copy + Clone + Send + 'static
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
{
    /// Collection of all variants considered by the Graph
    fn all() -> &'static [Self]
    where
        Self: Sized;

    /// Human-readable name shown in the node header and in the creation menu.
    fn name(&self) -> &'static str;

    /// Full port layout of the node, inputs and outputs alike.
    ///
    /// The order of the returned vector is the order in which ports are drawn.
    /// Port names are the keys used by [`RuntimeState::inputs_map`] and by the
    /// outputs of [`ComputeState::Done`], so they must be unique per node.
    fn ports(&self) -> Vec<PortDefinition<P, D>>;

    /// Width of the node body in editor units.
    fn width(&self) -> f32 {
        NODE_DEFAULT_WIDTH
    }

    /// Vertical space reserved for a single port row in editor units.
    ///
    /// The node grows in height with the number of ports returned by
    /// [`ports`](Self::ports).
    fn ports_height(&self) -> f32 {
        PORT_DEFAULT_HEIGHT
    }

    /// Whether the graph runner may execute this node away from the thread that
    /// drives the graph.
    ///
    /// This is independent of [`ComputeState::Running`]: a node that keeps its
    /// work off-thread by itself, for instance by polling a channel fed from a
    /// GPU submission, stays single threaded from the runner's point of view and
    /// leaves this at `false`.
    ///
    /// Takes effect only when
    /// [`Options::with_multithreading`](crate::options::Options::with_multithreading)
    /// is enabled.
    fn multithreading(&self) -> bool {
        false
    }

    /// Categories the node is filed under in the creation menu.
    ///
    /// Returning several entries lists the node under each of them. An empty
    /// string lists the node at the top level of the menu.
    fn categories(&self) -> Vec<&'static str>;

    /// Advances the evaluation of the node by one step.
    ///
    /// This is a polling entry point rather than a one-shot call. A node that
    /// cannot finish immediately kicks its work off and returns
    /// [`ComputeState::Running`]; the runner calls `compute` again until a
    /// terminal state is reached. Nodes that finish in one pass return
    /// [`ComputeState::Done`] on the first call.
    ///
    /// `self` is not a reliable place to keep resume state, since the blueprint
    /// may be cloned onto a worker thread. [`RuntimeState::user_state`] lives for
    /// the duration of one evaluation and is cleared when the node is
    /// invalidated, so it is the intended home for anything that has to survive
    /// between polls: a job handle, a partial result, a cursor into the work.
    ///
    /// The library does not track how many times `compute` has been called. A
    /// node that needs to distinguish the first call from later ones has to infer
    /// it, and the usual way is to treat the absence of its own keys in
    /// `user_state` as meaning the work has not started yet.
    ///
    /// Nothing written to `user_state` after the initial map expansion with the default values
    /// reaches the editor or other nodes. It is scratch space for the computation, not a channel back to the
    /// editor; results travel out through [`ComputeState::Done`].
    ///
    /// It's important to note that when dropping the computation the detached thread is releasing
    /// its resources asynchronously and some things it is holding a lock for might be still
    /// unavailable to the evaluator thread for a little while longer. On a worker thread,
    /// `drop_compute` runs on that worker after the in-flight `compute` call returns.
    ///
    /// # Errors
    ///
    /// Returns an error when the node cannot proceed at all: a required entry
    /// missing from the user state, a value of the wrong type, a dead channel,
    /// a failed device operation.
    fn compute(&mut self, runtime_state: &mut RuntimeState<D>) -> anyhow::Result<ComputeState<D>>;

    /// Releases anything [`compute`](Self::compute) is holding on to.
    ///
    /// Called when an evaluation is abandoned before reaching a terminal state,
    /// so that background jobs can be cancelled and buffers freed. The default
    /// implementation does nothing.
    ///
    /// # Errors
    ///
    /// Returns an error when the cleanup itself fails.
    fn drop_compute(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Name of the output port whose value is shown in the viewer panel.
    ///
    /// Returns `None` for nodes that have nothing to display, such as terminal
    /// visualization nodes that render on their own.
    fn port_for_viewer(&self) -> Option<String>;

    /// Turns the node's outputs into the state [`custom_ui`](Self::custom_ui)
    /// draws from.
    ///
    /// Called on the evaluator thread each time a computation of the
    /// node finishes, with `outputs` keyed by output port name. The returned
    /// store is kept until the node is invalidated, so this is the place for
    /// work that should happen once per result rather than once per frame,
    /// such as decoding or downscaling an image. Keep it short, because the
    /// evaluation of other nodes waits for it. The default
    /// implementation returns an empty store.
    fn prepare_custom_ui_data(&self, _outputs: &HashMap<String, Arc<D>>) -> GenericStore {
        GenericStore::default()
    }

    /// Draws the node's custom UI, such as a preview of its result.
    ///
    /// Called every frame below the node's parameters. `ui` spans the width of
    /// the node, and the node grows to fit the space the content allocates, so
    /// drawing with `ui.painter()` alone has to be paired with
    /// `ui.allocate_space` or `ui.allocate_painter`. Content that allocates
    /// nothing leaves the node layout unchanged. Text and spacing follow the
    /// canvas zoom; explicit sizes should be derived from `ui.available_width()`
    /// to scale with it too.
    ///
    /// `state` is the store returned by
    /// [`prepare_custom_ui_data`](Self::prepare_custom_ui_data) for the latest result. It
    /// is empty until the node has been computed and again after it is
    /// invalidated. It is mutable so that values only buildable while drawing,
    /// such as a texture that needs `ui.ctx()`, can be created on first use and
    /// kept there: they are dropped with the rest of the store when the node is
    /// invalidated or deleted. `ui.id()` is stable for the lifetime of the node.
    ///
    /// Interactive widgets take pointer input from the node inside the area they
    /// cover. The default implementation draws nothing.
    fn custom_ui(&self, _ui: &mut Ui, _canvas: &Canvas, _state: &mut GenericStore) {}

    /// Defines the reserved height of the custom UI, which will always be drawn. If there is no
    /// custom UI drawn it will be left empty, while if there is a custom UI drawn
    /// [`NodeBlueprintDefinition::custom_ui_height`] acts as a minimum boundary.
    fn custom_ui_height(&self) -> f32 {
        0.0
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum DataValidityState {
    Invalid,
    PendingEvaluation,
    PendingComputation,
    Valid,
    Error { description: &'static str },
}

impl DataValidityState {
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid { .. })
    }

    pub fn is_invalid(&self) -> bool {
        matches!(self, Self::Invalid)
    }

    #[allow(dead_code)]
    pub fn is_pending_evaluation(&self) -> bool {
        matches!(self, Self::PendingEvaluation { .. })
    }

    pub fn is_pending_computation(&self) -> bool {
        matches!(self, Self::PendingComputation { .. })
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) enum InvalidationReason {
    ObsoleteData,
    EvaluationError,
}

slotmap::new_key_type! {
    /// Identifies a node within one [`Graph`](crate::Graph).
    ///
    /// Ids are never reused: once a node is deleted, its id stops resolving even
    /// if a new node takes its place. Ids are not stable across saving and
    /// loading.
    pub struct NodeId;
}

/// A node placed in the graph: a blueprint together with its ports, position
/// and a UI-side copy of its validity.
#[derive(Debug)]
pub(crate) struct Node<P, D, K>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
    K: NodeBlueprintDefinition<P, D>,
{
    options: Options,
    pub(crate) id: NodeId,
    blueprint: K,
    pub(crate) name: String,
    pub(crate) position: Pos2,
    ports: BTreeMap<PortId, Port<P>>,
    ports_names: HashMap<String, PortId>,
    custom_ui_state: GenericStore,
    pub(crate) validity: DataValidityState,
    pub(crate) focused: bool,
    _marker: PhantomData<D>,
}

impl<P, D, K> Node<P, D, K>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
    K: NodeBlueprintDefinition<P, D>,
{
    pub(in crate::graph) fn new(
        options: Options,
        id: NodeId,
        name: String,
        blueprint: K,
        position: Pos2,
    ) -> anyhow::Result<Self> {
        let mut ports = BTreeMap::new();
        let mut ports_names = HashMap::new();

        for definition in blueprint.ports() {
            let port_id = port_id(id, ports.len() as u32);
            if ports_names.contains_key(definition.name.as_str()) {
                anyhow::bail!("Duplicate port: {}", definition.name);
            }

            ports_names.insert(definition.name.clone(), port_id);

            ports.insert(port_id, Port::from_definition(port_id, definition));
        }

        validate_enablers(input_types(&ports))?;

        Ok(Node {
            options,
            id,
            name,
            blueprint,
            position,
            ports,
            ports_names,
            custom_ui_state: GenericStore::default(),
            validity: DataValidityState::Invalid,
            focused: false,
            _marker: PhantomData,
        })
    }

    pub(in crate::graph) fn snapshot(&self, snapshot_id: SnapshotNodeId) -> NodeSnapshot<K> {
        let ports_snapshots: Vec<PortSnapshot> = self
            .ports
            .values()
            .map(|port| port.snapshot(snapshot_id))
            .collect();

        NodeSnapshot {
            id: snapshot_id,
            blueprint: self.blueprint,
            name: self.name.clone(),
            position: (self.position.x, self.position.y),
            ports: ports_snapshots,
        }
    }

    pub(in crate::graph) fn from_snapshot(
        options: Options,
        id: NodeId,
        snapshot: NodeSnapshot<K>,
    ) -> anyhow::Result<Self> {
        let position = pos2(snapshot.position.0, snapshot.position.1);
        let mut ports = BTreeMap::new();
        let mut ports_names = HashMap::new();

        let ports_data = snapshot
            .ports
            .iter()
            .map(|x| (&x.name, x))
            .collect::<HashMap<&String, &PortSnapshot>>();

        let mut next_available_index = snapshot
            .ports
            .iter()
            .map(|port| port.id.index())
            .max()
            .map_or(FIRST_PORT_INDEX, |max| max + 1);

        for definition in snapshot.blueprint.ports() {
            let name = definition.name.clone();

            if ports_names.contains_key(&name) {
                anyhow::bail!("Duplicate port: {name}");
            }

            let (port_id, constant) = match ports_data.get(&name) {
                Some(port_snapshot) => (
                    port_id(id, port_snapshot.id.index()),
                    port_snapshot.constant.as_ref(),
                ),
                None => {
                    let index = next_available_index;
                    next_available_index += 1;
                    (port_id(id, index), None)
                }
            };

            let mut port = Port::from_definition(port_id, definition);

            if let Some(constant) = &constant {
                match port.linkage_type {
                    PortLinkageType::Input(ref mut input_type) => {
                        let constant_type = input_type.constant_type_mut().ok_or_else(|| {
                            anyhow::anyhow!("Port {} does not hold a constant", port.name)
                        })?;

                        constant.apply_to(constant_type)?;
                    }
                    PortLinkageType::Output(_) => {
                        anyhow::bail!(
                            "Port {} is not the correct type to hold a constant",
                            port.name
                        )
                    }
                }
            }

            ports_names.insert(name, port_id);
            ports.insert(port_id, port);
        }

        validate_enablers(input_types(&ports))?;

        Ok(Node {
            options,
            id,
            name: snapshot.name,
            blueprint: snapshot.blueprint,
            position,
            ports,
            ports_names,
            custom_ui_state: GenericStore::default(),
            validity: DataValidityState::Invalid,
            focused: false,
            _marker: PhantomData,
        })
    }

    pub(crate) fn ports(&self) -> &BTreeMap<PortId, Port<P>> {
        &self.ports
    }

    pub(crate) fn is_constant_enabled(&self, port_id: &PortId) -> bool {
        let resolve = |signal| resolve_signal(input_types(&self.ports), signal);

        self.ports
            .get(port_id)
            .is_some_and(|port| match &port.linkage_type {
                PortLinkageType::Input(input_type) => input_type.is_constant_enabled(&resolve),
                PortLinkageType::Output(_) => false,
            })
    }

    #[allow(dead_code)]
    pub(crate) fn ports_mut(&mut self) -> impl Iterator<Item=(&PortId, &mut Port<P>)> {
        self.ports.iter_mut()
    }

    #[allow(dead_code)]
    pub(crate) fn ports_get_mut(&mut self, port_id: &PortId) -> Option<&mut Port<P>> {
        self.ports.get_mut(port_id)
    }

    #[allow(dead_code)]
    pub(crate) fn input_ports(&self) -> impl Iterator<Item=&Port<P>> {
        self.ports.values().filter(|port| {
            port.is_input()
                && !matches!(
                    port.linkage_type,
                    PortLinkageType::Input(PortInputType::ConstantOnly { .. })
                )
        })
    }

    #[allow(dead_code)]
    pub(crate) fn input_ports_mut(&mut self) -> impl Iterator<Item=&mut Port<P>> {
        self.ports.values_mut().filter(|port| {
            port.is_input()
                && !matches!(
                    port.linkage_type,
                    PortLinkageType::Input(PortInputType::ConstantOnly { .. })
                )
        })
    }

    #[allow(dead_code)]
    pub(crate) fn input_parameters(&self) -> impl Iterator<Item=&Port<P>> {
        self.ports.values().filter(|port| {
            port.is_input()
                && matches!(
                    port.linkage_type,
                    PortLinkageType::Input(PortInputType::ConstantOnly { .. })
                )
        })
    }

    #[allow(dead_code)]
    pub(crate) fn input_parameters_mut(&mut self) -> impl Iterator<Item=&mut Port<P>> {
        self.ports.values_mut().filter(|port| {
            port.is_input()
                && matches!(
                    port.linkage_type,
                    PortLinkageType::Input(PortInputType::ConstantOnly { .. })
                )
        })
    }

    #[allow(dead_code)]
    pub(crate) fn input_ports_and_parameters(&self) -> impl Iterator<Item=&Port<P>> {
        self.ports.values().filter(|port| port.is_input())
    }

    #[allow(dead_code)]
    pub(crate) fn output_ports(&self) -> impl Iterator<Item=&Port<P>> {
        self.ports.values().filter(|port| {
            port.is_output()
                && !matches!(
                    port.linkage_type,
                    PortLinkageType::Output(PortOutputType::ConstantOnly { .. })
                )
        })
    }

    #[allow(dead_code)]
    pub(crate) fn output_ports_mut(&mut self) -> impl Iterator<Item=&mut Port<P>> {
        self.ports.values_mut().filter(|port| {
            port.is_output()
                && !matches!(
                    port.linkage_type,
                    PortLinkageType::Output(PortOutputType::ConstantOnly { .. })
                )
        })
    }

    #[allow(dead_code)]
    pub(crate) fn output_inline_ports(&self) -> impl Iterator<Item=&Port<P>> {
        self.ports.values().filter(|port| {
            matches!(port.direction, PortDirection::Output)
                && matches!(
                    port.linkage_type,
                    PortLinkageType::Output(PortOutputType::ConstantOnly { .. })
                )
        })
    }

    #[allow(dead_code)]
    pub(crate) fn output_inline_ports_mut(&mut self) -> impl Iterator<Item=&mut Port<P>> {
        self.ports.values_mut().filter(|port| {
            matches!(port.direction, PortDirection::Output)
                && matches!(
                    port.linkage_type,
                    PortLinkageType::Output(PortOutputType::ConstantOnly { .. })
                )
        })
    }

    pub(crate) fn width(&self) -> f32 {
        self.blueprint.width()
    }

    /// Whether the node holds the result of its current inputs.
    #[allow(dead_code)]
    pub fn is_valid(&self) -> bool {
        self.validity.is_valid()
    }

    /// Whether the node has no result or a stale one.
    #[allow(dead_code)]
    pub fn is_invalid(&self) -> bool {
        self.validity.is_invalid()
    }

    /// Whether the node is scheduled but has not started computing.
    #[allow(dead_code)]
    pub fn is_pending_evaluation(&self) -> bool {
        self.validity.is_pending_evaluation()
    }

    /// Whether the node's computation has started and not finished.
    #[allow(dead_code)]
    pub fn is_pending_computation(&self) -> bool {
        self.validity.is_pending_computation()
    }

    #[allow(dead_code)]
    pub(crate) fn custom_ui_state(&self) -> &GenericStore {
        &self.custom_ui_state
    }

    pub(in crate::graph) fn set_custom_ui_state(&mut self, state: GenericStore) {
        self.custom_ui_state = state;
    }

    pub(in crate::graph) fn clear_custom_ui_state(&mut self) {
        self.custom_ui_state.clear();
    }

    pub(crate) fn show_custom_ui(&mut self, ui: &mut Ui, canvas: &Canvas) {
        self.blueprint
            .custom_ui(ui, canvas, &mut self.custom_ui_state);
    }

    pub(crate) fn custom_ui_height(&self) -> f32 {
        self.blueprint.custom_ui_height()
    }

    #[allow(dead_code)]
    fn declared_outputs(
        &self,
        outputs: &HashMap<String, Arc<D>>,
    ) -> anyhow::Result<HashMap<String, Arc<D>>> {
        let mut declared = HashMap::with_capacity(outputs.len());

        for (name, output) in outputs {
            let port_id = self
                .ports_names
                .get(name)
                .ok_or_else(|| anyhow::anyhow!("No port found for name: {name:?}"))?;

            let port = self
                .ports
                .get(port_id)
                .ok_or_else(|| anyhow::anyhow!("No port found for id: {port_id:?}"))?;

            if port.direction == PortDirection::Output {
                declared.insert(name.clone(), output.clone());
            }
        }

        Ok(declared)
    }

    pub(crate) fn clear_outputs_text(&mut self) {
        for port in self.ports.values_mut() {
            match port.linkage_type {
                PortLinkageType::Output(PortOutputType::ConstantOnly { ref mut text_value })
                | PortLinkageType::Output(PortOutputType::ConnectionAndConstant {
                                              ref mut text_value,
                                          }) => {
                    *text_value = None;
                }
                _ => {}
            }
        }
    }

    pub(crate) fn set_outputs_text(&mut self, outputs_text: HashMap<String, Option<String>>) {
        for (name, output) in &outputs_text {
            if let Some(port_id) = self.ports_names.get(name)
                && let Some(port) = self.ports.get_mut(port_id)
            {
                match port.linkage_type {
                    PortLinkageType::Output(PortOutputType::ConstantOnly {
                                                ref mut text_value,
                                            }) |
                    PortLinkageType::Output(PortOutputType::ConnectionAndConstant {
                                                ref mut text_value,
                                            }) => {
                        *text_value = output.clone();
                    }
                    _ => {}
                }
            }
        }
    }
}

fn input_types<P>(ports: &BTreeMap<PortId, Port<P>>) -> impl Iterator<Item=&PortInputType> + Clone
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
{
    ports.values().filter_map(|port| match &port.linkage_type {
        PortLinkageType::Input(input_type) => Some(input_type),
        PortLinkageType::Output(_) => None,
    })
}

#[cfg(test)]
mod enabler_tests;
#[cfg(test)]
mod outputs_tests;
#[cfg(test)]
mod threaded_tests;

#[derive(Debug)]
pub(crate) struct EvaluationNode<P, D, K>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
    K: NodeBlueprintDefinition<P, D>,
{
    pub(crate) id: NodeId,
    blueprint: K,
    ports: BTreeMap<PortId, EvaluationPort>,
    ports_names: HashMap<String, PortId>,
    pub(crate) outputs: HashMap<String, Arc<D>>,
    pub(crate) validity: DataValidityState,
    runtime_state: RuntimeState<D>,
    join_handle: Option<JoinHandle<anyhow::Result<ComputeState<D>>>>,
    cancel: Option<Arc<CancelSignal>>,
    options: Options,
    _marker: PhantomData<fn() -> P>,
}

impl<P, D, K> EvaluationNode<P, D, K>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
    K: NodeBlueprintDefinition<P, D>,
{
    pub(in crate::graph) fn from_node(node: &Node<P, D, K>) -> Self {
        let ports = node
            .ports
            .iter()
            .map(|(port_id, port)| (*port_id, EvaluationPort::from_port(port)))
            .collect();

        EvaluationNode {
            options: node.options,
            id: node.id,
            blueprint: node.blueprint,
            ports,
            ports_names: node.ports_names.clone(),
            validity: DataValidityState::Invalid,
            runtime_state: RuntimeState::default(),
            join_handle: None,
            cancel: None,
            outputs: HashMap::new(),
            _marker: PhantomData,
        }
    }

    pub(in crate::graph) fn set_constant(
        &mut self,
        port_id: &PortId,
        constant: ConstantInputType,
    ) -> anyhow::Result<()> {
        let constant_type = self
            .ports
            .get_mut(port_id)
            .ok_or_else(|| anyhow::anyhow!("Could not find port {port_id:?}"))?
            .input_type
            .as_mut()
            .and_then(PortInputType::constant_type_mut)
            .ok_or_else(|| anyhow::anyhow!("Port {port_id:?} does not hold a constant"))?;

        *constant_type = constant;

        Ok(())
    }

    pub(crate) fn invalidate(&mut self, reason: InvalidationReason) -> anyhow::Result<()> {
        log::debug!("[{:?}] Invalidating. Reason: {:#?}", self.id, reason);
        if self.is_pending_computation() {
            self.drop_compute()?;
        }

        self.validity = match reason {
            InvalidationReason::ObsoleteData => DataValidityState::Invalid,
            InvalidationReason::EvaluationError => DataValidityState::Error { description: "" },
        };

        self.outputs.clear();
        self.runtime_state.clear();

        Ok(())
    }

    pub(crate) fn compute(&mut self) -> anyhow::Result<ComputeState<D>>
    where
        ComputeState<D>: Send,
        K: Send,
    {
        log::debug!("[{:?}] Calling compute", self.id);

        let result = if self.multithreading() {
            if self.cancel.is_some() != self.join_handle.is_some() {
                anyhow::bail!(
                    "[{:?}] Inconsistent state between thread handler and sender. \
                Irrecoverable execution state, panicking",
                    self.id
                )
            }

            if self.cancel.is_some() && self.join_handle.is_some() {
                if self.join_handle.as_ref().unwrap().is_finished() {
                    let join_handle = self.join_handle.take().unwrap();
                    let result = join_handle.join();

                    let value: anyhow::Result<ComputeState<D>> = match result {
                        Ok(Ok(state)) => match state {
                            ComputeState::Done { outputs } => Ok(ComputeState::Done { outputs }),
                            ComputeState::Running => anyhow::bail!(
                                "[{:?}] Utterly broken state with compute still marked running but thread dead",
                                self.id
                            ),
                            ComputeState::Error => Ok(ComputeState::Error),
                            ComputeState::Stopped => Ok(ComputeState::Stopped),
                        },
                        Err(e) => {
                            log::error!("[{:?}] {:?}", self.id, e);
                            Ok(ComputeState::Error)
                        }
                        Ok(Err(e)) => {
                            log::error!("[{:?}] {:?}", self.id, e);
                            Ok(ComputeState::Error)
                        }
                    };

                    self.cancel = None;

                    value
                } else {
                    Ok(ComputeState::Running)
                }
            } else {
                log::info!("[{:?}] Starting compute", self.id);
                let cancel = Arc::new(CancelSignal::default());
                self.cancel = Some(cancel.clone());

                let blueprint = self.blueprint;
                let runtime_state = self.runtime_state.clone();

                let join_handle = std::thread::spawn(move || {
                    Self::compute_thread(blueprint, runtime_state, cancel)
                });

                self.join_handle = Some(join_handle);

                Ok(ComputeState::Running)
            }
        } else {
            log::info!("[{:?}] Starting compute", self.id);
            Ok(self.blueprint.compute(&mut self.runtime_state)?)
        };

        match result {
            Ok(ComputeState::Done { outputs }) => {
                if !self.validity.is_invalid() {
                    log::debug!(
                        "[{:?}] Filling output buffers on completed compute",
                        self.id
                    );
                    self.outputs = self.declared_outputs(&outputs)?;

                    Ok(ComputeState::Done { outputs })
                } else {
                    log::debug!(
                        "[{:?}] Node invalid after completed compute, assuming computation stopped \
                        and ignoring result",
                        self.id
                    );

                    Ok(ComputeState::Stopped)
                }
            }
            Err(_) => Ok(ComputeState::Error),

            Ok(ComputeState::Running) => Ok(ComputeState::Running),
            Ok(ComputeState::Error) => Ok(ComputeState::Error),
            Ok(ComputeState::Stopped) => Ok(ComputeState::Stopped),
        }
    }

    /// Polls `compute` until it returns a terminal state, waiting `POLL_INTERVAL` between
    /// polls. If cancelled meanwhile, calls `drop_compute` and returns `ComputeState::Stopped`.
    pub(crate) fn compute_thread(
        mut blueprint: K,
        mut runtime_state: RuntimeState<D>,
        cancel: Arc<CancelSignal>,
    ) -> anyhow::Result<ComputeState<D>> {
        loop {
            match blueprint.compute(&mut runtime_state)? {
                ComputeState::Running => {
                    if cancel.wait_or_cancelled(POLL_INTERVAL) {
                        blueprint.drop_compute()?;

                        return Ok(ComputeState::Stopped);
                    }
                }
                terminal => return Ok(terminal),
            }
        }
    }

    pub(crate) fn drop_compute(&mut self) -> anyhow::Result<()> {
        log::debug!("[{:?}] Dropping running compute", self.id);

        if self.multithreading() {
            if let Some(cancel) = self.cancel.take() {
                cancel.cancel();
            }
            self.join_handle = None;
            Ok(())
        } else {
            self.blueprint.drop_compute()
        }
    }

    fn declared_outputs(
        &self,
        outputs: &HashMap<String, Arc<D>>,
    ) -> anyhow::Result<HashMap<String, Arc<D>>> {
        let mut declared = HashMap::with_capacity(outputs.len());

        for (name, output) in outputs {
            let port_id = self
                .ports_names
                .get(name)
                .ok_or_else(|| anyhow::anyhow!("No port found for name: {name:?}"))?;

            let port = self
                .ports
                .get(port_id)
                .ok_or_else(|| anyhow::anyhow!("No port found for id: {port_id:?}"))?;

            if port.direction == PortDirection::Output {
                declared.insert(name.clone(), output.clone());
            }
        }

        Ok(declared)
    }

    pub(crate) fn data_view_from_outputs(
        &mut self,
        outputs: &HashMap<String, Arc<D>>,
    ) -> Option<Arc<D>> {
        Some(outputs[&self.blueprint.port_for_viewer()?].clone())
    }

    pub(crate) fn data_view(&self) -> Option<Arc<D>> {
        self.outputs
            .get(&self.blueprint.port_for_viewer()?)
            .cloned()
    }

    pub(crate) fn custom_ui_data(&self) -> GenericStore {
        self.blueprint.prepare_custom_ui_data(&self.outputs)
    }

    pub(crate) fn output_value(&self, port_id: &PortId) -> Option<&Arc<D>> {
        let port = self.ports.get(port_id)?;

        match port.direction {
            PortDirection::Input => None,
            PortDirection::Output => self.outputs.get(&port.name),
        }
    }

    /// Whether the node holds the result of its current inputs.
    #[allow(dead_code)]
    pub fn is_valid(&self) -> bool {
        self.validity.is_valid()
    }

    /// Whether the node has no result or a stale one.
    pub fn is_invalid(&self) -> bool {
        self.validity.is_invalid()
    }

    /// Whether the node is scheduled but has not started computing.
    #[allow(dead_code)]
    pub fn is_pending_evaluation(&self) -> bool {
        self.validity.is_pending_evaluation()
    }

    /// Whether the node's computation has started and not finished.
    pub fn is_pending_computation(&self) -> bool {
        self.validity.is_pending_computation()
    }

    pub(crate) fn ports(&self) -> &BTreeMap<PortId, EvaluationPort> {
        &self.ports
    }

    #[allow(dead_code)]
    pub(crate) fn ports_mut(&mut self) -> impl Iterator<Item=(&PortId, &mut EvaluationPort)> {
        self.ports.iter_mut()
    }

    pub(crate) fn ports_get_mut(&mut self, port_id: &PortId) -> Option<&mut EvaluationPort> {
        self.ports.get_mut(port_id)
    }

    #[allow(dead_code)]
    pub(crate) fn input_ports(&self) -> impl Iterator<Item=&EvaluationPort> {
        self.ports.values().filter(|port| {
            port.is_input() && !matches!(port.input_type, Some(PortInputType::ConstantOnly { .. }))
        })
    }

    #[allow(dead_code)]
    pub(crate) fn input_ports_mut(&mut self) -> impl Iterator<Item=&mut EvaluationPort> {
        self.ports.values_mut().filter(|port| {
            port.is_input() && !matches!(port.input_type, Some(PortInputType::ConstantOnly { .. }))
        })
    }

    #[allow(dead_code)]
    pub(crate) fn input_parameters(&self) -> impl Iterator<Item=&EvaluationPort> {
        self.ports.values().filter(|port| {
            port.is_input() && matches!(port.input_type, Some(PortInputType::ConstantOnly { .. }))
        })
    }

    #[allow(dead_code)]
    pub(crate) fn input_parameters_mut(&mut self) -> impl Iterator<Item=&mut EvaluationPort> {
        self.ports.values_mut().filter(|port| {
            port.is_input() && matches!(port.input_type, Some(PortInputType::ConstantOnly { .. }))
        })
    }

    pub(crate) fn input_ports_and_parameters(&self) -> impl Iterator<Item=&EvaluationPort> {
        self.ports.values().filter(|port| port.is_input())
    }

    pub(crate) fn set_inputs_map(&mut self, inputs: HashMap<String, Arc<D>>) {
        self.runtime_state.inputs_map = InputsMap::from_iter(inputs);
    }

    pub(crate) fn extend_store(&mut self, generic_store: &GenericStore) {
        self.runtime_state.user_state.extend(generic_store.clone());
    }

    fn multithreading(&self) -> bool {
        self.blueprint.multithreading() && self.options.multithreading
    }
}
