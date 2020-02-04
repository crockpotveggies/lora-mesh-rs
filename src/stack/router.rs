use std::net::Ipv4Addr;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Clone)]
pub enum RouteState {
    Invalid = 0,
    Discovering = 1,
    Valid = 2
}

#[derive(Clone)]
struct RoutingTableEntry {
    dest: u8,
    nexthop: u8,
    state: RouteState
}

#[derive(Clone)]
pub struct MeshRouter {
    maxhops: i32,
//    retransmissions,
    lastSequenceNumber: i32,
    timeout: Duration,
    retries: i32,
    seenIds: Vec<u8>,
    routes: Vec<RoutingTableEntry>,
    nat: HashMap<Ipv4Addr, u8>
}

impl MeshRouter {
    pub fn new(maxhops: i32, timeout: Duration) -> Self {
        MeshRouter{
            maxhops,
            lastSequenceNumber: 0,
            timeout,
            retries: 1, // TODO
            seenIds: Vec::new(),
            routes: Vec::new(),
            nat: HashMap::new()
        }
    }
}