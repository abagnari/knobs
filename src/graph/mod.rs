//! The editor widget and its evaluation runtime.
//!
//! [`Graph`] owns the nodes and connections, draws them, handles user input and
//! evaluates the graph.
//!
//! # Evaluation
//!
//! Every node carries a validity state:
//!
//! * **Invalid**: the node has no result, or its result is stale.
//! * **Pending evaluation**: the node is scheduled and will be computed once its
//!   dependencies are valid.
//! * **Pending computation**: [`compute`] returned [`crate::ComputeState::Running`] and
//!   the node is polled again by the evaluator thread after a short interval.
//! * **Valid**: the node's output buffers hold the result of its current inputs.
//! * **Error**: the inputs could not be gathered, or the computation failed.
//!
//! Editing a constant, adding a connection or removing one invalidates the
//! affected node and everything downstream of it. Running computations on those
//! nodes are dropped through [`drop_compute`].
//!
//! Nothing is recomputed until a node is focused. Evaluation runs on a dedicated
//! evaluator thread, independently of frames. Focusing a node that is not valid
//! schedules it: invalid upstream nodes are added and dependencies are computed
//! before the nodes that read them. A node returning
//! [`crate::ComputeState::Running`] is polled again about every 20 ms. Results
//! reach the editor on the next call to [`Graph::show`], and the evaluator
//! requests a repaint. Once the focused node becomes valid, its viewer output is
//! passed to [`GraphEventHandler::on_focused_output_change`].
//!
//! A failure moves the failing node and every node downstream of it to the error
//! state and abandons the schedule. [`crate::ComputeState::Stopped`] abandons the schedule
//! as well, but leaves the nodes invalid instead of erroneous.
//!
//! A graph containing a cycle is not evaluated: the nodes on the cycle are put in
//! the error state until a connection breaking the cycle is removed.
//!
//! [`compute`]: NodeBlueprintDefinition::compute
//! [`drop_compute`]: NodeBlueprintDefinition::drop_compute

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::thread::JoinHandle;

use canvas::Canvas;
use connection::{Connections, DragConnection};
use context_menu::ContextMenu;
use egui::layers::ShapeIdx;
use egui::{Color32, InnerResponse, Pos2, Rect, Response, Sense, Shape, Ui};
use node::port::{ConstantInputType, Port, PortDirection, PortId, PortInputType, PortTypeInfo};
use node::{Node, NodeBlueprintDefinition, NodeId};
use painter::{NodeSelectedState, Painter};
use slotmap::{SecondaryMap, SlotMap};

use crate::GenericStore;
use crate::graph::evaluator::Evaluator;
use crate::graph::evaluator::actions::{Command, Event};
use crate::graph::node::EvaluationNode;
use crate::graph::node::port::{PortLinkageType, PortOutputType};
use crate::graph::snapshot::{ConnectionSnapshot, GraphSnapshot, SNAPSHOT_VERSION, SnapshotIds};
use crate::options::Options;

pub(crate) mod canvas;
mod connection;
mod context_menu;
mod evaluator;
pub mod node;
pub(crate) mod painter;
pub mod snapshot;

#[allow(dead_code)]
#[derive(Debug)]
struct NodeState {
    rect: Rect,
    response: Response,
    ports_states: HashMap<PortId, PortState>,
    focus_button_response: Option<Response>,
}

#[allow(dead_code)]
#[derive(Debug)]
struct PortState {
    rect: Option<Rect>,
    response: Option<Response>,
    input_response: Option<Response>,
    connections_states: HashMap<PortId, ConnectionState>,
    color: Color32,
}

#[allow(dead_code)]
#[derive(Debug)]
struct ConnectionState {
    pub curve_shape_idx: ShapeIdx,
    pub handle_shape_idx: ShapeIdx,
    pub from_port: PortId,
    pub to_port: PortId,
}

