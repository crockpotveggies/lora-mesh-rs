//! Single-owner serial worker. Blocking reads have short, finite deadlines; no spin loop.
use super::{Action, Controller, ControllerConfig, LineCodec, State};
use crossbeam_channel::{Receiver, Sender, bounded};
use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub trait Transport: Read + Write + Send {}
impl<T: Read + Write + Send> Transport for T {}
pub type Factory = Box<dyn FnMut() -> io::Result<Box<dyn Transport>> + Send>;

pub fn serial_transport(path: &Path) -> io::Result<Box<dyn Transport>> {
    // Apply one complete standard-baud configuration on macOS. serialport's
    // builder first applies inherited settings, then uses IOSSIOSPEED; avoid
    // both intermediate reconfiguration and that ioctl for the CH340 bridge.
    #[cfg(target_os = "macos")]
    let port = {
        use serialport::SerialPort;
        use std::os::{
            fd::{FromRawFd, IntoRawFd},
            unix::fs::OpenOptionsExt,
        };
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
            .open(path)?;
        // SAFETY: ownership of this live serial descriptor is transferred exactly once.
        let mut port = unsafe { serialport::TTYPort::from_raw_fd(file.into_raw_fd()) };
        if !port.exclusive() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "serial port is already in use or cannot be locked",
            ));
        }
        configure_mac_serial(&port)?;
        port.set_timeout(Duration::from_millis(20))?;
        port.clear(serialport::ClearBuffer::All)?;
        port
    };
    #[cfg(not(target_os = "macos"))]
    let port = {
        let port = serialport::new(path.to_string_lossy(), 57600)
            .timeout(Duration::from_millis(20))
            .open()?;
        port.clear(serialport::ClearBuffer::All)?;
        port
    };
    Ok(Box::new(port))
}

