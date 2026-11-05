use std::any::Any;
use std::sync::Arc;
use std::sync::mpsc::channel;
use std::thread;
use std::time::Instant;

use egui::Pos2;
use slotmap::SlotMap;

use super::*;
use crate::graph::node::Node;
use crate::graph::node::port::port_id;
use crate::graph::node::threading::POLL_INTERVAL;
use crate::graph::tests::fixtures::*;

const TWO_INCREMENTS: f32 = 2.0 * INCREMENT;
const GATED_CHAIN_LENGTH: usize = 2;
const NO_CALLS: usize = 0;
const SINGLE_CALL: usize = 1;
const SINGLE_EVENT: usize = 1;
const EDITED_VALUE: f32 = 7.0;
const VALUE_MIN: f32 = -100.0;
const VALUE_MAX: f32 = 100.0;

type TestEvaluator = Evaluator<ThreadedPort, Number, ThreadedBlueprint, RecordingHandler>;
type TestNode = Node<ThreadedPort, Number, ThreadedBlueprint>;

struct TestBench {
    evaluator: TestEvaluator,
    handler: RecordingHandler,
    ids: SlotMap<NodeId, ()>,
    options: Options,
}

impl TestBench {
    fn new(options: Options) -> Self {
        let handler = RecordingHandler::default();
        let (_, command_receiver) = channel();
        let (event_sender, _) = channel();
        let evaluator = Evaluator::new(
            options,
            SecondaryMap::new(),
            Connections::default(),
            None,
            GenericStore::default(),
            handler.clone(),
            command_receiver,
            event_sender,
        );

        Self {
            evaluator,
            handler,
            ids: SlotMap::with_key(),
            options,
        }
    }

    fn ui_node(&mut self, blueprint: ThreadedBlueprint) -> TestNode {
        let id = self.ids.insert(());

        Node::new(
            self.options,
            id,
            blueprint.name().to_string(),
            blueprint,
            Pos2::ZERO,
        )
        .unwrap()
    }

    fn add_node(&mut self, blueprint: ThreadedBlueprint) -> NodeId {
        let node = self.ui_node(blueprint);
        self.evaluator
            .insert_node(EvaluationNode::from_node(&node))
            .unwrap();

        node.id
    }

    fn chain<const N: usize>(&mut self, blueprints: [ThreadedBlueprint; N]) -> [NodeId; N] {
        let nodes = blueprints.map(|blueprint| self.add_node(blueprint));

        for (source, target) in nodes.iter().zip(nodes.iter().skip(1)) {
            self.evaluator
                .insert_connection(
                    port_id(*source, OUTPUT_PORT_INDEX),
                    port_id(*target, INPUT_PORT_INDEX),
                )
                .unwrap();
        }

        nodes
    }

    fn add_default_entry<T>(&mut self, key: &str, value: T)
    where
        T: Any + Clone + Debug + Send + Sync,
    {
        let mut entries = GenericStore::default();
        entries.insert(key, value);
        self.evaluator.add_default_compute_entries(entries);
    }

    fn gated_chain(&mut self, gate: &Gate, probe: &ThreadProbe) -> [NodeId; GATED_CHAIN_LENGTH] {
        self.add_default_entry(GATE_KEY, gate.clone());
        self.add_default_entry(PROBE_KEY, probe.clone());

        self.chain([ThreadedBlueprint::Source, ThreadedBlueprint::Gated])
    }

    fn start_gated_worker(&mut self, gated: NodeId) {
        self.evaluator.focus_node(gated).unwrap();
        self.run_until(|evaluator| evaluator.nodes[gated].is_pending_computation());
    }

    fn run_until(&mut self, condition: impl Fn(&TestEvaluator) -> bool) {
        let deadline = Instant::now() + TEST_TIMEOUT;

        while !condition(&self.evaluator) {
            assert!(
                Instant::now() < deadline,
                "Evaluator did not reach the expected state in time"
            );
            self.evaluator.evaluate().unwrap();
            thread::sleep(POLL_INTERVAL);
        }
    }

    fn run_until_idle(&mut self) {
        self.evaluator.evaluate().unwrap();
        self.run_until(|evaluator| evaluator.evaluation_stack.is_empty());
    }

