use packet::ip::v4::Packet;
use crate::stack::Frame;
use crate::stack::frame::{FrameHeader, ToFromFrame};
use crate::message::MessageType;
use std::io::ErrorKind;

/// Container for IP-level packets
#[derive(Clone, Debug)]
pub struct IPPacketMessage {
    header: Option<FrameHeader>,
    packet: Packet<Vec<u8>>
}

impl IPPacketMessage {
    pub fn new(packet: Packet<Vec<u8>>) -> Self {
        IPPacketMessage{header: None, packet}
    }

    pub fn packet(&self) -> Packet<Vec<u8>> {
        return self.packet.clone();
    }
}

impl ToFromFrame for IPPacketMessage {
    fn from_frame(f: &mut Frame) -> std::io::Result<Box<Self>> {
        let header = f.header();
        let data = f.payload();
        let packet = Packet::new(data).ok().ok_or(ErrorKind::InvalidData)?;

        Ok(Box::new(IPPacketMessage {
            header: Some(header),
            packet
        }))
    }

    fn to_frame(&self, frameid: u8, sender: u8, route: Vec<u8>) -> Frame {
        // cast the route
        let route: Vec<u8> = route.clone().iter().map(|i| i.clone() as u8).collect();
        let routeoffset = route.len() as u8;

        // write the payload
        let payload: Vec<u8> = Vec::from(self.packet.as_ref());

        Frame::new(
            0i8 as u8,
            frameid,
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
    let hexmsg2 = "0000090002000445000023180440004011caa1ac100000ac100004e6ba0bb8000ff4914142433132330a";
    let mut frame2 = Frame::from_bytes(&hex::decode(&hexmsg2).unwrap()).unwrap();
    let msg2 = IPPacketMessage::from_frame(frame2.borrow_mut());
    let packet2 = msg2.unwrap().packet;

    assert_eq!(&frame2.sender(), &0u8);
    assert_eq!(&packet2.destination().to_string(), "172.16.0.4");
}