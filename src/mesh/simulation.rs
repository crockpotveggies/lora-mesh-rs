//! Deterministic secure mesh integration: production nodes, controllers and shared RF medium.
use super::{Config, Member, Node, security::Vault};
use crate::{
    radio::{Action, Controller, LineCodec, State, protocol::invalid},
    sim::{
        Scenario,
        medium::{Link, Medium},
        scenario::{Fault, NodeSpec, controller_config},
    },
};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
};
use zeroize::Zeroizing;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Traffic {
    pub at_us: u64,
    pub from: u16,
    pub to: u16,
    pub size: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Restart {
    pub at_us: u64,
    pub node: u16,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub version: u32,
    pub name: String,
    pub seed: u64,
    pub duration_us: u64,
    pub nodes: Vec<NodeSpec>,
    pub links: Vec<Link>,
    pub traffic: Vec<Traffic>,
    #[serde(default)]
    pub faults: Vec<Fault>,
    #[serde(default)]
    pub restarts: Vec<Restart>,
    #[serde(default = "announce")]
    pub announce_us: u64,
    #[serde(default = "timeout")]
    pub neighbor_timeout_us: u64,
    #[serde(default)]
    pub static_routes: BTreeMap<u16, BTreeMap<u16, u16>>,
    #[serde(default = "yes")]
    pub cache_routes: bool,
    #[serde(default)]
    pub expected_deliveries: Option<usize>,
}
fn announce() -> u64 {
    60_000_000
}
fn timeout() -> u64 {
    240_000_000
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Delivery {
    pub packet: usize,
    pub node: u16,
    pub latency_us: u64,
    pub ttl: u8,
}
#[derive(Serialize, Deserialize)]
pub struct Report {
    pub configuration: Case,
    pub deliveries: Vec<Delivery>,
    pub failures: Vec<String>,
    pub metrics: BTreeMap<u16, super::Metrics>,
    pub routes: BTreeMap<u16, BTreeMap<u16, super::routing::Route>>,
    pub route_recomputations: u64,
    pub airtime_us: u64,
    pub collisions: u64,
    pub control_frames: u64,
    pub goodput_bytes_sec: f64,
    pub p50_latency_us: u64,
    pub p95_latency_us: u64,
    pub trace: Vec<crate::sim::medium::Trace>,
    pub link_metrics: BTreeMap<u16, BTreeMap<u16, crate::link::Metrics>>,
}
pub fn signing_key(node: u16) -> SigningKey {
    let mut h = Sha256::new();
    h.update(b"SIMULATION ONLY SIGNER");
    h.update(node.to_be_bytes());
    SigningKey::from_bytes(&h.finalize().into())
}
pub fn pair_key(a: u16, b: u16) -> Zeroizing<[u8; 32]> {
    let mut h = Sha256::new();
    h.update(b"SIMULATION ONLY PAIRWISE KEY");
    h.update(a.min(b).to_be_bytes());
    h.update(a.max(b).to_be_bytes());
    Zeroizing::new(h.finalize().into())
}
fn configuration(case: &Case, id: u16) -> Config {
    let members = case
        .nodes
        .iter()
        .map(|n| {
            (
                n.id,
                Member {
                    address: std::net::Ipv4Addr::new(10, 107, (n.id >> 8) as u8, n.id as u8),
                    verifying_key: hex::encode(signing_key(n.id).verifying_key().to_bytes()),
                },
            )
        })
        .collect();
    let peers: BTreeSet<_> = case
        .links
        .iter()
        .filter_map(|l| {
            if l.from == id {
                Some(l.to)
            } else if l.to == id {
                Some(l.from)
            } else {
                None
            }
        })
        .collect();
    Config {
        members,
        peers: peers.into_iter().collect(),
        link: crate::link::Config {
            node: id,
            profile: case
                .nodes
                .iter()
                .find(|n| n.id == id)
                .unwrap()
                .profile
                .clone(),
            ..Default::default()
        },
        announce_us: case.announce_us,
        neighbor_timeout_us: case.neighbor_timeout_us,
        static_routes: case.static_routes.get(&id).cloned().unwrap_or_default(),
        cache_routes: case.cache_routes,
        ..Default::default()
    }
}
fn actions(
    id: u16,
    a: Vec<Action>,
    medium: &mut Medium,
    radio: &mut Controller,
    node: &mut Node,
) -> io::Result<()> {
    let mut pending: VecDeque<_> = a.into();
    while let Some(action) = pending.pop_front() {
        match action {
            Action::Write(line) => medium.bytes(id, format!("{}\r\n", line).as_bytes())?,
            Action::Reconnect => pending.extend(if medium.devices[&id].online {
                radio.connected(medium.now)
            } else {
                radio.disconnected(medium.now, "offline virtual device")
            }),
            Action::Received(bytes) => node.on_radio(&bytes, medium.now)?,
            Action::Transmitted(_) => node.transmitted(medium.now, true),
            Action::Failed(_, _) => node.transmitted(medium.now, false),
            _ => {}
        }
    }
    Ok(())
}
pub fn run(case: &Case) -> io::Result<Report> {
    let scenario = Scenario {
        version: case.version,
        name: case.name.clone(),
        seed: case.seed,
        duration_us: case.duration_us,
        max_events: 1_000_000,
        nodes: case.nodes.clone(),
        links: case.links.clone(),
        traffic: vec![],
        faults: case.faults.clone(),
        expect: Default::default(),
    };
    let mut medium = scenario.medium()?;
    let ids: BTreeSet<_> = case.nodes.iter().map(|n| n.id).collect();
    if ids.len() > 64
        || ids.contains(&0)
        || case.traffic.len() > 10000
        || case.restarts.len() > 1000
        || case.traffic.iter().any(|p| {
            p.from == p.to
                || !ids.contains(&p.from)
                || !ids.contains(&p.to)
                || !(32..=super::wire::MTU).contains(&p.size)
                || p.at_us >= case.duration_us
        })
        || case
            .restarts
            .iter()
            .any(|r| !ids.contains(&r.node) || r.at_us >= case.duration_us)
    {
        return Err(invalid("invalid mesh scenario"));
    }
    let mut nodes = BTreeMap::new();
    let mut radios = BTreeMap::new();
    let mut codecs = BTreeMap::new();
    for id in &ids {
        let cfg = configuration(case, *id);
        let keys = cfg.peers.iter().map(|p| (*p, pair_key(*id, *p))).collect();
        let vault = Vault::memory(cfg.link.network, *id, 1, keys)?;
        let mut rc = controller_config(&cfg.link.profile);
        rc.receive_guard_us = 0;
        nodes.insert(*id, Node::new(cfg, vault, signing_key(*id))?);
        radios.insert(*id, Controller::new(rc)?);
        codecs.insert(*id, LineCodec::default());
    }
    for id in &ids {
        actions(
            *id,
            vec![Action::Reconnect],
            &mut medium,
            radios.get_mut(id).unwrap(),
            nodes.get_mut(id).unwrap(),
        )?;
    }
    let mut traffic: Vec<_> = case.traffic.iter().enumerate().collect();
    traffic.sort_by_key(|(_, t)| t.at_us);
    let mut ti = 0;
    let mut faults = case.faults.clone();
    faults.sort_by_key(Fault::at);
    let mut fi = 0;
    let mut restarts = case.restarts.clone();
    restarts.sort_by_key(|r| r.at_us);
    let mut ri = 0;
    let mut deliveries = vec![];
    let mut delivered = BTreeSet::new();
    let mut failures = vec![];
    let mut finished = false;
    for _ in 0..1_000_000 {
        medium.advance(medium.now)?;
        while fi < faults.len() && faults[fi].at() <= medium.now {
            faults[fi].apply(&mut medium)?;
            if let Fault::Disconnect { node, .. } = faults[fi] {
                codecs.get_mut(&node).unwrap().reset();
                let a = radios
                    .get_mut(&node)
                    .unwrap()
                    .disconnected(medium.now, "injected disconnect");
                actions(
                    node,
                    a,
                    &mut medium,
                    radios.get_mut(&node).unwrap(),
                    nodes.get_mut(&node).unwrap(),
                )?;
            }
            fi += 1;
        }
        while ri < restarts.len() && restarts[ri].at_us <= medium.now {
            let id = restarts[ri].node;
            let old = nodes.remove(&id).unwrap();
            let vault = old.vault.restart()?;
            nodes.insert(
                id,
                Node::new(configuration(case, id), vault, signing_key(id))?,
            );
            let a = radios
                .get_mut(&id)
                .unwrap()
                .disconnected(medium.now, "node restart");
            actions(
                id,
                a,
                &mut medium,
                radios.get_mut(&id).unwrap(),
                nodes.get_mut(&id).unwrap(),
            )?;
            ri += 1;
        }
        while ti < traffic.len() && traffic[ti].1.at_us <= medium.now {
            let (index, t) = traffic[ti];
            let mut bytes = crate::link::simulation::packet(t.size, index as u32);
            bytes[12..16].copy_from_slice(&nodes[&t.from].config.members[&t.from].address.octets());
            bytes[16..20].copy_from_slice(&nodes[&t.from].config.members[&t.to].address.octets());
            crate::link::simulation::checksum(&mut bytes);
            if let Err(e) = nodes.get_mut(&t.from).unwrap().enqueue(bytes, medium.now) {
                failures.push(format!("packet {} ingress: {}", index, e));
            }
            ti += 1;
        }
        for id in &ids {
            for byte in medium.drain(*id, 4096) {
                if let Some(line) = codecs.get_mut(id).unwrap().push(byte) {
                    let a = radios.get_mut(id).unwrap().on_line(&line?, medium.now);
                    actions(
                        *id,
                        a,
                        &mut medium,
                        radios.get_mut(id).unwrap(),
                        nodes.get_mut(id).unwrap(),
                    )?;
                }
            }
            let node = nodes.get_mut(id).unwrap();
            node.tick(medium.now)?;
            if radios[id].state() == State::Receiving && radios[id].queued() == 0 {
                if let Some(bytes) = node.poll_transmit(medium.now)? {
                    if radios
                        .get_mut(id)
                        .unwrap()
                        .enqueue(0, bytes, medium.now)
                        .is_err()
                    {
                        node.transmitted(medium.now, false);
                    }
                }
            }
            let a = radios.get_mut(id).unwrap().tick(medium.now);
            actions(
                *id,
                a,
                &mut medium,
                radios.get_mut(id).unwrap(),
                nodes.get_mut(id).unwrap(),
            )?;
            while let Some(packet) = nodes.get_mut(id).unwrap().receive() {
                let index = u32::from_be_bytes(packet[28..32].try_into().unwrap()) as usize;
                if index >= case.traffic.len()
                    || case.traffic[index].to != *id
                    || !delivered.insert(index)
                {
                    return Err(invalid("wrong destination or duplicate mesh delivery"));
                }
                let expected =
                    crate::link::simulation::packet(case.traffic[index].size, index as u32);
                if packet[20..] != expected[20..] {
                    return Err(invalid("corrupt mesh payload"));
                }
                deliveries.push(Delivery {
                    packet: index,
                    node: *id,
                    latency_us: medium.now - case.traffic[index].at_us,
                    ttl: packet[8],
                });
            }
        }
        if medium.now >= case.duration_us {
            finished = true;
            break;
        }
        let next = if medium.devices.values().any(|d| !d.output.is_empty()) {
            medium.now
        } else {
            std::iter::once(Some(case.duration_us))
                .chain(nodes.values().map(Node::next_deadline))
                .chain(radios.values().map(Controller::next_deadline))
                .chain([
                    traffic.get(ti).map(|(_, t)| t.at_us),
                    faults.get(fi).map(Fault::at),
                    restarts.get(ri).map(|r| r.at_us),
                ])
                .flatten()
                .filter(|t| *t > medium.now)
                .min()
                .unwrap_or(case.duration_us)
                .min(
                    medium
                        .next_event()
                        .unwrap_or(case.duration_us)
                        .max(medium.now),
                )
        };
        medium.advance(next.min(case.duration_us))?;
    }
    if !finished {
        return Err(invalid("mesh scenario event budget exceeded"));
    }
    if let Some(expected) = case.expected_deliveries {
        if delivered.len() != expected {
            failures.push(format!(
                "expected {} deliveries, got {}",
                expected,
                delivered.len()
            ));
        }
    } else {
        for index in 0..case.traffic.len() {
            if !delivered.contains(&index) {
                failures.push(format!("packet {} not delivered", index));
            }
        }
    }
    let mut latencies: Vec<_> = deliveries.iter().map(|d| d.latency_us).collect();
    latencies.sort_unstable();
    let p = |n: usize| {
        latencies
            .get((latencies.len() * n).div_ceil(100).saturating_sub(1))
            .copied()
            .unwrap_or(0)
    };
    let first = case.traffic.iter().map(|t| t.at_us).min().unwrap_or(0);
    let last = deliveries
        .iter()
        .map(|d| case.traffic[d.packet].at_us + d.latency_us)
        .max()
        .unwrap_or(case.duration_us);
    let application: usize = deliveries
        .iter()
        .map(|d| case.traffic[d.packet].size - 28)
        .sum();
    Ok(Report {
        configuration: case.clone(),
        deliveries,
        failures,
        route_recomputations: nodes.values().map(|n| n.router.recomputations).sum(),
        routes: nodes
            .iter()
            .map(|(id, n)| (*id, n.router.routes.clone()))
            .collect(),
        airtime_us: medium.metrics.values().map(|m| m.airtime_us).sum(),
        collisions: medium.metrics.values().map(|m| m.collisions).sum(),
        control_frames: nodes
            .values()
            .map(|n| n.metrics.control_generated + n.metrics.control_forwarded)
            .sum(),
        goodput_bytes_sec: application as f64 * 1e6 / (last - first).max(1) as f64,
        p50_latency_us: p(50),
        p95_latency_us: p(95),
        link_metrics: nodes
            .iter()
            .map(|(id, n)| {
                (
                    *id,
                    n.links
                        .iter()
                        .map(|(p, l)| (*p, l.metrics.clone()))
                        .collect(),
                )
            })
            .collect(),
        metrics: nodes.into_iter().map(|(id, n)| (id, n.metrics)).collect(),
        trace: medium.trace,
    })
}
