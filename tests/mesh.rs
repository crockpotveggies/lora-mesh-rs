use ed25519_dalek::Signer;
use loramesh::{
    link::{self, wire::Header},
    mesh::{
        Config, Member, Node,
        security::{SPAN, Vault},
        simulation::{self, Case, pair_key, signing_key},
        wire::Packet,
    },
};
use std::collections::BTreeMap;
fn node(id: u16) -> Node {
    let members = (1..=3)
        .map(|n| {
            (
                n,
                Member {
                    address: format!("10.107.0.{}", n).parse().unwrap(),
                    verifying_key: hex::encode(signing_key(n).verifying_key().to_bytes()),
                },
            )
        })
        .collect();
    let peers: Vec<_> = (1..=3).filter(|n| *n != id).collect();
    let cfg = Config {
        members,
        peers: peers.clone(),
        link: link::Config {
            node: id,
            profile: loramesh::radio::RadioProfile {
                sf: 7,
                ..Default::default()
            },
            ..Default::default()
        },
        static_routes: peers.iter().map(|p| (*p, *p)).collect(),
        ..Default::default()
    };
    Node::new(
        cfg,
        Vault::memory(
            1,
            id,
            1,
            peers.into_iter().map(|p| (p, pair_key(id, p))).collect(),
        )
        .unwrap(),
        signing_key(id),
    )
    .unwrap()
}
fn ip(from: u16, to: u16) -> Vec<u8> {
    let mut p = link::simulation::packet(32, 7);
    p[15] = from as u8;
    p[19] = to as u8;
    link::simulation::checksum(&mut p);
    p
}
fn data(from: u16, to: u16, hops: u8) -> Packet {
    Packet {
        origin: from,
        destination: to,
        epoch: 1,
        sequence: 0,
        hops,
        announcement: false,
        payload: ip(from, to),
    }
}
fn inject(n: &mut Node, v: &mut Vault, from: u16, sequence: u32, p: &Packet) {
    let bytes = p.encode().unwrap();
    for (index, part) in bytes.chunks(SPAN).enumerate() {
        let h = Header {
            network: 1,
            source: from,
            destination: n.config.link.node,
            session: v.epoch(),
            sequence,
            total: bytes.len() as u16,
            index: index as u8,
            count: bytes.len().div_ceil(SPAN) as u8,
            span: SPAN as u8,
            ack: false,
            request_ack: index + 1 == bytes.len().div_ceil(SPAN),
        };
        n.on_radio(&v.seal(&h.encode(part).unwrap()).unwrap(), 1000)
            .unwrap();
    }
}
fn announcement(origin: u16, signer: u16, hops: u8) -> Packet {
    let mut p = Packet {
        origin,
        destination: 0,
        epoch: 1,
        sequence: 0,
        hops,
        announcement: true,
        payload: vec![0],
    };
    let signature = signing_key(signer).sign(&p.signing_bytes(1));
    p.payload.extend_from_slice(&signature.to_bytes());
    p
}
#[test]
fn origin_does_not_become_immediate_neighbor() {
    let mut n = node(2);
    let mut v = Vault::memory(1, 1, 1, BTreeMap::from([(2, pair_key(1, 2))])).unwrap();
    inject(&mut n, &mut v, 1, 0, &announcement(3, 3, 8));
    assert!(n.router.topology[&2].links.contains_key(&1));
    assert!(!n.router.topology[&2].links.contains_key(&3));
    assert!(n.router.topology.contains_key(&3));
    assert_eq!(n.metrics.control_forwarded, 1);
    inject(&mut n, &mut v, 1, 1, &announcement(3, 3, 8));
    assert_eq!(n.metrics.control_forwarded, 1);
    assert_eq!(n.metrics.duplicate, 1);
}
#[test]
fn signatures_hop_budget_and_duplicate_application_delivery() {
    let mut n = node(2);
    let mut v = Vault::memory(1, 1, 1, BTreeMap::from([(2, pair_key(1, 2))])).unwrap();
    inject(&mut n, &mut v, 1, 0, &announcement(3, 1, 8));
    assert_eq!(n.metrics.signature_rejected, 1);
    inject(&mut n, &mut v, 1, 1, &data(1, 3, 1));
    assert_eq!(n.metrics.hop_drops, 1);
    inject(&mut n, &mut v, 1, 2, &data(1, 2, 8));
    assert_eq!(n.receive(), Some(ip(1, 2)));
    inject(&mut n, &mut v, 1, 3, &data(1, 2, 8));
    assert!(n.receive().is_none());
    assert_eq!(n.metrics.duplicate, 1);
}
#[test]
fn unknown_ip_bounds_and_deadlines() {
    let mut n = node(1);
    let mut p = ip(1, 2);
    p[19] = 9;
    link::simulation::checksum(&mut p);
    assert!(n.enqueue(p, 0).is_err());
    assert!(n.enqueue(ip(2, 3), 0).is_err());
    assert!(n.enqueue(ip(1, 1), 0).is_err());
    for _ in 0..32 {
        n.enqueue(ip(1, 3), 0).unwrap();
    }
    assert!(n.enqueue(ip(1, 3), 0).is_err());
    n.tick(120_000_000).unwrap();
    assert_eq!(n.metrics.no_route_expired, 32);
    for size in 0..280 {
        n.on_radio(&vec![0; size], 0).unwrap();
    }
    assert_eq!(n.metrics.auth_rejected, 280);
}
#[test]
fn malformed_mesh_packets_and_origin_ip_binding() {
    let mut n = node(2);
    let mut v = Vault::memory(1, 1, 1, BTreeMap::from([(2, pair_key(1, 2))])).unwrap();
    let mut p = data(1, 2, 8);
    p.payload[15] = 3;
    link::simulation::checksum(&mut p.payload);
    inject(&mut n, &mut v, 1, 0, &p);
    assert_eq!(n.metrics.malformed, 1);
    let bytes = data(1, 2, 8).encode().unwrap();
    for size in 0..bytes.len() {
        assert!(Packet::decode(&bytes[..size]).is_err());
    }
    for offset in [0, 2, 3, 20, 21, 22] {
        let mut bad = bytes.clone();
        bad[offset] = 255;
        assert!(Packet::decode(&bad).is_err());
    }
}
#[test]
fn configured_mesh_scenarios_deliver_and_replay() {
    for name in [
        "line",
        "triangle",
        "diamond",
        "hidden-terminal",
        "disconnected",
        "restart",
        "diamond-repair",
    ] {
        let case: Case = serde_json::from_slice(
            &std::fs::read(format!("scenarios/mesh/{}.json", name)).unwrap(),
        )
        .unwrap();
        let report = simulation::run(&case).unwrap();
        assert!(
            report.failures.is_empty(),
            "{}: {:?}",
            name,
            report.failures
        );
        if name == "line" {
            assert_eq!(report.deliveries[0].ttl, 63);
        }
        if name == "diamond-repair" {
            assert_eq!(report.routes[&1][&4].next, 3);
        }
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::to_value(simulation::run(&case).unwrap()).unwrap()
        );
    }
}
#[test]
fn cached_routes_preserve_radio_behavior() {
    let mut case: Case = serde_json::from_str(include_str!("../scenarios/mesh/line.json")).unwrap();
    let cached = simulation::run(&case).unwrap();
    case.cache_routes = false;
    let recomputed = simulation::run(&case).unwrap();
    assert_eq!(cached.deliveries, recomputed.deliveries);
    assert_eq!(cached.airtime_us, recomputed.airtime_us);
    assert!(recomputed.route_recomputations > cached.route_recomputations * 10);
}
#[test]
fn many_authenticated_restarts_keep_receiver_memory_bounded() {
    let mut n = node(2);
    let mut v = Vault::memory(1, 1, 1, BTreeMap::from([(2, pair_key(1, 2))])).unwrap();
    for epoch in 1..=32 {
        let mut p = data(1, 2, 8);
        p.epoch = epoch;
        inject(&mut n, &mut v, 1, 0, &p);
        assert_eq!(n.receive(), Some(ip(1, 2)));
        v = v.restart().unwrap();
    }
    assert_eq!(n.metrics.delivered, 32);
}

#[test]
fn failed_next_hop_retries_on_alternate_before_passive_expiry() {
    let mut case: Case =
        serde_json::from_str(include_str!("../scenarios/mesh/diamond-repair.json")).unwrap();
    case.traffic[1].at_us = 100_000_000;
    let report = simulation::run(&case).unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(report.deliveries.len(), 2);
    assert!(report.deliveries[1].latency_us < 60_000_000);
    assert_eq!(report.metrics[&1].failovers, 1);
    assert_eq!(report.metrics[&4].delivered, 2);
}
