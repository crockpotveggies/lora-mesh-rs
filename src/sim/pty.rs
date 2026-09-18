//! Unix pseudo-terminal laboratory; intentionally uses real time for OS I/O tests.
use super::{
    medium::Medium,
    scenario::{Fault, Scenario},
};
use crate::radio::protocol::invalid;
use std::{
    collections::{BTreeMap, VecDeque},
    ffi::CStr,
    fs::{self, File},
    io::{self, Read, Write},
    os::unix::{
        fs::symlink,
        io::{AsRawFd, FromRawFd},
    },
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Instant,
};
static NEXT_LAB: AtomicU64 = AtomicU64::new(0);
struct Pair {
    master: File,
    _slave: File,
    pending: VecDeque<u8>,
}
impl Pair {
    fn open(link: &PathBuf) -> io::Result<Self> {
        let mut master = -1;
        let mut slave = -1;
        // SAFETY: openpty receives valid writable descriptors; optional arguments are null.
        if unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: descriptors are newly allocated and uniquely owned by these Files.
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        // SAFETY: termios is filled by tcgetattr before it is used.
        let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut termios) } != 0 {
            return Err(io::Error::last_os_error());
        }
        unsafe { libc::cfmakeraw(&mut termios) };
        if unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &termios) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                < 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut name = [0 as libc::c_char; 512];
        let result = unsafe { libc::ttyname_r(slave.as_raw_fd(), name.as_mut_ptr(), name.len()) };
        if result != 0 {
            return Err(io::Error::from_raw_os_error(result));
        }
        // SAFETY: successful ttyname_r returns a NUL-terminated string in this buffer.
        let target = unsafe { CStr::from_ptr(name.as_ptr()) }
            .to_str()
            .map_err(|_| invalid("invalid PTY path"))?;
        let temporary = link.with_extension("next");
        symlink(target, &temporary)?;
        fs::rename(&temporary, link)?;
        Ok(Self {
            master,
            _slave: slave,
            pending: VecDeque::new(),
        })
    }
}
struct LabState {
    medium: Medium,
    error: Option<String>,
    running: bool,
}
pub struct PtyLab {
    pub paths: BTreeMap<u16, PathBuf>,
    directory: PathBuf,
    state: Arc<Mutex<LabState>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl PtyLab {
    pub fn start(scenario: &Scenario) -> io::Result<Self> {
        let medium = scenario.medium()?;
        let directory = std::env::temp_dir().join(format!(
            "loramesh-pty-{}-{}-{}",
            std::process::id(),
            NEXT_LAB.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir(&directory)?;
        let mut paths = BTreeMap::new();
        let mut pairs = BTreeMap::new();
        for node in &scenario.nodes {
            let path = directory.join(format!("radio-{}", node.id));
            match Pair::open(&path) {
                Ok(pair) => {
                    pairs.insert(node.id, pair);
                    paths.insert(node.id, path);
                }
                Err(e) => {
                    let _ = fs::remove_dir_all(&directory);
                    return Err(e);
                }
            }
        }
        let state = Arc::new(Mutex::new(LabState {
            medium,
            error: None,
            running: true,
        }));
        let worker_state = state.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker_paths = paths.clone();
        let mut faults = scenario.faults.clone();
        faults.sort_by_key(Fault::at);
        let duration = scenario.duration_us;
        let worker = match thread::Builder::new()
            .name("virtual-lostik".into())
            .spawn(move || {
                let result = serve(
                    pairs,
                    worker_paths,
                    &worker_state,
                    &worker_stop,
                    faults,
                    duration,
                );
                let mut state = worker_state.lock().unwrap();
                state.running = false;
                if let Err(e) = result {
                    state.error = Some(e.to_string());
                }
            }) {
            Ok(w) => w,
            Err(e) => {
                let _ = fs::remove_dir_all(&directory);
                return Err(e);
            }
        };
        Ok(Self {
            paths,
            directory,
            state,
            stop,
            worker: Some(worker),
        })
    }
    pub fn trace(&self) -> Vec<super::Trace> {
        self.state.lock().unwrap().medium.trace.clone()
    }
    pub fn error(&self) -> Option<String> {
        self.state.lock().unwrap().error.clone()
    }
    pub fn running(&self) -> bool {
        self.state.lock().unwrap().running
    }
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = fs::remove_dir_all(&self.directory);
    }
}
impl Drop for PtyLab {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn serve(
    mut pairs: BTreeMap<u16, Pair>,
    paths: BTreeMap<u16, PathBuf>,
    state: &Arc<Mutex<LabState>>,
    stop: &AtomicBool,
    faults: Vec<Fault>,
    duration: u64,
) -> io::Result<()> {
    let started = Instant::now();
    let mut fi = 0;
    let mut bytes = [0u8; 1024];
    while !stop.load(Ordering::Acquire) {
        let now = started.elapsed().as_micros() as u64;
        if now >= duration {
            break;
        }
        {
            let mut state = state.lock().unwrap();
            let medium = &mut state.medium;
            medium.advance(now)?;
            while fi < faults.len() && faults[fi].at() <= now {
                faults[fi].apply(medium)?;
                match &faults[fi] {
                    Fault::Disconnect { node, .. } => {
                        pairs.remove(node);
                    }
                    Fault::Reconnect { node, .. } => {
                        pairs.insert(*node, Pair::open(&paths[node])?);
                    }
                    _ => {}
                }
                fi += 1;
            }
            for (&id, pair) in &mut pairs {
                // Bound each turn to avoid a noisy port starving its peers.
                match pair.master.read(&mut bytes) {
                    Ok(n) if n > 0 => medium.bytes(id, &bytes[..n])?,
                    Ok(_) => {}
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(e) => return Err(e),
                }
            }
            medium.advance(now)?;
            for (&id, pair) in &mut pairs {
                if pair.pending.len() < 8192 {
                    pair.pending.extend(medium.drain(id, 1024));
                }
                if !pair.pending.is_empty() {
                    // Small writes exercise stream fragmentation even for single-line replies.
                    let chunk: Vec<_> = pair.pending.iter().take(7).copied().collect();
                    match pair.master.write(&chunk) {
                        Ok(0) => {
                            return Err(io::Error::new(
                                io::ErrorKind::WriteZero,
                                "PTY write returned zero",
                            ));
                        }
                        Ok(n) => {
                            pair.pending.drain(..n);
                        }
                        Err(e)
                            if matches!(
                                e.kind(),
                                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                            ) => {}
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        let mut fds: Vec<_> = pairs
            .values()
            .map(|p| libc::pollfd {
                fd: p.master.as_raw_fd(),
                events: libc::POLLIN
                    | if p.pending.is_empty() {
                        0
                    } else {
                        libc::POLLOUT
                    },
                revents: 0,
            })
            .collect();
        // SAFETY: contiguous valid pollfd storage, never retained by poll.
        let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 5) };
        if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
