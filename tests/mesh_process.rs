#![cfg(unix)]
use loramesh::{
    link::simulation::{checksum, packet},
    mesh::daemon::DaemonConfig,
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
fn ready(children: &mut Children, paths: &[std::path::PathBuf]) {
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        if paths
            .iter()
            .all(|p| fs::read_to_string(p).unwrap().contains("ready"))
        {
            return;
        }
        assert!(Instant::now() < until, "mesh initialization timeout");
        for (i, c) in children.0.iter_mut().enumerate() {
            assert!(
                c.try_wait().unwrap().is_none(),
                "{}",
                fs::read_to_string(paths[i].with_file_name("stderr")).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
#[test]
fn three_secure_processes_forward_over_actual_ptys() {
    let case: loramesh::mesh::simulation::Case =
        serde_json::from_str(include_str!("../scenarios/mesh/line.json")).unwrap();
    let scenario = Scenario {
        version: 1,
        name: "secure-pty".into(),
        seed: 42,
        duration_us: 180_000_000,
        max_events: 100_000,
        nodes: case.nodes,
        links: case.links,
        traffic: vec![],
        faults: vec![],
        expect: Default::default(),
    };
    let lab = PtyLab::start(&scenario).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("keys");
    assert!(
        Command::new(env!("CARGO_BIN_EXE_loramesh-keygen"))
            .args([
                root.to_str().unwrap(),
                "1=10.107.0.1",
                "2=10.107.0.2",
                "3=10.107.0.3",
                "--links",
                "1-2,2-3"
            ])
            .output()
            .unwrap()
            .status
            .success()
    );
    let mut children = Children(vec![]);
    let mut sockets = vec![];
    let mut outputs = vec![];
    for id in 1..=3u16 {
        let dir = root.join(id.to_string());
        let path = dir.join("config.json");
        let mut cfg: DaemonConfig = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        cfg.radio = lab.paths[&id].clone();
        cfg.mesh.link.profile.sf = 7;
        cfg.mesh.static_routes = match id {
            1 => [(2, 2), (3, 2)].into(),
            2 => [(1, 1), (3, 3)].into(),
            _ => [(1, 2), (2, 2)].into(),
        };
        fs::write(&path, serde_json::to_vec(&cfg).unwrap()).unwrap();
        let sock = UnixDatagram::bind(dir.join("client.sock")).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(80)))
            .unwrap();
        sockets.push(sock);
        let out = dir.join("stdout");
        outputs.push(out.clone());
        children.0.push(
            Command::new(env!("CARGO_BIN_EXE_loramesh-mesh"))
                .arg(path)
                .stdout(Stdio::from(fs::File::create(out).unwrap()))
                .stderr(Stdio::from(fs::File::create(dir.join("stderr")).unwrap()))
                .spawn()
                .unwrap(),
        );
    }
    ready(&mut children, &outputs);
    let mut buf = [0; 1600];
    for (from, to) in [(1, 3), (3, 1)] {
        let mut bytes = packet(256, from);
        bytes[15] = from as u8;
        bytes[19] = to as u8;
        checksum(&mut bytes);
        sockets[from as usize - 1]
            .send_to(&bytes, root.join(from.to_string()).join("packets.sock"))
            .unwrap();
        let n = sockets[to as usize - 1].recv(&mut buf).unwrap();
        bytes[8] -= 1;
        checksum(&mut bytes);
        assert_eq!(&buf[..n], bytes.as_slice());
    }
    for child in &children.0 {
        unsafe {
            libc::kill(child.id() as i32, libc::SIGTERM);
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
    let middle: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("2/metrics.json")).unwrap()).unwrap();
    assert_eq!(middle["mesh"]["forwarded"], 2);
    assert!(lab.error().is_none());
}
