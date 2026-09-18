use loramesh::radio::protocol::RadioProfile;
use loramesh::sim::{
    Scenario,
    device::{Device, Firmware},
    medium::{Link, Medium},
    run,
    scenario::Fault,
};
fn scenario() -> Scenario {
    serde_json::from_str(include_str!("../scenarios/two-node.json")).unwrap()
}
#[test]
fn all_scenarios_are_reproducible_and_meet_expectations() {
    for path in std::fs::read_dir("scenarios").unwrap() {
        let path = path.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let s: Scenario = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let first = run(&s).unwrap();
        let second = run(&s).unwrap();
        assert_eq!(first, second, "{}", s.name);
        assert!(
            first.failures.is_empty(),
            "{}: {:?}",
            s.name,
            first.failures
        );
    }
}
#[test]
fn seeded_loss_duplicate_and_delay_are_explicit_in_trace() {
    let mut s = scenario();
    s.links[0].duplicate_per_mille = 1000;
    s.links[0].delay_us = 10000;
    s.links[0].jitter_us = 1000;
    s.expect.received.insert(2, 2);
    let r = run(&s).unwrap();
    assert!(r.failures.is_empty(), "{:?}", r.failures);
    assert!(r.trace.iter().any(|e| e.event == "injected_duplicate"));
    assert_eq!(r, run(&s).unwrap());
    s.links[0].loss_per_mille = 1000;
    s.expect.received.insert(2, 0);
    let r = run(&s).unwrap();
    assert!(r.failures.is_empty());
    assert!(r.trace.iter().any(|e| e.detail == "injected loss"));
}
#[test]
fn link_outage_midflight_drops_delivery() {
    let mut s = scenario();
    s.faults.push(Fault::Link {
        at_us: 1_010_000,
        from: 1,
        to: 2,
        enabled: false,
    });
    s.expect.received.insert(2, 0);
    let r = run(&s).unwrap();
    assert!(r.failures.is_empty());
    assert!(r.trace.iter().any(|e| e.detail == "link outage"));
}
fn medium() -> Medium {
    let mut m = Medium::new(1, 10000);
    for id in 1..=2 {
        m.devices.insert(
            id,
            Device::new(
                Firmware::RN2903,
                RadioProfile {
                    sf: 7,
                    ..RadioProfile::default()
                },
            ),
        );
        m.command(id, "mac pause").unwrap();
    }
    m.links = vec![
        Link {
            from: 1,
            to: 2,
            ..Link::default()
        },
        Link {
            from: 2,
            to: 1,
            ..Link::default()
        },
    ];
    m.advance(0).unwrap();
    m.drain(1, 16384);
    m.drain(2, 16384);
    m
}
#[test]
fn receiver_must_listen_for_entire_transmission() {
    let mut m = medium();
    m.command(1, "radio tx 010203").unwrap();
    m.advance(1000).unwrap();
    m.command(2, "radio rx 0").unwrap();
    m.advance(1_000_000).unwrap();
    assert!(!m.trace.iter().any(|e| e.event == "delivery"));
    let mut m = medium();
    m.command(2, "radio rx 0").unwrap();
    m.command(1, "radio tx 010203").unwrap();
    m.advance(1000).unwrap();
    m.command(2, "radio rxstop").unwrap();
    m.command(2, "radio tx 04").unwrap();
    m.advance(1_000_000).unwrap();
    assert!(!m.trace.iter().any(|e| e.event == "delivery"));
}
#[test]
fn profile_mismatch_and_tx_watchdog() {
    let mut m = medium();
    m.devices.get_mut(&2).unwrap().profile.sf = 9;
    m.command(2, "radio rx 0").unwrap();
    m.command(1, "radio tx 00").unwrap();
    m.advance(1_000_000).unwrap();
    assert!(!m.trace.iter().any(|e| e.event == "delivery"));
    let mut m = medium();
    m.command(2, "radio rx 0").unwrap();
    m.command(1, "radio set wdt 1").unwrap();
    m.command(1, "radio tx 00").unwrap();
    m.advance(1_000_000).unwrap();
    assert!(
        m.trace
            .iter()
            .any(|e| e.event == "serial" && e.detail == "radio_err")
    );
    assert!(!m.trace.iter().any(|e| e.event == "delivery"));
}
#[test]
fn event_budget_and_invalid_scenarios_fail_cleanly() {
    let mut s = scenario();
    s.max_events = 10;
    assert!(run(&s).is_err());
    let mut s = scenario();
    s.nodes.push(s.nodes[0].clone());
    assert!(run(&s).is_err());
    let mut s = scenario();
    s.links[0].loss_per_mille = 1001;
    assert!(run(&s).is_err());
    let mut s = scenario();
    s.traffic[0].payload = "a".into();
    assert!(run(&s).is_err());
}
#[test]
fn reports_airtime_latency_and_bounded_output() {
    let s = scenario();
    let r = run(&s).unwrap();
    assert_eq!(r.medium[&1].transmitted_frames, 1);
    assert_eq!(r.medium[&1].airtime_us, 30_976);
    assert_eq!(r.medium[&2].delivery_latency_us, vec![30_976]);
    assert_eq!(r.medium[&2].delivered_bytes, 5);
    assert!(r.medium[&2].serial_output_peak < 16384);
}
#[test]
fn stale_delayed_reply_is_not_delivered_after_reconnect() {
    let mut m = medium();
    m.devices.get_mut(&1).unwrap().reply_delay_us = 1000;
    m.command(1, "sys get ver").unwrap();
    m.connection(1, false).unwrap();
    m.connection(1, true).unwrap();
    m.advance(1000).unwrap();
    assert!(m.drain(1, 16384).is_empty());
}
#[test]
fn missing_initialization_response_recovers_without_spinning() {
    let mut s = scenario();
    s.duration_us = 6_000_000;
    s.traffic[0].at_us = 5_500_000;
    s.faults.push(Fault::DropReplies {
        at_us: 0,
        node: 1,
        count: 1,
    });
    let r = run(&s).unwrap();
    assert!(r.failures.is_empty(), "{:?}", r.failures);
    assert!(
        r.trace
            .iter()
            .any(|e| e.event == "state" && e.detail == "Recovering")
    );
    assert!(r.trace.len() < 1000);
}
