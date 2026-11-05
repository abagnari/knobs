use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use egui::{Pos2, pos2, vec2};
use rand::{Rng, RngExt};

use crate::graph::node::NodeId;

pub(crate) fn cubic_bezier(p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2, t: f32) -> Pos2 {
    let u = 1.0 - t;

    pos2(
        u.powi(3) * p0.x
            + 3.0 * u.powi(2) * t * p1.x
            + 3.0 * u * t.powi(2) * p2.x
            + t.powi(3) * p3.x,
        u.powi(3) * p0.y
            + 3.0 * u.powi(2) * t * p1.y
            + 3.0 * u * t.powi(2) * p2.y
            + t.powi(3) * p3.y,
    )
}

pub(crate) fn bezier_controls(start: Pos2, end: Pos2) -> (Pos2, Pos2) {
    let distance = (end.x - start.x).abs();
    let offset = (distance * 0.5).max(30.0);

    let direction = if end.x >= start.x { 1.0 } else { -1.0 };

    let c1 = start + vec2(offset * direction, 0.0);
    let c2 = end - vec2(offset * direction, 0.0);

    (c1, c2)
}

#[allow(dead_code)]
pub(crate) fn has_strongly_connected_pair(graph: &HashMap<NodeId, Vec<NodeId>>) -> bool {
    // Build the reversed graph.
    let mut reversed: HashMap<NodeId, Vec<NodeId>> = HashMap::new();

    for (&u, neighbors) in graph {
        // Make sure nodes with no outgoing edges also exist.
        reversed.entry(u).or_default();

        for &v in neighbors {
            reversed.entry(v).or_default().push(u);
        }
    }

    // First DFS: calculate finishing order.
    fn dfs1(
        u: NodeId,
        graph: &HashMap<NodeId, Vec<NodeId>>,
        visited: &mut HashSet<NodeId>,
        order: &mut Vec<NodeId>,
    ) {
        if !visited.insert(u) {
            return;
        }

        if let Some(neighbors) = graph.get(&u) {
            for &v in neighbors {
                dfs1(v, graph, visited, order);
            }
        }

        order.push(u);
    }

    let mut visited = HashSet::new();
    let mut order = Vec::new();

    for &u in graph.keys() {
        dfs1(u, graph, &mut visited, &mut order);
    }

    // Second DFS: find SCCs in the reversed graph.
    fn dfs2(
        u: NodeId,
        graph: &HashMap<NodeId, Vec<NodeId>>,
        visited: &mut HashSet<NodeId>,
        component_size: &mut usize,
    ) {
        if !visited.insert(u) {
            return;
        }

        *component_size += 1;

        if let Some(neighbors) = graph.get(&u) {
            for &v in neighbors {
                dfs2(v, graph, visited, component_size);
            }
        }
    }

    visited.clear();

    // Process vertices in decreasing finishing time.
    for &u in order.iter().rev() {
        if !visited.contains(&u) {
            let mut component_size = 0;

            dfs2(u, &reversed, &mut visited, &mut component_size);

            if component_size >= 2 {
                return true;
            }
        }
    }

    false
}