    fn node(&self, node_id: NodeId) -> &EvaluationNode<ThreadedPort, Number, ThreadedBlueprint> {
        &self.evaluator.nodes[node_id]
    }

    fn is_error(&self, node_id: NodeId) -> bool {
        matches!(self.node(node_id).validity, DataValidityState::Error { .. })
    }

    fn constant_of(&self, port: PortId) -> Option<&ConstantInputType> {
        self.node(port.node_id()).ports()[&port]
            .input_type
            .as_ref()
            .and_then(PortInputType::constant_type)
    }
}

fn edited_constant() -> ConstantInputType {
    ConstantInputType::bounded_float(EDITED_VALUE, VALUE_MIN, VALUE_MAX)
}

fn assert_send<T: Send>() {}

#[test]
fn evaluator_is_send() {
    assert_send::<TestEvaluator>();
}

#[test]
fn insert_node_registers_node() {
    let mut bench = TestBench::new(inline_options());
    let node = bench.ui_node(ThreadedBlueprint::Source);

    let result = bench
        .evaluator
        .insert_node(EvaluationNode::from_node(&node));

    assert!(result.is_ok());
    assert!(bench.evaluator.nodes.contains_key(node.id));
}

#[test]
fn insert_node_keeps_ui_port_ids() {
    let mut bench = TestBench::new(inline_options());
    let node = bench.ui_node(ThreadedBlueprint::Source);

    bench
        .evaluator
        .insert_node(EvaluationNode::from_node(&node))
        .unwrap();

    let expected: Vec<&PortId> = node.ports().keys().collect();
    let result: Vec<&PortId> = bench.node(node.id).ports().keys().collect();
    assert_eq!(result, expected);
}

#[test]
fn set_constant_updates_evaluation_port() {
    let mut bench = TestBench::new(inline_options());
    let source = bench.add_node(ThreadedBlueprint::Source);
    let input = port_id(source, INPUT_PORT_INDEX);

    bench
        .evaluator
        .set_constant(input, edited_constant())
        .unwrap();

    assert_eq!(bench.constant_of(input), Some(&edited_constant()));
}

#[test]
fn set_constant_on_output_port_returns_error() {
    let mut bench = TestBench::new(inline_options());
    let source = bench.add_node(ThreadedBlueprint::Source);

    let result = bench
        .evaluator
        .set_constant(port_id(source, OUTPUT_PORT_INDEX), edited_constant());

    assert!(result.is_err());
}

#[test]
fn set_constant_on_valid_focused_node_invalidates_and_requeues_it() {
    let mut bench = TestBench::new(inline_options());
    let source = bench.add_node(ThreadedBlueprint::Source);
    bench.evaluator.focus_node(source).unwrap();
    bench.run_until_idle();

    bench
        .evaluator
        .set_constant(port_id(source, INPUT_PORT_INDEX), edited_constant())
        .unwrap();

    assert!(bench.node(source).is_invalid());
    assert!(bench.evaluator.evaluation_stack.contains(&source));
}

#[test]
fn collect_node_events_after_computation_reports_result_once() {
    let mut bench = TestBench::new(inline_options());
    let source = bench.add_node(ThreadedBlueprint::Source);
    bench.evaluator.focus_node(source).unwrap();
    bench.run_until_idle();

    bench.evaluator.collect_node_events();
    bench.evaluator.collect_node_events();

    let results = bench
        .evaluator
        .events_to_send
        .iter()
        .filter(|event| matches!(event, Event::ResultReady { .. }))
        .count();
    assert_eq!(results, SINGLE_EVENT);
}

#[test]
fn apply_add_default_entries_reaches_worker_threads() {
    let mut bench = TestBench::new(threaded_options());
    let probe = ThreadProbe::default();
    let mut entries = GenericStore::default();
    entries.insert(PROBE_KEY, probe.clone());
    let [_, increment] = bench.chain([ThreadedBlueprint::Source, ThreadedBlueprint::Increment]);

    bench
        .evaluator
        .apply(Command::AddDefaultEntries { entries })
        .unwrap();
    bench.evaluator.focus_node(increment).unwrap();
    bench.run_until_idle();

    let threads = probe.threads();
    assert!(!threads.is_empty());
    assert!(!threads.contains(&thread::current().id()));
}

