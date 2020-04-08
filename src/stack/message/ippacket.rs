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
        let packet = Packet::new(data).unwrap();

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
        let mut payload: Vec<u8> = Vec::from(self.packet.as_ref());

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
    // check conversion from bytes
    let hexmsg2 = "00090002000445000023180440004011caa1ac100000ac100004e6ba0bb8000ff4914142433132330a";
    let mut frame2 = Frame::from_bytes(&hex::decode(&hexmsg2).unwrap()).unwrap();
    let msg2 = IPPacketMessage::from_frame(frame2.borrow_mut());
    let packet2 = msg2.unwrap().packet;

    assert_eq!(&frame2.sender(), &0i32);
    assert_eq!(&packet2.destination().to_string(), "172.16.0.4");
}