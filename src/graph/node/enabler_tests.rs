use egui::{Color32, Pos2};
use serde::{Deserialize, Serialize};

use super::*;
use crate::graph::node::port::{EnablerId, EnablerSignal, EnablerSpy, enabler_id, input_port};

const MODE_ID: usize = 0;
const TOGGLE_ID: usize = 1;
const LINK_ID: usize = 2;
const UNDECLARED_ID: usize = 9;

const FIRST_OPTION: usize = 0;
const SECOND_OPTION: usize = 1;

const GATED_PORT_INDEX: u32 = 0;
const MODE_PORT_INDEX: u32 = 1;
const TOGGLE_PORT_INDEX: u32 = 2;
const LINK_PORT_INDEX: u32 = 3;
const FALLBACK_PORT_INDEX: u32 = 4;
const UPSTREAM_PORT_INDEX: u32 = 0;
const UNKNOWN_PORT_INDEX: u32 = 99;

const VALUE: f32 = 1.0;
const VALUE_MIN: f32 = -10.0;
const VALUE_MAX: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TestPort;

impl PortTypeInfo for TestPort {
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
enum EnablerBlueprint {
    Wired,
    UnknownId,
    DuplicateId,
    DoubleDeclared,
}

impl NodeBlueprintDefinition<TestPort, TestValue> for EnablerBlueprint {
    fn all() -> &'static [Self] {
        &[
            EnablerBlueprint::Wired,
            EnablerBlueprint::UnknownId,
            EnablerBlueprint::DuplicateId,
            EnablerBlueprint::DoubleDeclared,
        ]
    }

    fn name(&self) -> &'static str {
        "Enabler"
    }

    fn ports(&self) -> Vec<PortDefinition<TestPort, TestValue>> {
        let input_types = match self {
            EnablerBlueprint::Wired => vec![
                PortInputType::constant_only_enabled_by(
                    float(),
                    EnablerSpy::when(EnablerSignal::Selected(id(MODE_ID), SECOND_OPTION))
                        .and(EnablerSpy::unless(EnablerSignal::Checked(id(TOGGLE_ID)))),
                ),
                PortInputType::constant_only_with_enabler(selection(), id(MODE_ID)),
                PortInputType::constant_only_with_enabler(
                    ConstantInputType::boolean(false),
                    id(TOGGLE_ID),
                ),
                PortInputType::connection_only_with_enabler(id(LINK_ID)),
                PortInputType::connection_and_constant_enabled_by(
                    float(),
                    EnablerSpy::when(EnablerSignal::Present(id(LINK_ID))),
                ),
            ],
            EnablerBlueprint::UnknownId => vec![PortInputType::constant_only_enabled_by(
                float(),
                EnablerSpy::when(EnablerSignal::Checked(id(UNDECLARED_ID))),
            )],
            EnablerBlueprint::DuplicateId => vec![
                PortInputType::connection_only_with_enabler(id(MODE_ID)),
                PortInputType::constant_only_with_enabler(float(), id(MODE_ID)),
            ],
            EnablerBlueprint::DoubleDeclared => {
                vec![PortInputType::connection_and_constant_with_enabler(
                    ConstantInputType::bounded_boolean_with_enabler(true, id(TOGGLE_ID)),
                    id(MODE_ID),
                )]
            }
        };

        input_types
            .into_iter()
            .enumerate()
            .map(|(index, input_type)| {
                let name = index.to_string();
                input_port(name.clone(), name, TestPort, input_type, false)
            })
            .collect()
    }

    fn categories(&self) -> Vec<&'static str> {
        vec![""]
    }

    fn compute(
        &mut self,
        _: &mut RuntimeState<TestValue>,
    ) -> anyhow::Result<ComputeState<TestValue>> {
        Ok(ComputeState::Error)
    }

    fn port_for_viewer(&self) -> Option<String> {
        None
    }
}

fn id(value: usize) -> EnablerId {
    enabler_id(value)
}

fn float() -> ConstantInputType {
    ConstantInputType::bounded_float(VALUE, VALUE_MIN, VALUE_MAX)
}

fn selection() -> ConstantInputType {
    ConstantInputType::selection(vec!["first".to_string(), "second".to_string()])
}

fn upstream() -> PortId {
    port_id(NodeId::default(), UPSTREAM_PORT_INDEX)
}

fn local(index: u32) -> PortId {
    port_id(NodeId::default(), index)
}

