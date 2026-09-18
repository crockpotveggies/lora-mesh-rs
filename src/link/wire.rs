use crate::radio::protocol::invalid;
use std::{convert::TryInto, io};
pub const HEADER: usize = 46;
pub const MAX_SPAN: usize = 255 - HEADER;
pub const MTU: usize = 1500;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub network: u32,
    pub source: u16,
    pub destination: u16,
    pub session: u64,
    pub sequence: u32,
    pub total: u16,
    pub index: u8,
    pub count: u8,
    pub span: u8,
    pub ack: bool,
    pub request_ack: bool,
}
impl Header {
    pub fn mask(self) -> u32 {
        u32::MAX >> (32 - self.count)
    }
    pub fn encode(self, payload: &[u8]) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(HEADER + payload.len());
        out.extend_from_slice(b"LM\x01");
        out.push(u8::from(self.ack) | (u8::from(self.request_ack) << 1));
        out.extend_from_slice(&self.network.to_be_bytes());
        out.extend_from_slice(&self.source.to_be_bytes());
        out.extend_from_slice(&self.destination.to_be_bytes());
        out.extend_from_slice(&self.session.to_be_bytes());
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.extend_from_slice(&self.total.to_be_bytes());
        out.extend_from_slice(&[self.index, self.count, 1, self.span]);
        out.extend_from_slice(&[0; 16]);
        out.extend_from_slice(payload);
        decode(&out)?;
        Ok(out)
    }
}
pub fn decode(bytes: &[u8]) -> io::Result<(Header, &[u8])> {
    if bytes.len() < HEADER
        || bytes.len() > 255
        || &bytes[..3] != b"LM\x01"
        || bytes[3] & !3 != 0
        || bytes[28] != 1
        || bytes[30..46].iter().any(|b| *b != 0)
    {
        return Err(invalid("invalid wire header"));
    }
    let h = Header {
        network: u32::from_be_bytes(bytes[4..8].try_into().unwrap()),
        source: u16::from_be_bytes(bytes[8..10].try_into().unwrap()),
        destination: u16::from_be_bytes(bytes[10..12].try_into().unwrap()),
        session: u64::from_be_bytes(bytes[12..20].try_into().unwrap()),
        sequence: u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
        total: u16::from_be_bytes(bytes[24..26].try_into().unwrap()),
        index: bytes[26],
        count: bytes[27],
        span: bytes[29],
        ack: bytes[3] & 1 != 0,
        request_ack: bytes[3] & 2 != 0,
    };
    let total = usize::from(h.total);
    let span = usize::from(h.span);
    if !(20..=MTU).contains(&total)
        || !(48..=MAX_SPAN).contains(&span)
        || h.session == 0
        || h.source == h.destination
        || h.count == 0
        || h.count > 32
        || usize::from(h.count) != total.div_ceil(span)
        || h.index >= h.count
    {
        return Err(invalid("invalid fragment dimensions"));
    }
    let payload = &bytes[HEADER..];
    if h.ack {
        if h.request_ack
            || h.index != 0
            || payload.len() != 4
            || u32::from_be_bytes(payload.try_into().unwrap()) & !h.mask() != 0
        {
            return Err(invalid("invalid ACK"));
        }
    } else if payload.len() != span.min(total - usize::from(h.index) * span) {
        return Err(invalid("invalid fragment length"));
    }
    Ok((h, payload))
}
/// Validate the complete IPv4 header, while preserving IP fragmentation and options.
pub fn ipv4(packet: &[u8]) -> io::Result<()> {
    if packet.len() < 20 || packet.len() > MTU || packet[0] >> 4 != 4 {
        return Err(invalid("invalid IPv4 size/version"));
    }
    let ihl = usize::from(packet[0] & 15) * 4;
    if ihl < 20
        || ihl > packet.len()
        || usize::from(u16::from_be_bytes([packet[2], packet[3]])) != packet.len()
    {
        return Err(invalid("invalid IPv4 lengths"));
    }
    let mut sum = 0u32;
    for w in packet[..ihl].chunks_exact(2) {
        sum += u32::from(u16::from_be_bytes([w[0], w[1]]));
    }
    while sum >> 16 != 0 {
        sum = (sum & 65535) + (sum >> 16);
    }
    if sum != 65535 {
        return Err(invalid("invalid IPv4 checksum"));
    }
    Ok(())
}
