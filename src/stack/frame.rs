use crate::stack::message::*;
use crate::MESH_MAX_MESSAGE_LEN;
use enumn::N;
use crate::stack::chunk::chunk_data;
use std::borrow::BorrowMut;

/// Defines continuity in current transmission
#[derive(Clone, PartialEq, Debug, N)]
pub enum TransmissionState {
    FinalChunk = 0,
    MoreChunks = 1,
    SlotExceeded = 2
}

impl TransmissionState {
    /// convert txflag to byte
    pub fn to_u8(&self) -> u8 {
        match self {
            TransmissionState::FinalChunk => 0u8,
            TransmissionState::MoreChunks => 1u8,
            TransmissionState::SlotExceeded => 2u8,
        }
    }

    /// boolean to determine if more rx is needed
    pub fn more_chunks(&self) -> bool {
        match self {
            TransmissionState::FinalChunk => false,
            TransmissionState::MoreChunks => true,
            TransmissionState::SlotExceeded => true,
        }
    }
}

/// header of a frame
#[derive(Clone)]
pub struct FrameHeader {
    txflag: TransmissionState,
    msgtype: MessageType,
    sender: i32,
    routeoffset: usize,
    route: Vec<i32>,
}

impl FrameHeader {
    /// constructor
    pub fn new(txflag: TransmissionState, msgtype: MessageType, sender: i32, route: Vec<i32>) -> Self {
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

    pub fn sender(&mut self) -> i32 {
        return self.sender as i32;
    }

    pub fn route(&mut self) -> Vec<i32> {
        return self.route.clone();
    }

    pub fn route_bytes(&mut self) -> Vec<u8> {
        return self.route.clone().iter().map(|byte| byte.clone() as u8).collect();
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

    /// construct a frame from a header and payload
    pub fn from_header(mut header: FrameHeader, payload: Vec<u8>) -> Self {
        Frame{
            txflag: header.txflag.to_u8(),
            msgtype: header.msgtype.to_u8(),
            sender: header.sender as u8,
            routeoffset: header.routeoffset as u8,
            route: header.route_bytes(),
            payload
        }
    }

    /// convert a packet to bytes
    pub fn to_bytes(&mut self) -> Vec<u8> {
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
    pub fn from_bytes(bytes: &Vec<u8>) -> Option<Self> {
        let txflag = bytes.get(0)?.clone();
        let msgtype = bytes.get(1)?.clone();
        let sender = bytes.get(2)?.clone();
        let routesoffset = bytes.get(3)?.clone();
        let routes = bytes.get(4..(4+routesoffset as usize))?;
        let (left, right) = bytes.split_at(4+routesoffset as usize);

        Some(Frame {
            txflag,
            msgtype,
            sender,
            routeoffset: routesoffset,
            route: Vec::from(routes),
            payload: Vec::from(right)
        })
    }

    /// remove the next hop in the route, and return the hop ID
    /// this is useful for message passing
    pub fn route_shift(&mut self) -> Option<i32> {
        self.routeoffset -= 1;
        let shift = self.route.drain(0..1);
        return shift.last().map(|byte| byte as i32);
    }

    /// insert a hop at the beginning of the route
    /// useful for when a message is rebroadcasted
    pub fn route_unshift(&mut self, nodeid: i32) {
        self.route.insert(0, nodeid as u8);
        self.routeoffset += 1;
    }

    /// chunk a frame into multiple frames
    pub fn chunked(&mut self, chunksize: &usize) -> Vec<Vec<u8>> {
        let mut payloadchunks = chunk_data(self.payload.clone(), chunksize);

        // add header data to each frame
        let mut chunks: Vec<Vec<u8>> = Vec::new();
        for (i, datachunk) in payloadchunks.iter().enumerate() {
            let mut chunk = self.header().bytes().clone();
            chunk.extend(datachunk.iter());
            // set tx flag
            if i < (payloadchunks.len()-1) {
                chunk[0] = 1 as u8;
            }
            chunks.push(chunk);
        }

        return chunks;
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
        return TransmissionState::n(self.txflag as i32).unwrap();
    }

    pub fn msgtype(&mut self) -> MessageType {
        return MessageType::n(self.msgtype as i32).unwrap();
    }

    pub fn sender(&mut self) -> i32 {
        return self.sender as i32;
    }

    pub fn routeoffset(&mut self) -> u8 {
        return self.routeoffset;
    }

    pub fn route(&mut self) -> Vec<i32> {
        return self.route.iter().map(|n| n.clone() as i32).collect();
    }

    pub fn route_bytes(&mut self) -> Vec<u8> {
        return self.route.clone();
    }

    pub fn payload(&mut self) -> Vec<u8> {
        return self.payload.clone();
    }
}

/// take a list of received chunked frames and recombine their payload
pub fn recombine_chunks(mut chunks: Vec<Frame>, mut header: FrameHeader) -> Frame {
    let mut combinedbytes = Vec::new();
    chunks.iter()
        .map(|chunk| combinedbytes.extend(chunk.payload.iter()) );

    Frame::from_header(
        header,
        combinedbytes
    )
}

/// Instantiate a new frame for tx
pub trait ToFromFrame {
    fn from_frame(f: &mut Frame) -> std::io::Result<Box<Self>>;

    fn to_frame(&self, sender: i32, route: Vec<i32>) -> Frame;
}