fn build(
    blueprint: EnablerBlueprint,
) -> anyhow::Result<Node<TestPort, TestValue, EnablerBlueprint>> {
    Node::new(
        Options::default(),
        NodeId::default(),
        blueprint.name().to_string(),
        blueprint,
        Pos2::ZERO,
    )
}

fn input_type_mut(port: &mut Port<TestPort>) -> Option<&mut PortInputType> {
    match &mut port.linkage_type {
        PortLinkageType::Input(input_type) => Some(input_type),
        PortLinkageType::Output(_) => None,
    }
}

fn constant_mut(
    node: &mut Node<TestPort, TestValue, EnablerBlueprint>,
    index: u32,
) -> &mut ConstantInputType {
    node.ports_get_mut(&local(index))
        .and_then(input_type_mut)
        .and_then(PortInputType::constant_type_mut)
        .unwrap()
}

fn connect(node: &mut Node<TestPort, TestValue, EnablerBlueprint>, index: u32) {
    match node.ports_get_mut(&local(index)).and_then(input_type_mut) {
        Some(PortInputType::ConnectionOnly { data_source, .. })
        | Some(PortInputType::ConnectionAndConstant { data_source, .. }) => {
            *data_source = Some(upstream())
        }
        other => panic!("Expected a connectable port, got {other:?}"),
    }
}

fn select(node: &mut Node<TestPort, TestValue, EnablerBlueprint>, option: usize) {
    if let ConstantInputType::Selection { selected, .. } = constant_mut(node, MODE_PORT_INDEX) {
        *selected = option;
    }
}

fn tick(node: &mut Node<TestPort, TestValue, EnablerBlueprint>) {
    if let ConstantInputType::Boolean { value, .. } = constant_mut(node, TOGGLE_PORT_INDEX) {
        *value = true;
    }
}

fn resolve_all_on(_: EnablerSignal) -> bool {
    true
}

fn resolve_all_off(_: EnablerSignal) -> bool {
    false
}

#[test]
fn id_of_every_signal_returns_the_wrapped_id() {
    let signals = [
        EnablerSignal::Present(id(MODE_ID)),
        EnablerSignal::Checked(id(MODE_ID)),
        EnablerSignal::Selected(id(MODE_ID), SECOND_OPTION),
    ];

    let ids = signals.map(|signal| signal.id());

    assert!(ids.iter().all(|signal_id| *signal_id == id(MODE_ID)));
}

#[test]
fn when_builds_a_spy_expecting_the_signal_on() {
    let signal = EnablerSignal::Present(id(LINK_ID));

    let spy = EnablerSpy::when(signal);

    assert_eq!(
        spy,
        EnablerSpy::When {
            when: signal,
            enabled: true
        }
    );
}

#[test]
fn unless_builds_a_spy_expecting_the_signal_off() {
    let signal = EnablerSignal::Present(id(LINK_ID));

    let spy = EnablerSpy::unless(signal);

    assert_eq!(
        spy,
        EnablerSpy::When {
            when: signal,
            enabled: false
        }
    );
}

#[test]
fn and_chained_stays_flat() {
    let first = EnablerSpy::when(EnablerSignal::Present(id(MODE_ID)));
    let second = EnablerSpy::when(EnablerSignal::Present(id(TOGGLE_ID)));
    let third = EnablerSpy::when(EnablerSignal::Present(id(LINK_ID)));

    let spy = first.clone().and(second.clone()).and(third.clone());

    assert_eq!(spy, EnablerSpy::All(vec![first, second, third]));
}

#[test]
fn and_of_two_combinations_merges_them() {
    let first = EnablerSpy::when(EnablerSignal::Present(id(MODE_ID)));
    let second = EnablerSpy::when(EnablerSignal::Present(id(TOGGLE_ID)));
    let left = first.clone().and(second.clone());
    let right = second.clone().and(first.clone());

    let spy = left.and(right);

    assert_eq!(
        spy,
        EnablerSpy::All(vec![first.clone(), second.clone(), second, first])
    );
}

#[test]
fn or_chained_stays_flat() {
    let first = EnablerSpy::when(EnablerSignal::Present(id(MODE_ID)));
    let second = EnablerSpy::when(EnablerSignal::Present(id(TOGGLE_ID)));
    let third = EnablerSpy::when(EnablerSignal::Present(id(LINK_ID)));

    let spy = first.clone().or(second.clone()).or(third.clone());

    assert_eq!(spy, EnablerSpy::Any(vec![first, second, third]));
}

