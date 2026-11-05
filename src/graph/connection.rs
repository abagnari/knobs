use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Debug;

use egui::Pos2;

use crate::PortTypeInfo;
use crate::graph::node::NodeId;
use crate::graph::node::port::PortId;

#[derive(Debug, Copy, Clone)]
pub(crate) struct DragConnection<P>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
{
    pub(crate) from_port: PortId,
    pub(crate) from_position: Pos2,
    pub(crate) from_type: P,
    pub(crate) from_input: bool,
}

impl<P> DragConnection<P>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
{
    pub(crate) fn is_input(&self) -> bool {
        self.from_input
    }
}

#[derive(Default, Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) struct Connection {
    pub(crate) from_port: PortId,
    pub(crate) to_port: PortId,
}

#[derive(Default, Debug, Clone)]
pub(crate) struct Connections {
    sources_by_destination: BTreeMap<PortId, PortId>,
}

impl Connections {
    pub(crate) fn insert(&mut self, from_port: PortId, to_port: PortId) -> Option<Connection> {
        self.sources_by_destination
            .insert(to_port, from_port)
            .map(|previous_source| Connection {
                from_port: previous_source,
                to_port,
            })
    }

    pub(crate) fn remove(&mut self, to_port: PortId) -> Option<Connection> {
        self.sources_by_destination
            .remove(&to_port)
            .map(|from_port| Connection { from_port, to_port })
    }

    pub(crate) fn contains_target(&self, to_port: PortId) -> bool {
        self.sources_by_destination.contains_key(&to_port)
    }

    pub(crate) fn clear(&mut self) {
        self.sources_by_destination.clear();
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item=Connection> + '_ {
        self.sources_by_destination
            .iter()
            .map(|(to_port, from_port)| Connection {
                from_port: *from_port,
                to_port: *to_port,
            })
    }

    pub(crate) fn touching(&self, node_id: NodeId) -> Vec<Connection> {
        self.iter()
            .filter(|connection| {
                connection.from_port.node_id() == node_id || connection.to_port.node_id() == node_id
            })
            .collect()
    }

    pub(crate) fn upstream_nodes(&self, node_id: NodeId) -> Vec<NodeId> {
        self.iter()
            .filter(|connection| connection.to_port.node_id() == node_id)
            .map(|connection| connection.from_port.node_id())
            .collect::<BTreeSet<NodeId>>()
            .into_iter()
            .collect()
    }

    pub(crate) fn downstream_nodes(&self, node_id: NodeId) -> Vec<NodeId> {
        self.iter()
            .filter(|connection| connection.from_port.node_id() == node_id)
            .map(|connection| connection.to_port.node_id())
            .collect::<BTreeSet<NodeId>>()
            .into_iter()
            .collect()
    }

    pub(crate) fn dependency_graph(&self) -> HashMap<NodeId, Vec<NodeId>> {
        let mut graph: HashMap<NodeId, Vec<NodeId>> = HashMap::new();

        for connection in self.iter() {
            graph
                .entry(connection.to_port.node_id())
                .or_default()
                .push(connection.from_port.node_id());
        }

        graph
    }

    #[allow(dead_code)]
    pub(crate) fn dependent_graph(&self) -> HashMap<NodeId, Vec<NodeId>> {
        let mut graph: HashMap<NodeId, Vec<NodeId>> = HashMap::new();

        for connection in self.iter() {
            graph
                .entry(connection.from_port.node_id())
                .or_default()
                .push(connection.to_port.node_id());
        }

        graph
    }
}

#[cfg(test)]
mod tests {
    use slotmap::SlotMap;

    use super::*;
    use crate::graph::node::port::port_id;

    const FIRST_PORT: u32 = 0;
    const SECOND_PORT: u32 = 1;
    const OUTPUT_PORT: u32 = 2;
    const NODE_COUNT: usize = 3;
    const PAIR: usize = 2;

    fn node_ids() -> [NodeId; NODE_COUNT] {
        let mut allocator: SlotMap<NodeId, ()> = SlotMap::with_key();

        std::array::from_fn(|_| allocator.insert(()))
    }

    #[test]
    fn insert_into_free_target_returns_none() {
        let [source, target, _] = node_ids();
        let mut connections = Connections::default();

        let replaced =
            connections.insert(port_id(source, OUTPUT_PORT), port_id(target, FIRST_PORT));

        assert_eq!(replaced, None);
    }

