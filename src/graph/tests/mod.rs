use egui::{Color32, Context, Pos2, RawInput};
use serde::{Deserialize, Serialize};

use super::*;
use crate::graph::evaluator::actions::Event;
use crate::graph::node::DataValidityState;
use crate::graph::node::port::{PortOutputType, port_id};
use crate::{ComputeState, PortDefinition, RuntimeState, input_port, output_port};

pub mod custom_ui;
pub mod fixtures;
pub mod threading;

const SNAPSHOT_V1_FIXTURE: &str = include_str!("snapshot_v1_fixture.ron");
const INPUT_PORT: u32 = 0;
const SECOND_INPUT_PORT: u32 = 1;
const PASS_OUTPUT_PORT: u32 = 1;
const FIRST_NODE: usize = 0;
const SECOND_NODE: usize = 1;
const EXPECTED_NODE_COUNT: usize = 2;
const EXPECTED_CONNECTION_COUNT: usize = 1;
const STORE_KEY: &str = "preview";
const STORE_VALUE: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestPortType {
    Number,
}

impl PortTypeInfo for TestPortType {
    fn color(&self) -> Color32 {
        Color32::WHITE
    }
}

#[derive(Debug, Clone, PartialEq)]
struct TestValue;

impl TryFrom<&TestValue> for String {
    type Error = ();

    fn try_from(_: &TestValue) -> Result<Self, Self::Error> {
        Err(())
    }
}

impl From<&ConstantInputType> for TestValue {
    fn from(_: &ConstantInputType) -> Self {
        TestValue
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum TestBlueprint {
    Pass,
    Dual,
}

impl NodeBlueprintDefinition<TestPortType, TestValue> for TestBlueprint {
    fn all() -> &'static [Self] {
        &[TestBlueprint::Pass, TestBlueprint::Dual]
    }

    fn name(&self) -> &'static str {
        match self {
            TestBlueprint::Pass => "Pass",
            TestBlueprint::Dual => "Dual",
        }
    }

    fn ports(&self) -> Vec<PortDefinition<TestPortType, TestValue>> {
        let number = TestPortType::Number;

        match self {
            TestBlueprint::Pass => vec![
                input_port("in", "in", number, PortInputType::connection_only(), false),
                output_port("out", "out", number, PortOutputType::ConnectionOnly, true),
            ],
            TestBlueprint::Dual => vec![
                input_port(
                    "first",
                    "first",
                    number,
                    PortInputType::connection_only(),
                    false,
                ),
                input_port(
                    "second",
                    "second",
                    number,
                    PortInputType::connection_only(),
                    false,
                ),
                output_port("out", "out", number, PortOutputType::ConnectionOnly, true),
            ],
        }
    }

    fn categories(&self) -> Vec<&'static str> {
        vec![""]
    }

    fn compute(
        &mut self,
        _: &mut RuntimeState<TestValue>,
    ) -> anyhow::Result<ComputeState<TestValue>> {
        Ok(ComputeState::Running)
    }

    fn port_for_viewer(&self) -> Option<String> {
        None
    }
}

struct TestHandler;

impl GraphEventHandler<TestValue> for TestHandler {}

type TestGraph = Graph<TestPortType, TestValue, TestBlueprint, TestHandler>;

fn empty_graph() -> TestGraph {
    Graph::new(Options::default(), TestHandler)
}

fn add_node(graph: &mut TestGraph, blueprint: TestBlueprint) -> NodeId {
    graph
        .create_node(blueprint.name().to_string(), blueprint, Pos2::ZERO)
        .unwrap()
        .id
}

fn connect(graph: &mut TestGraph, source: NodeId, target: NodeId, target_port: u32) {
    graph
        .insert_connection(
            port_id(source, PASS_OUTPUT_PORT),
            port_id(target, target_port),
        )
        .unwrap();
}

fn connected_pair() -> (TestGraph, NodeId, NodeId) {
    let mut graph = empty_graph();
    let source = add_node(&mut graph, TestBlueprint::Pass);
    let target = add_node(&mut graph, TestBlueprint::Pass);
    connect(&mut graph, source, target, INPUT_PORT);

    (graph, source, target)
}