#[test]
fn or_after_and_nests_the_combination() {
    let first = EnablerSpy::when(EnablerSignal::Present(id(MODE_ID)));
    let second = EnablerSpy::when(EnablerSignal::Present(id(TOGGLE_ID)));
    let third = EnablerSpy::when(EnablerSignal::Present(id(LINK_ID)));

    let spy = first.clone().and(second.clone()).or(third.clone());

    assert_eq!(
        spy,
        EnablerSpy::Any(vec![EnablerSpy::All(vec![first, second]), third])
    );
}

#[test]
fn evaluate_when_with_signal_on_holds() {
    let spy = EnablerSpy::when(EnablerSignal::Present(id(LINK_ID)));

    let result = spy.evaluate(&resolve_all_on);

    assert!(result);
}

#[test]
fn evaluate_unless_with_signal_on_does_not_hold() {
    let spy = EnablerSpy::unless(EnablerSignal::Present(id(LINK_ID)));

    let result = spy.evaluate(&resolve_all_on);

    assert!(!result);
}

#[test]
fn evaluate_empty_all_holds() {
    let spy = EnablerSpy::All(Vec::new());

    let result = spy.evaluate(&resolve_all_off);

    assert!(result);
}

#[test]
fn evaluate_empty_any_does_not_hold() {
    let spy = EnablerSpy::Any(Vec::new());

    let result = spy.evaluate(&resolve_all_on);

    assert!(!result);
}

#[test]
fn evaluate_when_and_unless_holds_only_with_first_on_and_second_off() {
    let first = EnablerSignal::Present(id(MODE_ID));
    let second = EnablerSignal::Present(id(TOGGLE_ID));
    let spy = EnablerSpy::when(first).and(EnablerSpy::unless(second));

    let results = [(true, false), (true, true), (false, false)].map(|(first_on, second_on)| {
        spy.evaluate(&|signal| if signal == first { first_on } else { second_on })
    });

    assert_eq!(results, [true, false, false]);
}

#[test]
fn signals_of_a_nested_spy_lists_every_leaf() {
    let first = EnablerSignal::Present(id(MODE_ID));
    let second = EnablerSignal::Checked(id(TOGGLE_ID));
    let third = EnablerSignal::Selected(id(LINK_ID), FIRST_OPTION);
    let spy = EnablerSpy::when(first)
        .and(EnablerSpy::unless(second))
        .or(EnablerSpy::when(third));

    let signals = spy.signals();

    assert_eq!(signals, vec![first, second, third]);
}

#[test]
fn boolean_starts_at_default_without_enabler() {
    let constant = ConstantInputType::boolean(true);

    assert_eq!(
        constant,
        ConstantInputType::Boolean {
            value: true,
            enables_id: None
        }
    );
}

#[test]
fn constant_only_with_enabler_declares_the_id() {
    let input_type = PortInputType::constant_only_with_enabler(float(), id(MODE_ID));

    assert_eq!(
        input_type,
        PortInputType::ConstantOnly {
            constant_type: float(),
            enables_id: Some(id(MODE_ID)),
            enabled_by: None
        }
    );
}

#[test]
fn constant_only_enabled_by_keeps_the_spy() {
    let spy = EnablerSpy::when(EnablerSignal::Checked(id(TOGGLE_ID)));

    let input_type = PortInputType::constant_only_enabled_by(float(), spy.clone());

    assert_eq!(
        input_type,
        PortInputType::ConstantOnly {
            constant_type: float(),
            enables_id: None,
            enabled_by: Some(spy)
        }
    );
}

#[test]
fn connection_and_constant_enabled_by_keeps_the_spy() {
    let spy = EnablerSpy::when(EnablerSignal::Present(id(LINK_ID)));

    let input_type = PortInputType::connection_and_constant_enabled_by(float(), spy.clone());

    assert_eq!(
        input_type,
        PortInputType::ConnectionAndConstant {
            constant_type: float(),
            data_source: None,
            enables_id: None,
            enabled_by: Some(spy),
        }
    );
}

#[test]
fn constant_only_enabled_by_id_listens_to_checked_or_present() {
    let expected = EnablerSpy::Any(vec![
        EnablerSpy::when(EnablerSignal::Checked(id(TOGGLE_ID))),
        EnablerSpy::when(EnablerSignal::Present(id(TOGGLE_ID))),
    ]);

    let input_type = PortInputType::constant_only_enabled_by_id(float(), id(TOGGLE_ID));

    assert_eq!(
        input_type,
        PortInputType::constant_only_enabled_by(float(), expected)
    );
}

