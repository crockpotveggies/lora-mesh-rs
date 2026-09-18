#![cfg(unix)]
use loramesh::{
    radio::{Action, State, runtime::RadioHandle},
    sim::{
        Scenario,
        pty::PtyLab,
        scenario::{Fault, controller_config},
    },
};
use std::time::{Duration, Instant};
fn scenario() -> Scenario {
    let mut s: Scenario =
        serde_json::from_str(include_str!("../scenarios/virtual-lab.json")).unwrap();
    s.duration_us = 10_000_000;
    s
}
#[test]
fn production_serial_worker_crosses_two_ptys_and_cleans_up() {
    let s = scenario();
    let mut lab = PtyLab::start(&s).unwrap();
    let paths = lab.paths.clone();
    let mut receiver =
        RadioHandle::serial(paths[&2].clone(), controller_config(&s.nodes[1].profile)).unwrap();
    receiver.wait_ready(Duration::from_secs(3)).unwrap();
    let mut sender =
        RadioHandle::serial(paths[&1].clone(), controller_config(&s.nodes[0].profile)).unwrap();
    sender.wait_ready(Duration::from_secs(3)).unwrap();
    sender.tx.try_send(vec![0, 1, 2, 254, 255]).unwrap();
    assert_eq!(
        receiver.rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        vec![0, 1, 2, 254, 255]
    );
    let until = Instant::now() + Duration::from_secs(2);
    let mut completed = 0;
    while Instant::now() < until {
        if let Ok(Action::Transmitted(_)) = sender.events.recv_timeout(Duration::from_millis(20)) {
            completed += 1;
            break;
        }
    }
    assert_eq!(completed, 1);
    assert!(
        receiver
            .rx
            .recv_timeout(Duration::from_millis(100))
            .is_err()
    );
    let now = Instant::now();
    sender.shutdown();
    receiver.shutdown();
    assert!(now.elapsed() < Duration::from_secs(1));
    assert!(lab.error().is_none(), "{:?}", lab.error());
    assert_eq!(
        lab.trace().iter().filter(|e| e.event == "tx_start").count(),
        1
    );
    lab.shutdown();
    assert!(paths.values().all(|p| !p.exists()));
}
#[test]
fn pty_replacement_recovers_via_stable_device_path() {
    let mut s = scenario();
    s.faults = vec![
        Fault::Disconnect {
            at_us: 1_500_000,
            node: 1,
        },
        Fault::Reconnect {
            at_us: 1_800_000,
            node: 1,
        },
    ];
    let lab = PtyLab::start(&s).unwrap();
    let config = controller_config(&s.nodes[0].profile);
    let mut radio = RadioHandle::serial(lab.paths[&1].clone(), config).unwrap();
    radio.wait_ready(Duration::from_secs(2)).unwrap();
    let until = Instant::now() + Duration::from_secs(4);
    let mut recovering = false;
    let mut recovered = false;
    while Instant::now() < until {
        match radio.events.recv_timeout(Duration::from_millis(50)) {
            Ok(Action::State(State::Recovering)) => recovering = true,
            Ok(Action::State(State::Receiving)) if recovering => {
                recovered = true;
                break;
            }
            _ => {}
        }
    }
    assert!(recovered, "{:?}; {:?}", lab.error(), lab.trace());
    radio.tx.try_send(vec![42]).unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    let mut transmitted = false;
    while Instant::now() < until {
        if let Ok(Action::Transmitted(_)) = radio.events.recv_timeout(Duration::from_millis(50)) {
            transmitted = true;
            break;
        }
    }
    assert!(transmitted);
    radio.shutdown();
    assert!(lab.error().is_none());
}
