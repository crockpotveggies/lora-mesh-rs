use crate::MESH_MAX_MESSAGE_LEN;
use enumn::N;
use std::net::Ipv4Addr;
use packet::ip::v4::Packet;
use crate::stack::Frame;
use crate::stack::frame::{FrameHeader, TransmissionState, ToFromFrame};
use crate::stack::util::{parse_bool, parse_ipv4, parse_string, parse_byte};
use crate::message::MessageType;

/// Container for IP-level packets
pub struct IPPacketMessage {
    header: Option<FrameHeader>,
    packet: Packet<Vec<u8>>
}

impl IPPacketMessage {
    pub fn new(packet: Packet<Vec<u8>>) -> Self {
        IPPacketMessage{header: None, packet}
    }
}

impl ToFromFrame for IPPacketMessage {
    fn from_frame(mut f: &mut Frame) -> std::io::Result<Box<Self>> {
        let header = f.header();
        let data = f.payload();
        let packet = Packet::new(Vec::from(data)).unwrap();

        Ok(Box::new(IPPacketMessage {
            header: Some(header),
            packet
        }))
    }

    fn to_frame(&self, sender: i32, route: Vec<i32>) -> Frame {
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

#[cfg(test)]
use hex;
use std::borrow::BorrowMut;

#[test]
fn ippacket_tofrom_frame() {
    let id = 5;
    let payloadhex = "45000023e40004011f6fbac100000ac100003ce760bb8000f0cd74142433132330a";
    let msg = IPPacketMessage {
        header: None,
        packet: Packet::new(hex::decode(&payloadhex).unwrap()).unwrap()
    };
    let mut route: Vec<i32> = Vec::new();
    route.push(id.clone());

    // check tofrom frame
    let mut frame = msg.to_frame(id, route);

    assert_eq!(frame.sender(), id);
    assert_eq!(hex::encode(frame.payload()), payloadhex);

    // check conversion from bytes
    let packet = Packet::new(hex::decode(&payloadhex).unwrap()).unwrap();

    let bytes = frame.to_bytes();
    let mut frame2 = Frame::from_bytes(&bytes).unwrap();
    let msg2 = IPPacketMessage::from_frame(frame2.borrow_mut());
    let packet2 = msg2.unwrap().packet;

    assert_eq!(&frame.sender(), &frame2.sender());
    assert_eq!(&packet.destination(), &packet2.destination())
}