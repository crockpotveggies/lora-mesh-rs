use crate::MESH_MAX_MESSAGE_LEN;

/// Defines the type of message in the protocol.
pub enum MessageType {
    RouteFailure = 0,
    RouteDiscovery = 1,
    RouteSuccess = 2,
    Application = 3,
    Ack = 4,
    Advertisement = 5
}

/// Header for a mesh message.
struct MessageHeader {
    msgType: MessageType,
    from: u8 // sender node
}

/// Transmits application-level data.
struct ApplicationMessage {
    header: MessageHeader,
    to: u8, // destination node
    data: [u8; MESH_MAX_MESSAGE_LEN] // application data
}

/// Broadcasts by a mesh node to discover a route to a node.
struct RouteDiscoveryMessage {
    header: MessageHeader,
    dest: u8, // destination node being sought
    invalid_hops: [u8; MESH_MAX_MESSAGE_LEN - 3] // nodes tried so far
}

/// Replies to a discovery message with a successful route.
struct RouteSuccessMessage {
    header: MessageHeader,
    to: u8, // the node requesting discovery
    dest: u8, // destination node being sought
    hops: [u8; MESH_MAX_MESSAGE_LEN - 4] // the nodes, in sequence, the requester must hop to deliver a message
}

/// A node is no longer reachable from the sender.
struct RouteFailureMessage {
    header: MessageHeader,
    failed_node: u8
}

/// Acknowledge a received message.
struct AckMessage {
    header: MessageHeader,
    to: u8
}

/// A direct advertisement to nearby nodes.
struct AdvertisementMessage {
    header: MessageHeader
}