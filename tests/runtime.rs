use loramesh::radio::{Action, ControllerConfig, LineCodec, State, runtime::RadioHandle};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
/// A byte transport that accepts only two bytes/write and returns one byte/read.
struct PartialPort {
    codec: LineCodec,
    replies: VecDeque<u8>,
    writes: Arc<AtomicUsize>,
}
impl Write for PartialPort {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        let count = data.len().min(2);
        for &byte in &data[..count] {
            if let Some(line) = self.codec.push(byte) {
                let line = line?;
                let reply = match line.as_str() {
                    "INVALIDCOMMAND" => {
                        // Deliberately leave oversized garbage without a newline. Startup
                        // must drain it and reset framing before the version response.
                        self.replies.extend(vec![b'x'; 2000]);
                        continue;
                    }
                    "sys get ver" => "RN2903 1.0.5 fixture",
                    "mac pause" => "4294967295",
                    "radio get mod" => "lora",
                    "radio get freq" => "915000000",
                    "radio get sf" => "sf7",
                    "radio get bw" => "125",
                    "radio get cr" => "4/5",
                    "radio get prlen" => "8",
                    "radio get crc" => "on",
                    "radio get sync" => "12",
                    _ => "ok",
                };
                self.replies.extend(reply.bytes());
                self.replies.extend(b"\r\n");
            }
        }
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Read for PartialPort {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if let Some(byte) = self.replies.pop_front() {
            buffer[0] = byte;
            return Ok(1);
        }
        std::thread::sleep(Duration::from_millis(1));
        Err(io::ErrorKind::TimedOut.into())
    }
}
#[test]
fn partial_io_and_unterminated_startup_garbage_initialize() {
    let writes = Arc::new(AtomicUsize::new(0));
    let count = writes.clone();
    let mut handle = RadioHandle::spawn(
        Box::new(move || {
            Ok(Box::new(PartialPort {
                codec: LineCodec::default(),
                replies: VecDeque::new(),
                writes: count.clone(),
            }))
        }),
        ControllerConfig::default(),
    )
    .unwrap();
    handle.wait_ready(Duration::from_secs(3)).unwrap();
    assert!(writes.load(Ordering::Relaxed) > 20);
    let started = Instant::now();
    handle.shutdown();
    assert!(started.elapsed() < Duration::from_secs(1));
}
#[test]
fn open_failure_is_reported_and_shutdown_interrupts_backoff() {
    let mut h = RadioHandle::spawn(
        Box::new(|| Err(io::ErrorKind::NotFound.into())),
        ControllerConfig::default(),
    )
    .unwrap();
    let error = h.wait_ready(Duration::from_secs(1)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(error.to_string().contains("open serial"));
    let now = Instant::now();
    h.shutdown();
    assert!(now.elapsed() < Duration::from_secs(1));
}
struct Eof;
impl Read for Eof {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Ok(0)
    }
}
impl Write for Eof {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn eof_reopens_transport_without_spin() {
    let calls = Arc::new(AtomicUsize::new(0));
    let opened = calls.clone();
    let writes = Arc::new(AtomicUsize::new(0));
    let config = ControllerConfig {
        reconnect_delay_us: 20_000,
        ..Default::default()
    };
    let mut h = RadioHandle::spawn(
        Box::new(move || {
            if opened.fetch_add(1, Ordering::Relaxed) == 0 {
                Ok(Box::new(Eof))
            } else {
                Ok(Box::new(PartialPort {
                    codec: LineCodec::default(),
                    replies: VecDeque::new(),
                    writes: writes.clone(),
                }))
            }
        }),
        config,
    )
    .unwrap();
    h.wait_ready(Duration::from_secs(3)).unwrap();
    let until = Instant::now() + Duration::from_secs(1);
    let mut recovered = false;
    while Instant::now() < until {
        if let Ok(Action::State(State::Receiving)) =
            h.events.recv_timeout(Duration::from_millis(10))
        {
            recovered = true;
            break;
        }
    }
    assert!(recovered);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    h.shutdown();
}

#[test]
fn transient_open_failure_does_not_fail_startup() {
    let calls = Arc::new(AtomicUsize::new(0));
    let opened = calls.clone();
    let mut radio = RadioHandle::spawn(
        Box::new(move || {
            if opened.fetch_add(1, Ordering::Relaxed) == 0 {
                return Err(io::ErrorKind::NotFound.into());
            }
            Ok(Box::new(PartialPort {
                codec: LineCodec::default(),
                replies: VecDeque::new(),
                writes: Arc::new(AtomicUsize::new(0)),
            }))
        }),
        ControllerConfig {
            reconnect_delay_us: 20_000,
            ..Default::default()
        },
    )
    .unwrap();
    radio.wait_ready(Duration::from_secs(3)).unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    radio.shutdown();
}

#[test]
fn shutdown_interrupts_startup_settling() {
    let mut radio = RadioHandle::spawn(
        Box::new(|| {
            Ok(Box::new(PartialPort {
                codec: LineCodec::default(),
                replies: VecDeque::new(),
                writes: Arc::new(AtomicUsize::new(0)),
            }))
        }),
        ControllerConfig::default(),
    )
    .unwrap();
    loop {
        if radio.events.recv_timeout(Duration::from_secs(1)).unwrap()
            == Action::State(State::Synchronizing)
        {
            break;
        }
    }
    let started = Instant::now();
    radio.shutdown();
    assert!(started.elapsed() < Duration::from_millis(250));
}
