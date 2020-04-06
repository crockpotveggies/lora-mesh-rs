use crate::MESH_MAX_MESSAGE_LEN;
use enumn::N;
use std::net::Ipv4Addr;
use packet::ip::v4::Packet;
use crate::stack::Frame;
use crate::stack::frame::{FrameHeader, TransmissionState, ToFromFrame};
use crate::stack::util::{parse_bool, parse_ipv4, parse_string, parse_byte};
use crate::message::MessageType;

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
        payload.push(parse_byte(self.isgateway));

        // write offset and octets if ip assigned
        if self.ipaddr.is_some() {
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

#[cfg(test)]
use hex;
#[test]
fn broadcast_tofrom_frame() {
    let id = 5;
    let isgateway = false;
    let msg = BroadcastMessage {
        header: None,
        isgateway: isgateway,
        ipOffset: 0,
        ipaddr: None
    };
    let mut route: Vec<i8> = Vec::new();
    route.push(id.clone());

    // check tofrom frame
    let mut frame = msg.to_frame(id, route);

    assert_eq!(frame.sender(), id);
    assert_eq!(frame.payload().get(0).unwrap().clone() as i8, 0i8);
    assert_eq!(frame.payload().get(1).unwrap().clone() as i8, 0i8);

    // ensure representation is same after hex encoding
    let bytes = frame.to_bytes();
    let encoded = hex::encode(bytes);
    let decoded = hex::decode(encoded).unwrap();

    let mut frame2 = Frame::from_bytes(&decoded).unwrap();
    let msg2 = BroadcastMessage::from_frame(&mut frame2).unwrap();

    assert_eq!(frame2.sender(), id);
    assert_eq!(msg2.header.unwrap().sender(), id);
    assert_eq!(msg2.isgateway, isgateway);
}