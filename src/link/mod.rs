//! Bounded single-hop reliable IPv4 link, driven by monotonic microseconds.
pub mod daemon;
pub mod simulation;
pub mod wire;
use crate::radio::protocol::{RadioProfile, invalid};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    convert::TryInto,
    io,
};
use wire::{HEADER, Header, MAX_SPAN, MTU};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub network: u32,
    pub node: u16,
    pub peer: u16,
    pub session: u64,
    pub profile: RadioProfile,
    pub span: usize,
    pub batch: usize,
    pub queue_packets: usize,
    pub queue_bytes: usize,
    pub queue_airtime_us: u64,
    pub reassembly_bytes: usize,
    pub reassembly_packets: usize,
    pub lifetime_us: u64,
    pub max_retries: u8,
    pub fast_loss_retries: bool,
    pub opaque: bool,
    pub monotonic_sessions: bool,
    pub retain_failures: bool,
    pub overhead_bytes: usize,
    pub turnaround_us: u64,
    pub duty_per_mille: u16,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            network: 1,
            node: 1,
            peer: 2,
            session: 1,
            profile: RadioProfile::default(),
            span: MAX_SPAN,
            batch: 4,
            queue_packets: 16,
            queue_bytes: 24_000,
            queue_airtime_us: 120_000_000,
            reassembly_bytes: 12_000,
            reassembly_packets: 8,
            lifetime_us: 120_000_000,
            max_retries: 5,
            fast_loss_retries: true,
            opaque: false,
            monotonic_sessions: false,
            retain_failures: false,
            overhead_bytes: 0,
            turnaround_us: 40_000,
            duty_per_mille: 1000,
        }
    }
}
impl Config {
    pub fn validate(&self) -> io::Result<()> {
        self.profile.validate()?;
        if self.node == self.peer
            || self.session == 0
            || !self.profile.crc
            || self.overhead_bytes > 16
            || self.span > MAX_SPAN.saturating_sub(self.overhead_bytes)
            || !(48..=MAX_SPAN).contains(&self.span)
            || !(1..=8).contains(&self.batch)
            || !(1..=64).contains(&self.queue_packets)
            || !(MTU..=96_000).contains(&self.queue_bytes)
            || !(1..=3_600_000_000).contains(&self.queue_airtime_us)
            || !(MTU..=96_000).contains(&self.reassembly_bytes)
            || !(1..=64).contains(&self.reassembly_packets)
            || !(1_000_000..=3_600_000_000).contains(&self.lifetime_us)
            || self.max_retries > 16
            || !(1_000..=1_000_000).contains(&self.turnaround_us)
            || !(1..=1000).contains(&self.duty_per_mille)
        {
            return Err(invalid("invalid link configuration"));
        }
        Ok(())
    }
    fn frame_period(&self, bytes: usize) -> u64 {
        self.profile
            .airtime_us((bytes + self.overhead_bytes).min(255))
            .unwrap()
            * 1000
            / u64::from(self.duty_per_mille)
            + self.turnaround_us
    }
    pub fn ack_wait_us(&self) -> u64 {
        self.frame_period(HEADER + 4) + self.turnaround_us * 4
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Metrics {
    pub admitted: u64,
    pub delivered: u64,
    pub delivered_bytes: u64,
    pub acknowledged: u64,
    pub expired: u64,
    pub retries_exhausted: u64,
    pub ingress_dropped: u64,
    pub malformed: u64,
    pub foreign: u64,
    pub duplicates: u64,
    pub replay_rejected: u64,
    pub reassembly_dropped: u64,
    pub reassembly_expired: u64,
    pub egress_dropped: u64,
    pub data_frames: u64,
    pub ack_frames: u64,
    pub retransmissions: u64,
    pub airtime_us: u64,
    pub queue_bytes_peak: usize,
    pub reassembly_bytes_peak: usize,
    pub radio_failures: u64,
    pub queue_delay_us: u64,
}
struct Outgoing {
    header: Header,
    data: Vec<u8>,
    born: u64,
    acked: u32,
    sent: u32,
    pending: u32,
    rounds: u8,
    deadline: Option<u64>,
    airtime: u64,
}
struct Assembly {
    header: Header,
    data: Vec<u8>,
    mask: u32,
    expires: u64,
}
struct Ack {
    header: Header,
    mask: u32,
    due: u64,
}
#[derive(Default)]
struct Replay {
    high: Option<u32>,
    bits: u64,
}
impl Replay {
    fn contains(&self, n: u32) -> bool {
        self.high
            .map(|high| n <= high && (high - n >= 64 || self.bits & (1u64 << (high - n)) != 0))
            .unwrap_or(false)
    }
    fn mark(&mut self, n: u32) {
        match self.high {
            None => {
                self.high = Some(n);
                self.bits = 1;
            }
            Some(h) if n > h => {
                self.bits = if n - h >= 64 {
                    1
                } else {
                    (self.bits << (n - h)) | 1
                };
                self.high = Some(n);
            }
            Some(h) if h - n < 64 => self.bits |= 1 << (h - n),
            _ => {}
        }
    }
}
pub struct Endpoint {
    config: Config,
    pub metrics: Metrics,
    sequence: Option<u32>,
    queue: VecDeque<Outgoing>,
    queue_bytes: usize,
    queue_airtime: u64,
    assemblies: BTreeMap<(u64, u32), Assembly>,
    assembly_bytes: usize,
    replay: BTreeMap<u64, Replay>,
    acks: BTreeMap<(u64, u32), Ack>,
    delivered: VecDeque<Vec<u8>>,
    failed: VecDeque<Vec<u8>>,
    busy: bool,
    burst_left: usize,
    next_tx: u64,
    last_airtime: u64,
    data_not_before: u64,
    peer_contended: bool,
}
impl Endpoint {
    pub fn new(config: Config) -> io::Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            metrics: Metrics::default(),
            sequence: Some(0),
            queue: VecDeque::new(),
            queue_bytes: 0,
            queue_airtime: 0,
            assemblies: BTreeMap::new(),
            assembly_bytes: 0,
            replay: BTreeMap::new(),
            acks: BTreeMap::new(),
            delivered: VecDeque::new(),
            failed: VecDeque::new(),
            busy: false,
            burst_left: 0,
            next_tx: 0,
            last_airtime: 0,
            data_not_before: 0,
            peer_contended: false,
        })
    }
    pub fn ack_ready(&self, now: u64) -> bool {
        !self.busy && now >= self.next_tx && self.acks.values().any(|a| a.due <= now)
    }
    pub fn queued(&self) -> usize {
        self.queue.len()
    }
    pub fn reassembly_bytes(&self) -> usize {
        self.assembly_bytes
    }
    pub fn receive_packet(&mut self) -> Option<Vec<u8>> {
        self.delivered.pop_front()
    }
    pub fn enqueue(&mut self, data: Vec<u8>, now: u64) -> io::Result<u32> {
        if let Err(e) = if self.config.opaque {
            if (20..=MTU).contains(&data.len()) {
                Ok(())
            } else {
                Err(invalid("invalid datagram size"))
            }
        } else {
            wire::ipv4(&data)
        } {
            self.metrics.ingress_dropped += 1;
            return Err(e);
        }
        let airtime = data
            .chunks(self.config.span)
            .map(|p| {
                self.config
                    .profile
                    .airtime_us(HEADER + p.len() + self.config.overhead_bytes)
                    .unwrap()
            })
            .sum::<u64>();
        if self.queue.len() >= self.config.queue_packets
            || self.queue_bytes + data.len() > self.config.queue_bytes
            || self.queue_airtime + airtime > self.config.queue_airtime_us
        {
            self.metrics.ingress_dropped += 1;
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "link queue full"));
        }
        let sequence = self
            .sequence
            .ok_or_else(|| invalid("sequence exhausted: create a new session"))?;
        self.sequence = sequence.checked_add(1);
        let header = Header {
            network: self.config.network,
            source: self.config.node,
            destination: self.config.peer,
            session: self.config.session,
            sequence,
            total: data.len() as u16,
            index: 0,
            count: data.len().div_ceil(self.config.span) as u8,
            span: self.config.span as u8,
            ack: false,
            request_ack: false,
        };
        self.queue_bytes += data.len();
        self.queue_airtime += airtime;
        self.metrics.queue_bytes_peak = self.metrics.queue_bytes_peak.max(self.queue_bytes);
        self.metrics.admitted += 1;
        self.queue.push_back(Outgoing {
            header,
            data,
            born: now,
            acked: 0,
            sent: 0,
            pending: header.mask(),
            rounds: 0,
            deadline: None,
            airtime,
        });
        Ok(sequence)
    }
    /// Insert bounded control traffic ahead of waiting data, without preempting a started packet.
    pub fn enqueue_control(&mut self, data: Vec<u8>, now: u64) -> io::Result<u32> {
        let id = self.enqueue(data, now)?;
        let packet = self.queue.pop_back().unwrap();
        let position = usize::from(self.queue.front().map(|p| p.sent != 0).unwrap_or(false));
        self.queue.insert(position, packet);
        Ok(id)
    }
    pub fn take_failed(&mut self) -> Option<Vec<u8>> {
        self.failed.pop_front()
    }
    fn fail(&mut self) {
        if self.config.retain_failures && self.failed.len() < self.config.queue_packets {
            if let Some(p) = self.queue.front() {
                self.failed.push_back(p.data.clone());
            }
        }
        self.pop();
    }
    fn pop(&mut self) {
        if let Some(p) = self.queue.pop_front() {
            self.queue_bytes -= p.data.len();
            self.queue_airtime -= p.airtime;
        }
        self.burst_left = 0;
    }
    pub fn tick(&mut self, now: u64) {
        while self
            .queue
            .front()
            .map(|p| now.saturating_sub(p.born) >= self.config.lifetime_us)
            .unwrap_or(false)
        {
            self.fail();
            self.metrics.expired += 1;
        }
        let assembly_bytes = &mut self.assembly_bytes;
        let expired = &mut self.metrics.reassembly_expired;
        self.assemblies.retain(|_, a| {
            if now >= a.expires {
                *assembly_bytes -= a.data.len();
                *expired += 1;
                false
            } else {
                true
            }
        });
        if let Some(p) = self.queue.front_mut() {
            if p.deadline.map(|d| now >= d).unwrap_or(false) && !self.busy {
                if p.rounds >= self.config.max_retries {
                    self.fail();
                    self.metrics.retries_exhausted += 1;
                } else {
                    p.rounds += 1;
                    p.pending = p.header.mask() & !p.acked;
                    p.deadline = None;
                    self.burst_left = 0;
                    // SplitMix64 avalanche avoids near-identical schedules for adjacent IDs.
                    let mut x = self.config.session
                        ^ (u64::from(self.config.node) << 32)
                        ^ u64::from(p.rounds);
                    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
                    x ^= x >> 31;
                    let window = if self.config.fast_loss_retries
                        && p.acked != 0
                        && p.rounds <= 2
                        && !self.peer_contended
                    {
                        // Established progress usually means packet loss, not contention.
                        // Escalate to a full burst window after repeated missing ACKs.
                        self.config.ack_wait_us()
                    } else {
                        self.config.frame_period(255)
                            * (self.config.batch as u64 + 1)
                            * u64::from(p.rounds.min(4))
                    };
                    let jitter = x % window.max(1);
                    self.next_tx = self.next_tx.max(now + jitter);
                }
            }
        }
    }
    /// Return at most one frame. Caller MUST report completion/failure before polling again.
    pub fn poll_transmit(&mut self, now: u64) -> Option<Vec<u8>> {
        self.tick(now);
        if self.busy || now < self.next_tx {
            return None;
        }
        let key = self
            .acks
            .iter()
            .filter(|(_, a)| a.due <= now)
            .min_by_key(|(_, a)| a.due)
            .map(|(k, _)| *k);
        let bytes = if let Some(key) = key {
            let a = self.acks.remove(&key).unwrap();
            let mut h = a.header;
            std::mem::swap(&mut h.source, &mut h.destination);
            h.ack = true;
            h.request_ack = false;
            h.index = 0;
            self.metrics.ack_frames += 1;
            h.encode(&a.mask.to_be_bytes()).unwrap()
        } else {
            if now < self.data_not_before {
                return None;
            }
            let p = self.queue.front_mut()?;
            if p.deadline.is_some() {
                return None;
            }
            if p.pending == 0 {
                p.deadline = Some(now + self.config.ack_wait_us());
                return None;
            }
            if self.burst_left == 0 {
                self.burst_left = self.config.batch;
            }
            let index = p.pending.trailing_zeros() as u8;
            let bit = 1u32 << index;
            p.pending &= !bit;
            let mut h = p.header;
            h.index = index;
            self.burst_left -= 1;
            h.request_ack = self.burst_left == 0 || p.pending == 0;
            if p.sent & bit != 0 {
                self.metrics.retransmissions += 1;
            }
            if p.sent == 0 {
                self.metrics.queue_delay_us += now.saturating_sub(p.born);
            }
            p.sent |= bit;
            if h.request_ack {
                p.deadline = Some(u64::MAX);
                self.burst_left = 0;
            }
            self.metrics.data_frames += 1;
            let start = usize::from(index) * usize::from(h.span);
            h.encode(&p.data[start..(start + usize::from(h.span)).min(p.data.len())])
                .unwrap()
        };
        self.last_airtime = self
            .config
            .profile
            .airtime_us(bytes.len() + self.config.overhead_bytes)
            .unwrap();
        self.metrics.airtime_us += self.last_airtime;
        self.busy = true;
        Some(bytes)
    }
    pub fn transmitted(&mut self, now: u64, success: bool) {
        if !self.busy {
            return;
        }
        self.busy = false;
        if !success {
            self.metrics.radio_failures += 1;
        }
        self.next_tx = now
            + self.config.turnaround_us
            + self.last_airtime * (1000 - u64::from(self.config.duty_per_mille))
                / u64::from(self.config.duty_per_mille);
        if let Some(p) = self.queue.front_mut() {
            if p.deadline == Some(u64::MAX) {
                p.deadline = Some(self.next_tx + self.config.ack_wait_us());
            }
        }
    }
    fn ack(&mut self, h: Header, mask: u32, now: u64) {
        let key = (h.session, h.sequence);
        if !self.acks.contains_key(&key) && self.acks.len() >= self.config.reassembly_packets + 8 {
            return;
        }
        let delay = if h.request_ack {
            self.config.turnaround_us
        } else {
            self.config.frame_period(255) * 8 + self.config.ack_wait_us()
        };
        self.acks
            .entry(key)
            .and_modify(|a| {
                a.mask |= mask;
                a.due = a.due.min(now + delay);
            })
            .or_insert(Ack {
                header: h,
                mask,
                due: now + delay,
            });
    }
    pub fn on_frame(&mut self, bytes: &[u8], now: u64) {
        self.tick(now);
        let (h, payload) = match wire::decode(bytes) {
            Ok(v) => v,
            Err(_) => {
                self.metrics.malformed += 1;
                return;
            }
        };
        if h.network != self.config.network
            || h.source != self.config.peer
            || h.destination != self.config.node
        {
            self.metrics.foreign += 1;
            return;
        }
        if h.ack {
            if let Some(p) = self.queue.front_mut() {
                if h.session == p.header.session
                    && h.sequence == p.header.sequence
                    && h.total == p.header.total
                    && h.count == p.header.count
                    && h.span == p.header.span
                {
                    let bits = u32::from_be_bytes(payload.try_into().unwrap());
                    // Ignore impossible acknowledgements of fragments never transmitted.
                    if bits & !p.sent != 0 {
                        self.metrics.malformed += 1;
                        return;
                    }
                    if bits & !p.acked != 0 {
                        p.rounds = 0;
                    }
                    p.acked |= bits;
                    if p.acked == p.header.mask() {
                        self.metrics.acknowledged += 1;
                        self.pop();
                    } else if p.deadline.is_some() && p.deadline != Some(u64::MAX) {
                        p.pending &= !p.acked;
                        if p.pending != 0 {
                            p.deadline = None;
                        }
                    }
                }
            }
            return;
        }
        self.peer_contended = true;
        // A received data frame reserves a short receive window for the rest of its burst.
        // ACK traffic remains eligible throughout that window.
        self.data_not_before = now
            + if h.request_ack {
                self.config.ack_wait_us()
            } else {
                self.config.frame_period(255) * 8 + self.config.ack_wait_us()
            };
        if self.config.monotonic_sessions {
            if let Some((&epoch, _)) = self.replay.last_key_value() {
                if h.session < epoch {
                    self.metrics.replay_rejected += 1;
                    return;
                }
                if h.session > epoch {
                    self.replay.clear();
                    self.assemblies.clear();
                    self.assembly_bytes = 0;
                    self.acks.clear();
                }
            }
        }
        if !self.replay.contains_key(&h.session) {
            if self.replay.len() >= 8 {
                self.metrics.replay_rejected += 1;
                return;
            }
            self.replay.insert(h.session, Replay::default());
        }
        if self.replay[&h.session].contains(h.sequence) {
            self.metrics.duplicates += 1;
            self.ack(h, h.mask(), now);
            return;
        }
        let key = (h.session, h.sequence);
        if !self.assemblies.contains_key(&key) {
            if self.assemblies.len() >= self.config.reassembly_packets
                || self.assembly_bytes + usize::from(h.total) > self.config.reassembly_bytes
            {
                self.metrics.reassembly_dropped += 1;
                return;
            }
            self.assembly_bytes += usize::from(h.total);
            self.metrics.reassembly_bytes_peak =
                self.metrics.reassembly_bytes_peak.max(self.assembly_bytes);
            self.assemblies.insert(
                key,
                Assembly {
                    header: h,
                    data: vec![0; usize::from(h.total)],
                    mask: 0,
                    expires: now + self.config.lifetime_us,
                },
            );
        }
        let a = self.assemblies.get_mut(&key).unwrap();
        if a.header.total != h.total || a.header.count != h.count || a.header.span != h.span {
            self.metrics.malformed += 1;
            return;
        }
        let start = usize::from(h.index) * usize::from(h.span);
        let bit = 1u32 << h.index;
        if a.mask & bit != 0 {
            if &a.data[start..start + payload.len()] != payload {
                self.metrics.malformed += 1;
                return;
            }
            self.metrics.duplicates += 1;
        } else {
            a.data[start..start + payload.len()].copy_from_slice(payload);
            a.mask |= bit;
        }
        let mask = a.mask;
        if mask == h.mask() {
            if self.delivered.len() >= self.config.queue_packets {
                self.metrics.egress_dropped += 1;
                return;
            }
            let a = self.assemblies.remove(&key).unwrap();
            self.assembly_bytes -= a.data.len();
            if !self.config.opaque && wire::ipv4(&a.data).is_err() {
                self.metrics.malformed += 1;
                return;
            }
            self.replay.get_mut(&h.session).unwrap().mark(h.sequence);
            self.metrics.delivered += 1;
            self.metrics.delivered_bytes += a.data.len() as u64;
            self.delivered.push_back(a.data);
        }
        self.ack(h, mask, now);
    }
    pub fn next_deadline(&self) -> Option<u64> {
        let expiry = self.queue.front().map(|p| p.born + self.config.lifetime_us);
        let tx = if self.busy {
            None
        } else {
            let ack = self.acks.values().map(|a| a.due.max(self.next_tx)).min();
            let data = self
                .queue
                .front()
                .map(|p| p.deadline.unwrap_or(self.next_tx.max(self.data_not_before)));
            [ack, data].iter().filter_map(|v| *v).min()
        };
        [
            expiry,
            tx,
            self.assemblies.values().map(|a| a.expires).min(),
        ]
        .iter()
        .filter_map(|v| *v)
        .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sequence_exhaustion_never_wraps() {
        let mut link = Endpoint::new(Config::default()).unwrap();
        link.sequence = Some(u32::MAX);
        assert_eq!(
            link.enqueue(simulation::packet(32, 0), 0).unwrap(),
            u32::MAX
        );
        assert!(link.enqueue(simulation::packet(32, 0), 0).is_err());
    }
}

#[cfg(target_os = "linux")]
pub(crate) mod tun;
