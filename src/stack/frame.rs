use crate::stack::message::*;
use crate::MESH_MAX_MESSAGE_LEN;
use enumn::N;

/// Defines continuity in current transmission
#[derive(PartialEq, Debug, N)]
pub enum TransmissionState {
    FinalPacket = 0,
    MorePackets = 1,
    SlotExceeded = 2
}

/// header of a frame
pub struct FrameHeader {
    txflag: u8,
    msgtype: u8,
    sender: u8,
}

/// A simple packet indicating the sender, message type, and transmission state
pub struct Frame {
    txflag: u8,
    msgtype: u8,
    sender: u8,
    data: Vec<u8>
}

impl Frame {
    /// public construct for Frame
    pub fn new(txflag: u8, msgtype: u8, sender: u8, data: Vec<u8>) -> Self {
        Frame {txflag, msgtype, sender, data}
    }

    /// convert a packet to bits
    pub fn bits(&mut self) -> Vec<u8> {
        let mut bits = Vec::new();
        bits.push(self.txflag);
        bits.push(self.msgtype);
        bits.push(self.sender);

        // push data, if any
        self.data.iter().for_each(|d| {
            let byte = d.clone();
            bits.push(byte);
        });

        return bits;
    }

    /// parse from raw bytes
    pub fn parse(bytes: Vec<u8>) -> std::io::Result<Self> {
        let txflag = bytes.get(0).unwrap().clone();
        let msgtype = bytes.get(1).unwrap().clone();
        let sender = bytes.get(2).unwrap().clone();
        let (left, right) = bytes.split_at(2);
        let data = Vec::from(right);

        Ok(Frame {
            txflag,
            msgtype,
            sender,
            data
        })
    }

    pub fn header(&mut self) -> FrameHeader {
        return FrameHeader{txflag: self.txflag, msgtype: self.msgtype, sender: self.sender};
    }

    pub fn txflag(&mut self) -> TransmissionState {
        return TransmissionState::n(self.txflag as i8).unwrap();
    }

    pub fn msgtype(&mut self) -> MessageType {
        return MessageType::n(self.msgtype as i8).unwrap();
    }

    pub fn sender(&mut self) -> i8 {
        return self.sender as i8;
    }

    pub fn data(&mut self) -> Vec<u8> {
        return self.data.clone();
    }
}