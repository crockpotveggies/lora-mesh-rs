use std::{fs, process::Command};
#[test]
fn cli_replay_and_failure_artifacts() {
    let temp = std::env::temp_dir().join(format!("loramesh-cli-test-{}", std::process::id()));
    fs::create_dir_all(&temp).unwrap();
    let report = temp.join("report.json");
    let replay = temp.join("replay.json");
    let bin = env!("CARGO_BIN_EXE_loramesh-sim");
    assert!(
        Command::new(bin)
            .args(["scenarios/two-node.json", "--output"])
            .arg(&report)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new(bin)
            .arg(&report)
            .args(["--replay", "--output"])
            .arg(&replay)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(fs::read(&report).unwrap(), fs::read(&replay).unwrap());
    let mut scenario: serde_json::Value =
        serde_json::from_str(include_str!("../scenarios/two-node.json")).unwrap();
    scenario["max_events"] = 10.into();
    let broken = temp.join("broken.json");
    fs::write(&broken, scenario.to_string()).unwrap();
    let output = Command::new(bin)
        .arg(&broken)
        .arg("--output")
        .arg(&report)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&fs::read(&report).unwrap()).unwrap();
    assert!(error["error"].as_str().unwrap().contains("budget"));
    assert_eq!(error["configuration"]["seed"], 42);
    fs::remove_dir_all(temp).unwrap();
}
#[cfg(unix)]
#[test]
fn process_runner_smoke() {
    let s: loramesh::sim::Scenario =
        serde_json::from_str(include_str!("../scenarios/two-node.json")).unwrap();
    let trace = loramesh::sim::process::smoke(
        &s,
        std::path::Path::new(env!("CARGO_BIN_EXE_loramesh-radio")),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(trace.iter().filter(|e| e.event == "tx_start").count(), 1);
}

#[cfg(unix)]
#[test]
fn interrupting_pty_runner_removes_device_paths() {
    use std::{
        process::{Child, Stdio},
        sync::mpsc,
        time::{Duration, Instant},
    };
    struct Guard(Child);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Guard(
        Command::new(env!("CARGO_BIN_EXE_loramesh-sim"))
            .args(["scenarios/virtual-lab.json", "--pty"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let paths = serde_json::Deserializer::from_reader(stdout)
            .into_iter::<std::collections::BTreeMap<u16, std::path::PathBuf>>()
            .next();
        let _ = tx.send(paths);
    });
    let paths = rx
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(paths.values().all(|p| p.exists()));
    // SAFETY: this is the live child created by this test; SIGTERM invokes its cleanup handler.
    assert_eq!(
        unsafe { libc::kill(child.0.id() as libc::pid_t, libc::SIGTERM) },
        0
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "runner failed to stop");
        std::thread::sleep(Duration::from_millis(10));
    }
    reader.join().unwrap();
    assert!(paths.values().all(|p| !p.exists()));
}
