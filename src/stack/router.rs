use log::*;
use std::net::Ipv4Addr;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use packet::ip::v4::Packet;
use petgraph::graph::{NodeIndex, UnGraph};
use petgraph::algo::{dijkstra, min_spanning_tree};
use petgraph::data::FromElements;
use petgraph::dot::{Dot, Config};
use std::collections::hash_map::RandomState;

#[derive(Clone)]
pub struct MeshRouter {
    nodeid: i8,
    nodeipaddr: Option<Ipv4Addr>,
    maxhops: i32,
    lastSequenceNumber: i32,
    timeout: Duration,
    retries: i32,
    observations: HashMap<i8, Instant>,
    graph: UnGraph::<(), ()>,
    id2idx: HashMap<i8, NodeIndex>,
    idx2id: HashMap<NodeIndex, i8>,
    id2ip: HashMap<i8, Ipv4Addr>,
    ip2id: HashMap<Ipv4Addr, i8>,

}

impl MeshRouter {
    pub fn new(nodeid: i8, nodeipaddr: Option<Ipv4Addr>, maxhops: i32, timeout: Duration) -> Self {
        MeshRouter{
            nodeid,
            nodeipaddr,
            maxhops,
            lastSequenceNumber: 0,
            timeout,
            retries: 1, // TODO
            observations: HashMap::new(),
            graph: UnGraph::<(), ()>::default(),
            id2idx: HashMap:: new(),
            idx2id: HashMap:: new(),
            id2ip: HashMap::new(),
            ip2id: HashMap::new()
        }
    }

    /// Adds a new node to the mesh, fail if route does not exist
    pub fn route_add(&mut self, nodeid: i8, route: Vec<(i8,i8)>) {
        route.iter().for_each( |(src, dest)| {
            // we track each observation of every node
            self.node_observe(src.clone());
            self.node_observe(dest.clone());

            // now add the node if necessary
            let srcidx = self.node_add(nodeid);
            let destidx = self.node_add(nodeid);

            // now add the edges to our mesh
            self.graph.add_edge(srcidx.clone(), destidx.clone(), ());
        });
    }

    /// Track each node observation for routing purposes
    fn node_observe(&mut self, nodeid: i8) {
        self.observations.insert(nodeid, Instant::now());
    }

    /// Add a new node to our mesh
    pub fn node_add(&mut self, nodeid: i8) -> &NodeIndex<u32> {
        match self.id2idx.get(&nodeid) {
            None => {
                let index = self.graph.add_node(());
                self.id2idx.insert(nodeid, index);
                self.idx2id.insert(index, nodeid);
                return &index
            }
            Some(idx) => return idx // ignore, we have it already
        }
    }

    /// Removes a node from the mesh
    pub fn node_remove(&mut self, nodeid: i8) {
        match self.id2idx.get(&nodeid) {
            None => {}, // didn't exist
            Some(index) => {
                self.graph.remove_node(index.clone());
            }
        }
    }

    /// Routes an IP packet to a node in the mesh, if it's possible
    pub fn packet_route(&mut self, packet: &Packet<Vec<u8>>) -> Option<HashMap<i8, i32, RandomState>> {
        trace!("IPv4 Source: {}", packet.source());
        trace!("IPv4 Destination: {}", packet.destination());

        // look up ip and ensure it's in our mesh
        let destip = self.ip2id.get(&packet.destination());
        match destip {
            None => return None, // drop the packet
            Some(destid) => {
                return Some(dijkstra(&self.graph, self.nodeid.into(), Some(destid.into()), |_| 1));
            }
        }
    }
}