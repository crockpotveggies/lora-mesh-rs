#![cfg(unix)]
use loramesh::{
    link::{
        Config,
        daemon::{Adapter, DaemonConfig},
        simulation::packet,
    },
    sim::{Scenario, pty::PtyLab},
};
use std::{
    fs,
    os::unix::net::UnixDatagram,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Children(Vec<Child>);
impl Drop for Children {
    fn drop(&mut self) {
        for c in &mut self.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}
#[test]
fn production_ipv4_daemons_over_virtual_serial() {
    let mut scenario: Scenario =
        serde_json::from_str(include_str!("../scenarios/virtual-lab.json")).unwrap();
    scenario.duration_us = 90_000_000;
    scenario.max_events = 100_000;
    let lab = PtyLab::start(&scenario).unwrap();
    let dir = lab.paths[&1].parent().unwrap();
    let mut children = Children(vec![]);
    let mut sockets = vec![];
    for id in 1..=2u16 {
        let client = dir.join(format!("client{}", id));
        let socket = UnixDatagram::bind(&client).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        sockets.push(socket);
        let config = DaemonConfig {
            radio: lab.paths[&id].clone(),
            power_dbm: 14,
            link: Config {
                node: id,
                peer: 3 - id,
                profile: scenario.nodes[0].profile.clone(),
                ..Default::default()
            },
            adapter: Adapter::Datagram {
                bind: dir.join(format!("packet{}", id)),
                peer: client,
            },
            metrics: Some(dir.join(format!("metrics{}.json", id))),
        };
        let path = dir.join(format!("config{}.json", id));
        fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        children.0.push(
            Command::new(env!("CARGO_BIN_EXE_loramesh-link"))
                .arg(path)
                .stdout(Stdio::from(
                    fs::File::create(dir.join(format!("stdout{}", id))).unwrap(),
                ))
                .stderr(Stdio::from(
                    fs::File::create(dir.join(format!("stderr{}", id))).unwrap(),
                ))
                .spawn()
                .unwrap(),
        );
    }
    let until = Instant::now() + Duration::from_secs(12);
    loop {
        if (1..=2).all(|id| {
            fs::read_to_string(dir.join(format!("stdout{}", id)))
                .unwrap()
                .contains("ready")
        }) {
            break;
        }
        assert!(Instant::now() < until, "daemon initialization timed out");
        for c in &mut children.0 {
            assert!(c.try_wait().unwrap().is_none());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut buf = [0; 1600];
    for (from, size) in [(0, 1500), (1, 576), (0, 32)] {
        let p = packet(size, size as u32);
        sockets[from]
            .send_to(&p, dir.join(format!("packet{}", from + 1)))
            .unwrap();
        let n = sockets[1 - from].recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], p.as_slice());
    }
    // Shutdown is bounded and flushes observable counters.
    for c in &children.0 {
        unsafe {
            libc::kill(c.id() as i32, libc::SIGTERM);
        }
    }
    let until = Instant::now() + Duration::from_secs(5);
    for c in &mut children.0 {
        loop {
            if let Some(status) = c.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    for id in 1..=2 {
        let m: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join(format!("metrics{}.json", id))).unwrap())
                .unwrap();
        assert_eq!(m["radio_events_dropped"], 0);
        assert_eq!(m["adapter_egress_dropped"], 0);
        assert!(!dir.join(format!("packet{}", id)).exists());
    }
    assert!(lab.error().is_none());
}
