use log::*;
use std::net::Ipv4Addr;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use std::result::Result;
use packet::ip::v4::Packet;
use petgraph::graphmap::UnGraphMap;
use petgraph::algo::{astar, min_spanning_tree};
use petgraph::data::FromElements;
use petgraph::dot::{Dot, Config};
use std::collections::hash_map::RandomState;
use std::cell::{RefCell, RefMut};
use std::rc::Rc;
use std::borrow::BorrowMut;
use petgraph::visit::{GraphBase, IntoEdges, VisitMap, Visitable};
use crate::stack::message::{BroadcastMessage, IPAssignFailureMessage};
use petgraph::graph::node_index;

#[derive(Clone)]
pub struct MeshRouter {
    nodeid: i8,
    nodeipaddr: Option<Ipv4Addr>,
    maxhops: i32,
    lastSequenceNumber: i32,
    timeout: Duration,
    retries: i32,
    observations: RefCell<HashMap<i8, Instant>>,
    graph: UnGraphMap<i8, i8>,
    id2ip: RefCell<HashMap<i8, Ipv4Addr>>,
    ip2id: RefCell<HashMap<Ipv4Addr, i8>>,

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
            observations: RefCell::new(HashMap::new()),
            graph: UnGraphMap::new(),
            id2ip: RefCell::new(HashMap::new()),
            ip2id: RefCell::new(HashMap::new())
        }
    }

    /// Adds a new node to the mesh, fail if route does not exist
    pub fn route_add(&mut self, nodeid: i8, route: Vec<(i8, i8)>) {
        route.iter().for_each( |(src, dest)| {
            // we track each observation of every node
            self.node_observe(src.clone());
            self.node_observe(dest.clone());

            // now add the node if necessary
            self.borrow_mut().node_add(*src);
            self.borrow_mut().node_add(*dest);

            // now add the edges to our mesh
            self.edge_add(*src, *dest);
        });
    }

    /// Handle a network broadcast, maybe node needs an IP?
    pub fn handle_broadcast(&mut self, broadcast: Box<BroadcastMessage>) -> Result<Option<Ipv4Addr>, IPAssignFailureMessage> {
        let srcid = broadcast.header.expect("Broadcast did not have a frame header.").sender();

        // observe our latest sighting
        self.node_observe(srcid);
        self.node_add(srcid);
        self.edge_add(self.nodeid, srcid);

        let mut ipaddr = None;
        if broadcast.ipOffset == 0i8 {
            ipaddr = Some(self.ip_assign(srcid)?);
        }
        return Ok(ipaddr);
    }

    /// Assign IP address to node
    // TODO implement proper DHCP later
    fn ip_assign(&mut self, nodeid: i8) -> Result<Ipv4Addr, IPAssignFailureMessage> {
        match self.id2ip.get_mut().get(&nodeid) {
            None => {
                let ipaddr = Ipv4Addr::new(10,0,0, nodeid as u8);
                self.id2ip.get_mut().insert(nodeid, ipaddr);
                self.ip2id.get_mut().insert(ipaddr, nodeid);
                return Ok(ipaddr);
            },
            Some(ip) => {
                return Err(IPAssignFailureMessage::new(String::from(format!("IP already assigned to node ID {}", nodeid))));
            }
        }
    }

    /// Track each node observation for routing purposes
    fn node_observe(&mut self, nodeid: i8) {
        self.observations.borrow_mut().insert(nodeid, Instant::now());
    }

    fn edge_add(&mut self, src: i8, dest: i8) {
        self.graph.add_edge(src.clone(), dest.clone(), 1);
    }

    /// Add a new node to our mesh
    fn node_add(&mut self, nodeid: i8) {
        self.graph.add_node(nodeid);
    }

    /// Removes a node from the mesh
    pub fn node_remove(&mut self, nodeid: i8) {
        self.graph.borrow_mut().remove_node(nodeid);
    }

    /// Routes an IP packet to a node in the mesh, if it's possible
    pub fn packet_route(&mut self, packet: &Packet<Vec<u8>>) -> Option<(Vec<i8>)> {
        trace!("IPv4 Source: {}", packet.source());
        trace!("IPv4 Destination: {}", packet.destination());

        // look up ip and ensure it's in our mesh
        let mut ip2id = self.ip2id.borrow_mut();
        let src = ip2id.get(&packet.source())?;
        let dest = ip2id.get(&packet.destination())?;

        match astar(
            &self.graph,
            src.clone(),
            |finish| finish == dest.clone(),
            |e| e.1,
            |e| 0,
        ) {
            None => None,
            Some(aresult) => Some(aresult.1)
        }
    }
}