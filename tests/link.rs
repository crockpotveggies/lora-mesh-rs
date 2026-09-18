use loramesh::link::{
    Config, Endpoint,
    simulation::{self, Case, packet},
    wire::{self, HEADER, Header},
};
fn config() -> Config {
    Config {
        profile: loramesh::radio::RadioProfile {
            sf: 7,
            ..Default::default()
        },
        ..Config::default()
    }
}
fn receiver() -> Endpoint {
    Endpoint::new(Config {
        node: 2,
        peer: 1,
        session: 2,
        ..config()
    })
    .unwrap()
}
fn frames(data: &[u8], session: u64, sequence: u32, span: usize) -> Vec<Vec<u8>> {
    data.chunks(span)
        .enumerate()
        .map(|(i, p)| {
            Header {
                network: 1,
                source: 1,
                destination: 2,
                session,
                sequence,
                total: data.len() as u16,
                index: i as u8,
                count: data.len().div_ceil(span) as u8,
                span: span as u8,
                ack: false,
                request_ack: i + 1 == data.len().div_ceil(span),
            }
            .encode(p)
            .unwrap()
        })
        .collect()
}
#[test]
fn independent_wire_fixture_and_rejection() {
    let fixture=hex::decode("4c4d010200000001000100020000000000000009000000070020000101d1000000000000000000000000000000004500002000000000401169f50a6b00010a6b000204d2162e000c000000000007").unwrap();
    // Header independently specified; payload checksum checked in the IP parser tests.
    let (h, p) = wire::decode(&fixture).unwrap();
    assert_eq!((h.session, h.sequence, h.total, h.count), (9, 7, 32, 1));
    assert_eq!(p.len(), 32);
    assert_eq!(h.encode(p).unwrap(), fixture);
    for n in 0..fixture.len() {
        assert!(wire::decode(&fixture[..n]).is_err());
    }
    for (offset, value) in [
        (0, 0),
        (2, 2),
        (3, 4),
        (28, 0),
        (29, 1),
        (30, 1),
        (26, 1),
        (27, 0),
    ] {
        let mut bad = fixture.clone();
        bad[offset] = value;
        assert!(wire::decode(&bad).is_err());
    }
}
#[test]
fn reordering_duplicates_and_conflicts() {
    let p = packet(1500, 4);
    let fs = frames(&p, 1, 0, 209);
    let mut r = receiver();
    for f in fs.iter().rev() {
        r.on_frame(f, 0);
        r.on_frame(f, 0);
    }
    assert_eq!(r.receive_packet(), Some(p));
    assert!(r.receive_packet().is_none());
    for f in &fs {
        r.on_frame(f, 1);
    }
    assert!(r.receive_packet().is_none());
    assert_eq!(r.reassembly_bytes(), 0);
    assert!(r.metrics.duplicates >= 8);
    let mut r = receiver();
    r.on_frame(&fs[0], 0);
    let mut bad = fs[0].clone();
    bad[HEADER + 30] ^= 1;
    r.on_frame(&bad, 0);
    assert_eq!(r.metrics.malformed, 1);
}
#[test]
fn size_matrix_roundtrips() {
    for size in [32, 48, 49, 128, 209, 210, 576, 1499, 1500] {
        for span in [48, 64, 128, 209] {
            let mut r = receiver();
            let p = packet(size, size as u32);
            for f in frames(&p, 1, 0, span) {
                assert!(f.len() <= 255);
                r.on_frame(&f, 0);
            }
            assert_eq!(r.receive_packet(), Some(p));
        }
    }
}
#[test]
fn malformed_input_never_allocates_unboundedly() {
    let mut r = receiver();
    let mut rng = 9u64;
    for n in 0..20000 {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let mut v = vec![rng as u8; n % 300];
        if n % 2 == 0 && v.len() > 3 {
            v[..3].copy_from_slice(b"LM\x01");
        }
        r.on_frame(&v, n as u64);
        assert!(r.reassembly_bytes() <= 12000);
    }
    assert!(r.metrics.malformed > 0);
}
#[test]
fn queue_reassembly_egress_and_session_bounds() {
    let mut e = Endpoint::new(Config {
        queue_packets: 1,
        ..config()
    })
    .unwrap();
    e.enqueue(packet(1500, 1), 0).unwrap();
    assert!(e.enqueue(packet(32, 2), 0).is_err());
    e.tick(120_000_000);
    assert_eq!(e.queued(), 0);
    assert_eq!(e.metrics.expired, 1);
    let mut r = Endpoint::new(Config {
        node: 2,
        peer: 1,
        reassembly_packets: 1,
        reassembly_bytes: 1500,
        ..config()
    })
    .unwrap();
    let a = frames(&packet(1500, 1), 1, 0, 209);
    r.on_frame(&a[0], 0);
    r.on_frame(&frames(&packet(1500, 2), 1, 1, 209)[0], 0);
    assert_eq!(r.metrics.reassembly_dropped, 1);
    r.tick(120_000_000);
    assert_eq!(r.reassembly_bytes(), 0);
    assert_eq!(r.metrics.reassembly_expired, 1);
    let mut r = receiver();
    for session in 1..=9 {
        for f in frames(&packet(32, 1), session, 0, 209) {
            r.on_frame(&f, 0);
        }
        r.receive_packet();
    }
    assert_eq!(r.metrics.delivered, 8);
    assert_eq!(r.metrics.replay_rejected, 1);
}
#[test]
fn restart_sequence_window_and_old_replay() {
    let mut r = receiver();
    for sequence in [0, 65, 1, 64] {
        for f in frames(&packet(32, sequence), 1, sequence, 209) {
            r.on_frame(&f, 0);
        }
    }
    assert_eq!(r.metrics.delivered, 3);
    for f in frames(&packet(32, 1), 2, 0, 209) {
        r.on_frame(&f, 0);
    }
    assert_eq!(r.metrics.delivered, 4);
}
#[test]
fn ipv4_df_options_fragments_and_oversize() {
    let mut p = packet(1500, 1);
    for flags in [0u16, 0x4000, 0x2000, 0x0001] {
        p[6..8].copy_from_slice(&flags.to_be_bytes());
        simulation::checksum(&mut p);
        wire::ipv4(&p).unwrap();
        let mut r = receiver();
        for f in frames(&p, 1, 0, 209) {
            r.on_frame(&f, 0);
        }
        assert_eq!(r.receive_packet(), Some(p.clone()));
    }
    p[0] = 0x46;
    simulation::checksum(&mut p);
    wire::ipv4(&p).unwrap();
    p.push(0);
    assert!(wire::ipv4(&p).is_err());
    p.pop();
    p[10] ^= 1;
    assert!(wire::ipv4(&p).is_err());
}
#[test]
fn foreign_and_inconsistent_frames_rejected() {
    let mut r = receiver();
    let f = frames(&packet(1500, 1), 1, 0, 209);
    let mut bad = f[0].clone();
    bad[7] = 9;
    r.on_frame(&bad, 0);
    assert_eq!(r.metrics.foreign, 1);
    assert_eq!(r.reassembly_bytes(), 0);
    r.on_frame(&f[0], 0);
    r.on_frame(&frames(&packet(1400, 1), 1, 0, 209)[1], 0);
    assert_eq!(r.metrics.malformed, 1);
}
#[test]
fn simulated_delivery_loss_duplicates_delay_and_duplex() {
    for case in [
        Case {
            packets: 8,
            ..Default::default()
        },
        Case {
            packets: 8,
            loss_per_mille: 150,
            duplicate_per_mille: 200,
            ..Default::default()
        },
        Case {
            packets: 4,
            delay_us: 50000,
            jitter_us: 10000,
            ..Default::default()
        },
        Case {
            packets: 4,
            bidirectional: true,
            ..Default::default()
        },
    ] {
        let r = simulation::run(&case).unwrap();
        assert_eq!(
            r.delivered,
            r.offered,
            "{}",
            serde_json::to_string(&r).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            serde_json::to_value(simulation::run(&case).unwrap()).unwrap()
        );
    }
}
#[test]
fn blackhole_bounded_retries_and_expiry() {
    let r = simulation::run(&Case {
        packets: 2,
        loss_per_mille: 1000,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(r.delivered, 0);
    assert!(r.metrics[0].retries_exhausted + r.metrics[0].expired > 0);
    assert!(r.metrics[0].data_frames <= 2 * 8 * 6);
}
#[test]
fn duty_airtime_and_failed_radio() {
    let c = Config {
        duty_per_mille: 10,
        ..config()
    };
    let mut e = Endpoint::new(c.clone()).unwrap();
    e.enqueue(packet(1500, 1), 0).unwrap();
    let f = e.poll_transmit(0).unwrap();
    assert!(e.poll_transmit(1).is_none());
    let air = c.profile.airtime_us(f.len()).unwrap();
    e.transmitted(air, false);
    assert!(e.poll_transmit(air * 99).is_none());
    assert_eq!(e.metrics.radio_failures, 1);
}
#[test]
fn batching_performance_regression() {
    let a = simulation::run(&Case {
        span: 64,
        batch: 1,
        packets: 4,
        ..Default::default()
    })
    .unwrap();
    let b = simulation::run(&Case {
        span: 209,
        batch: 4,
        packets: 4,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(a.delivered, 4);
    assert_eq!(b.delivered, 4);
    assert!(b.application_goodput_bytes_sec > a.application_goodput_bytes_sec * 2.0);
    assert!(b.metrics[1].ack_frames < a.metrics[1].ack_frames);
}

#[test]
fn selective_retry_and_lost_final_ack() {
    let c = config();
    let mut s = Endpoint::new(c.clone()).unwrap();
    let mut r = receiver();
    s.enqueue(packet(600, 1), 0).unwrap();
    let mut now = 0;
    for index in 0..3 {
        let f = s.poll_transmit(now).unwrap();
        assert_eq!(wire::decode(&f).unwrap().0.index, index);
        let end = now + c.profile.airtime_us(f.len()).unwrap();
        s.transmitted(end, true);
        if index != 1 {
            r.on_frame(&f, end);
        }
        now = end + c.turnaround_us;
    }
    let ack = r.poll_transmit(now).unwrap();
    assert!(wire::decode(&ack).unwrap().0.ack);
    now += c.profile.airtime_us(ack.len()).unwrap();
    r.transmitted(now, true);
    s.on_frame(&ack, now);
    now += 10_000_000;
    s.tick(now);
    now += 10_000_000;
    let f = s.poll_transmit(now).unwrap();
    assert_eq!(wire::decode(&f).unwrap().0.index, 1);
    s.transmitted(now + 1000, true);
    r.on_frame(&f, now + 1000);
    assert_eq!(r.receive_packet(), Some(packet(600, 1)));
    // Drop final ACK, then ensure retry cannot deliver the IP packet twice.
    let ack = r.poll_transmit(now + 100_000).unwrap();
    r.transmitted(now + 200_000, true);
    assert!(wire::decode(&ack).unwrap().0.ack);
    now += 10_000_000;
    s.tick(now);
    now += 10_000_000;
    let f = s.poll_transmit(now).unwrap();
    assert_eq!(wire::decode(&f).unwrap().0.index, 1);
    s.transmitted(now + 1000, true);
    r.on_frame(&f, now + 1000);
    assert!(r.receive_packet().is_none());
    let ack = r.poll_transmit(now + 100_000).unwrap();
    s.on_frame(&ack, now + 200_000);
    assert_eq!(s.queued(), 0);
    assert_eq!(s.metrics.acknowledged, 1);
}
#[test]
fn egress_pressure_retains_complete_reassembly_for_retry() {
    let mut r = Endpoint::new(Config {
        node: 2,
        peer: 1,
        queue_packets: 1,
        ..config()
    })
    .unwrap();
    let one = frames(&packet(32, 1), 1, 0, 209);
    let two = frames(&packet(32, 2), 1, 1, 209);
    r.on_frame(&one[0], 0);
    r.on_frame(&two[0], 0);
    assert_eq!(r.reassembly_bytes(), 32);
    assert_eq!(r.metrics.egress_dropped, 1);
    assert_eq!(r.receive_packet(), Some(packet(32, 1)));
    r.on_frame(&two[0], 1);
    assert_eq!(r.receive_packet(), Some(packet(32, 2)));
    assert_eq!(r.reassembly_bytes(), 0);
}
#[test]
fn control_priority_and_invalid_ack() {
    let mut r = receiver();
    r.enqueue(packet(1500, 8), 0).unwrap();
    r.on_frame(&frames(&packet(32, 1), 1, 0, 209)[0], 0);
    let ack = r.poll_transmit(40_000).unwrap();
    assert!(wire::decode(&ack).unwrap().0.ack);
    let mut s = Endpoint::new(config()).unwrap();
    s.enqueue(packet(1500, 1), 0).unwrap();
    let f = s.poll_transmit(0).unwrap();
    let (mut h, _) = wire::decode(&f).unwrap();
    h.source = 2;
    h.destination = 1;
    h.ack = true;
    h.request_ack = false;
    h.index = 0;
    s.on_frame(&h.encode(&255u32.to_be_bytes()).unwrap(), 1);
    assert_eq!(s.metrics.malformed, 1);
    assert_eq!(s.queued(), 1);
    assert!(h.encode(&256u32.to_be_bytes()).is_err());
}
#[test]
fn invalid_configs_packets_and_corruption() {
    for c in [
        Config {
            batch: 0,
            ..config()
        },
        Config {
            span: 210,
            ..config()
        },
        Config {
            node: 2,
            peer: 2,
            ..config()
        },
        Config {
            session: 0,
            ..config()
        },
        Config {
            duty_per_mille: 0,
            ..config()
        },
        Config {
            queue_bytes: 1,
            ..config()
        },
        Config {
            lifetime_us: 0,
            ..config()
        },
        Config {
            max_retries: 17,
            ..config()
        },
    ] {
        assert!(Endpoint::new(c).is_err());
    }
    let mut s = Endpoint::new(config()).unwrap();
    assert!(s.enqueue(vec![0; 20], 0).is_err());
    let mut p = packet(32, 0);
    p[0] = 0x44;
    assert!(wire::ipv4(&p).is_err());
    p[0] = 0x46;
    p[2] = 1;
    assert!(wire::ipv4(&p).is_err());
    let mut p = packet(32, 0);
    p[10] ^= 1;
    let mut r = receiver();
    r.on_frame(&frames(&p, 1, 0, 209)[0], 0);
    assert!(r.receive_packet().is_none());
    assert_eq!(r.metrics.malformed, 1);
    assert!(
        simulation::run(&Case {
            packets: 0,
            ..Default::default()
        })
        .is_err()
    );
}
#[test]
fn burst_outage_recovers_and_multiple_seeds_deliver() {
    let r = simulation::run(&Case {
        packets: 6,
        outage_us: Some((1_000_000, 4_000_000)),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(r.delivered, 6);
    assert!(r.metrics[0].retransmissions > 0);
    let mut delivered = 0;
    for seed in 1..=20 {
        let r = simulation::run(&Case {
            seed,
            packets: 4,
            loss_per_mille: 100,
            ..Default::default()
        })
        .unwrap();
        delivered += r.delivered;
        assert!(r.metrics[0].retransmissions < 200);
    }
    assert!(delivered >= 78, "only {}/80 delivered", delivered);
}
#[test]
fn duplex_seed_sweep_bounds_contention() {
    let mut delivered = 0;
    for seed in 1..=12 {
        let r = simulation::run(&Case {
            seed,
            packets: 3,
            bidirectional: true,
            ..Default::default()
        })
        .unwrap();
        delivered += r.delivered;
        assert!(r.metrics.iter().all(|m| m.data_frames < 250));
    }
    assert!(
        delivered >= 68,
        "only {}/72 delivered under simultaneous contention",
        delivered
    );
}
#[test]
fn event_clock_matches_fixed_clock_and_retry_baseline_replays() {
    let base = Case {
        packets: 4,
        ..Default::default()
    };
    let mut event = serde_json::to_value(simulation::run(&base).unwrap()).unwrap();
    let mut fixed = serde_json::to_value(
        simulation::run(&Case {
            clock_step_us: Some(10000),
            ..base.clone()
        })
        .unwrap(),
    )
    .unwrap();
    event.as_object_mut().unwrap().remove("configuration");
    fixed.as_object_mut().unwrap().remove("configuration");
    assert_eq!(event, fixed);
    let r = simulation::run(&Case {
        fast_loss_retries: false,
        loss_per_mille: 100,
        ..base
    })
    .unwrap();
    assert_eq!(r.delivered, 4);
    assert!(
        simulation::run(&Case {
            clock_step_us: Some(0),
            ..Default::default()
        })
        .is_err()
    );
}
#[test]
fn independent_ipv4_checksum_vector() {
    let mut p = hex::decode("45000073000040004011b861c0a80001c0a800c7").unwrap();
    p.resize(115, 0);
    wire::ipv4(&p).unwrap();
    p[8] -= 1;
    assert!(wire::ipv4(&p).is_err());
}