#[test]
fn connection_and_constant_enabled_by_id_listens_to_checked_or_present() {
    let expected = EnablerSpy::Any(vec![
        EnablerSpy::when(EnablerSignal::Checked(id(TOGGLE_ID))),
        EnablerSpy::when(EnablerSignal::Present(id(TOGGLE_ID))),
    ]);

    let input_type = PortInputType::connection_and_constant_enabled_by_id(float(), id(TOGGLE_ID));

    assert_eq!(
        input_type,
        PortInputType::connection_and_constant_enabled_by(float(), expected)
    );
}

#[test]
fn signal_state_present_follows_the_connection() {
    let unconnected = PortInputType::connection_only_with_enabler(id(LINK_ID));
    let connected = PortInputType::ConnectionOnly {
        data_source: Some(upstream()),
        enables_id: Some(id(LINK_ID)),
    };
    let signal = EnablerSignal::Present(id(LINK_ID));

    let states = [
        unconnected.signal_state(signal),
        connected.signal_state(signal),
    ];

    assert_eq!(states, [false, true]);
}

#[test]
fn signal_state_checked_follows_the_boolean() {
    let unticked =
        PortInputType::constant_only_with_enabler(ConstantInputType::boolean(false), id(TOGGLE_ID));
    let ticked =
        PortInputType::constant_only_with_enabler(ConstantInputType::boolean(true), id(TOGGLE_ID));
    let signal = EnablerSignal::Checked(id(TOGGLE_ID));

    let states = [unticked.signal_state(signal), ticked.signal_state(signal)];

    assert_eq!(states, [false, true]);
}

#[test]
fn signal_state_checked_while_connected_is_off() {
    let input_type = PortInputType::ConnectionAndConstant {
        constant_type: ConstantInputType::boolean(true),
        data_source: Some(upstream()),
        enables_id: Some(id(TOGGLE_ID)),
        enabled_by: None,
    };

    let state = input_type.signal_state(EnablerSignal::Checked(id(TOGGLE_ID)));

    assert!(!state);
}

#[test]
fn signal_state_checked_on_a_non_boolean_is_off() {
    let input_type = PortInputType::constant_only_with_enabler(float(), id(MODE_ID));

    let state = input_type.signal_state(EnablerSignal::Checked(id(MODE_ID)));

    assert!(!state);
}

#[test]
fn signal_state_selected_matches_only_the_selected_option() {
    let input_type = PortInputType::constant_only_with_enabler(selection(), id(MODE_ID));

    let states = [FIRST_OPTION, SECOND_OPTION]
        .map(|option| input_type.signal_state(EnablerSignal::Selected(id(MODE_ID), option)));

    assert_eq!(states, [true, false]);
}

#[test]
fn is_constant_enabled_on_a_connection_only_port_returns_false() {
    let input_type = PortInputType::connection_only();

    let enabled = input_type.is_constant_enabled(&resolve_all_on);

    assert!(!enabled);
}

#[test]
fn is_constant_enabled_without_spy_returns_true() {
    let input_type = PortInputType::constant_only(float());

    let enabled = input_type.is_constant_enabled(&resolve_all_off);

    assert!(enabled);
}

#[test]
fn is_constant_enabled_follows_the_spy() {
    let input_type = PortInputType::constant_only_enabled_by(
        float(),
        EnablerSpy::when(EnablerSignal::Present(id(LINK_ID))),
    );

    let states = [
        input_type.is_constant_enabled(&resolve_all_off),
        input_type.is_constant_enabled(&resolve_all_on),
    ];

    assert_eq!(states, [false, true]);
}

#[test]
fn is_constant_enabled_while_connected_returns_false_even_when_the_spy_holds() {
    let input_type = PortInputType::ConnectionAndConstant {
        constant_type: float(),
        data_source: Some(upstream()),
        enables_id: None,
        enabled_by: Some(EnablerSpy::when(EnablerSignal::Present(id(LINK_ID)))),
    };

    let enabled = input_type.is_constant_enabled(&resolve_all_on);

    assert!(!enabled);
}

#[test]
fn resolve_signal_without_a_declaring_port_is_off() {
    let input_types = [PortInputType::constant_only(ConstantInputType::boolean(
        true,
    ))];

    let state = resolve_signal(&input_types, EnablerSignal::Checked(id(TOGGLE_ID)));

    assert!(!state);
}

#[test]
fn resolve_signal_reads_the_declaring_port() {
    let input_types = [
        PortInputType::constant_only_with_enabler(ConstantInputType::boolean(false), id(MODE_ID)),
        PortInputType::constant_only_with_enabler(ConstantInputType::boolean(true), id(TOGGLE_ID)),
    ];

    let state = resolve_signal(&input_types, EnablerSignal::Checked(id(TOGGLE_ID)));

    assert!(state);
}

