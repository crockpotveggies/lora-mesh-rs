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
    nodeid: i32,
    nodeipaddr: Option<Ipv4Addr>,
    gatewayipaddr: Option<Ipv4Addr>,
    maxhops: i32,
    lastSequenceNumber: i32,
    timeout: Duration,
    retries: i32,
    observations: RefCell<HashMap<i32, Instant>>,
    graph: UnGraphMap<i32, i32>,
    id2ip: RefCell<HashMap<i32, Ipv4Addr>>,
    ip2id: RefCell<HashMap<Ipv4Addr, i32>>,
    isgateway: bool

}

impl MeshRouter {
    pub fn new(nodeid: i32, nodeipaddr: Option<Ipv4Addr>, gatewayipaddr: Option<Ipv4Addr>, maxhops: i32, timeout: Duration, isgateway: bool) -> Self {
        let mut router = MeshRouter{
            nodeid,
            nodeipaddr,
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
        };

        // ensure this router appears in our routing table
        nodeipaddr.map(|ipaddr| {
            router.node_add(nodeid);
            router.id2ip.borrow_mut().insert(nodeid, ipaddr);
            router.ip2id.borrow_mut().insert(ipaddr, nodeid);
        });

        return router;
    }

    /// Applies a spanning tree algorithm to the mesh graph
    pub fn min_spanning_tree(&mut self) {
        let graph = UnGraphMap::from_elements(min_spanning_tree(&self.graph));
        self.graph = graph;
    }

    /// Adds a new route to the mesh, fail if route does not exist
    pub fn route_add(&mut self, route: Vec<(i32, i32)>) {
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

    /// Handle a network broadcast, maybe node needs an IP?
    pub fn handle_broadcast(&mut self, broadcast: Box<BroadcastMessage>, route: Vec<i32>) -> Result<Option<(Ipv4Addr, bool)>, IPAssignFailureMessage> {
        let srcid = broadcast.header.expect("Broadcast did not have a frame header.").sender();
        if broadcast.isgateway && srcid != self.nodeid {
            info!("New gateway {} discovered with IP {}", &srcid, &broadcast.ipaddr.expect("Gateways must broadcast their IP"));
            self.gatewayipaddr = broadcast.ipaddr;
        }

        // observe our latest sighting
        route.iter().for_each(|nodeid| {
            self.node_observe_put(nodeid.clone());
            self.node_add(nodeid.clone());
        });

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
    fn ip_assign(&mut self, nodeid: i32) -> Result<(Ipv4Addr, bool), IPAssignFailureMessage> {
        match self.id2ip.get_mut().get(&nodeid) {
            None => {
                let ipaddr = Ipv4Addr::new(172,16,0, nodeid as u8);
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
    fn node_observe_put(&mut self, nodeid: i32) {
        self.observations.borrow_mut().insert(nodeid, Instant::now());
    }

    pub fn node_observe_get(&mut self, nodeid: &i32) -> Option<&Instant> {
        self.observations.get_mut().get(nodeid)
    }

    fn edge_add(&mut self, src: i32, dest: i32) {
        self.graph.add_edge(src.clone(), dest.clone(), 1);
    }

    /// Add a new node to our mesh
    fn node_add(&mut self, nodeid: i32) {
        self.graph.add_node(nodeid);
    }

    /// Removes a node from the mesh
    pub fn node_remove(&mut self, nodeid: i32) {
        self.graph.borrow_mut().remove_node(nodeid);
    }

    /// Routes an IP packet to a node in the mesh, if it's possible
    pub fn packet_route(&mut self, packet: &Packet<Vec<u8>>) -> Option<Vec<i32>> {
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