#[test]
fn evaluate_threaded_chain_reports_value_computed_on_workers() {
    let mut bench = TestBench::new(threaded_options());
    let [_, _, last] = bench.chain([
        ThreadedBlueprint::Source,
        ThreadedBlueprint::Increment,
        ThreadedBlueprint::Increment,
    ]);
    bench.evaluator.focus_node(last).unwrap();

    bench.run_until_idle();

    assert_eq!(
        bench.handler.last_output(),
        Some(Arc::new(Number(SOURCE_VALUE + TWO_INCREMENTS)))
    );
}

#[test]
fn evaluate_threaded_chain_leaves_every_node_valid() {
    let mut bench = TestBench::new(threaded_options());
    let [source, increment] =
        bench.chain([ThreadedBlueprint::Source, ThreadedBlueprint::Increment]);
    bench.evaluator.focus_node(increment).unwrap();

    bench.run_until_idle();

    assert!(bench.node(source).is_valid());
    assert!(bench.node(increment).is_valid());
}

#[test]
fn evaluate_mixed_inline_and_threaded_chain_reports_combined_value() {
    let mut bench = TestBench::new(threaded_options());
    let [_, _, last] = bench.chain([
        ThreadedBlueprint::Source,
        ThreadedBlueprint::InlineIncrement,
        ThreadedBlueprint::Increment,
    ]);
    bench.evaluator.focus_node(last).unwrap();

    bench.run_until_idle();

    assert_eq!(
        bench.handler.last_output(),
        Some(Arc::new(Number(SOURCE_VALUE + TWO_INCREMENTS)))
    );
}

#[test]
fn evaluate_once_with_threading_leaves_work_on_the_stack() {
    let mut bench = TestBench::new(threaded_options());
    let [_, increment] = bench.chain([ThreadedBlueprint::Source, ThreadedBlueprint::Increment]);
    bench.evaluator.focus_node(increment).unwrap();

    bench.evaluator.evaluate().unwrap();

    assert!(!bench.evaluator.evaluation_stack.is_empty());
}

#[test]
fn evaluate_once_without_threading_completes_the_whole_chain() {
    let mut bench = TestBench::new(inline_options());
    let [_, increment] = bench.chain([ThreadedBlueprint::Source, ThreadedBlueprint::Increment]);
    bench.evaluator.focus_node(increment).unwrap();

    bench.evaluator.evaluate().unwrap();

    assert!(bench.evaluator.evaluation_stack.is_empty());
    assert_eq!(
        bench.handler.last_output(),
        Some(Arc::new(Number(SOURCE_VALUE + INCREMENT)))
    );
}

#[test]
fn evaluate_with_running_worker_keeps_focused_node_on_the_stack() {
    let mut bench = TestBench::new(threaded_options());
    let gate = Gate::default();
    let [_, gated] = bench.gated_chain(&gate, &ThreadProbe::default());

    bench.start_gated_worker(gated);

    assert!(bench.evaluator.evaluation_stack.contains(&gated));
    assert_eq!(bench.handler.call_count(), NO_CALLS);
    gate.open();
}

#[test]
fn evaluate_after_running_worker_finishes_reports_its_value() {
    let mut bench = TestBench::new(threaded_options());
    let gate = Gate::default();
    let [_, gated] = bench.gated_chain(&gate, &ThreadProbe::default());
    bench.start_gated_worker(gated);

    gate.open();
    bench.run_until_idle();

    assert!(bench.node(gated).is_valid());
    assert_eq!(
        bench.handler.last_output(),
        Some(Arc::new(Number(SOURCE_VALUE)))
    );
}

#[test]
fn evaluate_with_failing_upstream_worker_puts_downstream_in_error() {
    let mut bench = TestBench::new(threaded_options());
    let [_, failing, downstream] = bench.chain([
        ThreadedBlueprint::Source,
        ThreadedBlueprint::Failing,
        ThreadedBlueprint::Increment,
    ]);
    bench.evaluator.focus_node(downstream).unwrap();

    bench.run_until_idle();

    assert!(bench.is_error(failing));
    assert!(bench.is_error(downstream));
    assert_eq!(bench.handler.call_count(), NO_CALLS);
}