#[test]
fn resolve_signal_reads_a_legacy_boolean_enabler() {
    let input_types = [PortInputType::constant_only(
        ConstantInputType::bounded_boolean_with_enabler(true, id(TOGGLE_ID)),
    )];

    let state = resolve_signal(&input_types, EnablerSignal::Checked(id(TOGGLE_ID)));

    assert!(state);
}

#[test]
fn validate_enablers_with_declared_ids_succeeds() {
    let input_types = [
        PortInputType::connection_only_with_enabler(id(LINK_ID)),
        PortInputType::constant_only_enabled_by(
            float(),
            EnablerSpy::when(EnablerSignal::Present(id(LINK_ID))),
        ),
    ];

    let result = validate_enablers(&input_types);

    assert!(result.is_ok());
}

#[test]
fn validate_enablers_with_duplicate_id_returns_error() {
    let input_types = [
        PortInputType::connection_only_with_enabler(id(MODE_ID)),
        PortInputType::constant_only_with_enabler(float(), id(MODE_ID)),
    ];

    let result = validate_enablers(&input_types);

    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Duplicate EnablerId")
    );
}

#[test]
fn validate_enablers_with_undeclared_id_returns_error() {
    let input_types = [PortInputType::constant_only_enabled_by(
        float(),
        EnablerSpy::when(EnablerSignal::Checked(id(UNDECLARED_ID))),
    )];

    let result = validate_enablers(&input_types);

    assert!(result.unwrap_err().to_string().contains("No port declares"));
}

#[test]
fn validate_enablers_with_port_and_constant_ids_returns_error() {
    let input_types = [PortInputType::connection_and_constant_with_enabler(
        ConstantInputType::bounded_boolean_with_enabler(true, id(TOGGLE_ID)),
        id(MODE_ID),
    )];

    let result = validate_enablers(&input_types);

    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("both itself and its constant")
    );
}

#[test]
fn new_with_unknown_enabler_id_returns_error() {
    let result = build(EnablerBlueprint::UnknownId);

    assert!(result.is_err());
}

#[test]
fn new_with_duplicate_enabler_id_returns_error() {
    let result = build(EnablerBlueprint::DuplicateId);

    assert!(result.is_err());
}

#[test]
fn new_with_port_and_constant_enabler_ids_returns_error() {
    let result = build(EnablerBlueprint::DoubleDeclared);

    assert!(result.is_err());
}

#[test]
fn is_constant_enabled_with_initial_signals_disables_the_gated_port() {
    let node = build(EnablerBlueprint::Wired).unwrap();

    let enabled = node.is_constant_enabled(&local(GATED_PORT_INDEX));

    assert!(!enabled);
}

#[test]
fn is_constant_enabled_reads_producers_declared_after_the_receiver() {
    let mut node = build(EnablerBlueprint::Wired).unwrap();
    select(&mut node, SECOND_OPTION);

    let enabled = node.is_constant_enabled(&local(GATED_PORT_INDEX));

    assert!(enabled);
}

#[test]
fn is_constant_enabled_with_unless_signal_on_disables_the_port() {
    let mut node = build(EnablerBlueprint::Wired).unwrap();
    select(&mut node, SECOND_OPTION);
    tick(&mut node);

    let enabled = node.is_constant_enabled(&local(GATED_PORT_INDEX));

    assert!(!enabled);
}

#[test]
fn is_constant_enabled_follows_the_presence_of_a_connection() {
    let mut node = build(EnablerBlueprint::Wired).unwrap();
    let before = node.is_constant_enabled(&local(FALLBACK_PORT_INDEX));
    connect(&mut node, LINK_PORT_INDEX);

    let after = node.is_constant_enabled(&local(FALLBACK_PORT_INDEX));

    assert_eq!([before, after], [false, true]);
}

#[test]
fn is_constant_enabled_on_a_connected_receiver_returns_false() {
    let mut node = build(EnablerBlueprint::Wired).unwrap();
    connect(&mut node, LINK_PORT_INDEX);
    connect(&mut node, FALLBACK_PORT_INDEX);

    let enabled = node.is_constant_enabled(&local(FALLBACK_PORT_INDEX));

    assert!(!enabled);
}

#[test]
fn is_constant_enabled_on_an_unknown_port_returns_false() {
    let node = build(EnablerBlueprint::Wired).unwrap();

    let enabled = node.is_constant_enabled(&local(UNKNOWN_PORT_INDEX));

    assert!(!enabled);
}