fn graph_with_event_sender() -> (TestGraph, Sender<Event>) {
    let mut graph = empty_graph();
    let (sender, receiver) = channel();
    graph.receiver = receiver;

    (graph, sender)
}

fn store_with_entry() -> GenericStore {
    let mut store = GenericStore::default();
    store.insert(STORE_KEY, STORE_VALUE);

    store
}

fn snapshot_with_nodes(count: usize) -> GraphSnapshot<TestBlueprint> {
    let mut graph = empty_graph();

    for _ in 0..count {
        add_node(&mut graph, TestBlueprint::Pass);
    }

    graph.snapshot().unwrap()
}

#[test]
fn delete_node_with_reused_slot_leaves_old_id_invalid() {
    let mut graph = empty_graph();
    let deleted = add_node(&mut graph, TestBlueprint::Pass);
    graph.delete_node(deleted).unwrap();

    let replacement = add_node(&mut graph, TestBlueprint::Pass);

    assert_ne!(deleted, replacement);
    assert!(graph.nodes.get(deleted).is_none());
}

#[test]
fn delete_connection_to_port_with_parallel_connection_keeps_the_dependency() {
    let mut graph = empty_graph();
    let source = add_node(&mut graph, TestBlueprint::Pass);
    let target = add_node(&mut graph, TestBlueprint::Dual);
    connect(&mut graph, source, target, INPUT_PORT);
    connect(&mut graph, source, target, SECOND_INPUT_PORT);

    graph
        .delete_connection_to_port(port_id(target, INPUT_PORT))
        .unwrap();

    assert_eq!(graph.connections.upstream_nodes(target), vec![source]);
}

#[test]
fn delete_node_with_connections_removes_inbound_and_outbound() {
    let (mut graph, _, middle) = connected_pair();
    let downstream = add_node(&mut graph, TestBlueprint::Pass);
    connect(&mut graph, middle, downstream, INPUT_PORT);

    graph.delete_node(middle).unwrap();

    assert_eq!(graph.connections.iter().count(), 0);
}

#[test]
fn load_snapshot_from_before_slotmap_ids_succeeds() {
    let snapshot = GraphSnapshot::<TestBlueprint>::from_ron(SNAPSHOT_V1_FIXTURE).unwrap();
    let mut graph = empty_graph();

    let result = graph.load(snapshot);

    assert!(result.is_ok());
    assert_eq!(graph.nodes.len(), EXPECTED_NODE_COUNT);
    assert_eq!(graph.connections.iter().count(), EXPECTED_CONNECTION_COUNT);
}

#[test]
fn from_snapshot_with_v1_file_returns_loaded_graph() {
    let snapshot = GraphSnapshot::<TestBlueprint>::from_ron(SNAPSHOT_V1_FIXTURE).unwrap();

    let result = TestGraph::from_snapshot(snapshot, Options::default(), TestHandler);

    let graph = result.unwrap();
    assert_eq!(graph.nodes.len(), EXPECTED_NODE_COUNT);
    assert_eq!(graph.connections.iter().count(), EXPECTED_CONNECTION_COUNT);
}

#[test]
fn show_menu_bar_without_clicks_returns_no_event_and_host_contents() {
    const HOST_VALUE: u32 = 7;
    let graph = empty_graph();
    let mut menu_output = None;

    let mut frame_output = Context::default().run_ui(RawInput::default(), |ui| {
        menu_output = Some(graph.show_menu_bar(ui, |_| HOST_VALUE).inner);
    });
    frame_output.textures_delta.clear();

    let menu_output = menu_output.unwrap();
    assert!(menu_output.event.is_none());
    assert_eq!(menu_output.inner, HOST_VALUE);
}

#[test]
fn snapshot_after_loading_v1_file_reproduces_the_file() {
    let snapshot = GraphSnapshot::<TestBlueprint>::from_ron(SNAPSHOT_V1_FIXTURE).unwrap();
    let mut graph = empty_graph();
    graph.load(snapshot).unwrap();

    let saved = graph.snapshot().unwrap().to_ron().unwrap();

    assert_eq!(saved.trim(), SNAPSHOT_V1_FIXTURE.trim());
}

