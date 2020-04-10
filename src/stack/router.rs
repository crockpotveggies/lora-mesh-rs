use log::*;
use std::net::Ipv4Addr;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use std::result::Result;
use packet::ip::v4::Packet;
use petgraph::graphmap::UnGraphMap;
use petgraph::algo::{astar, min_spanning_tree};
use petgraph::data::FromElements;
use std::cell::{RefCell};
use std::borrow::{BorrowMut};
use crate::stack::message::{BroadcastMessage, IPAssignFailureMessage};

#[derive(Clone)]
pub struct MeshRouter {
    nodeid: u8,
    gatewayipaddr: Option<Ipv4Addr>,
    maxhops: u8,
    lastSequenceNumber: u8,
    timeout: Duration,
    retries: u8,
    observations: RefCell<HashMap<u8, Instant>>,
    graph: UnGraphMap<u8, u8>,
    id2ip: RefCell<HashMap<u8, Ipv4Addr>>,
    ip2id: RefCell<HashMap<Ipv4Addr, u8>>,
    isgateway: bool

}

impl MeshRouter {
    pub fn new(nodeid: u8, gatewayipaddr: Option<Ipv4Addr>, maxhops: u8, timeout: Duration, isgateway: bool) -> Self {
        MeshRouter{
            nodeid,
            gatewayipaddr,
            maxhops,
            lastSequenceNumber: 0,
            timeout,
            retries: 1, // TODO
            observations: RefCell::new(HashMap::new()),
            graph: UnGraphMap::new(),
            id2ip: RefCell::new(HashMap::new()),
            ip2id: RefCell::new(HashMap::new()),
            isgateway
        }
    }

    /// Applies a spanning tree algorithm to the mesh graph
    pub fn min_spanning_tree(&mut self) {
        let graph = UnGraphMap::from_elements(min_spanning_tree(&self.graph));
        self.graph = graph;
    }

    /// Adds a new route to the mesh, fail if route does not exist
    pub fn route_add(&mut self, route: Vec<(u8, u8)>) {
        route.iter().for_each( |(src, dest)| {
            // we track each observation of every node
            self.node_observe_put(src.clone());
            self.node_observe_put(dest.clone());

            // now add the node if necessary
            self.borrow_mut().node_add(*src);
            self.borrow_mut().node_add(*dest);

            // now add the edges to our mesh
            self.edge_add(*src, *dest);
        });
    }

    /// Update our router with new IP
    pub fn handle_ip_assignment(&mut self, ipaddr: &Ipv4Addr) {
        self.node_add(self.nodeid.clone());
        self.id2ip.borrow_mut().insert(self.nodeid.clone(), ipaddr.clone());
        self.ip2id.borrow_mut().insert(ipaddr.clone(), self.nodeid.clone());
    }

    pub fn handle_gateway_assignment(&mut self, gatewayip: &Ipv4Addr) {
        self.gatewayipaddr = Some(gatewayip.clone());
    }

    /// Handle a network broadcast, maybe node needs an IP?
    pub fn handle_broadcast(&mut self, broadcast: Box<BroadcastMessage>, route: Vec<u8>) -> Result<Option<(Ipv4Addr, bool)>, IPAssignFailureMessage> {
        let srcid = broadcast.header.expect("Broadcast did not have a frame header.").sender();
        if broadcast.isgateway && srcid != self.nodeid {
            info!("Gateway {} observed with IP {}", &srcid, &broadcast.ipaddr.expect("Gateways must broadcast their IP"));
            self.handle_gateway_assignment(&broadcast.ipaddr.unwrap());
        }

        // observe our latest sighting
        route.iter().for_each(|nodeid| {
            self.node_observe_put(nodeid.clone());
            self.node_add(nodeid.clone());
        });

        // add IP to graph
        if broadcast.ipaddr.is_some() {
            self.id2ip.borrow_mut().insert(srcid.clone(), broadcast.ipaddr.unwrap());
            self.ip2id.borrow_mut().insert(broadcast.ipaddr.unwrap(), srcid.clone());
        }

        // add edges for each node in the route
        route.windows(2).for_each(|pair| self.edge_add(pair[0], pair[1]));

        // add edge for ourself
        self.edge_add(self.nodeid, route.last().expect("Received broadcast with empty route").clone());

        let mut ipaddrtup = None;
        if broadcast.ipOffset == 0 && self.isgateway {
            ipaddrtup = Some(self.ip_assign(srcid)?);
        }
        return Ok(ipaddrtup);
    }

    /// Assign IP address to node
    // TODO implement proper DHCP later
    fn ip_assign(&mut self, nodeid: u8) -> Result<(Ipv4Addr, bool), IPAssignFailureMessage> {
        match self.id2ip.get_mut().get(&nodeid) {
            None => {
                let ipaddr = Ipv4Addr::new(172,16,0, nodeid);
                self.id2ip.get_mut().insert(nodeid, ipaddr);
                self.ip2id.get_mut().insert(ipaddr, nodeid);
                return Ok((ipaddr, true));
            },
            Some(ip) => {
                return Ok((ip.clone(), false));
            }
        }
    }

    /// Track each node observation for routing purposes
    fn node_observe_put(&mut self, nodeid: u8) {
        self.observations.borrow_mut().insert(nodeid, Instant::now());
    }

    pub fn node_observe_get(&mut self, nodeid: &u8) -> Option<&Instant> {
        self.observations.get_mut().get(nodeid)
    }

    fn edge_add(&mut self, src: u8, dest: u8) {
        self.graph.add_edge(src.clone(), dest.clone(), 1);
    }

    /// Add a new node to our mesh
    fn node_add(&mut self, nodeid: u8) {
        self.graph.add_node(nodeid);
    }

    /// Removes a node from the mesh
    pub fn node_remove(&mut self, nodeid: u8) {
        self.graph.borrow_mut().remove_node(nodeid);
    }

    /// Routes an IP packet to a node in the mesh, if it's possible
    pub fn packet_route(&mut self, packet: &Packet<Vec<u8>>) -> Option<Vec<u8>> {
        trace!("Routing packet from {} to {}", &packet.source(), &packet.destination());

        // look up ip and ensure it's in our mesh
        let ip2id = self.ip2id.borrow_mut();
        let src = ip2id.get(&packet.source())?;
        let dest = ip2id.get(&packet.destination())?;
        trace!("Found node route source {:?} destination {:?}", &src, &dest);

        match astar(
            &self.graph,
            src.clone(),
            |finish| finish == dest.clone(),
            |e| e.1,
            |_e| 0,
        ) {
            None => None,
            Some(aresult) => Some(aresult.1)
        }
    }
}