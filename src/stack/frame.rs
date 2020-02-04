use crate::stack::message::*;
use crate::MESH_MAX_MESSAGE_LEN;

/// Defines continuity in current transmission
// 0 if no more packets, 1 more data to send, 2 txslot exceeded must receive
pub enum TransmissionState {
    FinalPacket = 0,
    MorePackets = 1,
    SlotExceeded = 2
}

/// A simple packet indicating the sender, message type, and transmission state
pub struct Frame {
    pub txflag: u8,
    pub msgtype: u8,
    pub sender: u8,
    pub data: Option<[u8; MESH_MAX_MESSAGE_LEN - 3]>
}

impl Frame {
    /// convert a packet to bits
    pub fn bits(&mut self) -> Vec<u8> {
        let mut bits = Vec::new();
        bits.push(self.txflag);
        bits.push(self.msgtype);
        bits.push(self.sender);

        // push data, if any
        self.data.map(|d| {
            let mut data = d;
            for (i, elem) in data.iter_mut().enumerate() {
                bits.push(elem.clone());
            }
        });

        return bits;
    }

    /// convert a discovery message to a packet
    pub fn from_broadcast(m: BroadcastMessage) -> Self {
        Frame {
            txflag: TransmissionState::FinalPacket as u8,
            msgtype: m.header.msgtype as u8,
            sender: m.header.sender as u8,
            data: None
        }
    }
}