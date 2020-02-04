use crate::MESH_MAX_MESSAGE_LEN;
use std::net::Ipv4Addr;

/// Defines the type of message in the protocol.
pub enum MessageType {
    RouteFailure = 0,
    RouteDiscovery = 1,
    RouteSuccess = 2,
    Application = 3,
    Broadcast = 4,
    TransmitRequest = 5,
    TransmitConfirm = 6
}

pub trait MeshMessage {
    fn getmsgtype(&self) -> MessageType;
}

/// Header for a mesh message.
pub struct MessageHeader {
    pub msgtype: MessageType,
    pub sender: i8
}

/// Transmits application-level data.
pub struct ApplicationMessage {
    pub header: MessageHeader,
    pub to: i8, // destination node
    pub data: [u8; MESH_MAX_MESSAGE_LEN - 3] // application data
}

/// Broadcasts by a mesh node to discover a route to a node.
pub struct RouteDiscoveryMessage {
    pub header: MessageHeader,
    pub dest: i8, // destination node being sought
    pub invalidhops: [u8; MESH_MAX_MESSAGE_LEN - 3] // nodes tried so far
}

/// Replies to a discovery message with a successful route.
pub struct RouteSuccessMessage {
    pub header: MessageHeader,
    pub to: i8, // the node requesting discovery
    pub dest: i8, // destination node being sought
    pub hops: [i8; MESH_MAX_MESSAGE_LEN - 4] // the nodes, in sequence, the requester must hop to deliver a message
}

/// A node is no longer reachable from the sender.
pub struct RouteFailureMessage {
    pub header: MessageHeader,
    pub failednodeid: i8
}

/// Broadcast this node to nearby devices.
pub struct BroadcastMessage {
    pub header: MessageHeader,
    pub isgateway: bool,
    pub ipaddr: Option<Ipv4Addr>
}

/// Assign an IP address to a node.
pub struct AssignIPMessage {
    pub header: MessageHeader,
    pub to: i8,
    pub ipaddr: [i8; 4]
}

/// Request destination node if okay to transmit.
pub struct TransmitRequestMessage {
    pub header: MessageHeader,
    pub dest: i8 // the intended receiver
}

/// Confirm to original requester that it is okay to transmit.
pub struct TransmitConfirmMessage {
    pub header: MessageHeader,
    pub requester: i8 // the original requester
}