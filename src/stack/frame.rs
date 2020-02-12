use crate::stack::message::*;
use crate::MESH_MAX_MESSAGE_LEN;
use enumn::N;
use crate::stack::chunk::chunk_data;

/// Defines continuity in current transmission
#[derive(PartialEq, Debug, N)]
pub enum TransmissionState {
    FinalChunk = 0,
    MoreChunks = 1,
    SlotExceeded = 2
}

impl TransmissionState {
    pub fn to_u8(&self) -> u8 {
        match self {
            TransmissionState::FinalChunk => 0 as u8,
            TransmissionState::MoreChunks => 1 as u8,
            TransmissionState::SlotExceeded => 2 as u8,
        }
    }
}

/// header of a frame
pub struct FrameHeader {
    txflag: TransmissionState,
    msgtype: MessageType,
    sender: i8,
    routeoffset: usize,
    route: Vec<i8>,
}

impl FrameHeader {
    /// constructor
    pub fn new(txflag: TransmissionState, msgtype: MessageType, sender: i8, route: Vec<i8>) -> Self {
        FrameHeader{txflag, msgtype, sender, routeoffset: route.len(), route}
    }

    /// convert a packet to bytes
    pub fn bytes(&mut self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(self.txflag.to_u8());
        bytes.push(self.msgtype.to_u8());
        bytes.push(self.sender.clone() as u8);
        bytes.push(self.routeoffset.clone() as u8);
        self.route.iter().for_each(|n| bytes.push(n.clone() as u8));

        return bytes;
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

    /// convert a packet to bytes
    pub fn bytes(&mut self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(self.txflag);
        bytes.push(self.msgtype);
        bytes.push(self.sender);
        bytes.push(self.routeoffset);
        self.route.iter().for_each(|n| bytes.push(n.clone()));

        // push data, if any
        self.payload.iter().for_each(|d| bytes.push(d.clone()));

        return bytes;
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

    /// remove the next hop in the route, and return the hop ID
    /// this is useful for message passing
    pub fn route_shift(&mut self) -> Option<i8> {
        let shift = self.route.drain(0..1);
        return shift.last().map(|byte| byte as i8);
    }

    /// insert a hop at the beginning of the route
    /// useful for when a message is rebroadcasted
    pub fn route_unshift(&mut self, nodeid: i8) {
        self.route.insert(0, nodeid as u8);
    }

    /// chunk a frame into multiple frames
    pub fn chunked(&mut self, chunksize: &usize) -> Vec<Vec<u8>> {
        let mut payloadchunks = chunk_data(self.payload.clone(), chunksize);
        /// add header data to each frame
        let chunks: Vec<Vec<u8>> = Vec::new();
        for (i, datachunk) in payloadchunks.iter().enumerate() {
            let mut chunk = self.header().bytes();
            datachunk.iter().for_each(|byte| chunk.push(byte.clone()));
            // set tx flag
            if i < (payloadchunks.len()-1) {
                chunk[0] = 1 as u8;
            }
        }

        return payloadchunks;
    }

    pub fn header(&mut self) -> FrameHeader {
        return FrameHeader{
            txflag: self.txflag(),
            msgtype: self.msgtype(),
            sender: self.sender(),
            routeoffset: self.route().len(),
            route: self.route()
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

    pub fn route(&mut self) -> Vec<i8> {
        return self.route.iter().map(|n| n.clone() as i8).collect();
    }

    pub fn payload(&mut self) -> Vec<u8> {
        return self.payload.clone();
    }
}