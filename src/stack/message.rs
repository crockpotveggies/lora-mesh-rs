use crate::MESH_MAX_MESSAGE_LEN;
use enumn::N;
use std::net::Ipv4Addr;
use packet::ip::v4::Packet;
use crate::stack::Frame;
use crate::stack::frame::FrameHeader;
use crate::stack::util::{parse_bool, parse_ipv4};

/// Defines the type of message in the protocol.
#[derive(PartialEq, Debug, N)]
pub enum MessageType {
    RouteFailure = 0,
    RouteDiscovery = 1,
    RouteSuccess = 2,
    IPPacket = 3,
    Broadcast = 4,
    TransmitRequest = 5,
    TransmitConfirm = 6
}

/// Instantiate a new frame for tx
pub trait ToFromFrame {
    fn from_frame(f: Frame) -> std::io::Result<Box<Self>>;

    fn to_frame(&self, sender: i8) -> Frame;
}

/// Container for IP-level packets
pub struct IPPacketMessage {
    header: Option<FrameHeader>,
    to: i8, // destination node ID,
    routesOffset: i8,
    routes: Vec<i8>, // a list of ordered node ids that should transfer packet
    packet: Packet<Vec<u8>>
}

impl ToFromFrame for IPPacketMessage {
    fn from_frame(mut f: Frame) -> std::io::Result<Box<Self>> {
        let header = f.header();
        let data = f.data();
        let to = data[0];
        let offset = data[1] as usize;
        let routes = data[3..(offset+3)].to_vec().iter().map(|r| r.clone() as i8).collect();
        let payload = &data[(offset+4)..(data.len() as usize)];
        let packet = Packet::new(Vec::from(payload)).unwrap();

        Ok(Box::new(IPPacketMessage {
            header: Some(header),
            to: to as i8,
            routesOffset: offset as i8,
            routes,
            packet
        }))
    }

    fn to_frame(&self, sender: i8) -> Frame {
        let mut data: Vec<u8> = Vec::new();
        if self.routes.len() > 0 {// write offset and routed node IDs
            data.push(self.routesOffset as u8);
            let ids = self.routes.iter();
            ids.for_each(|id| data.push(id.clone() as u8));
        } else {
            data.push(0i8 as u8);
        }

        Frame {
            txflag: 0i8 as u8,
            msgtype: MessageType::Broadcast as u8,
            sender: sender as u8,
            data
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

/// Broadcast this node to nearby devices.
pub struct BroadcastMessage {
    pub header: Option<FrameHeader>,
    pub isgateway: bool,
    pub ipOffset: i8,
    pub ipaddr: Option<Ipv4Addr>
}

impl ToFromFrame for BroadcastMessage {
    fn from_frame(mut f: Frame) -> std::io::Result<Box<Self>> {
        let header = f.header();
        let data = f.data();
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

    fn to_frame(&self, sender: i8) -> Frame {
        let mut data: Vec<u8> = Vec::new();
        if self.ipaddr.is_some() {// write offset and octets if ip assigned
            data.push(4i8 as u8);
            let ip = self.ipaddr.unwrap();
            let octets = ip.octets();
            octets.iter().for_each(|oct| data.push(oct.clone()));
        } else {
            data.push(0i8 as u8);
        }

        Frame {
            txflag: 0i8 as u8,
            msgtype: MessageType::Broadcast as u8,
            sender: sender as u8,
            data
        }
    }
}

/// Assign an IP address to a node.
pub struct AssignIPMessage {
    pub header: Option<FrameHeader>,
    pub to: i8,
    pub ipaddr: [i8; 4]
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