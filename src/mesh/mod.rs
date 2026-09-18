//! Bounded authenticated multihop IPv4 overlay over the reliable link core.
pub mod routing;
pub mod security;
pub mod wire;
use crate::{
    link::{self, Endpoint},
    radio::protocol::invalid,
};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use security::Vault;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    net::Ipv4Addr,
};
use wire::Packet;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub address: Ipv4Addr,
    pub verifying_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub link: link::Config,
    pub members: BTreeMap<u16, Member>,
    pub peers: Vec<u16>,
    pub static_routes: BTreeMap<u16, u16>,
    pub dynamic: bool,
    pub announce_us: u64,
    pub neighbor_timeout_us: u64,
    pub hop_limit: u8,
    pub pending_packets: usize,
    pub cache_routes: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            link: link::Config::default(),
            members: BTreeMap::new(),
            peers: vec![],
            static_routes: BTreeMap::new(),
            dynamic: true,
            announce_us: 60_000_000,
            neighbor_timeout_us: 240_000_000,
            hop_limit: 8,
            pending_packets: 32,
            cache_routes: true,
        }
    }
}
impl Config {
    pub fn validate(&self) -> io::Result<()> {
        let mut link = self.link.clone();
        link.peer = self.peers.first().copied().unwrap_or(self.link.node ^ 1);
        link.validate()?;
        let ids: BTreeSet<_> = self.peers.iter().copied().collect();
        let addresses: BTreeSet<_> = self.members.values().map(|m| m.address).collect();
        if self.link.node == 0
            || self.members.len() < 2
            || self.members.len() > 64
            || !self.members.contains_key(&self.link.node)
            || addresses.len() != self.members.len()
            || self.peers.is_empty()
            || self.peers.len() > 32
            || ids.len() != self.peers.len()
            || ids.contains(&self.link.node)
            || ids.iter().any(|p| !self.members.contains_key(p))
            || !(100_000..=600_000_000).contains(&self.announce_us)
            || self.neighbor_timeout_us < self.announce_us.saturating_mul(3)
            || self.neighbor_timeout_us > 3_600_000_000
            || !(1..=16).contains(&self.hop_limit)
            || !(1..=64).contains(&self.pending_packets)
        {
            return Err(invalid("invalid mesh configuration"));
        }
        let mut signing_identities = BTreeSet::new();
        for (id, m) in &self.members {
            if *id == 0
                || m.address.is_unspecified()
                || m.address.is_multicast()
                || m.address.is_broadcast()
            {
                return Err(invalid("invalid member address"));
            }
            let bytes = hex::decode(&m.verifying_key)
                .map_err(|_| invalid("invalid identity public key"))?;
            let key: [u8; 32] = bytes
                .try_into()
                .map_err(|_| invalid("identity key length"))?;
            if !signing_identities.insert(key) {
                return Err(invalid("duplicate signing identity"));
            }
            VerifyingKey::from_bytes(&key).map_err(|_| invalid("invalid identity public key"))?;
        }
        for (&dest, &next) in &self.static_routes {
            if dest == self.link.node || !self.members.contains_key(&dest) || !ids.contains(&next) {
                return Err(invalid("invalid static next hop"));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Metrics {
    pub delivered: u64,
    pub delivered_bytes: u64,
    pub forwarded: u64,
    pub duplicate: u64,
    pub hop_drops: u64,
    pub malformed: u64,
    pub auth_rejected: u64,
    pub foreign_frames: u64,
    pub no_route_expired: u64,
    pub queue_drops: u64,
    pub control_generated: u64,
    pub control_forwarded: u64,
    pub signature_rejected: u64,
    pub route_changes: u64,
    pub failovers: u64,
    pub pending_peak: usize,
    pub radio_airtime_us: u64,
}
struct Neighbor {
    seen: Option<u64>,
    cost: u32,
    last_frames: u64,
    last_retries: u64,
}
struct Pending {
    packet: Packet,
    expires: u64,
}
pub struct Node {
    pub config: Config,
    pub metrics: Metrics,
    pub router: routing::Router,
    pub links: BTreeMap<u16, Endpoint>,
    pub vault: Vault,
    signer: SigningKey,
    identities: BTreeMap<u16, VerifyingKey>,
    neighbors: BTreeMap<u16, Neighbor>,
    pending: VecDeque<Pending>,
    control: BTreeMap<(u16, u16), Vec<u8>>,
    seen: BTreeMap<u16, (u64, u32, u64)>,
    delivered: VecDeque<Vec<u8>>,
    sequence: Option<u32>,
    announcement_sequence: Option<u32>,
    next_announce: u64,
    dirty: bool,
    active: Option<u16>,
    cursor: usize,
    next_tx: u64,
    last_airtime: u64,
    receive_until: u64,
    requested_ack: Option<(u16, u64, u32, u8)>,
    last_now: u64,
    last_requests_ack: bool,
    failover_history: BTreeMap<(u16, u64, u32), (u8, u64)>,
}
impl Node {
    pub fn new(config: Config, vault: Vault, signer: SigningKey) -> io::Result<Self> {
        config.validate()?;
        let node = config.link.node;
        if !vault.matches(config.link.network, node, &config.peers) {
            return Err(invalid(
                "vault identity/peers do not match mesh configuration",
            ));
        }
        let identities: BTreeMap<_, _> = config
            .members
            .iter()
            .map(|(id, m)| {
                let bytes = hex::decode(&m.verifying_key).unwrap();
                (
                    *id,
                    VerifyingKey::from_bytes(&bytes.try_into().unwrap()).unwrap(),
                )
            })
            .collect();
        if signer.verifying_key() != identities[&node] {
            return Err(invalid("signing key does not match configured identity"));
        }
        let mut links = BTreeMap::new();
        let mut neighbors = BTreeMap::new();
        for peer in &config.peers {
            let mut lc = config.link.clone();
            lc.peer = *peer;
            lc.session = vault.epoch();
            lc.span = lc.span.min(security::SPAN);
            lc.overhead_bytes = security::OVERHEAD;
            lc.opaque = true;
            lc.monotonic_sessions = true;
            lc.retain_failures = true;
            links.insert(*peer, Endpoint::new(lc)?);
            neighbors.insert(
                *peer,
                Neighbor {
                    seen: None,
                    cost: config.link.profile.airtime_us(255)?.min(1_000_000_000) as u32,
                    last_frames: 0,
                    last_retries: 0,
                },
            );
        }
        let initial_phase = (u64::from(node).wrapping_mul(0x9e3779b97f4a7c15) >> 32)
            % (config.announce_us / 4).max(1);
        Ok(Self {
            config,
            metrics: Metrics::default(),
            router: routing::Router::new(node),
            links,
            vault,
            signer,
            identities,
            neighbors,
            pending: VecDeque::new(),
            control: BTreeMap::new(),
            seen: BTreeMap::new(),
            delivered: VecDeque::new(),
            sequence: Some(0),
            announcement_sequence: Some(0),
            next_announce: initial_phase,
            dirty: true,
            active: None,
            cursor: 0,
            next_tx: 0,
            last_airtime: 0,
            receive_until: 0,
            requested_ack: None,
            last_now: 0,
            last_requests_ack: false,
            failover_history: BTreeMap::new(),
        })
    }
    fn live(&self, peer: u16, now: u64) -> bool {
        self.neighbors
            .get(&peer)
            .and_then(|n| n.seen)
            .map(|t| now.saturating_sub(t) < self.config.neighbor_timeout_us)
            .unwrap_or(false)
    }
    fn local_links(&self, now: u64) -> BTreeMap<u16, u32> {
        self.neighbors
            .iter()
            .filter(|(p, _)| self.live(**p, now))
            .map(|(p, n)| (*p, n.cost))
            .collect()
    }
    fn refresh_routes(&mut self, now: u64) {
        let links = self.local_links(now);
        let changed =
            self.router.update(self.config.link.node, links, u64::MAX) | self.router.expire(now);
        if changed || !self.config.cache_routes {
            let old = self.router.routes.clone();
            self.router.rebuild();
            if old != self.router.routes {
                self.metrics.route_changes += 1;
            }
        }
    }
    pub fn route(&self, destination: u16, now: u64) -> Option<u16> {
        self.config
            .static_routes
            .get(&destination)
            .copied()
            .filter(|p| self.live(*p, now))
            .or_else(|| {
                self.router
                    .routes
                    .get(&destination)
                    .map(|r| r.next)
                    .filter(|p| self.config.dynamic && self.live(*p, now))
            })
    }
    pub fn enqueue(&mut self, bytes: Vec<u8>, now: u64) -> io::Result<()> {
        link::wire::ipv4(&bytes)?;
        if bytes.len() > wire::MTU
            || bytes[12..16] != self.config.members[&self.config.link.node].address.octets()
        {
            return Err(invalid("invalid local IPv4 source or MTU"));
        }
        let destination = self
            .config
            .members
            .iter()
            .find(|(_, m)| bytes[16..20] == m.address.octets())
            .map(|(id, _)| *id)
            .ok_or_else(|| invalid("destination not in mesh membership"))?;
        if destination == self.config.link.node {
            return Err(invalid("self-directed mesh packet"));
        }
        if self.pending.len() >= self.config.pending_packets {
            self.metrics.queue_drops += 1;
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "mesh pending queue full",
            ));
        }
        let sequence = self
            .sequence
            .ok_or_else(|| invalid("packet sequence exhausted: restart"))?;
        self.sequence = sequence.checked_add(1);
        self.pending.push_back(Pending {
            packet: Packet {
                origin: self.config.link.node,
                destination,
                epoch: self.vault.epoch(),
                sequence,
                hops: self.config.hop_limit,
                announcement: false,
                payload: bytes,
            },
            expires: now + self.config.link.lifetime_us,
        });
        self.metrics.pending_peak = self.metrics.pending_peak.max(self.pending.len());
        Ok(())
    }
    pub fn receive(&mut self) -> Option<Vec<u8>> {
        self.delivered.pop_front()
    }
    fn seen(&mut self, p: &Packet) -> bool {
        let entry = self
            .seen
            .entry(p.origin)
            .or_insert((p.epoch, p.sequence, 0));
        if p.epoch < entry.0 {
            return true;
        }
        if p.epoch > entry.0 {
            *entry = (p.epoch, p.sequence, 0);
        }
        if p.sequence > entry.1 {
            entry.2 = if p.sequence - entry.1 >= 64 {
                0
            } else {
                entry.2 << (p.sequence - entry.1)
            };
            entry.1 = p.sequence;
        }
        if entry.1 - p.sequence >= 64 {
            return true;
        }
        let bit = 1 << (entry.1 - p.sequence);
        if entry.2 & bit != 0 {
            return true;
        }
        entry.2 |= bit;
        false
    }
    fn message(&mut self, peer: u16, bytes: &[u8], now: u64) -> io::Result<()> {
        let mut p = match Packet::decode(bytes) {
            Ok(p) => p,
            Err(_) => {
                self.metrics.malformed += 1;
                return Ok(());
            }
        };
        if !self.identities.contains_key(&p.origin) || p.origin == self.config.link.node {
            self.metrics.duplicate += 1;
            return Ok(());
        }
        if p.announcement {
            if p.payload.len() < 65 {
                self.metrics.malformed += 1;
                return Ok(());
            }
            let signature = Signature::from_slice(&p.payload[p.payload.len() - 64..])
                .map_err(|_| invalid("signature length"))?;
            p.payload.truncate(p.payload.len() - 64);
            if self.identities[&p.origin]
                .verify_strict(&p.signing_bytes(self.config.link.network), &signature)
                .is_err()
            {
                self.metrics.signature_rejected += 1;
                return Ok(());
            }
            let members = self.identities.keys().copied().collect();
            let links = match routing::decode_links(&p.payload, p.origin, &members) {
                Ok(l) => l,
                Err(_) => {
                    self.metrics.malformed += 1;
                    return Ok(());
                }
            };
            if !self.vault.announcement(p.origin, p.epoch, p.sequence)? {
                self.metrics.duplicate += 1;
                return Ok(());
            }
            let changed =
                self.router
                    .update(p.origin, links, now + self.config.neighbor_timeout_us);
            if changed {
                self.router.rebuild();
                self.metrics.route_changes += 1;
            }
            if p.hops > 1 {
                p.hops -= 1;
                p.payload.extend_from_slice(&signature.to_bytes());
                let bytes = p.encode()?;
                for next in &self.config.peers {
                    if *next != peer {
                        self.control.insert((*next, p.origin), bytes.clone());
                        self.metrics.control_forwarded += 1;
                    }
                }
            }
            return Ok(());
        }
        if !self.config.members.contains_key(&p.destination)
            || link::wire::ipv4(&p.payload).is_err()
            || p.payload[12..16] != self.config.members[&p.origin].address.octets()
            || p.payload[16..20] != self.config.members[&p.destination].address.octets()
        {
            self.metrics.malformed += 1;
            return Ok(());
        }
        if p.destination == self.config.link.node {
            if self.delivered.len() >= self.config.pending_packets {
                self.metrics.queue_drops += 1;
                return Ok(());
            }
            if self.seen(&p) {
                self.metrics.duplicate += 1;
                return Ok(());
            }
            self.metrics.delivered += 1;
            self.metrics.delivered_bytes += p.payload.len() as u64;
            self.delivered.push_back(p.payload);
            return Ok(());
        }
        if p.hops <= 1 || p.payload[8] <= 1 {
            self.metrics.hop_drops += 1;
            return Ok(());
        }
        if self.pending.len() >= self.config.pending_packets {
            self.metrics.queue_drops += 1;
            return Ok(());
        }
        if self.seen(&p) {
            self.metrics.duplicate += 1;
            return Ok(());
        }
        p.hops -= 1;
        p.payload[8] -= 1;
        crate::link::simulation::checksum(&mut p.payload);
        self.pending.push_back(Pending {
            packet: p,
            expires: now + self.config.link.lifetime_us,
        });
        self.metrics.forwarded += 1;
        self.metrics.pending_peak = self.metrics.pending_peak.max(self.pending.len());
        Ok(())
    }
    pub fn on_radio(&mut self, bytes: &[u8], now: u64) -> io::Result<()> {
        if bytes.len() >= 12
            && &bytes[..3] == b"LM\x02"
            && (u32::from_be_bytes(bytes[4..8].try_into().unwrap()) != self.config.link.network
                || u16::from_be_bytes(bytes[10..12].try_into().unwrap()) != self.config.link.node)
        {
            self.metrics.foreign_frames += 1;
            return Ok(());
        }
        let plain = match self.vault.open(bytes) {
            Ok(p) => p,
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                self.metrics.auth_rejected += 1;
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        let (h, payload) = link::wire::decode(&plain)?;
        if h.ack {
            if let Some((peer, session, sequence, index)) = self.requested_ack {
                if (peer, session, sequence) == (h.source, h.session, h.sequence)
                    && u32::from_be_bytes(payload.try_into().unwrap()) & (1 << index) != 0
                {
                    self.receive_until = now + self.config.link.turnaround_us;
                    self.requested_ack = None;
                }
            }
        } else {
            self.receive_until = now
                + if h.request_ack {
                    self.config.link.ack_wait_us()
                } else {
                    (self.config.link.profile.airtime_us(255)? + self.config.link.turnaround_us) * 8
                        + self.config.link.ack_wait_us()
                };
        }
        let peer = h.source;
        let was_live = self.live(peer, now);
        self.neighbors
            .get_mut(&peer)
            .ok_or_else(|| invalid("unconfigured authenticated peer"))?
            .seen = Some(now);
        if !was_live {
            self.dirty = true;
        }
        self.links.get_mut(&peer).unwrap().on_frame(&plain, now);
        while let Some(packet) = self.links.get_mut(&peer).unwrap().receive_packet() {
            self.message(peer, &packet, now)?;
        }
        self.refresh_routes(now);
        Ok(())
    }
    pub fn tick(&mut self, now: u64) -> io::Result<()> {
        self.last_now = now;
        for link in self.links.values_mut() {
            link.tick(now);
        }
        let mut changed = false;
        self.failover_history
            .retain(|_, (_, expires)| *expires > now);
        let mut failed = Vec::new();
        for (peer, link) in &mut self.links {
            while let Some(bytes) = link.take_failed() {
                failed.push((*peer, bytes));
            }
        }
        for (peer, bytes) in failed {
            if let Ok(packet) = Packet::decode(&bytes) {
                if packet.announcement {
                    continue;
                }
                let n = self.neighbors.get_mut(&peer).unwrap();
                if n.seen
                    .map(|t| now.saturating_sub(t) > self.config.link.ack_wait_us() * 2)
                    .unwrap_or(false)
                {
                    n.seen = None;
                    changed = true;
                }
                let key = (packet.origin, packet.epoch, packet.sequence);
                if self.pending.len() < self.config.pending_packets
                    && (self.failover_history.contains_key(&key)
                        || self.failover_history.len() < 64)
                {
                    let entry = self
                        .failover_history
                        .entry(key)
                        .or_insert((0, now + self.config.link.lifetime_us * 3));
                    if entry.0 < 2 {
                        entry.0 += 1;
                        self.pending.push_back(Pending {
                            packet,
                            expires: now + self.config.link.lifetime_us,
                        });
                        self.metrics.failovers += 1;
                    }
                }
            }
        }
        for (&peer, n) in &mut self.neighbors {
            if n.seen
                .map(|t| now.saturating_sub(t) >= self.config.neighbor_timeout_us)
                .unwrap_or(false)
            {
                n.seen = None;
                changed = true;
            }
            let m = &self.links[&peer].metrics;
            if m.data_frames.saturating_sub(n.last_frames) >= 32 {
                let frames = m.data_frames - n.last_frames;
                let retries = m.retransmissions - n.last_retries;
                let original = frames.saturating_sub(retries).max(1);
                let measured = (self.config.link.profile.airtime_us(255)? * frames / original)
                    .min(1_000_000_000) as u32;
                let next = (u64::from(n.cost) * 3 + u64::from(measured)) / 4;
                let next = next as u32;
                if n.cost.abs_diff(next) > n.cost / 5 {
                    changed = true;
                    n.cost = next;
                }
                n.last_frames = m.data_frames;
                n.last_retries = m.retransmissions;
            }
        }
        self.dirty |= changed;
        self.refresh_routes(now);
        if now >= self.next_announce {
            let sequence = self
                .announcement_sequence
                .ok_or_else(|| invalid("announcement sequence exhausted"))?;
            self.announcement_sequence = sequence.checked_add(1);
            let mut p = Packet {
                origin: self.config.link.node,
                destination: 0,
                epoch: self.vault.epoch(),
                sequence,
                hops: self.config.hop_limit,
                announcement: true,
                payload: routing::encode_links(&self.local_links(now))?,
            };
            let signature = self.signer.sign(&p.signing_bytes(self.config.link.network));
            p.payload.extend_from_slice(&signature.to_bytes());
            let bytes = p.encode()?;
            for peer in &self.config.peers {
                self.control.insert((*peer, p.origin), bytes.clone());
            }
            self.metrics.control_generated += 1;
            let phase = (u64::from(self.config.link.node).wrapping_mul(7919)
                ^ u64::from(sequence).wrapping_mul(104729))
                % (self.config.announce_us / 4).max(1);
            self.next_announce = now + self.config.announce_us + phase;
            self.dirty = false;
        } else if self.dirty {
            self.next_announce = self.next_announce.min(now + self.config.announce_us / 4);
            self.dirty = false;
        }
        for key in self.control.keys().copied().collect::<Vec<_>>() {
            if self.links[&key.0].queued() >= 2 {
                continue;
            }
            if self
                .links
                .get_mut(&key.0)
                .unwrap()
                .enqueue_control(self.control[&key].clone(), now)
                .is_ok()
            {
                self.control.remove(&key);
            }
        }
        let count = self.pending.len();
        for _ in 0..count {
            let pending = self.pending.pop_front().unwrap();
            if now >= pending.expires {
                self.metrics.no_route_expired += 1;
                continue;
            }
            if let Some(peer) = self.route(pending.packet.destination, now) {
                if self
                    .links
                    .get_mut(&peer)
                    .unwrap()
                    .enqueue(pending.packet.encode()?, now)
                    .is_ok()
                {
                    continue;
                }
            }
            self.pending.push_back(pending);
        }
        Ok(())
    }
    pub fn poll_transmit(&mut self, now: u64) -> io::Result<Option<Vec<u8>>> {
        self.tick(now)?;
        if self.active.is_some() || now < self.next_tx {
            return Ok(None);
        }
        let ack_peer = self
            .links
            .iter()
            .find(|(_, l)| l.ack_ready(now))
            .map(|(p, _)| *p);
        if ack_peer.is_none() && now < self.receive_until {
            return Ok(None);
        }
        let peers = self.config.peers.clone();
        for offset in 0..peers.len() {
            let index = (self.cursor + offset) % peers.len();
            let peer = peers[index];
            if ack_peer.map(|p| p != peer).unwrap_or(false) {
                continue;
            }
            if let Some(frame) = self.links.get_mut(&peer).unwrap().poll_transmit(now) {
                let (header, _) = link::wire::decode(&frame)?;
                self.last_requests_ack = header.request_ack;
                if header.request_ack {
                    self.requested_ack =
                        Some((peer, header.session, header.sequence, header.index));
                }
                let sealed = self.vault.seal(&frame)?;
                self.last_airtime = self.config.link.profile.airtime_us(sealed.len())?;
                self.metrics.radio_airtime_us += self.last_airtime;
                self.active = Some(peer);
                self.cursor = (index + 1) % peers.len();
                return Ok(Some(sealed));
            }
        }
        Ok(None)
    }
    pub fn transmitted(&mut self, now: u64, success: bool) {
        if let Some(peer) = self.active.take() {
            self.links.get_mut(&peer).unwrap().transmitted(now, success);
            if self.last_requests_ack {
                self.receive_until =
                    now + self.config.link.ack_wait_us() + self.config.link.turnaround_us;
            }
            self.next_tx = now
                + self.config.link.turnaround_us
                + self.last_airtime * (1000 - u64::from(self.config.link.duty_per_mille))
                    / u64::from(self.config.link.duty_per_mille);
        }
    }
    pub fn next_deadline(&self) -> Option<u64> {
        std::iter::once(Some(self.next_announce))
            .chain([Some(self.next_tx), Some(self.receive_until)])
            .chain(self.pending.iter().map(|p| Some(p.expires)))
            .chain(
                self.neighbors
                    .values()
                    .filter_map(|n| n.seen.map(|t| Some(t + self.config.neighbor_timeout_us))),
            )
            .chain(self.links.values().map(Endpoint::next_deadline))
            .flatten()
            .filter(|d| *d > self.last_now)
            .min()
    }
}

pub mod simulation;

#[cfg(unix)]
pub mod daemon;
