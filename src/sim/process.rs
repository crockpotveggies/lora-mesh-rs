//! Bounded child-process smoke runner for the actual radio probe executable.
use super::{Scenario, Trace, pty::PtyLab};
use std::{
    io::{self, BufRead, BufReader, Read},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
fn error(message: &str) -> io::Error {
    io::Error::other(message)
}
struct Probe {
    child: Child,
    ready: mpsc::Receiver<()>,
    stdout: Option<JoinHandle<io::Result<String>>>,
    stderr: Option<JoinHandle<io::Result<String>>>,
}
impl Probe {
    fn spawn(program: &Path, port: &Path, payload: Option<&str>) -> io::Result<Self> {
        let mut command = Command::new(program);
        command
            .arg(port)
            .args(["--sf", "7", "--duration-ms", "1200"]);
        if let Some(payload) = payload {
            command.args(["--send", payload]);
        }
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let stdout = thread::spawn(move || {
            let mut output = String::new();
            for line in BufReader::new(stdout.take(65537)).lines() {
                let line = line?;
                if line == "ready" {
                    let _ = ready_tx.try_send(());
                }
                output.push_str(&line);
                output.push('\n');
                if output.len() > 65536 {
                    return Err(error("probe stdout limit exceeded"));
                }
            }
            Ok(output)
        });
        let stderr = thread::spawn(move || {
            let mut output = String::new();
            stderr.take(65537).read_to_string(&mut output)?;
            if output.len() > 65536 {
                return Err(error("probe stderr limit exceeded"));
            }
            Ok(output)
        });
        Ok(Self {
            child,
            ready,
            stdout: Some(stdout),
            stderr: Some(stderr),
        })
    }
    fn ready(&mut self, stop: &AtomicBool) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !stop.load(Ordering::Acquire) {
            if self.ready.recv_timeout(Duration::from_millis(20)).is_ok() {
                return Ok(());
            }
            if self.child.try_wait()?.is_some() {
                return Err(error("radio probe exited before ready"));
            }
        }
        Err(error("radio probe readiness timed out or interrupted"))
    }
    fn finish(&mut self, stop: &AtomicBool) -> io::Result<String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !stop.load(Ordering::Acquire) {
            if let Some(status) = self.child.try_wait()? {
                let stdout = self
                    .stdout
                    .take()
                    .unwrap()
                    .join()
                    .map_err(|_| error("stdout reader panicked"))??;
                let stderr = self
                    .stderr
                    .take()
                    .unwrap()
                    .join()
                    .map_err(|_| error("stderr reader panicked"))??;
                if !status.success() {
                    return Err(error(&format!("probe failed: {}\n{}", status, stderr)));
                }
                return Ok(stdout);
            }
            thread::sleep(Duration::from_millis(10));
        }
        Err(error("radio probe timed out or interrupted"))
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(t) = self.stdout.take() {
            let _ = t.join();
        }
        if let Some(t) = self.stderr.take() {
            let _ = t.join();
        }
    }
}
/// Fixed two-node SF7 smoke, deliberately separate from arbitrary scenario assertions.
pub fn smoke(scenario: &Scenario, program: &Path, stop: &AtomicBool) -> io::Result<Vec<Trace>> {
    if scenario.nodes.len() != 2 {
        return Err(error("PTY smoke requires exactly two nodes"));
    }
    let mut scenario = scenario.clone();
    scenario.duration_us = 15_000_000;
    scenario.traffic.clear();
    for node in &mut scenario.nodes {
        node.profile.sf = 7;
    }
    let mut lab = PtyLab::start(&scenario)?;
    let a = scenario.nodes[0].id;
    let b = scenario.nodes[1].id;
    let mut receiver = Probe::spawn(program, &lab.paths[&b], None)?;
    receiver.ready(stop)?;
    let mut sender = Probe::spawn(program, &lab.paths[&a], Some("000102feff"))?;
    sender.ready(stop)?;
    sender.finish(stop)?;
    let received = receiver.finish(stop)?;
    if received.lines().filter(|s| *s == "rx 000102feff").count() != 1 {
        return Err(error(
            "PTY smoke did not receive exactly one expected payload",
        ));
    }
    let trace = lab.trace();
    if trace.iter().filter(|e| e.event == "tx_start").count() != 1 {
        return Err(error(
            "PTY smoke transmitted an unexpected number of frames",
        ));
    }
    if let Some(error) = lab.error() {
        return Err(self::error(&error));
    }
    lab.shutdown();
    Ok(trace)
}
