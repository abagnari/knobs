use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use egui::{Color32, Context, Id, Pos2, RawInput, Ui, vec2};
use serde::{Deserialize, Serialize};

use crate::graph::canvas::Canvas;
use crate::graph::node::port::PortOutputType;
use crate::graph::node::threading::POLL_INTERVAL;
use crate::graph::node::{DataValidityState, EvaluationNode, InvalidationReason, Node};
use crate::graph::painter::{NodeSelectedState, Painter};
use crate::graph::tests::fixtures::{Number, TEST_TIMEOUT, ThreadedBlueprint};
use crate::graph::*;
use crate::store::GenericStore;
use crate::{ComputeState, PortDefinition, RuntimeState, input_port, output_port};

const INPUT: &str = "in";
const OUTPUT: &str = "out";
const PREPARED_KEY: &str = "prepared";
const DRAWN_FRAMES_KEY: &str = "drawn_frames";
const CONSTANT_VALUE: f32 = 3.0;
const CONTENT_HEIGHT: f32 = 40.0;
const TALL_CONTENT_HEIGHT: f32 = 90.0;
const NARROW_CONTENT_WIDTH: f32 = 20.0;
const POSITION_TOLERANCE: f32 = 0.001;
const HEIGHT_TOLERANCE: f32 = 0.001;
const NO_ZOOM_CHANGE: f32 = 0.0;
const FIRST_POSITION: Pos2 = Pos2::ZERO;
const SECOND_POSITION: Pos2 = Pos2::new(300.0, 0.0);
const SINGLE_FRAME: usize = 1;
const FRAMES: usize = 2;
const NODE_COUNT: usize = 2;
const FIRST_DRAWN_FRAME: usize = 1;
const STORE_KEY: &str = "store";
const STORE_VALUE: i32 = 1;

thread_local! {
    static CALLS: RefCell<Vec<CustomUiCall>> = const { RefCell::new(Vec::new()) };
}

#[derive(Debug, Clone, PartialEq)]
struct CustomUiCall {
    ui_id: Id,
    prepared: Option<Value>,
    drawn_frames: usize,
}

fn recorded_calls() -> Vec<CustomUiCall> {
    CALLS.with(|calls| calls.borrow().clone())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Number,
}

