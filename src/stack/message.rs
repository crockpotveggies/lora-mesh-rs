use crate::MESH_MAX_MESSAGE_LEN;
use enumn::N;
use std::net::Ipv4Addr;
use packet::ip::v4::Packet;
use crate::stack::Frame;
use crate::stack::frame::FrameHeader;
use crate::stack::util::{parse_bool, parse_ipv4, parse_string};
use std::borrow::BorrowMut;

/// Defines the type of message in the protocol.
#[derive(PartialEq, Debug, N)]
pub enum MessageType {
    Broadcast = 1,
    IPAssignSuccess = 2,
    IPAssignFailure = 3,
    RouteDiscovery = 4,
    RouteFailure = 5,
    RouteSuccess = 6,
    TransmitRequest = 7,
    TransmitConfirm = 8,
    IPPacket = 9,
}

/// Instantiate a new frame for tx
pub trait ToFromFrame {
    fn from_frame(f: &mut Frame) -> std::io::Result<Box<Self>>;

    fn to_frame(&self, sender: i8, route: Vec<i8>) -> Frame;
}

/// Container for IP-level packets
pub struct IPPacketMessage {
    header: Option<FrameHeader>,
    packet: Packet<Vec<u8>>
}

impl ToFromFrame for IPPacketMessage {
    fn from_frame(mut f: &mut Frame) -> std::io::Result<Box<Self>> {
        let header = f.header();
        let data = f.payload();
        let (left, right) = data.split_at(1);
        let packet = Packet::new(Vec::from(right)).unwrap();

        Ok(Box::new(IPPacketMessage {
            header: Some(header),
            packet
        }))
    }

    fn to_frame(&self, sender: i8, route: Vec<i8>) -> Frame {
        // cast the route
        let route: Vec<u8> = route.clone().iter().map(|i| i.clone() as u8).collect();
        let routeoffset = route.len() as u8;

        // write the payload
        let mut payload: Vec<u8> = Vec::new();
        self.packet.as_ref().iter().for_each(|byte| payload.push(byte.clone()));

        Frame::new(
            0i8 as u8,
            MessageType::IPPacket as u8,
            sender as u8,
            routeoffset as u8,
            route,
            payload
        )
    }
}

/// Broadcast this node to nearby devices.
pub struct BroadcastMessage {
    pub header: Option<FrameHeader>,
    pub isgateway: bool,
    pub ipOffset: i8,
    pub ipaddr: Option<Ipv4Addr>
}

impl ToFromFrame for BroadcastMessage {
    fn from_frame(mut f: &mut Frame) -> std::io::Result<Box<Self>> {
        let header = f.header();
        let data = f.payload();
        let isgateway = parse_bool(data[0]).unwrap();
        let offset = data[1] as usize;
        let ipaddr: Option<Ipv4Addr> = None;
        if offset == 4 {
            let octets = &data[2..6];
            Some(parse_ipv4(octets));
        }

        Ok(Box::new(BroadcastMessage {
            header: Some(header),
            isgateway,
            ipOffset: offset as i8,
            ipaddr
        }))
    }

    fn to_frame(&self, sender: i8, route: Vec<i8>) -> Frame {
        // cast the route
        let route: Vec<u8> = route.clone().iter().map(|i| i.clone() as u8).collect();
        let routeoffset = route.len() as u8;

        // write the payload
        let mut payload: Vec<u8> = Vec::new();
        if self.ipaddr.is_some() {// write offset and octets if ip assigned
            payload.push(4i8 as u8);
            let ip = self.ipaddr.unwrap();
            let octets = ip.octets();
            octets.iter().for_each(|oct| payload.push(oct.clone()));
        } else {
            payload.push(0i8 as u8);
        }

        Frame::new(
            0i8 as u8,
            MessageType::Broadcast as u8,
            sender as u8,
            routeoffset as u8,
            route,
            payload
        )
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