pub(crate) fn compute_strongly_connected_nodes<N>(graph: &HashMap<N, Vec<N>>) -> Vec<Vec<N>>
where
    N: Copy + Eq + Hash,
{
    struct Tarjan<'a, N> {
        graph: &'a HashMap<N, Vec<N>>,

        // DFS discovery index for each node.
        indices: HashMap<N, usize>,

        // Lowest discovery index reachable from each node.
        lowlinks: HashMap<N, usize>,

        // Nodes currently on the DFS stack.
        stack: Vec<N>,

        // Fast lookup for whether a node is on the stack.
        on_stack: HashSet<N>,

        // Next discovery index.
        next_index: usize,

        // Resulting SCCs.
        components: Vec<Vec<N>>,
    }

    impl<'a, N: Copy + Eq + Hash> Tarjan<'a, N> {
        fn new(graph: &'a HashMap<N, Vec<N>>) -> Self {
            Self {
                graph,
                indices: HashMap::new(),
                lowlinks: HashMap::new(),
                stack: Vec::new(),
                on_stack: HashSet::new(),
                next_index: 0,
                components: Vec::new(),
            }
        }

        fn visit(&mut self, node: N) {
            let index = self.next_index;
            self.next_index += 1;

            self.indices.insert(node, index);
            self.lowlinks.insert(node, index);

            self.stack.push(node);
            self.on_stack.insert(node);

            // Clone the neighbors so we don't hold an immutable
            // borrow of self.graph while recursively calling visit().
            let neighbors = self.graph.get(&node).cloned().unwrap_or_default();

            for neighbor in neighbors {
                if !self.indices.contains_key(&neighbor) {
                    // Neighbor has not been visited yet.
                    self.visit(neighbor);

                    let neighbor_lowlink = self.lowlinks[&neighbor];
                    let node_lowlink = self.lowlinks[&node];

                    self.lowlinks
                        .insert(node, node_lowlink.min(neighbor_lowlink));
                }
                else if self.on_stack.contains(&neighbor) {
                    // Back edge to a node in the current SCC.
                    let neighbor_index = self.indices[&neighbor];
                    let node_lowlink = self.lowlinks[&node];

                    self.lowlinks.insert(node, node_lowlink.min(neighbor_index));
                }
            }

            // If node is the root of an SCC, pop the SCC.
            if self.lowlinks[&node] == self.indices[&node] {
                let mut component = Vec::new();

                loop {
                    let member = self.stack.pop().expect("Tarjan stack should not be empty");

                    self.on_stack.remove(&member);
                    component.push(member);

                    if member == node {
                        break;
                    }
                }

                // Only return SCCs containing multiple nodes.
                if component.len() >= 2 {
                    self.components.push(component);
                }
            }
        }

        fn run(mut self) -> Vec<Vec<N>> {
            // Include nodes that only occur as destinations.
            let mut nodes: HashSet<N> = self.graph.keys().copied().collect();

            for neighbors in self.graph.values() {
                nodes.extend(neighbors.iter().copied());
            }

            for node in nodes {
                if !self.indices.contains_key(&node) {
                    self.visit(node);
                }
            }

            self.components
        }
    }

    Tarjan::new(graph).run()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODE_A: u32 = 0;
    const NODE_B: u32 = 1;
    const NODE_C: u32 = 2;
    const NODE_D: u32 = 3;
    const NODE_E: u32 = 4;
    const NODE_F: u32 = 5;
    const REPEATED_RUN_COUNT: usize = 50;

    fn graph_of(edges: &[(u32, &[u32])]) -> HashMap<u32, Vec<u32>> {
        edges
            .iter()
            .map(|(source, targets)| (*source, targets.to_vec()))
            .collect()
    }

    fn normalized(mut components: Vec<Vec<u32>>) -> Vec<Vec<u32>> {
        for component in components.iter_mut() {
            component.sort();
        }

        components.sort();
        components
    }

    fn expected_components(components: &[&[u32]]) -> Vec<Vec<u32>> {
        normalized(
            components
                .iter()
                .map(|component| component.to_vec())
                .collect(),
        )
    }

    fn two_node_ids() -> (NodeId, NodeId) {
        let mut ids: slotmap::SlotMap<NodeId, ()> = slotmap::SlotMap::with_key();
        (ids.insert(()), ids.insert(()))
    }

    #[test]
    fn has_strongly_connected_pair_with_two_node_cycle_returns_true() {
        let (first, second) = two_node_ids();
        let graph = HashMap::from([(first, vec![second]), (second, vec![first])]);

        let result = has_strongly_connected_pair(&graph);

        assert!(result);
    }

    #[test]
    fn has_strongly_connected_pair_with_acyclic_chain_returns_false() {
        let (first, second) = two_node_ids();
        let graph = HashMap::from([(first, vec![second]), (second, vec![])]);

        let result = has_strongly_connected_pair(&graph);

        assert!(!result);
    }

    #[test]
    fn compute_strongly_connected_nodes_with_empty_graph_returns_no_components() {
        let graph = graph_of(&[]);

        let actual = compute_strongly_connected_nodes(&graph);

        assert!(actual.is_empty());
    }

    #[test]
    fn compute_strongly_connected_nodes_with_isolated_node_returns_no_components() {
        let graph = graph_of(&[(NODE_A, &[])]);

        let actual = compute_strongly_connected_nodes(&graph);

        assert!(actual.is_empty());
    }

    #[test]
    fn compute_strongly_connected_nodes_with_acyclic_chain_returns_no_components() {
        let graph = graph_of(&[(NODE_A, &[NODE_B]), (NODE_B, &[NODE_C])]);

        let actual = compute_strongly_connected_nodes(&graph);

        assert!(actual.is_empty());
    }

    #[test]
    fn compute_strongly_connected_nodes_with_diamond_shape_returns_no_components() {
        let graph = graph_of(&[
            (NODE_A, &[NODE_B, NODE_C]),
            (NODE_B, &[NODE_D]),
            (NODE_C, &[NODE_D]),
        ]);

        let actual = compute_strongly_connected_nodes(&graph);

        assert!(actual.is_empty());
    }

    #[test]
    fn compute_strongly_connected_nodes_with_self_loop_returns_no_components() {
        let graph = graph_of(&[(NODE_A, &[NODE_A])]);

        let actual = compute_strongly_connected_nodes(&graph);

        assert!(actual.is_empty());
    }

    #[test]
    fn compute_strongly_connected_nodes_with_two_node_cycle_returns_that_cycle() {
        let graph = graph_of(&[(NODE_A, &[NODE_B]), (NODE_B, &[NODE_A])]);

        let actual = compute_strongly_connected_nodes(&graph);

        assert_eq!(
            normalized(actual),
            expected_components(&[&[NODE_A, NODE_B]])
        );
    }

    #[test]
    fn compute_strongly_connected_nodes_with_three_node_cycle_returns_all_members() {
        let graph = graph_of(&[
            (NODE_A, &[NODE_B]),
            (NODE_B, &[NODE_C]),
            (NODE_C, &[NODE_A]),
        ]);

        let actual = compute_strongly_connected_nodes(&graph);

        assert_eq!(
            normalized(actual),
            expected_components(&[&[NODE_A, NODE_B, NODE_C]])
        );
    }

    #[test]
    fn compute_strongly_connected_nodes_with_duplicate_edges_returns_single_component() {
        let graph = graph_of(&[(NODE_A, &[NODE_B, NODE_B]), (NODE_B, &[NODE_A, NODE_A])]);

        let actual = compute_strongly_connected_nodes(&graph);

        assert_eq!(
            normalized(actual),
            expected_components(&[&[NODE_A, NODE_B]])
        );
    }

    #[test]
    fn compute_strongly_connected_nodes_with_cycle_and_attached_tails_returns_only_the_cycle() {
        let graph = graph_of(&[
            (NODE_A, &[NODE_B]),
            (NODE_B, &[NODE_C]),
            (NODE_C, &[NODE_B, NODE_D]),
            (NODE_D, &[]),
        ]);

        let actual = compute_strongly_connected_nodes(&graph);

        assert_eq!(
            normalized(actual),
            expected_components(&[&[NODE_B, NODE_C]])
        );
    }

    #[test]
    fn compute_strongly_connected_nodes_with_overlapping_cycles_merges_them() {
        let graph = graph_of(&[
            (NODE_A, &[NODE_B]),
            (NODE_B, &[NODE_C]),
            (NODE_C, &[NODE_A, NODE_D]),
            (NODE_D, &[NODE_C]),
        ]);

        let actual = compute_strongly_connected_nodes(&graph);

        assert_eq!(
            normalized(actual),
            expected_components(&[&[NODE_A, NODE_B, NODE_C, NODE_D]])
        );
    }

    #[test]
    fn compute_strongly_connected_nodes_with_disjoint_cycles_returns_both_components() {
        let graph = graph_of(&[
            (NODE_A, &[NODE_B]),
            (NODE_B, &[NODE_A]),
            (NODE_C, &[NODE_D]),
            (NODE_D, &[NODE_C]),
        ]);

        let actual = compute_strongly_connected_nodes(&graph);

        assert_eq!(
            normalized(actual),
            expected_components(&[&[NODE_A, NODE_B], &[NODE_C, NODE_D]])
        );
    }

    #[test]
    fn compute_strongly_connected_nodes_with_chained_cycles_returns_separate_components() {
        let graph = graph_of(&[
            (NODE_A, &[NODE_B]),
            (NODE_B, &[NODE_C]),
            (NODE_C, &[NODE_A, NODE_D]),
            (NODE_D, &[NODE_E]),
            (NODE_E, &[NODE_D, NODE_F]),
            (NODE_F, &[]),
        ]);

        let actual = compute_strongly_connected_nodes(&graph);

        assert_eq!(
            normalized(actual),
            expected_components(&[&[NODE_A, NODE_B, NODE_C], &[NODE_D, NODE_E]])
        );
    }

    #[test]
    fn compute_strongly_connected_nodes_with_self_loop_next_to_cycle_returns_only_the_cycle() {
        let graph = graph_of(&[
            (NODE_A, &[NODE_A]),
            (NODE_B, &[NODE_C]),
            (NODE_C, &[NODE_B]),
        ]);

        let actual = compute_strongly_connected_nodes(&graph);

        assert_eq!(
            normalized(actual),
            expected_components(&[&[NODE_B, NODE_C]])
        );
    }

    #[test]
    fn compute_strongly_connected_nodes_with_destination_only_node_does_not_panic() {
        let graph = graph_of(&[(NODE_A, &[NODE_B])]);

        let actual = compute_strongly_connected_nodes(&graph);

        assert!(actual.is_empty());
    }

    #[test]
    fn compute_strongly_connected_nodes_with_node_reachable_from_two_roots_returns_it_once() {
        let graph = graph_of(&[
            (NODE_A, &[NODE_C]),
            (NODE_B, &[NODE_C]),
            (NODE_C, &[NODE_D]),
            (NODE_D, &[NODE_C]),
        ]);

        let actual = compute_strongly_connected_nodes(&graph);

        assert_eq!(
            normalized(actual),
            expected_components(&[&[NODE_C, NODE_D]])
        );
    }

    #[test]
    fn compute_strongly_connected_nodes_with_repeated_runs_returns_stable_results() {
        let graph = graph_of(&[
            (NODE_A, &[NODE_B]),
            (NODE_B, &[NODE_A, NODE_C]),
            (NODE_C, &[NODE_D]),
            (NODE_D, &[NODE_C, NODE_E]),
            (NODE_E, &[NODE_F]),
            (NODE_F, &[NODE_E]),
        ]);
        let expected =
            expected_components(&[&[NODE_A, NODE_B], &[NODE_C, NODE_D], &[NODE_E, NODE_F]]);

        for _ in 0..REPEATED_RUN_COUNT {
            assert_eq!(
                normalized(compute_strongly_connected_nodes(&graph)),
                expected
            );
        }
    }
}

pub(crate) fn grid_line_positions(start: f32, end: f32, spacing: f32) -> Vec<f32> {
    if spacing <= 0.0 || start > end {
        return Vec::new();
    }

    let first_index = (start / spacing).ceil() as i64;
    let last_index = (end / spacing).floor() as i64;

    (first_index..=last_index)
        .map(|index| index as f32 * spacing)
        .collect()
}

/// Returns a random u32 within a bounded range
pub(crate) fn random_in_range(rng: &mut impl Rng, min_bound: u32, max_bound: u32) -> u32 {
    rng.random_range(min_bound..=max_bound)
}