#[test]
fn evaluate_with_panicking_upstream_worker_puts_downstream_in_error() {
    let mut bench = TestBench::new(threaded_options());
    let [_, panicking, downstream] = bench.chain([
        ThreadedBlueprint::Source,
        ThreadedBlueprint::Panicking,
        ThreadedBlueprint::Increment,
    ]);
    bench.evaluator.focus_node(downstream).unwrap();

    bench.run_until_idle();

    assert!(bench.is_error(panicking));
    assert!(bench.is_error(downstream));
    assert_eq!(bench.handler.call_count(), NO_CALLS);
}

#[test]
fn evaluate_with_stopped_worker_leaves_downstream_invalid() {
    let mut bench = TestBench::new(threaded_options());
    let [_, stopping, downstream] = bench.chain([
        ThreadedBlueprint::Source,
        ThreadedBlueprint::Stopping,
        ThreadedBlueprint::Increment,
    ]);
    bench.evaluator.focus_node(downstream).unwrap();

    bench.run_until_idle();

    assert!(bench.node(stopping).is_invalid());
    assert!(bench.node(downstream).is_invalid());
    assert_eq!(bench.handler.call_count(), NO_CALLS);
}

#[test]
fn propagate_state_during_running_worker_cancels_it() {
    let mut bench = TestBench::new(threaded_options());
    let gate = Gate::default();
    let probe = ThreadProbe::default();
    let [source, gated] = bench.gated_chain(&gate, &probe);
    bench.start_gated_worker(gated);

    bench
        .evaluator
        .propagate_state(source, InvalidationReason::ObsoleteData)
        .unwrap();

    assert!(wait_until(|| compute_was_dropped_on_any(&probe.threads())));
    gate.open();
}

#[test]
fn evaluate_after_upstream_edit_during_running_worker_recomputes_focused_node() {
    let mut bench = TestBench::new(threaded_options());
    let gate = Gate::default();
    let [source, gated] = bench.gated_chain(&gate, &ThreadProbe::default());
    bench.start_gated_worker(gated);
    bench
        .evaluator
        .propagate_state(source, InvalidationReason::ObsoleteData)
        .unwrap();
    bench.evaluator.requeue_focused_node();

    gate.open();
    bench.run_until_idle();

    assert!(bench.node(source).is_valid());
    assert!(bench.node(gated).is_valid());
    assert_eq!(
        bench.handler.last_output(),
        Some(Arc::new(Number(SOURCE_VALUE)))
    );
}

#[test]
fn delete_node_with_running_worker_cancels_it() {
    let mut bench = TestBench::new(threaded_options());
    let gate = Gate::default();
    let probe = ThreadProbe::default();
    let [_, gated] = bench.gated_chain(&gate, &probe);
    bench.start_gated_worker(gated);

    bench.evaluator.delete_node(gated).unwrap();

    assert!(wait_until(|| compute_was_dropped_on_any(&probe.threads())));
    assert!(!bench.evaluator.evaluation_stack.contains(&gated));
    gate.open();
}

#[test]
fn reset_with_running_worker_cancels_it() {
    let mut bench = TestBench::new(threaded_options());
    let gate = Gate::default();
    let probe = ThreadProbe::default();
    let [_, gated] = bench.gated_chain(&gate, &probe);
    bench.start_gated_worker(gated);

    bench.evaluator.reset().unwrap();

    assert!(wait_until(|| compute_was_dropped_on_any(&probe.threads())));
    assert!(bench.evaluator.evaluation_stack.is_empty());
    gate.open();
}

#[test]
fn focus_node_on_already_computed_threaded_node_reports_value_immediately() {
    let mut bench = TestBench::new(threaded_options());
    let [_, increment] = bench.chain([ThreadedBlueprint::Source, ThreadedBlueprint::Increment]);
    bench.evaluator.focus_node(increment).unwrap();
    bench.run_until_idle();
    let calls_before_refocus = bench.handler.call_count();

    bench.evaluator.focus_node(increment).unwrap();

    assert_eq!(
        bench.handler.call_count(),
        calls_before_refocus + SINGLE_CALL
    );
    assert_eq!(
        bench.handler.last_output(),
        Some(Arc::new(Number(SOURCE_VALUE + INCREMENT)))
    );
}
