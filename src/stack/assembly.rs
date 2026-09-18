//! Bounded compatibility reassembly. Legacy frames still cannot identify missing/reordered chunks.
use super::Frame;
use std::{
    collections::HashMap,
    io,
    time::{Duration, Instant},
};
const MAX_PENDING: usize = 64;
const MAX_PACKET: usize = 1500;
const MAX_CHUNKS: usize = 150;
struct Pending {
    created: Instant,
    chunks: Vec<Frame>,
    bytes: usize,
}
pub struct LegacyAssemblies {
    pending: HashMap<(u8, u8), Pending>,
    timeout: Duration,
}
impl LegacyAssemblies {
    pub fn new(timeout: Duration) -> Self {
        Self {
            pending: HashMap::new(),
            timeout,
        }
    }
    pub fn expire(&mut self, now: Instant) {
        let timeout = self.timeout;
        self.pending
            .retain(|_, p| now.duration_since(p.created) < timeout);
    }
    pub fn push(&mut self, mut frame: Frame, now: Instant) -> io::Result<Option<Frame>> {
        self.expire(now);
        let key = (frame.sender(), frame.frameid());
        let length = frame.payload_len();
        if length > MAX_PACKET {
            return Err(io::ErrorKind::InvalidData.into());
        }
        if self
            .pending
            .get(&key)
            .map(|p| p.bytes + length > MAX_PACKET || p.chunks.len() >= MAX_CHUNKS)
            .unwrap_or(false)
        {
            self.pending.remove(&key);
            return Err(io::ErrorKind::InvalidData.into());
        }
        if frame.txflag().more_chunks() {
            if !self.pending.contains_key(&key) && self.pending.len() >= MAX_PENDING {
                return Err(io::ErrorKind::OutOfMemory.into());
            }
            let p = self.pending.entry(key).or_insert_with(|| Pending {
                created: now,
                chunks: vec![],
                bytes: 0,
            });
            p.bytes += length;
            p.chunks.push(frame);
            Ok(None)
        } else {
            if let Some(mut p) = self.pending.remove(&key) {
                let header = frame.header();
                p.chunks.push(frame);
                frame = super::frame::recombine_chunks(p.chunks, header);
            }
            Ok(Some(frame))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn chunk(id: u8, len: usize) -> Frame {
        Frame::new(1, id, 9, 1, 0, vec![], vec![0; len])
    }
    #[test]
    fn incomplete_packets_have_count_byte_and_time_bounds() {
        let now = Instant::now();
        let mut a = LegacyAssemblies::new(Duration::from_secs(1));
        for id in 0..64 {
            assert!(a.push(chunk(id, 1), now).unwrap().is_none());
        }
        assert!(a.push(chunk(64, 1), now).is_err());
        assert_eq!(a.pending.len(), 64);
        a.expire(now + Duration::from_secs(1));
        assert!(a.pending.is_empty());
        for _ in 0..6 {
            a.push(chunk(1, 250), now).unwrap();
        }
        assert!(a.push(chunk(1, 1), now).is_err());
        assert!(a.pending.is_empty());
        for _ in 0..150 {
            a.push(chunk(1, 0), now).unwrap();
        }
        assert!(a.push(chunk(1, 0), now).is_err());
    }
}
