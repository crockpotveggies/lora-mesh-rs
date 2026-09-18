//! Shared production loop: one link, one radio, one bounded packet adapter.
use super::Config;
#[cfg(unix)]
use super::Endpoint;
#[cfg(unix)]
use crate::{
    radio::{Action, runtime::RadioHandle},
    sim::scenario::controller_config,
};
use serde::{Deserialize, Serialize};
use std::{io, io::Read, path::PathBuf};
#[cfg(unix)]
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DaemonConfig {
    pub radio: PathBuf,
    pub power_dbm: i8,
    pub link: Config,
    pub adapter: Adapter,
    pub metrics: Option<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Adapter {
    Tun {
        name: String,
        address: String,
        peer_address: String,
    },
    Datagram {
        bind: PathBuf,
        peer: PathBuf,
    },
}
#[cfg(unix)]
pub(crate) enum Port {
    Datagram(std::os::unix::net::UnixDatagram, PathBuf),
    #[cfg(target_os = "linux")]
    Tun(crate::link::tun::Tun),
}
#[cfg(unix)]
impl Port {
    pub(crate) fn open(adapter: &Adapter) -> io::Result<Self> {
        match adapter {
            Adapter::Datagram { bind, peer } => {
                let socket = std::os::unix::net::UnixDatagram::bind(bind)?;
                socket.set_nonblocking(true)?;
                Ok(Self::Datagram(socket, peer.clone()))
            }
            Adapter::Tun {
                name,
                address,
                peer_address,
            } => {
                #[cfg(target_os = "linux")]
                {
                    use std::process::Command;
                    if name.is_empty()
                        || name.len() > 15
                        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    {
                        return Err(io::Error::other("invalid TUN name"));
                    }
                    let address: std::net::Ipv4Addr = address
                        .parse()
                        .map_err(|_| io::Error::other("invalid local IPv4"))?;
                    let peer: std::net::Ipv4Addr = peer_address
                        .parse()
                        .map_err(|_| io::Error::other("invalid peer IPv4"))?;
                    if address == peer
                        || address.is_unspecified()
                        || peer.is_unspecified()
                        || address.is_multicast()
                        || peer.is_multicast()
                    {
                        return Err(io::Error::other("invalid point-to-point addresses"));
                    }
                    // Exclusive creation: never mutate an existing interface owned by another process.
                    if Command::new("ip")
                        .args(["link", "show", "dev", name])
                        .output()?
                        .status
                        .success()
                    {
                        return Err(io::Error::other("TUN name already exists"));
                    }
                    let iface = crate::link::tun::Tun::open(name)?;
                    for args in [
                        vec![
                            "addr".into(),
                            "add".into(),
                            address.to_string(),
                            "peer".into(),
                            peer.to_string(),
                            "dev".into(),
                            iface.name().into(),
                        ],
                        vec![
                            "link".into(),
                            "set".into(),
                            "dev".into(),
                            iface.name().into(),
                            "mtu".into(),
                            "1500".into(),
                            "up".into(),
                        ],
                    ] {
                        let output = Command::new("ip").args(&args).output()?;
                        if !output.status.success() {
                            return Err(io::Error::other(
                                String::from_utf8_lossy(&output.stderr).to_string(),
                            ));
                        }
                    }
                    Ok(Self::Tun(iface))
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = (name, address, peer_address);
                    Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "TUN requires Linux",
                    ))
                }
            }
        }
    }
    pub(crate) fn recv(&self, bytes: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Datagram(s, _) => s.recv(bytes),
            #[cfg(target_os = "linux")]
            Self::Tun(i) => i.recv(bytes),
        }
    }
    pub(crate) fn send(&self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Datagram(s, p) => s.send_to(bytes, p),
            #[cfg(target_os = "linux")]
            Self::Tun(i) => i.send(bytes),
        }
    }
    pub(crate) fn fd(&self) -> std::os::unix::io::RawFd {
        use std::os::unix::io::AsRawFd;
        match self {
            Self::Datagram(s, _) => s.as_raw_fd(),
            #[cfg(target_os = "linux")]
            Self::Tun(i) => i.as_raw_fd(),
        }
    }
}
#[cfg(unix)]
struct Cleanup(Option<PathBuf>);
#[cfg(unix)]
impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(p) = &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}
pub fn load(path: &std::path::Path) -> io::Result<DaemonConfig> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(65537)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err(io::Error::other("config exceeds 64 KiB"));
    }
    Ok(serde_json::from_slice(&bytes)?)
}
#[cfg(unix)]
pub fn run(mut config: DaemonConfig, stop: Arc<AtomicBool>) -> io::Result<()> {
    let process_started = Instant::now();
    // Sessions are always fresh on process restart, regardless of the config placeholder.
    let mut session = [0u8; 8];
    getrandom::fill(&mut session).map_err(|e| io::Error::other(e.to_string()))?;
    config.link.session = u64::from_ne_bytes(session).max(1);
    config.link.validate()?;
    if !(-3..=20).contains(&config.power_dbm) {
        return Err(io::Error::other(
            "power_dbm must be -3..20; firmware may impose narrower limits",
        ));
    }
    let mut link = Endpoint::new(config.link.clone())?;
    let mut rc = controller_config(&config.link.profile);
    rc.initialization
        .push(format!("radio set pwr {}", config.power_dbm));
    rc.receive_guard_us = 0;
    rc.queue_frames = 1;
    let mut radio = RadioHandle::serial(config.radio.clone(), rc)?;
    radio.wait_ready(Duration::from_secs(10))?;
    let port = Port::open(&config.adapter)?;
    let _cleanup = Cleanup(match &config.adapter {
        Adapter::Datagram { bind, .. } => Some(bind.clone()),
        _ => None,
    });
    let started = Instant::now();
    let mut pending = false;
    let mut observed_tx = 0;
    let mut observed_failures = 0;
    let mut buffer = [0u8; 65536];
    let mut output = std::collections::VecDeque::new();
    let mut output_drops = 0u64;
    println!("ready");
    while !stop.load(Ordering::Acquire) {
        let now = started.elapsed().as_micros() as u64;
        for _ in 0..64 {
            match radio.rx.try_recv() {
                Ok(bytes) => link.on_frame(&bytes, now),
                Err(_) => break,
            }
        }
        for _ in 0..64 {
            match radio.events.try_recv() {
                Ok(Action::Diagnostic(message)) => eprintln!("radio: {}", message),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        let tx = radio.stats.transmitted.load(Ordering::Relaxed);
        let failures = radio.stats.failures.load(Ordering::Relaxed);
        if pending && (tx != observed_tx || failures != observed_failures) {
            link.transmitted(now, failures == observed_failures);
            pending = false;
        }
        observed_tx = tx;
        observed_failures = failures;
        for _ in 0..32 {
            match port.recv(&mut buffer) {
                Ok(n) => {
                    if let Err(e) = link.enqueue(buffer[..n].to_vec(), now) {
                        eprintln!("ingress drop: {}", e);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        link.tick(now);
        if !pending {
            if let Some(bytes) = link.poll_transmit(now) {
                match radio.tx.try_send(bytes) {
                    Ok(()) => pending = true,
                    Err(_) => link.transmitted(now, false),
                }
            }
        }
        while let Some(bytes) = link.receive_packet() {
            if output.len() < config.link.queue_packets {
                output.push_back(bytes);
            } else {
                output_drops += 1;
            }
        }
        while let Some(bytes) = output.front() {
            match port.send(bytes) {
                Ok(n) if n == bytes.len() => {
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
                other => {
                    eprintln!("egress drop: {:?}", other);
                    output.pop_front();
                    output_drops += 1;
                }
            }
        }
        let wait = link
            .next_deadline()
            .map(|d| d.saturating_sub(now).div_ceil(1000))
            .unwrap_or(20)
            .clamp(1, 20) as i32;
        let mut fd = libc::pollfd {
            fd: port.fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: a single valid pollfd; all packet and serial I/O is bounded/nonblocking.
        let result = unsafe { libc::poll(&mut fd, 1, wait) };
        if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return Err(io::Error::last_os_error());
        }
    }
    radio.shutdown();
    if let Some(path) = config.metrics {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(
                &serde_json::json!({"link":link.metrics,"process_wall_us":process_started.elapsed().as_micros(),"process_usage":process_usage(),"adapter_egress_dropped":output_drops,"radio_rx_dropped":radio.stats.received_dropped.load(Ordering::Relaxed),"radio_events_dropped":radio.stats.events_dropped.load(Ordering::Relaxed)}),
            )?,
        )?;
    }
    Ok(())
}

/// Process CPU and peak resident memory; includes initialization and shutdown.
pub fn process_usage() -> serde_json::Value {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: writable rusage initialized on successful return.
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
            let u = unsafe { usage.assume_init() };
            let scale = if cfg!(target_os = "macos") { 1 } else { 1024 };
            return serde_json::json!({"user_cpu_us":u.ru_utime.tv_sec as i128*1_000_000+u.ru_utime.tv_usec as i128,"system_cpu_us":u.ru_stime.tv_sec as i128*1_000_000+u.ru_stime.tv_usec as i128,"peak_rss_bytes":u.ru_maxrss*scale});
        }
    }
    serde_json::Value::Null
}