#[test]
fn load_with_duplicate_node_ids_returns_error() {
    let mut snapshot = snapshot_with_nodes(EXPECTED_NODE_COUNT);
    snapshot.nodes[SECOND_NODE].id = snapshot.nodes[FIRST_NODE].id;
    let mut graph = empty_graph();

    let result = graph.load(snapshot);

    assert!(result.is_err());
}

#[test]
fn focus_node_marks_node_as_focused() {
    let mut graph = empty_graph();
    let node = add_node(&mut graph, TestBlueprint::Pass);

    graph.focus_node(node).unwrap();

    assert!(graph.nodes[node].focused);
    assert_eq!(graph.focused_node, Some(node));
}

#[test]
fn focus_node_on_second_node_clears_previous_focus() {
    let (mut graph, first, second) = connected_pair();
    graph.focus_node(first).unwrap();

    graph.focus_node(second).unwrap();

    assert!(!graph.nodes[first].focused);
    assert_eq!(graph.focused_node, Some(second));
}

#[test]
fn focus_node_on_missing_node_returns_error() {
    let mut graph = empty_graph();
    let deleted = add_node(&mut graph, TestBlueprint::Pass);
    graph.delete_node(deleted).unwrap();

    let result = graph.focus_node(deleted);

    assert!(result.is_err());
}

#[test]
fn unfocus_node_clears_focus() {
    let mut graph = empty_graph();
    let node = add_node(&mut graph, TestBlueprint::Pass);
    graph.focus_node(node).unwrap();

    graph.unfocus_node();

    assert!(!graph.nodes[node].focused);
    assert_eq!(graph.focused_node, None);
}

#[test]
fn drain_events_leaving_valid_clears_custom_ui_state() {
    let (mut graph, events) = graph_with_event_sender();
    let node = add_node(&mut graph, TestBlueprint::Pass);
    graph.nodes[node].validity = DataValidityState::Valid;
    graph.nodes[node].set_custom_ui_state(store_with_entry());
    events
        .send(Event::ValidityChanged {
            node_id: node,
            validity: DataValidityState::Invalid,
        })
        .unwrap();

    graph.drain_events();

    assert!(!graph.nodes[node].custom_ui_state().contains_key(STORE_KEY));
}

#[test]
fn drain_events_between_non_valid_states_keeps_custom_ui_state() {
    let (mut graph, events) = graph_with_event_sender();
    let node = add_node(&mut graph, TestBlueprint::Pass);
    graph.nodes[node].set_custom_ui_state(store_with_entry());
    events
        .send(Event::ValidityChanged {
            node_id: node,
            validity: DataValidityState::PendingEvaluation,
        })
        .unwrap();

    graph.drain_events();

    assert!(graph.nodes[node].custom_ui_state().contains_key(STORE_KEY));
}

#[test]
fn drain_events_with_validity_change_updates_ui_validity() {
    let (mut graph, events) = graph_with_event_sender();
    let node = add_node(&mut graph, TestBlueprint::Pass);
    events
        .send(Event::ValidityChanged {
            node_id: node,
            validity: DataValidityState::Valid,
        })
        .unwrap();

    graph.drain_events();

    assert!(graph.nodes[node].is_valid());
}

#[test]
fn drain_events_with_result_ready_replaces_custom_ui_state() {
    let (mut graph, events) = graph_with_event_sender();
    let node = add_node(&mut graph, TestBlueprint::Pass);
    events
        .send(Event::ResultReady {
            node_id: node,
            data: store_with_entry(),
            outputs_text: HashMap::new(),
        })
        .unwrap();

    graph.drain_events();

    assert!(graph.nodes[node].custom_ui_state().contains_key(STORE_KEY));
}

#[test]
fn drain_events_for_deleted_node_is_ignored() {
    let (mut graph, events) = graph_with_event_sender();
    let node = add_node(&mut graph, TestBlueprint::Pass);
    graph.delete_node(node).unwrap();
    events
        .send(Event::ValidityChanged {
            node_id: node,
            validity: DataValidityState::Valid,
        })
        .unwrap();

    graph.drain_events();

    assert!(graph.nodes.is_empty());
}
