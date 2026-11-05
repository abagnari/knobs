use std::any::Any;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use egui::Pos2;

use super::*;
use crate::GenericStore;
use crate::graph::tests::fixtures::*;

const CLOSED_GATE_POLLS: usize = 5;
const CONCURRENT_WORKERS: usize = 2;

type TestNode = EvaluationNode<ThreadedPort, Number, ThreadedBlueprint>;

fn pending_node(blueprint: ThreadedBlueprint, options: Options) -> TestNode {
    let ui_node = Node::new(
        options,
        NodeId::default(),
        blueprint.name().to_string(),
        blueprint,
        Pos2::ZERO,
    )
    .unwrap();
    let mut node = EvaluationNode::from_node(&ui_node);
    node.validity = DataValidityState::PendingEvaluation;

    node
}

fn add_to_store<T>(node: &mut TestNode, key: &str, value: T)
where
    T: Any + Clone + Debug + Send + Sync,
{
    let mut entries = GenericStore::default();
    entries.insert(key, value);
    node.extend_store(&entries);
}

fn pending_node_with_input(blueprint: ThreadedBlueprint, options: Options, input: f32) -> TestNode {
    let mut node = pending_node(blueprint, options);
    node.set_inputs_map(HashMap::from([(
        INPUT.to_string(),
        Arc::new(Number(input)),
    )]));

    node
}

fn poll_to_completion(node: &mut TestNode) -> ComputeState<Number> {
    let deadline = Instant::now() + TEST_TIMEOUT;

    loop {
        let state = node.compute().unwrap();

        if state != ComputeState::Running {
            return state;
        }

        assert!(Instant::now() < deadline, "Worker did not finish in time");
        thread::sleep(POLL_INTERVAL);
    }
}

fn done_with(value: f32) -> ComputeState<Number> {
    ComputeState::done(vec![(OUTPUT.to_string(), Arc::new(Number(value)))])
}

fn gated_node(gate: &Gate) -> TestNode {
    let mut node =
        pending_node_with_input(ThreadedBlueprint::Gated, threaded_options(), SOURCE_VALUE);
    add_to_store(&mut node, GATE_KEY, gate.clone());

    node
}

#[test]
fn compute_with_threaded_blueprint_returns_running_on_first_call() {
    let gate = Gate::default();
    let mut node = gated_node(&gate);

    let result = node.compute().unwrap();

    assert_eq!(result, ComputeState::Running);
    gate.open();
}

#[test]
fn compute_with_threaded_blueprint_eventually_returns_worker_outputs() {
    let mut node = pending_node_with_input(
        ThreadedBlueprint::Increment,
        threaded_options(),
        SOURCE_VALUE,
    );

    let result = poll_to_completion(&mut node);

    assert_eq!(result, done_with(SOURCE_VALUE + INCREMENT));
}

#[test]
fn compute_with_threaded_blueprint_fills_output_buffer_on_completion() {
    let mut node = pending_node_with_input(
        ThreadedBlueprint::Increment,
        threaded_options(),
        SOURCE_VALUE,
    );

    poll_to_completion(&mut node);

    assert_eq!(
        node.data_view(),
        Some(Arc::new(Number(SOURCE_VALUE + INCREMENT)))
    );
}

#[test]
fn compute_with_threaded_blueprint_leaves_output_buffer_empty_while_running() {
    let gate = Gate::default();
    let mut node = gated_node(&gate);

    node.compute().unwrap();

    assert_eq!(node.data_view(), None);
    gate.open();
}

#[test]
fn compute_with_threaded_blueprint_runs_off_the_calling_thread() {
    let probe = ThreadProbe::default();
    let mut node = pending_node_with_input(
        ThreadedBlueprint::Increment,
        threaded_options(),
        SOURCE_VALUE,
    );
    add_to_store(&mut node, PROBE_KEY, probe.clone());

    poll_to_completion(&mut node);

    let threads = probe.threads();
    assert!(!threads.is_empty());
    assert!(!threads.contains(&thread::current().id()));
}

#[test]
fn compute_with_inline_blueprint_runs_on_the_calling_thread_despite_threading_option() {
    let probe = ThreadProbe::default();
    let mut node = pending_node_with_input(
        ThreadedBlueprint::InlineIncrement,
        threaded_options(),
        SOURCE_VALUE,
    );
    add_to_store(&mut node, PROBE_KEY, probe.clone());

    let result = node.compute().unwrap();

    assert_eq!(result, done_with(SOURCE_VALUE + INCREMENT));
    assert_eq!(probe.threads(), vec![thread::current().id()]);
}

#[test]
fn compute_with_threading_option_disabled_runs_threaded_blueprint_inline() {
    let probe = ThreadProbe::default();
    let mut node =
        pending_node_with_input(ThreadedBlueprint::Increment, inline_options(), SOURCE_VALUE);
    add_to_store(&mut node, PROBE_KEY, probe.clone());

    let result = node.compute().unwrap();

    assert_eq!(result, done_with(SOURCE_VALUE + INCREMENT));
    assert_eq!(probe.threads(), vec![thread::current().id()]);
}

#[test]
fn compute_with_worker_returning_error_result_reports_error_state() {
    let mut node = pending_node(ThreadedBlueprint::Failing, threaded_options());

    let result = poll_to_completion(&mut node);

    assert_eq!(result, ComputeState::Error);
}

#[test]
fn compute_with_panicking_worker_reports_error_state() {
    let mut node = pending_node(ThreadedBlueprint::Panicking, threaded_options());

    let result = poll_to_completion(&mut node);

    assert_eq!(result, ComputeState::Error);
}

