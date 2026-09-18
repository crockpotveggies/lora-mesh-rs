//! Privileged interface setup finishes before keys, crypto state or the serial worker are opened.
use super::{
    Config, Node,
    security::{Vault, read_secret},
};
use crate::{
    link::daemon::{Adapter, Port},
    radio::{Action, runtime::RadioHandle},
    sim::scenario::controller_config,
};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub uid: u32,
    pub gid: u32,
    #[serde(default)]
    pub groups: Vec<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DaemonConfig {
    pub radio: PathBuf,
    pub power_dbm: i8,
    pub mesh: Config,
    pub adapter: Adapter,
    pub signing_key: PathBuf,
    pub peer_keys: BTreeMap<u16, PathBuf>,
    pub state: PathBuf,
    pub run_as: Option<Identity>,
    pub metrics: Option<PathBuf>,
}
pub fn load(path: &Path) -> io::Result<DaemonConfig> {
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take(65537)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err(io::Error::other("config exceeds 64 KiB"));
    }
    Ok(serde_json::from_slice(&bytes)?)
}
#[cfg(target_os = "linux")]
fn drop_privileges(identity: Option<&Identity>, tun: bool) -> io::Result<()> {
    let uid = unsafe { libc::geteuid() };
    if uid == 0 && tun && identity.is_none() {
        return Err(io::Error::other(
            "root TUN setup requires a non-root run_as identity",
        ));
    }
    if let Some(i) = identity {
        if i.uid == 0 || i.gid == 0 || i.groups.len() > 32 || i.groups.contains(&0) {
            return Err(io::Error::other(
                "run_as must exclude root identities/groups",
            ));
        }
        if uid == 0 {
            if unsafe { libc::setgroups(i.groups.len(), i.groups.as_ptr()) } != 0
                || unsafe { libc::setgid(i.gid) } != 0
                || unsafe { libc::setuid(i.uid) } != 0
            {
                return Err(io::Error::last_os_error());
            }
        } else if uid != i.uid || unsafe { libc::getegid() } != i.gid {
            return Err(io::Error::other(
                "already-running identity differs from run_as",
            ));
        }
        let (mut real, mut effective, mut saved) = (0, 0, 0);
        if unsafe { libc::getresuid(&mut real, &mut effective, &mut saved) } != 0
            || [real, effective, saved].iter().any(|id| *id != i.uid)
        {
            return Err(io::Error::other("failed to discard saved root identity"));
        }
    }
    // Prevent privilege gain through exec and prevent unprivileged process-memory inspection.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
        || unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn drop_privileges(identity: Option<&Identity>, _tun: bool) -> io::Result<()> {
    if identity.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "run_as requires Linux",
        ));
    }
    Ok(())
}
struct Cleanup(Option<PathBuf>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(p) = &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}
pub fn run(config: DaemonConfig, stop: Arc<AtomicBool>) -> io::Result<()> {
    config.mesh.validate()?;
    if !(-3..=20).contains(&config.power_dbm)
        || config
            .peer_keys
            .keys()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            != config.mesh.peers.iter().copied().collect()
    {
        return Err(io::Error::other("invalid power or peer-key configuration"));
    }
    #[cfg(target_os = "linux")]
    if matches!(config.adapter, Adapter::Tun { .. })
        && unsafe { libc::geteuid() } == 0
        && config.run_as.is_none()
    {
        return Err(io::Error::other("root TUN setup requires run_as"));
    }
    let port = Port::open(&config.adapter)?;
    let _cleanup = Cleanup(match &config.adapter {
        Adapter::Datagram { bind, .. } => Some(bind.clone()),
        _ => None,
    });
    #[cfg(target_os = "linux")]
    if let Adapter::Tun { name, address, .. } = &config.adapter {
        if address
            != &config.mesh.members[&config.mesh.link.node]
                .address
                .to_string()
        {
            return Err(io::Error::other(
                "TUN address must match configured node identity",
            ));
        }
        for args in std::iter::once(vec![
            "link".to_string(),
            "set".into(),
            "dev".into(),
            name.clone(),
            "mtu".into(),
            super::wire::MTU.to_string(),
        ])
        .chain(
            config
                .mesh
                .members
                .iter()
                .filter(|(id, _)| **id != config.mesh.link.node)
                .map(|(_, m)| {
                    vec![
                        "route".into(),
                        "replace".into(),
                        format!("{}/32", m.address),
                        "dev".into(),
                        name.clone(),
                    ]
                }),
        ) {
            let output = std::process::Command::new("ip").args(args).output()?;
            if !output.status.success() {
                return Err(io::Error::other(
                    String::from_utf8_lossy(&output.stderr).to_string(),
                ));
            }
        }
    }
    drop_privileges(
        config.run_as.as_ref(),
        matches!(config.adapter, Adapter::Tun { .. }),
    )?;
    let keys = config
        .peer_keys
        .iter()
        .map(|(p, path)| Ok((*p, read_secret(path)?)))
        .collect::<io::Result<BTreeMap<_, _>>>()?;
    let signer = SigningKey::from_bytes(&*read_secret(&config.signing_key)?);
    let vault = Vault::persistent(
        &config.state,
        config.mesh.link.network,
        config.mesh.link.node,
        keys,
    )?;
    let mut node = Node::new(config.mesh.clone(), vault, signer)?;
    let mut rc = controller_config(&config.mesh.link.profile);
    rc.initialization
        .push(format!("radio set pwr {}", config.power_dbm));
    rc.queue_frames = 1;
    rc.receive_guard_us = 0;
    let mut radio = RadioHandle::serial(config.radio.clone(), rc)?;
    radio.wait_ready(Duration::from_secs(10))?;
    let started = Instant::now();
    let mut pending = false;
    let (mut observed_tx, mut observed_failures) = (0, 0);
    let mut buffer = [0u8; 65536];
    let mut output = VecDeque::new();
    let mut adapter_drops = 0u64;
    println!("ready uid={} gid={}", unsafe { libc::geteuid() }, unsafe {
        libc::getegid()
    });
    while !stop.load(Ordering::Acquire) {
        let now = started.elapsed().as_micros() as u64;
        for _ in 0..64 {
            match radio.rx.try_recv() {
                Ok(frame) => node.on_radio(&frame, now)?,
                Err(_) => break,
            }
        }
        for _ in 0..64 {
            match radio.events.try_recv() {
                Ok(Action::Diagnostic(s)) => eprintln!("radio: {}", s),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        let tx = radio.stats.transmitted.load(Ordering::Relaxed);
        let failed = radio.stats.failures.load(Ordering::Relaxed);
        if pending && (tx != observed_tx || failed != observed_failures) {
            node.transmitted(now, failed == observed_failures);
            pending = false;
        }
        observed_tx = tx;
        observed_failures = failed;
        for _ in 0..32 {
            match port.recv(&mut buffer) {
                Ok(n) => {
                    if node.enqueue(buffer[..n].to_vec(), now).is_err() {
                        adapter_drops += 1;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        node.tick(now)?;
        if !pending {
            if let Some(frame) = node.poll_transmit(now)? {
                match radio.tx.try_send(frame) {
                    Ok(()) => pending = true,
                    Err(_) => node.transmitted(now, false),
                }
            }
        }
        while let Some(packet) = node.receive() {
            if output.len() < config.mesh.pending_packets {
                output.push_back(packet);
            } else {
                adapter_drops += 1;
            }
        }
        while let Some(packet) = output.front() {
            match port.send(packet) {
                Ok(n) if n == packet.len() => {
                    output.pop_front();
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    break;
                }
                _ => {
                    output.pop_front();
                    adapter_drops += 1;
                }
            }
        }
        let wait = node
            .next_deadline()
            .map(|d| d.saturating_sub(now).div_ceil(1000))
            .unwrap_or(20)
            .clamp(1, 20) as i32;
        let mut fd = libc::pollfd {
            fd: port.fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut fd, 1, wait) } < 0
            && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted
        {
            return Err(io::Error::last_os_error());
        }
    }
    radio.shutdown();
    if let Some(path) = config.metrics {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(
                &serde_json::json!({"mesh":node.metrics,"links":node.links.iter().map(|(p,l)|(*p,&l.metrics)).collect::<BTreeMap<_,_>>(),"routes":node.router.routes,"route_recomputations":node.router.recomputations,"adapter_drops":adapter_drops,"radio_transmitted":radio.stats.transmitted.load(Ordering::Relaxed),"radio_failures":radio.stats.failures.load(Ordering::Relaxed),"uid":unsafe{libc::geteuid()},"gid":unsafe{libc::getegid()},"elapsed_us":started.elapsed().as_micros(),"process_usage":crate::link::daemon::process_usage()}),
            )?,
        )?;
    }
    Ok(())
}
