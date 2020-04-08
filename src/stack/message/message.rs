use crate::MESH_MAX_MESSAGE_LEN;
use enumn::N;
use std::net::Ipv4Addr;
use packet::ip::v4::Packet;
use crate::stack::Frame;
use crate::stack::frame::{FrameHeader, TransmissionState, ToFromFrame};
use crate::stack::util::{parse_bool, parse_ipv4, parse_string, parse_byte};

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

/// Broadcasts by a mesh node to discover a route to a node.
pub struct RouteDiscoveryMessage {
    pub header: Option<FrameHeader>,
    pub dest: i8, // destination node being sought
    pub invalidhops: [u8; MESH_MAX_MESSAGE_LEN - 3] // nodes tried so far
}

/// Replies to a discovery message with a successful route.
pub struct RouteSuccessMessage {
    pub header: Option<FrameHeader>,
    pub to: i8, // the node requesting discovery
    pub dest: i8, // destination node being sought
    pub hops: [i8; MESH_MAX_MESSAGE_LEN - 4] // the nodes, in sequence, the requester must hop to deliver a message
}

/// A node is no longer reachable from the sender.
pub struct RouteFailureMessage {
    pub header: Option<FrameHeader>,
    pub failednodeid: i8
}

/// Request destination node if okay to transmit.
pub struct TransmitRequestMessage {
    pub header: Option<FrameHeader>,
    pub dest: i8 // the intended receiver
}

/// Confirm to original requester that it is okay to transmit.
pub struct TransmitConfirmMessage {
    pub header: Option<FrameHeader>,
    pub requester: i8 // the original requester
}

/// Notify node of their new IP address.
pub struct IPAssignSuccessMessage {
    pub header: Option<FrameHeader>,
    pub ipaddr: Ipv4Addr
}

impl IPAssignSuccessMessage {
    pub fn new(ipaddr: Ipv4Addr) -> Self {
        return IPAssignSuccessMessage{ header: None, ipaddr}
    }
}

impl ToFromFrame for IPAssignSuccessMessage {
    fn from_frame(mut f: &mut Frame) -> std::io::Result<Box<Self>> {
        let header = f.header();
        let data = f.payload();
        let octets = &data[0..data.len()];
        let ipaddr = parse_ipv4(octets);

        Ok(Box::new(IPAssignSuccessMessage {
            header: Some(header),
            ipaddr
        }))
    }

    fn to_frame(&self, sender: i8, route: Vec<i8>) -> Frame {
        // cast the route
        let route: Vec<u8> = route.clone().iter().map(|i| i.clone() as u8).collect();
        let routeoffset = route.len() as u8;

        // write the payload
        let mut data: Vec<u8> = Vec::new();
        let octets = self.ipaddr.octets();
        octets.iter().for_each(|oct| data.push(oct.clone()));

        Frame::new(
            0i8 as u8,
            MessageType::IPAssignSuccess as u8,
            sender as u8,
            routeoffset as u8,
            route,
            data
        )
    }
}

/// Assigning IP to node failed, tell them.
pub struct IPAssignFailureMessage {
    pub header: Option<FrameHeader>,
    pub reason: String
}

impl IPAssignFailureMessage {
    pub fn new(reason: String) -> Self {
        return IPAssignFailureMessage{ header: None, reason}
    }
}

impl ToFromFrame for IPAssignFailureMessage {
    fn from_frame(mut f: &mut Frame) -> std::io::Result<Box<Self>> {
        let header = f.header();
        let reason = String::from_utf8(f.payload()).expect("Could not parse UTF-8 message");

        Ok(Box::new(IPAssignFailureMessage {
            header: Some(header),
            reason
        }))
    }

    fn to_frame(&self, sender: i8, route: Vec<i8>) -> Frame {
        // cast the route
        let route: Vec<u8> = route.clone().iter().map(|i| i.clone() as u8).collect();
        let routeoffset = route.len() as u8;

        let payload = &self.reason;

        Frame::new(
            0i8 as u8,
            MessageType::IPAssignFailure as u8,
            sender as u8,
            routeoffset,
            route,
            payload.clone().into_bytes()
        )
    }
}