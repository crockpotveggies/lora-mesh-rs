use super::protocol::{MAX_FRAME, Micros, RadioProfile, Reply, invalid};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, io};

#[derive(Clone, Debug)]
pub struct ControllerConfig {
    pub initialization: Vec<String>,
    pub command_timeout_us: Micros,
    pub reconnect_delay_us: Micros,
    pub receive_guard_us: Micros,
    pub queue_frames: usize,
    pub queue_airtime_us: Micros,
    pub max_packet_age_us: Micros,
}
impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            initialization: [
                "mac pause",
                "radio set mod lora",
                "radio set pwr auto",
                "radio set sf sf12",
                "radio set bw 125",
                "radio set cr 4/5",
                "radio set prlen 8",
                "radio set crc on",
                "radio set wdt 60000",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            command_timeout_us: 2_000_000,
            reconnect_delay_us: 1_000_000,
            receive_guard_us: 1_000_000,
            queue_frames: 32,
            queue_airtime_us: 120_000_000,
            max_packet_age_us: 120_000_000,
        }
    }
}
impl ControllerConfig {
    pub fn validate(&self) -> io::Result<()> {
        if self.command_timeout_us == 0
            || self.command_timeout_us > 60_000_000
            || self.reconnect_delay_us == 0
            || self.reconnect_delay_us > 60_000_000
            || self.receive_guard_us > 60_000_000
            || self.queue_frames == 0
            || self.queue_frames > 1024
            || self.queue_airtime_us == 0
            || self.queue_airtime_us > 3_600_000_000
            || self.max_packet_age_us == 0
            || self.max_packet_age_us > 3_600_000_000
            || self.initialization.len() > 64
        {
            return Err(invalid("invalid radio controller limits"));
        }
        for command in &self.initialization {
            validate_init(command)?;
        }
        Ok(())
    }
}
/// Validate the supported command subset before touching hardware.
pub fn validate_init(command: &str) -> io::Result<()> {
    if command.len() > 100 || command.contains(['\r', '\n']) {
        return Err(invalid("invalid initialization command"));
    }
    let words: Vec<_> = command.split_whitespace().collect();
    match words.as_slice() {
        ["sys", "get", "ver"] | ["mac", "pause"] | ["mac", "reset"] => Ok(()),
        ["mac", "reset", "868"] | ["mac", "reset", "433"] => Ok(()),
        ["radio", "get", field]
            if RadioProfile::default().get(field).is_some() || ["pwr", "wdt"].contains(field) =>
        {
            Ok(())
        }
        ["sys", "set", "pindig", pin, value]
            if ["GPIO10", "GPIO11"].contains(pin) && ["0", "1"].contains(value) =>
        {
            Ok(())
        }
        ["radio", "set", "pwr", "auto"] => Ok(()),
        ["radio", "set", "pwr", value]
            if value
                .parse::<i8>()
                .map(|v| (-3..=20).contains(&v))
                .unwrap_or(false) =>
        {
            Ok(())
        }
        ["radio", "set", "wdt", value]
            if value
                .parse::<u32>()
                .map(|v| v == 0 || (10_000..=600_000).contains(&v))
                .unwrap_or(false) =>
        {
            Ok(())
        }
        ["radio", "set", field, value] => RadioProfile::default().update(field, value),
        _ => Err(invalid("unsupported initialization command")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Disconnected,
    Synchronizing,
    Initializing,
    StartingReceive,
    Receiving,
    StoppingReceive,
    AwaitingTxAck,
    Transmitting,
    Recovering,
    Stopped,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Write(String),
    Reconnect,
    Close,
    Received(Vec<u8>),
    Transmitted(u64),
    Failed(u64, String),
    Diagnostic(String),
    State(State),
}
#[derive(Debug)]
struct Packet {
    id: u64,
    data: Vec<u8>,
    expires: Micros,
}
/// Pure event-driven controller. Callers supply monotonic time and perform emitted actions.
/// Failed/ambiguous transmissions are never retried automatically (the link layer owns retries).
pub struct Controller {
    config: ControllerConfig,
    state: State,
    deadline: Option<Micros>,
    init: VecDeque<String>,
    pending_command: String,
    queue: VecDeque<Packet>,
    active: Option<Packet>,
    profile: RadioProfile,
    next_tx: Micros,
    rx_ended_before_ack: bool,
    stale_tx_error_seen: bool,
    firmware: String,
}
impl Controller {
    pub fn new(config: ControllerConfig) -> io::Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            state: State::Disconnected,
            deadline: None,
            init: VecDeque::new(),
            pending_command: String::new(),
            queue: VecDeque::new(),
            active: None,
            profile: RadioProfile::default(),
            next_tx: 0,
            rx_ended_before_ack: false,
            stale_tx_error_seen: false,
            firmware: String::new(),
        })
    }
    pub fn state(&self) -> State {
        self.state
    }
    pub fn profile(&self) -> &RadioProfile {
        &self.profile
    }
    pub fn queued(&self) -> usize {
        self.queue.len() + usize::from(self.active.is_some())
    }
    pub fn next_deadline(&self) -> Option<Micros> {
        let expiry = self.queue.front().map(|p| p.expires);
        let tx = if self.state == State::Receiving && !self.queue.is_empty() {
            Some(self.next_tx)
        } else {
            None
        };
        [self.deadline, expiry, tx].iter().filter_map(|v| *v).min()
    }
    fn state_to(&mut self, state: State, out: &mut Vec<Action>) {
        self.state = state;
        out.push(Action::State(state));
    }
    fn command(&mut self, command: String, state: State, now: Micros, out: &mut Vec<Action>) {
        self.pending_command = command.clone();
        self.deadline = Some(now.saturating_add(self.config.command_timeout_us));
        self.state_to(state, out);
        out.push(Action::Write(command));
    }
    pub fn connected(&mut self, now: Micros) -> Vec<Action> {
        if self.state == State::Stopped {
            return vec![];
        }
        let mut out = vec![];
        self.init = self.config.initialization.clone().into();
        // Reset the MAC, not the entire module. Resolve the RN2483 band after identity.
        self.init.push_front("mac reset auto".into());
        // Read back all parameters that affect airtime or compatibility, including retained settings.
        for field in ["mod", "freq", "sf", "bw", "cr", "prlen", "crc", "sync"] {
            self.init.push_back(format!("radio get {}", field));
        }
        self.rx_ended_before_ack = false;
        // Match the original LoStik session synchronization: discard old replies for
        // one second before requesting identity. Never reboot the module on reopen.
        self.command("INVALIDCOMMAND".into(), State::Synchronizing, now, &mut out);
        self.deadline = Some(now.saturating_add(1_000_000));
        out
    }
    fn next_init(&mut self, now: Micros, out: &mut Vec<Action>) {
        if let Some(mut command) = self.init.pop_front() {
            if command == "mac reset auto" {
                command =
                    if self.firmware.starts_with("RN2483") {
                        let frequency =
                            self.config.initialization.iter().rev().find_map(|c| {
                                c.strip_prefix("radio set freq ")?.parse::<u32>().ok()
                            });
                        format!(
                            "mac reset {}",
                            if frequency.is_some_and(|f| f < 500_000_000) {
                                433
                            } else {
                                868
                            }
                        )
                    } else {
                        "mac reset".into()
                    };
            }
            if command == "radio set pwr auto" {
                command = format!(
                    "radio set pwr {}",
                    if self.firmware.starts_with("RN2483") {
                        14
                    } else {
                        20
                    }
                );
            }
            self.command(command, State::Initializing, now, out);
        } else {
            self.start_rx(now, out);
        }
    }
    fn start_rx(&mut self, now: Micros, out: &mut Vec<Action>) {
        self.rx_ended_before_ack = false;
        self.command("radio rx 0".into(), State::StartingReceive, now, out);
    }
    pub fn enqueue(&mut self, id: u64, data: Vec<u8>, now: Micros) -> io::Result<()> {
        if self.state == State::Stopped || data.is_empty() || data.len() > MAX_FRAME {
            return Err(invalid("invalid or stopped radio transmission"));
        }
        let airtime = self.profile.airtime_us(data.len())?;
        let total = self
            .queue
            .iter()
            .chain(self.active.iter())
            .try_fold(airtime, |sum, p| {
                self.profile
                    .airtime_us(p.data.len())
                    .map(|n| sum.saturating_add(n))
            })?;
        if self.queued() >= self.config.queue_frames || total > self.config.queue_airtime_us {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "radio queue capacity exceeded",
            ));
        }
        self.queue.push_back(Packet {
            id,
            data,
            expires: now.saturating_add(self.config.max_packet_age_us),
        });
        Ok(())
    }
    pub fn disconnected(&mut self, now: Micros, reason: &str) -> Vec<Action> {
        if self.state == State::Stopped {
            return vec![];
        }
        let mut out = vec![Action::Diagnostic(reason.to_owned()), Action::Close];
        if let Some(p) = self.active.take() {
            out.push(Action::Failed(p.id, reason.to_owned()));
        }
        self.pending_command.clear();
        self.init.clear();
        self.state_to(State::Recovering, &mut out);
        self.deadline = Some(now.saturating_add(self.config.reconnect_delay_us));
        out
    }
    pub fn shutdown(&mut self) -> Vec<Action> {
        let mut out = vec![Action::Close];
        if let Some(p) = self.active.take() {
            out.push(Action::Failed(p.id, "shutdown".into()));
        }
        for p in self.queue.drain(..) {
            out.push(Action::Failed(p.id, "shutdown".into()));
        }
        self.deadline = None;
        self.state_to(State::Stopped, &mut out);
        out
    }
    pub fn tick(&mut self, now: Micros) -> Vec<Action> {
        if self.state == State::Stopped {
            return vec![];
        }
        let mut out = vec![];
        while self
            .queue
            .front()
            .map(|p| p.expires <= now)
            .unwrap_or(false)
        {
            let p = self.queue.pop_front().unwrap();
            out.push(Action::Failed(p.id, "queue deadline expired".into()));
        }
        if self.deadline.map(|d| now >= d).unwrap_or(false) {
            if self.state == State::Synchronizing {
                self.command("sys get ver".into(), State::Initializing, now, &mut out);
            } else if self.state == State::Recovering {
                self.deadline = None;
                self.state_to(State::Disconnected, &mut out);
                out.push(Action::Reconnect);
            } else {
                out.extend(self.disconnected(now, "radio command/completion timeout"));
            }
        } else if self.state == State::Receiving && now >= self.next_tx {
            if let Some(p) = self.queue.pop_front() {
                self.active = Some(p);
                self.command("radio rxstop".into(), State::StoppingReceive, now, &mut out);
            }
        }
        out
    }
    pub fn on_line(&mut self, line: &str, now: Micros) -> Vec<Action> {
        if matches!(
            self.state,
            State::Disconnected | State::Synchronizing | State::Recovering | State::Stopped
        ) {
            return vec![];
        }
        let reply = match Reply::parse(line) {
            Ok(r) => r,
            Err(e) => {
                let mut out = vec![Action::Diagnostic(e.to_string())];
                if line.trim_start().starts_with("radio_rx") {
                    if self.state == State::Receiving {
                        self.start_rx(now, &mut out);
                    } else if self.state == State::StartingReceive {
                        self.rx_ended_before_ack = true;
                    }
                }
                return out;
            }
        };
        let mut out = vec![];
        if let Reply::Rx(data) = reply {
            out.push(Action::Received(data));
            if self.state == State::Receiving {
                self.start_rx(now, &mut out);
            } else if self.state == State::StartingReceive {
                self.rx_ended_before_ack = true;
            }
            return out;
        }
        if matches!(reply, Reply::Busy | Reply::Invalid) {
            return self.disconnected(
                now,
                &format!("radio rejected {}: {:?}", self.pending_command, reply),
            );
        }
        if reply == Reply::RadioError {
            match self.state {
                State::Receiving => self.start_rx(now, &mut out),
                State::StartingReceive => {
                    self.rx_ended_before_ack = true;
                }
                State::AwaitingTxAck if !self.stale_tx_error_seen => {
                    // RN firmware can deliver one lingering RX watchdog error before
                    // the TX command's `ok`. Keep the original acknowledgment deadline.
                    self.stale_tx_error_seen = true;
                    out.push(Action::Diagnostic(
                        "ignored lingering receive error before TX acknowledgment".into(),
                    ));
                }
                State::Transmitting | State::AwaitingTxAck => {
                    return self.disconnected(now, "radio transmission failed");
                }
                // A receive watchdog event may precede the response to rxstop/rxstart.
                _ => out.push(Action::Diagnostic("asynchronous receive error".into())),
            }
            return out;
        }
        match self.state {
            State::Initializing => {
                let command = self.pending_command.clone();
                let valid = if command == "sys get ver" {
                    if let Reply::Value(v) = &reply {
                        if v.starts_with("RN2903 ") || v.starts_with("RN2483 ") {
                            self.firmware = v.clone();
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else if command == "mac pause" {
                    matches!(&reply,Reply::Value(v) if v.parse::<u32>().map(|n|n>0).unwrap_or(false))
                } else if let Some(field) = command.strip_prefix("radio get ") {
                    if let Reply::Value(v) = &reply {
                        if ["pwr", "wdt"].contains(&field) {
                            v.parse::<i64>().is_ok()
                        } else {
                            self.profile.update(field, v).is_ok()
                        }
                    } else {
                        false
                    }
                } else {
                    reply == Reply::Ok
                };
                if valid {
                    self.next_init(now, &mut out);
                } else {
                    return self.disconnected(
                        now,
                        &format!(
                            "unexpected initialization response to {}: {:?}",
                            command, reply
                        ),
                    );
                }
            }
            State::StartingReceive if reply == Reply::Ok => {
                self.deadline = None;
                if self.rx_ended_before_ack {
                    self.start_rx(now, &mut out);
                } else {
                    self.state_to(State::Receiving, &mut out);
                }
            }
            State::StoppingReceive if reply == Reply::Ok => {
                self.stale_tx_error_seen = false;
                let data = &self.active.as_ref().expect("stop RX owns a packet").data;
                self.command(
                    format!("radio tx {}", hex::encode(data)),
                    State::AwaitingTxAck,
                    now,
                    &mut out,
                );
            }
            State::AwaitingTxAck if reply == Reply::Ok => {
                let airtime = self
                    .profile
                    .airtime_us(self.active.as_ref().unwrap().data.len())
                    .expect("validated profile");
                self.state_to(State::Transmitting, &mut out);
                self.deadline = Some(
                    now.saturating_add(airtime)
                        .saturating_add(self.config.command_timeout_us),
                );
            }
            State::Transmitting if reply == Reply::TxDone => {
                if let Some(p) = self.active.take() {
                    out.push(Action::Transmitted(p.id));
                }
                self.next_tx = now.saturating_add(self.config.receive_guard_us);
                self.start_rx(now, &mut out);
            }
            _ => out.push(Action::Diagnostic(format!(
                "unsolicited response: {:?}",
                reply
            ))),
        }
        out
    }
}
