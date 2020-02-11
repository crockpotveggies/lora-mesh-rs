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
    routeoffset: u8,
    route: Vec<u8>,
}

impl FrameHeader {
    pub fn txflag(&mut self) -> TransmissionState {
        return TransmissionState::n(self.txflag as i8).unwrap();
    }

    pub fn msgtype(&mut self) -> MessageType {
        return MessageType::n(self.msgtype as i8).unwrap();
    }

    pub fn sender(&mut self) -> i8 {
        return self.sender as i8;
    }

    pub fn routes(&mut self) -> Vec<i8> {
        return self.route.clone().iter().map(|byte| byte.clone() as i8).collect();
    }
}

/// A simple packet indicating the sender, message type, and transmission state
#[derive(Clone)]
pub struct Frame {
    txflag: u8, // indicates if chunked
    msgtype: u8, // a flag for message type
    sender: u8, // which node ID sent this frame?
    routeoffset: u8, // size of array of route for frame
    route: Vec<u8>, // a list of node IDs that frame should pass
    payload: Vec<u8>, // payload data
}

impl Frame {
    /// public construct for Frame
    pub fn new(txflag: u8, msgtype: u8, sender: u8, routeoffset: u8, route: Vec<u8>, payload: Vec<u8>) -> Self {
        Frame {txflag, msgtype, sender, routeoffset, route, payload }
    }

    /// convert a packet to bits
    pub fn bits(&mut self) -> Vec<u8> {
        let mut bits = Vec::new();
        bits.push(self.txflag);
        bits.push(self.msgtype);
        bits.push(self.sender);

        // push data, if any
        self.payload.iter().for_each(|d| {
            let byte = d.clone();
            bits.push(byte);
        });

        return bits;
    }

    /// parse from raw bytes
    pub fn parse(bytes: &Vec<u8>) -> std::io::Result<Self> {
        let txflag = bytes[0].clone();
        let msgtype = bytes[1].clone();
        let sender = bytes[2].clone();
        let routesoffset = bytes[3].clone();
        let routes = &bytes[4..(4+routesoffset as usize)];
        let (left, right) = bytes.split_at(2);
        let data = Vec::from(right);

        Ok(Frame {
            txflag,
            msgtype,
            sender,
            routeoffset: routesoffset,
            route: Vec::from(routes),
            payload: data
        })
    }

    pub fn header(&mut self) -> FrameHeader {
        return FrameHeader{
            txflag: self.txflag,
            msgtype: self.msgtype,
            sender: self.sender,
            routeoffset: self.routeoffset,
            route: self.route.clone()
        };
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

    pub fn payload(&mut self) -> Vec<u8> {
        return self.payload.clone();
    }
}