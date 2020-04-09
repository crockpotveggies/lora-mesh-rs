use crate::stack::message::*;
use enumn::N;
use crate::stack::chunk::chunk_data;
use std::io::ErrorKind;
use packet::ip::v4::Packet;

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
#[derive(Clone, Debug)]
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
    pub fn from_bytes(bytes: &Vec<u8>) -> std::io::Result<Self> {
        let txflag = bytes.get(0).ok_or(ErrorKind::InvalidData)?.clone();
        let msgtype = bytes.get(1).ok_or(ErrorKind::InvalidData)?.clone();
        let sender = bytes.get(2).ok_or(ErrorKind::InvalidData)?.clone();
        let routesoffset = bytes.get(3).ok_or(ErrorKind::InvalidData)?.clone();
        let routes = bytes.get(4..(4+routesoffset as usize)).ok_or(ErrorKind::InvalidData)?;
        let (_left, right) = bytes.split_at(4+routesoffset as usize);

        Ok(Frame {
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
        let payloadchunks = chunk_data(self.payload.clone(), chunksize);

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
pub fn recombine_chunks(chunks: Vec<Frame>, header: FrameHeader) -> Frame {
    let mut combinedbytes = Vec::new();
    for chunk in chunks {
        combinedbytes.extend(chunk.payload.iter());
    }

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

#[cfg(test)]
use format_escape_default::format_escape_default;
use hex;
#[test]
fn frame_chunking() {
    // check sizes during chunking
    let sender = 3i32;
    let raw = vec![0x45u8, 0x00, 0x00, 0x42, 0x47, 0x07, 0x40, 0x00, 0x40, 0x11, 0x6e, 0xcc, 0xc0, 0xa8, 0x01, 0x89, 0xc0, 0xa8, 0x01, 0xfe, 0xba, 0x2f, 0x00, 0x35, 0x00, 0x2e, 0x1d, 0xf8, 0xbc, 0x81, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x61, 0x70, 0x69, 0x0c, 0x73, 0x74, 0x65, 0x61, 0x6d, 0x70, 0x6f, 0x77, 0x65, 0x72, 0x65, 0x64, 0x03, 0x63, 0x6f, 0x6d, 0x00, 0x00, 0x1c, 0x00, 0x01];
    let hex1 = hex::encode(&raw);
    let originalsize = raw.len();
    let packet = Packet::new(raw.clone()).expect("Invalid packet");

    // ensure the packet buffer is same as original packet
    assert_eq!(&hex1, &hex::encode(&raw));

    let msg = IPPacketMessage::new(packet);
    let mut frame = msg.to_frame(sender, Vec::new());

    let chunksize = 45usize;
    let framesize = chunksize.clone()+4usize;
    let mut chunks = frame.chunked(&chunksize);

    // ensure the sizes of the chunked packet are correct
    assert_eq!(&originalsize, &66usize);
    assert_eq!(&chunks[0].len(), &framesize);
    assert_eq!(&chunks[1].len(), &25usize);

    // check recombination
    let mut chunkedframes = Vec::new();
    for chunk in chunks {
        chunkedframes.push(Frame::from_bytes(&chunk).expect("Invalid chunked frame"));
    }

    let mut rawchunks = &mut chunkedframes[0].clone().payload;
    rawchunks.extend(&mut chunkedframes[1].clone().payload.iter());

    // check that manually recombined chunks are correct
    assert_eq!(&hex1, &hex::encode(&rawchunks.clone()));

    let packet2 = Packet::new(rawchunks.clone()).expect("Invalid manually recombined packet");
    let msg2 = IPPacketMessage::new(packet2);

    let mut frame3 = recombine_chunks(chunkedframes, frame.header());
    let msg3 = IPPacketMessage::from_frame(&mut frame3).expect("Invalid recombined IPPacketMessage");
    let packet3 = msg2.clone().packet();
    let raw3 = packet3.as_ref();

    assert_eq!(&raw3[0], &raw[0]);
    assert_eq!(&raw3[50], &raw[50]);
}