    #[test]
    fn insert_into_occupied_target_returns_replaced_connection() {
        let [first_source, target, second_source] = node_ids();
        let mut connections = Connections::default();
        let target_port = port_id(target, FIRST_PORT);
        connections.insert(port_id(first_source, OUTPUT_PORT), target_port);

        let replaced = connections.insert(port_id(second_source, OUTPUT_PORT), target_port);

        assert_eq!(
            replaced,
            Some(Connection {
                from_port: port_id(first_source, OUTPUT_PORT),
                to_port: target_port,
            })
        );
    }

    #[test]
    fn remove_existing_target_returns_connection() {
        let [source, target, _] = node_ids();
        let mut connections = Connections::default();
        let target_port = port_id(target, FIRST_PORT);
        connections.insert(port_id(source, OUTPUT_PORT), target_port);

        let removed = connections.remove(target_port);

        assert_eq!(
            removed,
            Some(Connection {
                from_port: port_id(source, OUTPUT_PORT),
                to_port: target_port,
            })
        );
    }

    #[test]
    fn remove_missing_target_returns_none() {
        let [_, target, _] = node_ids();
        let mut connections = Connections::default();

        let removed = connections.remove(port_id(target, FIRST_PORT));

        assert_eq!(removed, None);
    }

    #[test]
    fn contains_target_reports_connected_port() {
        let [source, target, _] = node_ids();
        let mut connections = Connections::default();
        connections.insert(port_id(source, OUTPUT_PORT), port_id(target, FIRST_PORT));

        let contains = connections.contains_target(port_id(target, FIRST_PORT));

        assert!(contains);
    }

    #[test]
    fn clear_removes_every_connection() {
        let [source, target, _] = node_ids();
        let mut connections = Connections::default();
        connections.insert(port_id(source, OUTPUT_PORT), port_id(target, FIRST_PORT));

        connections.clear();

        assert_eq!(connections.iter().count(), 0);
    }

    #[test]
    fn iter_yields_every_connection() {
        let [source, target, _] = node_ids();
        let mut connections = Connections::default();
        connections.insert(port_id(source, OUTPUT_PORT), port_id(target, FIRST_PORT));
        connections.insert(port_id(source, OUTPUT_PORT), port_id(target, SECOND_PORT));

        let count = connections.iter().count();

        assert_eq!(count, PAIR);
    }

    #[test]
    fn touching_returns_inbound_and_outbound_connections() {
        let [upstream, middle, downstream] = node_ids();
        let mut connections = Connections::default();
        connections.insert(port_id(upstream, OUTPUT_PORT), port_id(middle, FIRST_PORT));
        connections.insert(
            port_id(middle, OUTPUT_PORT),
            port_id(downstream, FIRST_PORT),
        );

        let touching = connections.touching(middle);

        assert_eq!(touching.len(), PAIR);
    }

    #[test]
    fn upstream_nodes_with_parallel_connections_returns_source_once() {
        let [source, target, _] = node_ids();
        let mut connections = Connections::default();
        connections.insert(port_id(source, OUTPUT_PORT), port_id(target, FIRST_PORT));
        connections.insert(port_id(source, OUTPUT_PORT), port_id(target, SECOND_PORT));

        let upstream = connections.upstream_nodes(target);

        assert_eq!(upstream, vec![source]);
    }

    #[test]
    fn upstream_nodes_after_removing_one_parallel_connection_keeps_source() {
        let [source, target, _] = node_ids();
        let mut connections = Connections::default();
        connections.insert(port_id(source, OUTPUT_PORT), port_id(target, FIRST_PORT));
        connections.insert(port_id(source, OUTPUT_PORT), port_id(target, SECOND_PORT));
        connections.remove(port_id(target, FIRST_PORT));

        let upstream = connections.upstream_nodes(target);

        assert_eq!(upstream, vec![source]);
    }

    #[test]
    fn downstream_nodes_returns_every_target_node() {
        let [source, first_target, second_target] = node_ids();
        let mut connections = Connections::default();
        connections.insert(
            port_id(source, OUTPUT_PORT),
            port_id(first_target, FIRST_PORT),
        );
        connections.insert(
            port_id(source, OUTPUT_PORT),
            port_id(second_target, FIRST_PORT),
        );
        let mut expected = vec![first_target, second_target];
        expected.sort();

        let downstream = connections.downstream_nodes(source);

        assert_eq!(downstream, expected);
    }

    #[test]
    fn dependency_graph_maps_targets_to_their_sources() {
        let [source, target, _] = node_ids();
        let mut connections = Connections::default();
        connections.insert(port_id(source, OUTPUT_PORT), port_id(target, FIRST_PORT));

        let graph = connections.dependency_graph();

        assert_eq!(graph[&target], vec![source]);
    }
}
