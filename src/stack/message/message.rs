use crate::{MESH_MAX_MESSAGE_LEN};
use enumn::N;
use std::net::Ipv4Addr;
use crate::stack::Frame;
use crate::stack::frame::{FrameHeader, ToFromFrame};
use crate::stack::util::{parse_ipv4};

/// Defines the type of message in the protocol.
#[derive(Clone, PartialEq, Debug, N)]
pub enum MessageType {
    Broadcast = 1,
    IPAssignSuccess = 2,
    IPAssignFailure = 3,
    RouteDiscovery = 4,
    RouteSuccess = 5,
    RouteFailure = 6,
    TransmitRequest = 7,
    TransmitConfirm = 8,
    IPPacket = 9,
}

impl MessageType {
    pub fn to_u8(&self) -> u8 {
        match self {
            MessageType::Broadcast => 1 as u8,
            MessageType::IPAssignSuccess => 2 as u8,
            MessageType::IPAssignFailure => 3 as u8,
            MessageType::RouteDiscovery => 4 as u8,
            MessageType::RouteSuccess => 5 as u8,
            MessageType::RouteFailure => 6 as u8,
            MessageType::TransmitRequest => 7 as u8,
            MessageType::TransmitConfirm => 8 as u8,
            MessageType::IPPacket => 9 as u8,
        }
    }
}

/// A node is no longer reachable from the sender.
pub struct RouteFailureMessage {
    pub header: Option<FrameHeader>,
    pub failednodeid: u8
}

/// Request destination node if okay to transmit.
pub struct TransmitRequestMessage {
    pub header: Option<FrameHeader>,
    pub dest: u8 // the intended receiver
}

/// Confirm to original requester that it is okay to transmit.
pub struct TransmitConfirmMessage {
    pub header: Option<FrameHeader>,
    pub requester: u8 // the original requester
}