
impl MeshRouter {
    pub fn new() -> Self {}
    pub fn setThisAddress() {}
    pub fn sendTo(buf: u8, len: u8, address: u8) -> bool {}
    pub fn recvfrom(buf: u8, len: &[u8], from: u8=NULL, to: u8=NULL, id: u8=NULL, flags: u8=NULL) -> bool {}
    pub fn available() -> bool {}
    pub fn waitAvailable() {}
    pub fn waitPacketSent(timeout: u16) -> bool {}
    pub fn waitAvailableTimeout(timeout: u16) -> bool {}
    pub fn setHeaderTo(to: u8) {}
    pub fn setHeaderFrom(from: u8) {}
    pub fn setHeaderId(id: u8) {}
    pub fn setHeaderFlags(set: u8, clear: u8=RH_FLAGS_NONE) {}
    pub fn headerTo() -> u8 {}
    pub fn headerFrom() -> u8 {}
    pub fn headerId() -> u8 {}
    pub fn headerFlags() -> u8 {}
    pub fn thisAddress() -> u8 {}
    fn acknowledge(id: u8, from: u8) {}
    pub fn setMaxHops(max_hops: u8) {}
    pub fn addRouteTo(dest: u8, next_hop: u8, state: ) {}
    pub fn getRouteTo(dest: u8) -> RoutingTableEntry {}
    pub fn sendtoWait(buf: &[u8], len: u8, dest: u8, flags: u8=0) -> u8 {}
    pub fn sendtoFromSourceWait(buf: &[u8], len: u8, dest: u8, source: u8, flags: u8=0) -> u8 {}
    pub fn recvfromAck(buf: u8, len: u8, source: u8=NULL, dest: u8=NULL, id: u8=NULL, flags: u8=NULL) -> bool {}
    pub fn recvfromAckTimeout(buf: u8, len: u8, timeout: u8, source: u8=NULL, dest: u8=NULL, id: u8=NULL, flags: u8=NULL) -> bool {}
    fn peekAtMessage() {}
    fn route() -> u8 {}
    fn deleteRoute(index: u8) {}
    fn doArp(address: u8) -> bool {}
    fn isPhyiscalAddress(address: u8, address_len: u8) -> bool {}
}