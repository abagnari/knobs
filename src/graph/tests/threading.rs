use std::sync::Arc;
use std::thread;
use std::time::Instant;

use egui::Pos2;

use crate::graph::node::port::port_id;
use crate::graph::node::threading::POLL_INTERVAL;
use crate::graph::tests::fixtures::*;
use crate::graph::*;

const TWO_INCREMENTS: f32 = 2.0 * INCREMENT;

type ThreadedGraph = Graph<ThreadedPort, Number, ThreadedBlueprint, RecordingHandler>;

fn graph_with(options: Options) -> (ThreadedGraph, RecordingHandler) {
    let handler = RecordingHandler::default();

    (Graph::new(options, handler.clone()), handler)
}

fn chain<const N: usize>(
    graph: &mut ThreadedGraph,
    blueprints: [ThreadedBlueprint; N],
) -> [NodeId; N] {
    let nodes = blueprints.map(|blueprint| {
        graph
            .create_node(blueprint.name().to_string(), blueprint, Pos2::ZERO)
            .unwrap()
            .id
    });

    for (source, target) in nodes.iter().zip(nodes.iter().skip(1)) {
        graph
            .insert_connection(
                port_id(*source, OUTPUT_PORT_INDEX),
                port_id(*target, INPUT_PORT_INDEX),
            )
            .unwrap();
    }

    nodes
}

fn run_until(graph: &mut ThreadedGraph, condition: impl Fn(&ThreadedGraph) -> bool) {
    let deadline = Instant::now() + TEST_TIMEOUT;

    while !condition(graph) {
        assert!(
            Instant::now() < deadline,
            "Graph did not reach the expected state in time"
        );
        graph.drain_events();
        thread::sleep(POLL_INTERVAL);
    }
}

#[test]
fn focus_node_on_threaded_chain_reports_value_to_handler() {
    let (mut graph, handler) = graph_with(threaded_options());
    let [_, _, last] = chain(
        &mut graph,
        [
            ThreadedBlueprint::Source,
            ThreadedBlueprint::Increment,
            ThreadedBlueprint::Increment,
        ],
    );

    graph.focus_node(last).unwrap();

    let expected = Some(Arc::new(Number(SOURCE_VALUE + TWO_INCREMENTS)));
    assert!(wait_until(|| handler.last_output() == expected));
}

#[test]
fn focus_node_on_threaded_chain_marks_ui_nodes_valid() {
    let (mut graph, _) = graph_with(threaded_options());
    let [source, increment] = chain(
        &mut graph,
        [ThreadedBlueprint::Source, ThreadedBlueprint::Increment],
    );

    graph.focus_node(increment).unwrap();
    run_until(&mut graph, |graph| graph.nodes[increment].is_valid());

    assert!(graph.nodes[source].is_valid());
}

#[test]
fn add_default_compute_entry_reaches_worker_threads() {
    let (mut graph, _) = graph_with(threaded_options());
    let probe = ThreadProbe::default();
    graph.add_default_compute_entry(PROBE_KEY, probe.clone());
    let [_, increment] = chain(
        &mut graph,
        [ThreadedBlueprint::Source, ThreadedBlueprint::Increment],
    );

    graph.focus_node(increment).unwrap();
    run_until(&mut graph, |graph| graph.nodes[increment].is_valid());

    let threads = probe.threads();
    assert!(!threads.is_empty());
    assert!(!threads.contains(&thread::current().id()));
}