#[cfg(target_os = "macos")]
fn configure_mac_serial(port: &serialport::TTYPort) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let fd = port.as_raw_fd();
    let mut settings = std::mem::MaybeUninit::<libc::termios>::uninit();
    // SAFETY: fd is owned by a live TTYPort; tcgetattr initializes settings on success.
    if unsafe { libc::tcgetattr(fd, settings.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut settings = unsafe { settings.assume_init() };
    // SAFETY: cfmakeraw only modifies the initialized local termios value.
    unsafe { libc::cfmakeraw(&mut settings) };
    settings.c_cflag |= libc::CREAD | libc::CLOCAL;
    settings.c_cflag &= !(libc::CSTOPB | libc::CRTSCTS);
    settings.c_iflag &= !(libc::IXON | libc::IXOFF | libc::IXANY);
    // SAFETY: settings is initialized and these calls borrow it only for their duration.
    if unsafe { libc::cfsetispeed(&mut settings, libc::B57600) } != 0
        || unsafe { libc::cfsetospeed(&mut settings, libc::B57600) } != 0
        || unsafe { libc::tcsetattr(fd, libc::TCSANOW, &settings) } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
#[derive(Default)]
pub struct Stats {
    pub received_dropped: AtomicU64,
    pub events_dropped: AtomicU64,
    pub failures: AtomicU64,
    pub transmitted: AtomicU64,
}
pub struct RadioHandle {
    pub tx: Sender<Vec<u8>>,
    pub rx: Receiver<Vec<u8>>,
    pub events: Receiver<Action>,
    pub stats: Arc<Stats>,
    ready: Receiver<()>,
    last_diagnostic: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl RadioHandle {
    pub fn serial(path: PathBuf, config: ControllerConfig) -> io::Result<Self> {
        Self::spawn(Box::new(move || serial_transport(&path)), config)
    }
    /// Custom transports must return from read/write within a bounded time (20 ms recommended).
    pub fn spawn(factory: Factory, config: ControllerConfig) -> io::Result<Self> {
        config.validate()?;
        let controller = Controller::new(config.clone())?;
        let (tx, requests) = bounded(config.queue_frames);
        let (received, rx) = bounded(config.queue_frames);
        let (notices, events) = bounded(256);
        let (ready_tx, ready) = bounded(1);
        let last_diagnostic = Arc::new(Mutex::new(None));
        let worker_diagnostic = last_diagnostic.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let stats = Arc::new(Stats::default());
        let worker_stats = stats.clone();
        let worker = thread::Builder::new()
            .name("loramesh-radio".into())
            .spawn(move || {
                Worker {
                    controller,
                    factory,
                    port: None,
                    codec: LineCodec::default(),
                    requests,
                    received,
                    notices,
                    ready: Some(ready_tx),
                    last_diagnostic: worker_diagnostic,
                    stop: worker_stop,
                    stats: worker_stats,
                    started: Instant::now(),
                    sequence: 0,
                    trace: std::env::var_os("LORAMESH_RADIO_TRACE").is_some(),
                }
                .run();
            })?;
        Ok(Self {
            tx,
            rx,
            events,
            ready,
            last_diagnostic,
            stop,
            stats,
            worker: Some(worker),
        })
    }
    pub fn wait_ready(&self, timeout: Duration) -> io::Result<()> {
        self.ready.recv_timeout(timeout).map_err(|e| {
            let kind = match e {
                crossbeam_channel::RecvTimeoutError::Timeout => io::ErrorKind::TimedOut,
                crossbeam_channel::RecvTimeoutError::Disconnected => io::ErrorKind::BrokenPipe,
            };
            let diagnostic = self.last_diagnostic.lock().unwrap();
            io::Error::new(
                kind,
                format!(
                    "waiting for radio readiness: {e}; last diagnostic: {}",
                    diagnostic.as_deref().unwrap_or("none")
                ),
            )
        })
    }
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for RadioHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}
struct Worker {
    controller: Controller,
    factory: Factory,
    port: Option<Box<dyn Transport>>,
    codec: LineCodec,
    requests: Receiver<Vec<u8>>,
    received: Sender<Vec<u8>>,
    notices: Sender<Action>,
    ready: Option<Sender<()>>,
    last_diagnostic: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    stats: Arc<Stats>,
    started: Instant,
    sequence: u64,
    trace: bool,
}
impl Worker {
    fn now(&self) -> u64 {
        self.started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
    }
    fn announce(&self, action: Action) {
        if self.notices.try_send(action).is_err() {
            self.stats.events_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn actions(&mut self, actions: Vec<Action>) {
        let mut pending: std::collections::VecDeque<_> = actions.into();
        while let Some(action) = pending.pop_front() {
            if self.trace {
                eprintln!("radio: {action:?}");
            }
            match action {
                Action::Reconnect => match (self.factory)() {
                    Ok(port) => {
                        self.port = Some(port);
                        self.codec.reset();
                        pending.extend(self.controller.connected(self.now()));
                    }
                    Err(e) => pending.extend(
                        self.controller
                            .disconnected(self.now(), &format!("open serial: {}", e)),
                    ),
                },
                Action::Close => {
                    self.port = None;
                    self.codec.reset();
                }
                Action::Write(ref line) => {
                    if line == "sys get ver" {
                        // The synchronization window may have ended with a partial old line.
                        self.codec.reset();
                    }
                    let mut bytes = line.as_bytes().to_vec();
                    bytes.extend_from_slice(b"\r\n");
                    let result = match self.port.as_mut() {
                        Some(port) => port.write_all(&bytes),
                        None => Err(io::Error::new(
                            io::ErrorKind::NotConnected,
                            "serial disconnected",
                        )),
                    };
                    if let Err(e) = result {
                        pending.clear();
                        pending.extend(
                            self.controller
                                .disconnected(self.now(), &format!("write serial: {}", e)),
                        );
                    }
                }
                Action::Received(ref bytes) if self.received.try_send(bytes.clone()).is_err() => {
                    self.stats.received_dropped.fetch_add(1, Ordering::Relaxed);
                }
                Action::Transmitted(_) => {
                    self.stats.transmitted.fetch_add(1, Ordering::Relaxed);
                }
                Action::Failed(_, _) => {
                    self.stats.failures.fetch_add(1, Ordering::Relaxed);
                }
                Action::State(State::Receiving) => {
                    if let Some(ready) = self.ready.take() {
                        let _ = ready.try_send(());
                    }
                }
                Action::Diagnostic(ref message) => {
                    *self.last_diagnostic.lock().unwrap() = Some(message.clone());
                }
                _ => {}
            }
            self.announce(action);
        }
    }
    fn request(&mut self, packet: Vec<u8>) {
        self.sequence += 1;
        if let Err(e) = self.controller.enqueue(self.sequence, packet, self.now()) {
            self.actions(vec![Action::Failed(self.sequence, e.to_string())]);
        }
    }
    fn run(mut self) {
        self.actions(vec![Action::Reconnect]);
        let mut buffer = [0u8; 1024];
        while !self.stop.load(Ordering::Acquire) {
            // Bound work so sustained producers cannot starve receive events or deadlines.
            for _ in 0..32 {
                match self.requests.try_recv() {
                    Ok(p) => self.request(p),
                    Err(_) => break,
                }
            }
            let actions = self.controller.tick(self.now());
            self.actions(actions);
            if let Some(port) = self.port.as_mut() {
                match port.read(&mut buffer) {
                    Ok(0) => {
                        let actions = self.controller.disconnected(self.now(), "serial EOF");
                        self.actions(actions);
                    }
                    Ok(n) => {
                        for &byte in &buffer[..n] {
                            if let Some(line) = self.codec.push(byte) {
                                let actions = match line {
                                    Ok(line) => self.controller.on_line(&line, self.now()),
                                    Err(_) if self.controller.state() == State::Synchronizing => {
                                        // Old bytes can be malformed or unterminated. The
                                        // settling window drains them without restarting it.
                                        vec![]
                                    }
                                    Err(e) => self.controller.disconnected(
                                        self.now(),
                                        &format!("invalid serial framing: {}", e),
                                    ),
                                };
                                self.actions(actions);
                            }
                        }
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::TimedOut
                                | io::ErrorKind::WouldBlock
                                | io::ErrorKind::Interrupted
                        ) => {}
                    Err(e) => {
                        let actions = self
                            .controller
                            .disconnected(self.now(), &format!("read serial: {}", e));
                        self.actions(actions);
                    }
                }
            } else {
                let wait = self
                    .controller
                    .next_deadline()
                    .map(|d| d.saturating_sub(self.now()))
                    .unwrap_or(20_000)
                    .min(20_000);
                if let Ok(p) = self
                    .requests
                    .recv_timeout(Duration::from_micros(wait.max(1)))
                {
                    self.request(p);
                }
            }
        }
        let actions = self.controller.shutdown();
        self.actions(actions);
        for _ in 0..self.requests.len() {
            if self.requests.try_recv().is_ok() {
                self.sequence += 1;
                self.actions(vec![Action::Failed(self.sequence, "shutdown".into())]);
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn standard_mac_baud_is_applied_without_iossiospeed() {
        let (_master, slave) = serialport::TTYPort::pair().unwrap();
        configure_mac_serial(&slave).unwrap();
        let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: the live PTY owns the descriptor and the output pointer is valid.
        assert_eq!(
            unsafe { libc::tcgetattr(slave.as_raw_fd(), termios.as_mut_ptr()) },
            0
        );
        let termios = unsafe { termios.assume_init() };
        assert_eq!(termios.c_ispeed, libc::B57600);
        assert_eq!(termios.c_ospeed, libc::B57600);
        assert_eq!(termios.c_cflag & libc::CSIZE, libc::CS8);
        assert_eq!(
            termios.c_cflag & (libc::PARENB | libc::CSTOPB | libc::CRTSCTS),
            0
        );
        assert_eq!(
            termios.c_iflag & (libc::IXON | libc::IXOFF | libc::IXANY),
            0
        );
    }
}
