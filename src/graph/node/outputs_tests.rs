use std::collections::HashMap;
use std::sync::Arc;

use egui::Pos2;

use super::*;
use crate::graph::tests::fixtures::*;

const UNDECLARED_PORT: &str = "undeclared";

type TestNode = EvaluationNode<ThreadedPort, Number, ThreadedBlueprint>;

fn new_node(blueprint: ThreadedBlueprint) -> TestNode {
    let node = Node::new(
        Options::default(),
        NodeId::default(),
        blueprint.name().to_string(),
        blueprint,
        Pos2::ZERO,
    )
    .unwrap();

    EvaluationNode::from_node(&node)
}

fn computed_node(input: f32) -> TestNode {
    let mut node = new_node(ThreadedBlueprint::InlineIncrement);
    node.validity = DataValidityState::PendingEvaluation;
    node.set_inputs_map(HashMap::from([(
        INPUT.to_string(),
        Arc::new(Number(input)),
    )]));
    node.compute().unwrap();

    node
}

fn output_port_id() -> PortId {
    port_id(NodeId::default(), OUTPUT_PORT_INDEX)
}

fn input_port_id() -> PortId {
    port_id(NodeId::default(), INPUT_PORT_INDEX)
}

fn single_output(name: &str, value: f32) -> HashMap<String, Arc<Number>> {
    HashMap::from([(name.to_string(), Arc::new(Number(value)))])
}

#[test]
fn compute_with_done_state_stores_output_by_port_name() {
    let node = computed_node(SOURCE_VALUE);

    let value = node.output_value(&output_port_id());

    assert_eq!(value, Some(&Arc::new(Number(SOURCE_VALUE + INCREMENT))));
}

#[test]
fn outputs_after_compute_returns_computed_values() {
    let node = computed_node(SOURCE_VALUE);

    let outputs = &node.outputs;

    assert_eq!(outputs, &single_output(OUTPUT, SOURCE_VALUE + INCREMENT));
}

#[test]
fn outputs_before_compute_returns_empty_map() {
    let node = new_node(ThreadedBlueprint::InlineIncrement);

    let outputs = &node.outputs;

    assert!(outputs.is_empty());
}

#[test]
fn invalidate_after_compute_clears_outputs() {
    let mut node = computed_node(SOURCE_VALUE);

    node.invalidate(InvalidationReason::ObsoleteData).unwrap();

    assert_eq!(node.output_value(&output_port_id()), None);
}

#[test]
fn output_value_for_input_port_returns_none() {
    let node = computed_node(SOURCE_VALUE);

    let value = node.output_value(&input_port_id());

    assert_eq!(value, None);
}

#[test]
fn output_value_before_compute_returns_none() {
    let node = new_node(ThreadedBlueprint::InlineIncrement);

    let value = node.output_value(&output_port_id());

    assert_eq!(value, None);
}

#[test]
fn declared_outputs_with_output_port_name_keeps_value() {
    let node = new_node(ThreadedBlueprint::InlineIncrement);
    let outputs = single_output(OUTPUT, SOURCE_VALUE);

    let declared = node.declared_outputs(&outputs).unwrap();

    assert_eq!(declared, outputs);
}

#[test]
fn declared_outputs_with_input_port_name_drops_value() {
    let node = new_node(ThreadedBlueprint::InlineIncrement);
    let outputs = single_output(INPUT, SOURCE_VALUE);

    let declared = node.declared_outputs(&outputs).unwrap();

    assert!(declared.is_empty());
}

#[test]
fn declared_outputs_with_undeclared_port_name_returns_error() {
    let node = new_node(ThreadedBlueprint::InlineIncrement);
    let outputs = single_output(UNDECLARED_PORT, SOURCE_VALUE);

    let result = node.declared_outputs(&outputs);

    assert!(result.is_err());
}