impl PortTypeInfo for Kind {
    fn color(&self) -> Color32 {
        Color32::WHITE
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Value(f32);

impl From<&Value> for String {
    fn from(Value(value): &Value) -> Self {
        value.to_string()
    }
}

impl From<&ConstantInputType> for Value {
    fn from(constant: &ConstantInputType) -> Self {
        match constant {
            ConstantInputType::Float { value, .. } => Value(*value),
            _ => Value(f32::default()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Blueprint {
    Preview,
    Tall,
    Narrow,
    Plain,
}

impl NodeBlueprintDefinition<Kind, Value> for Blueprint {
    fn all() -> &'static [Self] {
        &[
            Blueprint::Preview,
            Blueprint::Tall,
            Blueprint::Narrow,
            Blueprint::Plain,
        ]
    }

    fn name(&self) -> &'static str {
        match self {
            Blueprint::Preview => "Preview",
            Blueprint::Tall => "Tall",
            Blueprint::Narrow => "Narrow",
            Blueprint::Plain => "Plain",
        }
    }

    fn ports(&self) -> Vec<PortDefinition<Kind, Value>> {
        let constant = PortInputType::constant_only(ConstantInputType::Float {
            value: CONSTANT_VALUE,
            min_bound: f32::MIN,
            max_bound: f32::MAX,
            scale_display: 1.0,
        });

        vec![
            input_port(INPUT, INPUT, Kind::Number, constant, true),
            output_port(
                OUTPUT,
                OUTPUT,
                Kind::Number,
                PortOutputType::ConnectionOnly,
                true,
            ),
        ]
    }

    fn categories(&self) -> Vec<&'static str> {
        vec![""]
    }

    fn compute(&mut self, state: &mut RuntimeState<Value>) -> anyhow::Result<ComputeState<Value>> {
        let input = state.inputs_map.get(INPUT)?.clone();

        Ok(ComputeState::done(vec![(
            OUTPUT.to_string(),
            Arc::new(input),
        )]))
    }

    fn port_for_viewer(&self) -> Option<String> {
        None
    }

    fn prepare_custom_ui_data(&self, outputs: &HashMap<String, Arc<Value>>) -> GenericStore {
        let mut state = GenericStore::default();

        if let Some(output) = outputs.get(OUTPUT) {
            state.insert(PREPARED_KEY, output.as_ref().clone());
        }

        state
    }

    fn custom_ui(&self, ui: &mut Ui, _canvas: &Canvas, state: &mut GenericStore) {
        match self {
            Blueprint::Preview => {
                let drawn_frames = state
                    .get::<usize>(DRAWN_FRAMES_KEY)
                    .copied()
                    .unwrap_or_default()
                    + 1;
                state.insert(DRAWN_FRAMES_KEY, drawn_frames);

                let call = CustomUiCall {
                    ui_id: ui.scope_id(),
                    prepared: state.get::<Value>(PREPARED_KEY).ok().cloned(),
                    drawn_frames,
                };

                CALLS.with(|calls| calls.borrow_mut().push(call));
                ui.allocate_space(vec2(ui.available_width(), CONTENT_HEIGHT));
            }
            Blueprint::Tall => {
                ui.allocate_space(vec2(ui.available_width(), TALL_CONTENT_HEIGHT));
            }
            Blueprint::Narrow => {
                ui.allocate_space(vec2(NARROW_CONTENT_WIDTH, CONTENT_HEIGHT));
            }
            Blueprint::Plain => {}
        }
    }
}

struct Handler;

impl GraphEventHandler<Value> for Handler {}

type PreviewGraph = Graph<Kind, Value, Blueprint, Handler>;

fn graph_with(blueprints: &[(Blueprint, Pos2)]) -> (PreviewGraph, Vec<NodeId>) {
    let mut graph = Graph::new(Options::default(), Handler);
    let nodes = blueprints
        .iter()
        .map(|(blueprint, position)| {
            graph
                .create_node(blueprint.name().to_string(), *blueprint, *position)
                .unwrap()
                .id
        })
        .collect();

    (graph, nodes)
}

fn show_frames(graph: &mut PreviewGraph, frames: usize) {
    let ctx = Context::default();

    for _ in 0..frames {
        let mut output = ctx.run_ui(RawInput::default(), |ui| graph.show(ui));
        output.textures_delta.clear();
    }
}

fn new_node(blueprint: Blueprint) -> Node<Kind, Value, Blueprint> {
    Node::new(
        Options::default(),
        NodeId::default(),
        blueprint.name().to_string(),
        blueprint,
        FIRST_POSITION,
    )
    .unwrap()
}

fn show_until(graph: &mut PreviewGraph, condition: impl Fn() -> bool) {
    let ctx = Context::default();
    let deadline = Instant::now() + TEST_TIMEOUT;

    while !condition() {
        assert!(
            Instant::now() < deadline,
            "Graph did not reach the expected state in time"
        );
        let mut output = ctx.run_ui(RawInput::default(), |ui| graph.show(ui));
        output.textures_delta.clear();
        thread::sleep(POLL_INTERVAL);
    }
}

fn last_call_is_prepared() -> bool {
    recorded_calls()
        .last()
        .is_some_and(|call| call.prepared == Some(Value(CONSTANT_VALUE)))
}

fn store_with_entry() -> GenericStore {
    let mut store = GenericStore::default();
    store.insert(STORE_KEY, STORE_VALUE);

    store
}

fn computed_node(blueprint: Blueprint) -> EvaluationNode<Kind, Value, Blueprint> {
    let mut node = EvaluationNode::from_node(&new_node(blueprint));
    node.validity = DataValidityState::PendingEvaluation;
    node.set_inputs_map(HashMap::from([(
        INPUT.to_string(),
        Arc::new(Value(CONSTANT_VALUE)),
    )]));
    node.compute().unwrap();

    node
}

fn drawn_node_state(blueprint: Blueprint) -> NodeState {
    let options = Options::default();
    let mut painter = Painter::new(options, options.theme);
    let mut canvas = Canvas::default();
    let mut node = new_node(blueprint);
    let mut node_state = None;

    let mut output = Context::default().run_ui(RawInput::default(), |ui| {
        canvas.zoom(FIRST_POSITION, ui.clip_rect(), NO_ZOOM_CHANGE);
        let (state, _) = painter.show_node(
            ui,
            &canvas,
            &mut node,
            NodeSelectedState::NotSelected,
            DataValidityState::Invalid,
            &None,
        );
        node_state = Some(state);
    });
    output.textures_delta.clear();

    node_state.unwrap()
}

fn drawn_node_height(blueprint: Blueprint) -> f32 {
    drawn_node_state(blueprint).rect.height()
}

fn focus_button_right_edge(blueprint: Blueprint) -> f32 {
    drawn_node_state(blueprint)
        .focus_button_response
        .unwrap()
        .rect
        .right()
}

#[test]
fn prepare_custom_ui_data_without_override_returns_empty_store() {
    let blueprint = ThreadedBlueprint::Source;
    let outputs = HashMap::from([(OUTPUT.to_string(), Arc::new(Number(CONSTANT_VALUE)))]);

    let state = blueprint.prepare_custom_ui_data(&outputs);

    assert!(!state.contains_key(OUTPUT));
}

#[test]
fn custom_ui_state_before_compute_is_empty() {
    let node = new_node(Blueprint::Preview);

    let state = node.custom_ui_state();

    assert!(!state.contains_key(PREPARED_KEY));
}

#[test]
fn custom_ui_data_after_compute_returns_prepared_state() {
    let node = computed_node(Blueprint::Preview);

    let data = node.custom_ui_data();

    assert_eq!(
        data.get::<Value>(PREPARED_KEY).unwrap(),
        &Value(CONSTANT_VALUE)
    );
}

#[test]
fn custom_ui_data_after_invalidate_has_no_prepared_state() {
    let mut node = computed_node(Blueprint::Preview);
    node.invalidate(InvalidationReason::ObsoleteData).unwrap();

    let data = node.custom_ui_data();

    assert!(!data.contains_key(PREPARED_KEY));
}

#[test]
fn set_custom_ui_state_replaces_store() {
    let mut node = new_node(Blueprint::Preview);

    node.set_custom_ui_state(store_with_entry());

    assert!(node.custom_ui_state().contains_key(STORE_KEY));
}

#[test]
fn clear_custom_ui_state_empties_store() {
    let mut node = new_node(Blueprint::Preview);
    node.set_custom_ui_state(store_with_entry());

    node.clear_custom_ui_state();

    assert!(!node.custom_ui_state().contains_key(STORE_KEY));
}

#[test]
fn show_node_with_custom_ui_content_grows_taller_than_without() {
    let without_content = drawn_node_height(Blueprint::Plain);

    let with_content = drawn_node_height(Blueprint::Preview);

    assert!(with_content - without_content >= CONTENT_HEIGHT);
}

#[test]
fn show_node_with_taller_custom_ui_content_grows_by_the_difference() {
    let shorter = drawn_node_height(Blueprint::Preview);

    let taller = drawn_node_height(Blueprint::Tall);

    let growth = taller - shorter;
    assert!((growth - (TALL_CONTENT_HEIGHT - CONTENT_HEIGHT)).abs() < HEIGHT_TOLERANCE);
}

#[test]
fn show_node_with_narrow_custom_ui_content_keeps_focus_button_aligned() {
    let without_content = focus_button_right_edge(Blueprint::Plain);

    let with_narrow_content = focus_button_right_edge(Blueprint::Narrow);

    assert!((with_narrow_content - without_content).abs() < POSITION_TOLERANCE);
}

#[test]
fn show_with_computed_node_passes_prepared_state_to_custom_ui() {
    let (mut graph, nodes) = graph_with(&[(Blueprint::Preview, FIRST_POSITION)]);
    graph.focus_node(nodes[0]).unwrap();

    show_until(&mut graph, last_call_is_prepared);

    assert_eq!(
        recorded_calls().last().unwrap().prepared,
        Some(Value(CONSTANT_VALUE))
    );
}

#[test]
fn show_before_evaluation_passes_state_without_prepared_values() {
    let (mut graph, _) = graph_with(&[(Blueprint::Preview, FIRST_POSITION)]);

    show_frames(&mut graph, SINGLE_FRAME);

    assert_eq!(recorded_calls().last().unwrap().prepared, None);
}

#[test]
fn show_on_consecutive_frames_keeps_values_written_by_custom_ui() {
    let (mut graph, _) = graph_with(&[(Blueprint::Preview, FIRST_POSITION)]);

    show_frames(&mut graph, FRAMES);

    let drawn_frames: Vec<usize> = recorded_calls()
        .iter()
        .map(|call| call.drawn_frames)
        .collect();
    assert_eq!(
        drawn_frames,
        (FIRST_DRAWN_FRAME..=FRAMES).collect::<Vec<usize>>()
    );
}

#[test]
fn show_on_consecutive_frames_keeps_custom_ui_id() {
    let (mut graph, _) = graph_with(&[(Blueprint::Preview, FIRST_POSITION)]);

    show_frames(&mut graph, FRAMES);

    let calls = recorded_calls();
    assert_eq!(calls.len(), FRAMES);
    assert_eq!(calls[0].ui_id, calls[1].ui_id);
}

#[test]
fn show_with_two_nodes_gives_each_custom_ui_its_own_id() {
    let (mut graph, _) = graph_with(&[
        (Blueprint::Preview, FIRST_POSITION),
        (Blueprint::Preview, SECOND_POSITION),
    ]);

    show_frames(&mut graph, SINGLE_FRAME);

    let calls = recorded_calls();
    assert_eq!(calls.len(), NODE_COUNT);
    assert_ne!(calls[0].ui_id, calls[1].ui_id);
}