/// Receives notifications from a [`Graph`] about evaluation results.
///
/// Every method has a default implementation that does nothing, so an
/// implementor only overrides what it needs.
pub trait GraphEventHandler<D>: Send + Sync
where
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
{
    /// Called when the value shown for the focused node changes.
    ///
    /// `data` is the value of the output named by
    /// [`port_for_viewer`](NodeBlueprintDefinition::port_for_viewer), or `None`
    /// when the node has no viewer port or that output is empty. The call happens
    /// when a focused node finishes computing, and immediately when an already
    /// valid node gains focus.
    ///
    /// It is called on the evaluator thread, not on the UI thread: request a
    /// repaint or forward the value to the UI yourself.
    ///
    /// # Errors
    ///
    /// A returned error is logged and otherwise ignored; the node's result is
    /// kept either way.
    fn on_focused_output_change(&self, _data: Option<Arc<D>>) -> anyhow::Result<()> {
        Ok(())
    }
}

const FILE_MENU_LABEL: &str = "File";
const NEW_MENU_LABEL: &str = "New";
const SAVE_MENU_LABEL: &str = "Save";
const LOAD_MENU_LABEL: &str = "Load";

/// A command chosen in the menu bar drawn by [`Graph::show_menu_bar`].
///
/// The graph only reports the choice. Picking a file and reading or writing it
/// is left to the host, through [`Graph::snapshot`] and [`Graph::load`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuEvent {
    /// The user asked to start a new, empty graph.
    New,
    /// The user asked to save the graph.
    Save,
    /// The user asked to load a graph.
    Load,
}

/// What [`Graph::show_menu_bar`] produced during one frame.
pub struct MenuBarOutput<R> {
    /// The command chosen this frame, if any. It is `Some` only on the frame the
    /// menu item is clicked.
    pub event: Option<MenuEvent>,
    /// The value returned by the host's own menu contents.
    pub inner: R,
}

/// The node graph editor.
///
/// # Type parameters
///
/// * `P` - the port type, see [`PortTypeInfo`].
/// * `D` - the value type carried by connections.
/// * `K` - the blueprint type listing every node kind, see
///   [`NodeBlueprintDefinition`].
/// * `A` - the handler receiving evaluation results, see [`GraphEventHandler`].
///
/// Create it once with [`Graph::new`], keep it in the application state and call
/// [`Graph::show`] every frame.
pub struct Graph<P, D, K, A>
where
    P: PortTypeInfo,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
    K: NodeBlueprintDefinition<P, D>,
    A: GraphEventHandler<D>,
{
    options: Options,
    painter: Painter,
    canvas: Canvas,

    nodes: SlotMap<NodeId, Node<P, D, K>>,
    connections: Connections,

    dragged_connection: Option<DragConnection<P>>,
    dragging_node: Option<NodeId>,
    selected_node: Option<NodeId>,
    selected_connection: Option<PortId>,
    spawned_node: Option<K>,
    focused_node: Option<NodeId>,
    hovered_port: Option<PortId>,

    context_menu_position: Option<Pos2>,
    context_menu: ContextMenu<P, D, K>,

    first_frame: bool,
    evaluator_join_handle: Option<JoinHandle<()>>,
    sender: Sender<Command<P, D, K>>,
    receiver: Receiver<Event>,
    _marker: PhantomData<fn() -> A>,
}

