use crate::MESH_MAX_MESSAGE_LEN;

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
    msgtype: MessageType,
    from: u8 // sender node
}

/// Transmits application-level data.
pub struct ApplicationMessage {
    header: MessageHeader,
    to: u8, // destination node
    data: [u8; MESH_MAX_MESSAGE_LEN] // application data
}

/// Broadcasts by a mesh node to discover a route to a node.
pub struct RouteDiscoveryMessage {
    header: MessageHeader,
    dest: u8, // destination node being sought
    invalidhops: [u8; MESH_MAX_MESSAGE_LEN - 3] // nodes tried so far
}

/// Replies to a discovery message with a successful route.
pub struct RouteSuccessMessage {
    header: MessageHeader,
    to: u8, // the node requesting discovery
    dest: u8, // destination node being sought
    hops: [u8; MESH_MAX_MESSAGE_LEN - 4] // the nodes, in sequence, the requester must hop to deliver a message
}

/// A node is no longer reachable from the sender.
pub struct RouteFailureMessage {
    header: MessageHeader,
    failednode: u8
}
/// Broadcast this node to nearby devices.
pub struct BroadcastMessage {
    header: MessageHeader
}

/// Request destination node if okay to transmit.
pub struct TransmitRequestMessage {
    header: MessageHeader,
    dest: u8 // the intended receiver
}

/// Confirm to original requester that it is okay to transmit.
pub struct TransmitConfirmMessage {
    header: MessageHeader,
    requester: u8 // the original requester
}