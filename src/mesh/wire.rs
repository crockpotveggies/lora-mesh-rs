use crate::radio::protocol::invalid;
use std::io;
pub const HEADER: usize = 24;
pub const MTU: usize = 1500 - HEADER;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub origin: u16,
    pub destination: u16,
    pub epoch: u64,
    pub sequence: u32,
    pub hops: u8,
    pub announcement: bool,
    pub payload: Vec<u8>,
}
impl Packet {
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        if self.epoch == 0
            || self.hops == 0
            || self.hops > 16
            || self.payload.is_empty()
            || self.payload.len() > MTU
            || self.announcement != (self.destination == 0)
            || self.origin == 0
        {
            return Err(invalid("invalid mesh packet"));
        }
        let mut out = Vec::with_capacity(HEADER + self.payload.len());
        out.extend_from_slice(b"MS\x01");
        out.push(u8::from(self.announcement));
        out.extend_from_slice(&self.origin.to_be_bytes());
        out.extend_from_slice(&self.destination.to_be_bytes());
        out.extend_from_slice(&self.epoch.to_be_bytes());
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.push(self.hops);
        out.push(0);
        out.extend_from_slice(&(self.payload.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.payload);
        Ok(out)
    }
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < HEADER + 1
            || bytes.len() > 1500
            || &bytes[..3] != b"MS\x01"
            || bytes[3] > 1
            || bytes[21] != 0
            || usize::from(u16::from_be_bytes([bytes[22], bytes[23]])) != bytes.len() - HEADER
        {
            return Err(invalid("invalid mesh header"));
        }
        let p = Self {
            origin: u16::from_be_bytes(bytes[4..6].try_into().unwrap()),
            destination: u16::from_be_bytes(bytes[6..8].try_into().unwrap()),
            epoch: u64::from_be_bytes(bytes[8..16].try_into().unwrap()),
            sequence: u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            hops: bytes[20],
            announcement: bytes[3] == 1,
            payload: bytes[24..].to_vec(),
        };
        p.encode()?;
        Ok(p)
    }
    pub fn signing_bytes(&self, network: u32) -> Vec<u8> {
        let mut out = b"loramesh-lsa-v1".to_vec();
        out.extend_from_slice(&network.to_be_bytes());
        out.extend_from_slice(&self.origin.to_be_bytes());
        out.extend_from_slice(&self.epoch.to_be_bytes());
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.extend_from_slice(&self.payload);
        out
    }
}