impl<P, D, K, A> Graph<P, D, K, A>
where
    P: PortTypeInfo + 'static,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
    K: NodeBlueprintDefinition<P, D>,
    A: GraphEventHandler<D> + 'static,
{
    /// Creates an empty graph and registers every blueprint of `K` in the node
    /// creation menu.
    pub fn new(options: Options, event_handler: A) -> Self {
        let painter = Painter::new(options, options.theme);
        let mut context_menu = ContextMenu::default();

        for blueprint in K::all() {
            log::debug!("[Graph] Register {blueprint:?}");
            context_menu.register_node_blueprint(*blueprint);
        }

        let (command_sender, command_receiver) = channel::<Command<P, D, K>>();
        let (event_sender, event_receiver) = channel::<Event>();

        let nodes = SlotMap::with_key();
        let connections = Connections::default();
        let mut evaluator = Evaluator::new(
            options,
            SecondaryMap::new(),
            connections.clone(),
            None,
            GenericStore::default(),
            event_handler,
            command_receiver,
            event_sender,
        );

        let evaluator_join_handle = thread::Builder::new()
            .name("Evaluator Thread".to_string())
            .spawn(move || evaluator.run_thread())
            .expect("The operating system could not start the evaluator thread");

        Self {
            options,
            painter,
            canvas: Canvas::default(),
            nodes,
            dragged_connection: None,
            connections,
            dragging_node: None,
            selected_node: None,
            selected_connection: None,
            spawned_node: None,
            focused_node: None,
            hovered_port: None,
            context_menu_position: None,
            context_menu,
            first_frame: true,
            evaluator_join_handle: Some(evaluator_join_handle),
            sender: command_sender,
            receiver: event_receiver,
            _marker: PhantomData,
        }
    }

    /// Captures the nodes, their constants, the connections and the canvas
    /// position.
    ///
    /// Computed results, the focus, the selection and running computations are
    /// not part of the snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when a connection refers to a node that no longer exists,
    /// which indicates corrupted internal state.
    pub fn snapshot(&self) -> anyhow::Result<GraphSnapshot<K>> {
        let ids = SnapshotIds::assign(self.nodes.keys());

        let nodes = self
            .nodes
            .iter()
            .map(|(node_id, node)| Ok(node.snapshot(ids.snapshot_node(node_id)?)))
            .collect::<anyhow::Result<Vec<_>>>()?;

        let connections = self
            .connections
            .iter()
            .map(|connection| ConnectionSnapshot::new(&connection, &ids))
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(GraphSnapshot {
            version: SNAPSHOT_VERSION,
            nodes,
            connections,
            canvas: self.canvas.snapshot(),
        })
    }

    /// Creates a graph as [`Graph::new`] does and loads `snapshot` into it.
    ///
    /// # Errors
    ///
    /// Fails for the same reasons as [`Graph::load`].
    pub fn from_snapshot(
        snapshot: GraphSnapshot<K>,
        options: Options,
        event_handler: A,
    ) -> anyhow::Result<Self> {
        let mut graph = Self::new(options, event_handler);
        graph.load(snapshot)?;

        Ok(graph)
    }

    /// Replaces the contents of the graph with `snapshot`.
    ///
    /// Ports are matched by name against the current blueprints, so a snapshot
    /// stays loadable when a blueprint gains ports; ports that no longer exist are
    /// dropped. Running computations are dropped and every node starts invalid.
    ///
    /// # Errors
    ///
    /// Returns an error when the snapshot has an unsupported version, lists the
    /// same node twice, or has a connection that refers to a missing node or port,
    /// connects incompatible ports, or targets an input that is already
    /// connected. It also fails when a saved constant no longer matches its port
    /// (a different kind, or a port that no longer holds a constant), when a saved
    /// selection index is out of range, or when a blueprint declares the same port
    /// name twice. The graph is cleared before the snapshot is read, so after an
    /// error it holds whatever was loaded up to that point.
    pub fn load(&mut self, snapshot: GraphSnapshot<K>) -> anyhow::Result<()> {
        if snapshot.version != SNAPSHOT_VERSION {
            anyhow::bail!("Unsupported snapshot version: {}", snapshot.version);
        }

        self.reset()?;

        self.canvas = Canvas::from_snapshot(snapshot.canvas);

        let mut ids = SnapshotIds::default();

        for node_snapshot in snapshot.nodes {
            let snapshot_id = node_snapshot.id;
            let options = self.options;
            let node_id = self.nodes.try_insert_with_key(|node_id| {
                Node::from_snapshot(options, node_id, node_snapshot)
            })?;

            if !ids.bind(snapshot_id, node_id) {
                anyhow::bail!("Duplicate node in snapshot: {snapshot_id:?}");
            }

            if let Err(error) = self.sender.send(Command::AddNode {
                node: EvaluationNode::from_node(&self.nodes[node_id]),
            }) {
                log::error!("[Graph] Failed to add node to evaluator: {error:?}");
            }
        }

        for connection in snapshot.connections {
            self.restore_connection(
                ids.runtime_port(connection.from)?,
                ids.runtime_port(connection.to)?,
            )?;
        }

        Ok(())
    }

    /// Stores a value that every node finds in its
    /// [`user_state`](crate::RuntimeState::user_state) when an evaluation starts.
    ///
    /// Use it to hand nodes shared resources owned by the host, such as a device
    /// handle or a cache. Defaults are copied into a node before each evaluation
    /// and replace entries with the same key.
    pub fn add_default_compute_entry<T>(&mut self, key: impl Into<String>, value: T)
    where
        T: Any + Clone + Debug + Send + Sync,
    {
        let mut entries = GenericStore::default();
        entries.insert(key, value);

        if let Err(error) = self.sender.send(Command::AddDefaultEntries { entries }) {
            log::error!("[Graph] Failed to add default compute entry to evaluator: {error:?}");
        }
    }

    fn focus_node(&mut self, node_id: NodeId) -> anyhow::Result<()> {
        if let Some(previous_id) = self
            .focused_node
            .filter(|previous_id| *previous_id != node_id)
            && let Some(previous_node) = self.nodes.get_mut(previous_id)
        {
            previous_node.focused = false;
        }

        self.nodes
            .get_mut(node_id)
            .ok_or_else(|| anyhow::anyhow!("Focused node {node_id:?} does not exist"))?
            .focused = true;
        self.focused_node = Some(node_id);

        if let Err(error) = self.sender.send(Command::FocusNode { node_id }) {
            log::error!("[Graph] Failed to add node focus to evaluator: {error:?}");
        }

        Ok(())
    }

    fn unfocus_node(&mut self) {
        if let Some(previous_id) = self.focused_node.take()
            && let Some(previous_node) = self.nodes.get_mut(previous_id)
        {
            previous_node.focused = false;
        }

        if let Err(error) = self.sender.send(Command::UnfocusNode) {
            log::error!("[Graph] Failed to delete node focus from evaluator: {error:?}");
        }
    }

    fn reset(&mut self) -> anyhow::Result<()> {
        if let Err(error) = self.sender.send(Command::Reset) {
            log::error!("[Graph] Failed to request reset from evaluator: {error:?}");
        }

        self.nodes.clear();
        self.connections.clear();
        self.dragged_connection = None;
        self.dragging_node = None;
        self.selected_node = None;
        self.selected_connection = None;
        self.spawned_node = None;
        self.focused_node = None;
        self.hovered_port = None;
        self.context_menu_position = None;
        self.first_frame = true;

        Ok(())
    }

    #[allow(dead_code)]
    fn validate_blueprints() -> anyhow::Result<()> {
        for blueprint in K::all() {
            let mut names = HashSet::new();

            for definition in blueprint.ports() {
                if !names.insert(definition.name.clone()) {
                    anyhow::bail!("Duplicate port {} in {blueprint:?}", definition.name);
                }
            }

            if let Some(viewer) = blueprint.port_for_viewer()
                && !names.contains(&viewer)
            {
                anyhow::bail!("{blueprint:?} views unknown port {viewer}");
            }
        }

        Ok(())
    }

    fn port(&self, port_id: PortId) -> anyhow::Result<&Port<P>> {
        self.nodes
            .get(port_id.node_id())
            .ok_or_else(|| anyhow::anyhow!("Unknown node in snapshot: {port_id:?}"))?
            .ports()
            .get(&port_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown port in snapshot: {port_id:?}"))
    }

    pub(crate) fn create_node(
        &mut self,
        name: String,
        node_blueprint: K,
        position: Pos2,
    ) -> anyhow::Result<&Node<P, D, K>> {
        let options = self.options;
        let id = self
            .nodes
            .try_insert_with_key(|id| Node::new(options, id, name, node_blueprint, position))?;
        log::debug!("[Graph] Created new node with id {id:?}, blueprint {node_blueprint:?}");

        if let Err(error) = self.sender.send(Command::AddNode {
            node: EvaluationNode::from_node(&self.nodes[id]),
        }) {
            log::error!("[Graph] Failed to add node to evaluator: {error:?}");
        }

        Ok(&self.nodes[id])
    }

    fn are_ports_compatible(
        &self,
        from_port_id: PortId,
        dragging_connection: &DragConnection<P>,
    ) -> bool {
        if from_port_id.node_id() == dragging_connection.from_port.node_id() {
            return false;
        }

        self.nodes[from_port_id.node_id()].ports()[&from_port_id]
            .is_connection_compatible(dragging_connection)
    }

    fn create_connection(
        &mut self,
        dragged_connection: &DragConnection<P>,
        to_port: PortId,
    ) -> anyhow::Result<()> {
        let (from_port, to_port) = if dragged_connection.is_input() {
            (to_port, dragged_connection.from_port)
        }
        else {
            (dragged_connection.from_port, to_port)
        };

        // Delete any previous connection that targeted the same input port
        self.delete_connection_to_port(to_port)?;

        log::debug!("[{:?}] Adding connection to {:?}", from_port, to_port);

        self.insert_connection(from_port, to_port)?;

        Ok(())
    }

    fn delete_node(&mut self, node_id: NodeId) -> anyhow::Result<()> {
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

        for port_id in outbound_connections.iter() {
            self.delete_connection_to_port(*port_id)?;
        }
        self.nodes.remove(node_id);

        if let Err(error) = self.sender.send(Command::DeleteNode { node_id }) {
            log::error!("[Graph] Failed to delete node from evaluator: {error:?}");
        }

        Ok(())
    }

    fn delete_connection_to_port(&mut self, to_port_id: PortId) -> anyhow::Result<()> {
        log::debug!("[Graph] Deleting all connections to {to_port_id:?}");

        if let Some(connection) = self.connections.remove(to_port_id) {
            if Some(to_port_id) == self.selected_connection {
                self.selected_connection = None;
            }

            log::debug!("[Graph] Deleting data_source in {to_port_id:?}");
            match self
                .nodes
                .get_mut(connection.to_port.node_id())
                .ok_or_else(|| anyhow::anyhow!("Could not find node"))?
                .ports_get_mut(&connection.to_port)
                .ok_or_else(|| anyhow::anyhow!("Could not find port"))?
                .linkage_type
            {
                PortLinkageType::Input(PortInputType::ConnectionAndConstant {
                    ref mut data_source,
                    ..
                })
                | PortLinkageType::Input(PortInputType::ConnectionOnly {
                    ref mut data_source,
                    ..
                }) => {
                    *data_source = None;
                }
                PortLinkageType::Output(PortOutputType::ConstantOnly { ref mut text_value })
                | PortLinkageType::Output(PortOutputType::ConnectionAndConstant {
                    ref mut text_value,
                }) => {
                    *text_value = None;
                }
                _ => {}
            }
        }

        if let Err(error) = self
            .sender
            .send(Command::DeleteConnection { to: to_port_id })
        {
            log::error!("[Graph] Failed to delete connection from evaluator: {error:?}");
        }

        Ok(())
    }

    fn restore_connection(&mut self, from_port: PortId, to_port: PortId) -> anyhow::Result<()> {
        let from = self.port(from_port)?;
        let to = self.port(to_port)?;

        if !matches!(from.direction, PortDirection::Output) {
            anyhow::bail!("Snapshot connection source {from_port:?} is not an output port");
        }

        if !to.is_input() {
            anyhow::bail!("Snapshot connection target {to_port:?} is not an input port");
        }

        if from.port_type != to.port_type {
            anyhow::bail!("Incompatible port types between {from_port:?} and {to_port:?}");
        }

        if self.connections.contains_target(to_port) {
            anyhow::bail!("Duplicate connection to {to_port:?}");
        }

        self.insert_connection(from_port, to_port)
    }

    fn insert_connection(&mut self, from_port: PortId, to_port: PortId) -> anyhow::Result<()> {
        self.connections.insert(from_port, to_port);

        match self
            .nodes
            .get_mut(to_port.node_id())
            .ok_or_else(|| anyhow::anyhow!("Could not find node"))?
            .ports_get_mut(&to_port)
            .ok_or_else(|| anyhow::anyhow!("Could not find port"))?
            .linkage_type
        {
            PortLinkageType::Input(PortInputType::ConnectionAndConstant {
                ref mut data_source,
                ..
            })
            | PortLinkageType::Input(PortInputType::ConnectionOnly {
                ref mut data_source,
                ..
            }) => {
                *data_source = Some(from_port);
            }
            _ => {}
        }

        if let Err(error) = self.sender.send(Command::AddConnection {
            from: from_port,
            to: to_port,
        }) {
            log::error!("[Graph] Failed to add connection to evaluator: {error:?}");
        }
        Ok(())
    }

    /// Draws a menu bar with a File menu offering New, Save and Load, followed by the
    /// host's own menus added in `add_contents`.
    ///
    /// Call it every frame, before [`Graph::show`]. The returned
    /// [`MenuBarOutput::event`] reports the command chosen this frame.
    pub fn show_menu_bar<R>(
        &self,
        ui: &mut Ui,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> InnerResponse<MenuBarOutput<R>> {
        self.painter.show_menu_bar(ui, |ui| MenuBarOutput {
            event: Self::show_file_menu(ui),
            inner: add_contents(ui),
        })
    }

    fn show_file_menu(ui: &mut Ui) -> Option<MenuEvent> {
        let mut event = None;

        ui.menu_button(FILE_MENU_LABEL, |ui| {
            if ui.button(NEW_MENU_LABEL).clicked() {
                event = Some(MenuEvent::New);
            }

            if ui.button(SAVE_MENU_LABEL).clicked() {
                event = Some(MenuEvent::Save);
            }

            if ui.button(LOAD_MENU_LABEL).clicked() {
                event = Some(MenuEvent::Load);
            }
        });

        event
    }

    /// Applies the evaluator's latest results, draws the graph and handles user
    /// input for one frame.
    ///
    /// Call it once per frame. The graph fills the available space of
    /// `parent_ui`. The user can:
    ///
    /// * drag the background to pan and scroll to zoom,
    /// * right click the background to open the node creation menu,
    /// * drag nodes to move them,
    /// * drag from a port to a compatible port to connect them,
    /// * click a node or a connection to select it, and press Delete to remove it,
    /// * press a node's eye button to focus it and start evaluation.
    ///
    /// # Panics
    ///
    /// Panics when the graph reaches an inconsistent internal state, for instance
    /// a connection to a node that does not exist, or when a blueprint picked from
    /// the creation menu declares invalid ports, such as duplicate names or
    /// invalid enablers.
    pub fn show(&mut self, parent_ui: &mut Ui) {
        self.drain_events();

        let mut nodes_states: HashMap<NodeId, NodeState> = HashMap::new();
        let mut connections_states: Vec<ConnectionState> = Vec::new();

        let canvas_rect = parent_ui.available_rect_before_wrap();

        let mut canvas_ui = self
            .painter
            .scoped_child(parent_ui, self.canvas.current_zoom());
        let ui = &mut canvas_ui;
        ui.set_clip_rect(ui.max_rect());

        let pointer_position = ui.input(|i| i.pointer.hover_pos().unwrap_or(Pos2::ZERO));

        if self.first_frame {
            if let Err(error) = self.sender.send(Command::AttachContext {
                context: parent_ui.ctx().clone(),
            }) {
                log::error!(
                    "[Graph] Error while sending attach context to the sender: {:?}",
                    error
                );
            }

            self.canvas.zoom(pointer_position, ui.clip_rect(), 0.0);
            self.first_frame = false;
        }

        self.painter.show_grid(ui, &self.canvas, canvas_rect);

        // Canvas Pan
        let window_response = ui.interact(
            canvas_rect,
            ui.scope_id().with("canvas"),
            Sense::click_and_drag(),
        );

        // Canvas Zoom
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);

        if scroll != 0.0
            && let Some(interact_position) = ui.pointer_interact_pos()
        {
            self.canvas.zoom(interact_position, ui.clip_rect(), scroll)
        }

        let mut hovered_port: Option<PortId> = None;

        for connection in self.connections.iter() {
            connections_states.push(ConnectionState {
                curve_shape_idx: ui.painter().add(Shape::Noop),
                handle_shape_idx: ui.painter().add(Shape::Noop),
                to_port: connection.to_port,
                from_port: connection.from_port,
            });
        }

        let drawing_order: Vec<NodeId> = self.nodes.keys().collect();

        for node_id in drawing_order.into_iter().rev() {
            let node = &mut self.nodes[node_id];
            let (node_state, hover) = self.painter.show_node(
                ui,
                &self.canvas,
                node,
                if self.selected_node == Some(node_id) {
                    NodeSelectedState::Selected
                }
                else {
                    NodeSelectedState::NotSelected
                },
                node.validity,
                &self.dragged_connection,
            );

            hovered_port = hovered_port.or(hover);

            nodes_states.insert(node_id, node_state);
        }

        self.hovered_port = hovered_port;

        for (node_id, node_state) in nodes_states.iter() {
            if node_state.response.drag_started() {
                self.dragging_node = Some(*node_id);
            }

            if node_state.response.drag_stopped() {
                self.dragging_node = None;
            }

            if node_state.response.clicked() {
                self.selected_node = Some(*node_id);
            }

            if node_state.response.clicked_elsewhere() && self.selected_node == Some(*node_id) {
                self.selected_node = None;
            }

            if self.options.show_focus_button
                && let Some(focus_button_response) = &node_state.focus_button_response
                && focus_button_response.changed()
            {
                if self.nodes[*node_id].focused {
                    self.focus_node(*node_id).unwrap();
                }
                else if self.focused_node == Some(*node_id) {
                    self.unfocus_node();
                }
                // The absence of a response with the boolean option enabled probably warrants a
                // warning of some kind
            }

            if self.dragging_node == Some(*node_id) && node_state.response.dragged() {
                {
                    let node = self.nodes.get_mut(*node_id).unwrap();
                    node.position += self.canvas.zoomed_out(node_state.response.drag_delta());
                }
            }

            for (port_id, port_state) in node_state.ports_states.iter() {
                if let Some(port_response) = &port_state.response {
                    if port_response.drag_started() {
                        self.dragged_connection = Some(DragConnection {
                            from_type: self.nodes[*node_id].ports()[port_id].port_type,
                            from_port: *port_id,
                            from_position: port_state.rect.unwrap().center(),
                            from_input: matches!(
                                self.nodes[*node_id].ports()[port_id].direction,
                                PortDirection::Input
                            ),
                        });
                    }

                    if port_response.drag_stopped()
                        && let Some(dragged_connection) = self.dragged_connection.take()
                        && let Some(to_port_id) = self.hovered_port
                        && self.are_ports_compatible(to_port_id, &dragged_connection)
                    {
                        self.create_connection(&dragged_connection, to_port_id)
                            .unwrap();
                    }
                }

                if let Some(port_input_response) = &port_state.input_response
                    && port_input_response.changed()
                {
                    log::debug!("Forwarding the constant of {port_id:?} to the evaluator");

                    let constant = match self.nodes[*node_id].ports()[port_id].linkage_type {
                        PortLinkageType::Input(ref input_type) => {
                            input_type.constant_type().cloned()
                        }
                        PortLinkageType::Output(_) => None,
                    };

                    if let Some(constant) = constant
                        && let Err(error) = self.sender.send(Command::SetConstant {
                            port_id: *port_id,
                            constant,
                        })
                    {
                        log::error!("[Graph] Failed to set constant in evaluator: {error:?}");
                    }
                }
            }
        }

        for connection_state in connections_states {
            let handle_rect = self.painter.show_connection(
                ui,
                &self.canvas,
                connection_state.curve_shape_idx,
                nodes_states[&connection_state.from_port.node_id()].ports_states
                    [&connection_state.from_port]
                    .rect
                    .unwrap()
                    .center(),
                nodes_states[&connection_state.to_port.node_id()].ports_states
                    [&connection_state.to_port]
                    .rect
                    .unwrap()
                    .center(),
                Some(connection_state.to_port) == self.selected_connection,
            );

            let handle_response = ui.interact(
                handle_rect,
                ui.scope_id()
                    .with((connection_state.from_port, connection_state.to_port)),
                Sense::click() | Sense::hover(),
            );

            if handle_response.clicked() {
                self.selected_node = None;
                self.selected_connection = Some(connection_state.to_port);
            }

            self.painter.show_connection_handle(
                ui,
                &self.canvas,
                handle_rect,
                handle_response,
                connection_state.handle_shape_idx,
                Some(connection_state.to_port) == self.selected_connection,
            )
        }

        if window_response.dragged()
            && self.dragged_connection.is_none()
            && self.dragging_node.is_none()
        {
            self.canvas.pan(window_response.drag_delta());
        }

        if window_response.secondary_clicked()
            && let Some(interact_position) = ui.pointer_interact_pos()
        {
            self.context_menu.open_at_position(interact_position);
            self.context_menu_position = Some(interact_position);
        }

        if window_response.clicked() {
            self.context_menu.close();
            self.context_menu_position = None;
        }

        // Keyboard checks
        if ui.input(|i| i.key_pressed(egui::Key::Delete)) && !ui.ctx().egui_wants_keyboard_input() {
            if let Some(node_id) = self.selected_node.take() {
                self.delete_node(node_id).unwrap();
            }

            if let Some(port_id) = self.selected_connection.take() {
                self.delete_connection_to_port(port_id).unwrap();
            }
        }

        // Context Menu
        if self.context_menu_position.is_some() {
            self.spawned_node = self.context_menu.show(ui, &self.painter);
        }

        if let Some(spawned_node) = self.spawned_node.take() {
            self.create_node(
                spawned_node.name().to_string(),
                spawned_node,
                self.canvas
                    .screen_to_world_coordinates(self.context_menu_position.unwrap()),
            )
            .expect("Could not create node");

            self.context_menu_position = None;
        }

        parent_ui.advance_cursor_after_rect(canvas_ui.min_rect());
    }

    fn drain_events(&mut self) {
        for event in self.receiver.try_iter() {
            match event {
                Event::ValidityChanged { node_id, validity } => {
                    if let Some(node) = self.nodes.get_mut(node_id) {
                        if node.validity.is_valid() && !validity.is_valid() {
                            node.clear_outputs_text();
                            node.clear_custom_ui_state();
                        }

                        node.validity = validity;
                    }
                }
                Event::ResultReady {
                    node_id,
                    data,
                    outputs_text,
                } => {
                    if let Some(node) = self.nodes.get_mut(node_id) {
                        node.set_outputs_text(outputs_text);
                        node.set_custom_ui_state(data);
                    }
                }
                Event::Failed { node_id, error } => {
                    log::error!("[Graph] Evaluation failed for {node_id:?}: {error:?}");
                }
            }
        }
    }
}

impl<P, D, K, A> Drop for Graph<P, D, K, A>
where
    P: PortTypeInfo,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
    K: NodeBlueprintDefinition<P, D>,
    A: GraphEventHandler<D>,
{
    fn drop(&mut self) {
        if self.sender.send(Command::Shutdown).is_err() {
            log::warn!("[Graph] Evaluator thread had already stopped");
        }

        if let Some(handle) = self.evaluator_join_handle.take()
            && handle.join().is_err()
        {
            log::error!("[Graph] Evaluator thread panicked");
        }
    }
}

#[cfg(test)]
mod tests;