#[test]
fn compute_with_worker_returning_error_state_reports_error_state() {
    let mut node = pending_node(ThreadedBlueprint::Erroring, threaded_options());

    let result = poll_to_completion(&mut node);

    assert_eq!(result, ComputeState::Error);
}

#[test]
fn compute_with_worker_returning_stopped_reports_stopped() {
    let mut node = pending_node(ThreadedBlueprint::Stopping, threaded_options());

    let result = poll_to_completion(&mut node);

    assert_eq!(result, ComputeState::Stopped);
}

#[test]
fn compute_while_gate_is_closed_keeps_reporting_running() {
    let gate = Gate::default();
    let mut node = gated_node(&gate);

    let results: Vec<ComputeState<Number>> = (0..CLOSED_GATE_POLLS)
        .map(|_| {
            thread::sleep(POLL_INTERVAL);
            node.compute().unwrap()
        })
        .collect();

    assert!(results.iter().all(|state| *state == ComputeState::Running));
    gate.open();
}

#[test]
fn compute_after_gate_opens_returns_worker_outputs() {
    let gate = Gate::default();
    let mut node = gated_node(&gate);
    node.compute().unwrap();

    gate.open();
    let result = poll_to_completion(&mut node);

    assert_eq!(result, done_with(SOURCE_VALUE));
}

#[test]
fn compute_with_worker_missing_user_state_entry_reports_error_state() {
    let mut node =
        pending_node_with_input(ThreadedBlueprint::Gated, threaded_options(), SOURCE_VALUE);

    let result = poll_to_completion(&mut node);

    assert_eq!(result, ComputeState::Error);
}

#[test]
fn compute_on_two_nodes_runs_their_workers_concurrently() {
    let rendezvous = Rendezvous::new(CONCURRENT_WORKERS);
    let mut nodes: Vec<TestNode> = (0..CONCURRENT_WORKERS)
        .map(|_| {
            let mut node = pending_node_with_input(
                ThreadedBlueprint::Meeting,
                threaded_options(),
                SOURCE_VALUE,
            );
            add_to_store(&mut node, RENDEZVOUS_KEY, rendezvous.clone());
            node
        })
        .collect();
    nodes.iter_mut().for_each(|node| {
        node.compute().unwrap();
    });

    let results: Vec<ComputeState<Number>> = nodes.iter_mut().map(poll_to_completion).collect();

    assert!(
        results
            .iter()
            .all(|state| *state == done_with(SOURCE_VALUE))
    );
}

#[test]
fn invalidate_during_threaded_compute_releases_the_worker() {
    let gate = Gate::default();
    let lifetime = Lifetime::default();
    let mut node = gated_node(&gate);
    add_to_store(&mut node, LIFETIME_KEY, lifetime.clone());
    node.compute().unwrap();
    node.validity = DataValidityState::PendingComputation;

    node.invalidate(InvalidationReason::ObsoleteData).unwrap();

    assert!(wait_until(|| lifetime.holders() == SOLE_HOLDER));
}

#[test]
fn invalidate_during_threaded_compute_calls_blueprint_drop_compute_on_the_worker() {
    let gate = Gate::default();
    let probe = ThreadProbe::default();
    let mut node = gated_node(&gate);
    add_to_store(&mut node, PROBE_KEY, probe.clone());
    node.compute().unwrap();
    node.validity = DataValidityState::PendingComputation;
    assert!(wait_until(|| !probe.threads().is_empty()));

    node.invalidate(InvalidationReason::ObsoleteData).unwrap();

    assert!(wait_until(|| compute_was_dropped_on_any(&probe.threads())));
}

#[test]
fn compute_after_invalidating_a_running_worker_starts_a_fresh_worker() {
    let abandoned_gate = Gate::default();
    let mut node = gated_node(&abandoned_gate);
    node.compute().unwrap();
    node.validity = DataValidityState::PendingComputation;
    node.invalidate(InvalidationReason::ObsoleteData).unwrap();
    let fresh_gate = Gate::default();
    fresh_gate.open();
    node.validity = DataValidityState::PendingEvaluation;
    node.set_inputs_map(HashMap::from([(
        INPUT.to_string(),
        Arc::new(Number(SOURCE_VALUE)),
    )]));
    add_to_store(&mut node, GATE_KEY, fresh_gate);

    let result = poll_to_completion(&mut node);

    assert_eq!(result, done_with(SOURCE_VALUE));
    abandoned_gate.open();
}

#[test]
fn compute_thread_with_immediate_result_returns_it() {
    let mut runtime_state = RuntimeState::default();
    runtime_state.inputs_map =
        InputsMap::from_iter([(INPUT.to_string(), Arc::new(Number(SOURCE_VALUE)))]);

    let result = TestNode::compute_thread(
        ThreadedBlueprint::Increment,
        runtime_state,
        Arc::new(CancelSignal::default()),
    )
    .unwrap();

    assert_eq!(result, done_with(SOURCE_VALUE + INCREMENT));
}

#[test]
fn compute_thread_with_cancelled_signal_returns_stopped() {
    let mut runtime_state = RuntimeState::default();
    runtime_state.user_state.insert(GATE_KEY, Gate::default());
    let cancel = Arc::new(CancelSignal::default());
    cancel.cancel();

    let result = TestNode::compute_thread(ThreadedBlueprint::Gated, runtime_state, cancel).unwrap();

    assert_eq!(result, ComputeState::Stopped);
}

#[test]
fn compute_thread_with_failing_blueprint_returns_error() {
    let result = TestNode::compute_thread(
        ThreadedBlueprint::Failing,
        RuntimeState::default(),
        Arc::new(CancelSignal::default()),
    );

    assert!(result.is_err());
}
