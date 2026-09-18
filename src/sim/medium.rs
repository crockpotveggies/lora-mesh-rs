use super::device::{Device, Effect, Mode};
use crate::radio::protocol::{Micros, RadioProfile, invalid};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Trace {
    pub at_us: Micros,
    pub node: u16,
    pub event: String,
    pub detail: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Link {
    pub from: u16,
    pub to: u16,
    pub loss_per_mille: u16,
    pub delay_us: Micros,
    pub jitter_us: Micros,
    pub duplicate_per_mille: u16,
    pub enabled: bool,
}
impl Default for Link {
    fn default() -> Self {
        Self {
            from: 0,
            to: 0,
            loss_per_mille: 0,
            delay_us: 0,
            jitter_us: 0,
            duplicate_per_mille: 0,
            enabled: true,
        }
    }
}
#[derive(Clone, Debug)]
struct Reception {
    node: u16,
    epoch: u64,
    collision: bool,
}
struct Transmission {
    started: Micros,
    source: u16,
    epoch: u64,
    profile: RadioProfile,
    data: Vec<u8>,
    receivers: Vec<Reception>,
    watchdog: bool,
}
enum Event {
    Line(u16, u64, String, Option<(Micros, usize)>),
    TxEnd(u64),
    RxTimeout(u16, u64),
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediumMetrics {
    pub transmitted_frames: u64,
    pub airtime_us: u64,
    pub delivered_frames: u64,
    pub delivered_bytes: u64,
    pub delivery_latency_us: Vec<Micros>,
    pub collisions: u64,
    pub injected_loss: u64,
    pub unavailable: u64,
    pub serial_output_peak: usize,
}
/// Bounded event scheduler. All maps and tie-breaks use stable ordering.
pub struct Medium {
    pub now: Micros,
    pub devices: BTreeMap<u16, Device>,
    pub links: Vec<Link>,
    pub trace: Vec<Trace>,
    pub metrics: BTreeMap<u16, MediumMetrics>,
    events: BTreeMap<(Micros, u64), Event>,
    tx: BTreeMap<u64, Transmission>,
    sequence: u64,
    rng: u64,
    max_events: usize,
    processed: usize,
}
impl Medium {
    pub fn new(seed: u64, max_events: usize) -> Self {
        Self {
            now: 0,
            devices: BTreeMap::new(),
            links: vec![],
            trace: vec![],
            metrics: BTreeMap::new(),
            events: BTreeMap::new(),
            tx: BTreeMap::new(),
            sequence: 0,
            rng: seed.max(1),
            max_events,
            processed: 0,
        }
    }
    pub fn record(&mut self, node: u16, event: &str, detail: String) -> io::Result<()> {
        if self.trace.len() >= self.max_events {
            return Err(invalid("trace/event budget exceeded"));
        }
        self.trace.push(Trace {
            at_us: self.now,
            node,
            event: event.into(),
            detail,
        });
        Ok(())
    }
    fn schedule(&mut self, at: Micros, event: Event) -> io::Result<()> {
        if self.events.len() >= self.max_events {
            return Err(invalid("pending event budget exceeded"));
        }
        self.sequence += 1;
        self.events.insert((at, self.sequence), event);
        Ok(())
    }
    fn chance(&mut self, per_mille: u16) -> bool {
        self.random() % 1000 < u64::from(per_mille)
    }
    fn random(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }
    pub fn next_event(&self) -> Option<Micros> {
        self.events.keys().next().map(|k| k.0)
    }
    pub fn device_line(&mut self, node: u16, line: String) -> io::Result<()> {
        let d = self
            .devices
            .get_mut(&node)
            .ok_or_else(|| invalid("unknown device"))?;
        if d.drop_replies > 0 {
            d.drop_replies -= 1;
            return self.record(node, "serial_drop", line);
        }
        let delay = d.reply_delay_us;
        self.schedule(
            self.now.saturating_add(delay),
            Event::Line(node, self.devices[&node].generation, line, None),
        )
    }
    pub fn inject_line(&mut self, node: u16, line: String, delay: Micros) -> io::Result<()> {
        if line.len() > 600 || !self.devices.contains_key(&node) {
            return Err(invalid("invalid injected line"));
        }
        self.record(node, "injected_serial", line.clone())?;
        self.schedule(
            self.now.saturating_add(delay),
            Event::Line(node, self.devices[&node].generation, line, None),
        )
    }
    pub fn bytes(&mut self, node: u16, bytes: &[u8]) -> io::Result<()> {
        for byte in bytes {
            let d = self
                .devices
                .get_mut(&node)
                .ok_or_else(|| invalid("unknown device"))?;
            if !d.online {
                continue;
            }
            if let Some(line) = d.codec.push(*byte) {
                match line {
                    Ok(line) => self.command(node, &line)?,
                    Err(_) => self.device_line(node, "invalid_param".into())?,
                }
            }
        }
        Ok(())
    }
    pub fn command(&mut self, node: u16, command: &str) -> io::Result<()> {
        self.record(node, "command", command.to_owned())?;
        let actions = self
            .devices
            .get_mut(&node)
            .ok_or_else(|| invalid("unknown device"))?
            .command(command);
        for action in actions {
            match action {
                Effect::Line(line) => self.device_line(node, line)?,
                Effect::ReceiveTimer(delay, epoch) => self.schedule(
                    self.now.saturating_add(delay),
                    Event::RxTimeout(node, epoch),
                )?,
                Effect::Transmit(data) => self.transmit(node, data)?,
            }
        }
        Ok(())
    }
    fn reachable(&self, from: u16, to: u16) -> bool {
        self.links
            .iter()
            .any(|l| l.from == from && l.to == to && l.enabled)
    }
    fn transmit(&mut self, node: u16, data: Vec<u8>) -> io::Result<()> {
        let d = &self.devices[&node];
        let profile = d.profile.clone();
        let epoch = d.epoch;
        let airtime = profile.airtime_us(data.len())?;
        let watchdog = d.watchdog_us > 0 && d.watchdog_us < airtime;
        let duration = if watchdog { d.watchdog_us } else { airtime };
        let mut receivers = vec![];
        for (&id, other) in &self.devices {
            if id != node
                && other.online
                && other.mode == Mode::Receive
                && other.profile.compatible(&profile)
                && self.reachable(node, id)
            {
                let collision = self.tx.values().any(|tx| {
                    self.devices[&tx.source].online
                        && self.devices[&tx.source].epoch == tx.epoch
                        && tx.profile.compatible(&profile)
                        && self.reachable(tx.source, id)
                });
                receivers.push(Reception {
                    node: id,
                    epoch: other.epoch,
                    collision,
                });
            }
        }
        let links = &self.links;
        for tx in self.tx.values_mut() {
            if self.devices[&tx.source].online
                && self.devices[&tx.source].epoch == tx.epoch
                && tx.profile.compatible(&profile)
            {
                for rx in &mut tx.receivers {
                    if links
                        .iter()
                        .any(|l| l.from == node && l.to == rx.node && l.enabled)
                    {
                        rx.collision = true;
                    }
                }
            }
        }
        let metrics = self.metrics.entry(node).or_default();
        metrics.transmitted_frames += 1;
        metrics.airtime_us += duration;
        self.record(
            node,
            "tx_start",
            format!("{} bytes, {} us", data.len(), duration),
        )?;
        self.sequence += 1;
        let token = self.sequence;
        self.tx.insert(
            token,
            Transmission {
                started: self.now,
                source: node,
                epoch,
                profile,
                data,
                receivers,
                watchdog,
            },
        );
        self.schedule(self.now.saturating_add(duration), Event::TxEnd(token))
    }
    pub fn drain(&mut self, node: u16, max: usize) -> Vec<u8> {
        let d = self.devices.get_mut(&node).expect("known node");
        let count = max.min(d.output.len());
        d.output.drain(..count).collect()
    }
    pub fn connection(&mut self, node: u16, online: bool) -> io::Result<()> {
        let d = self
            .devices
            .get_mut(&node)
            .ok_or_else(|| invalid("unknown device"))?;
        d.invalidate();
        d.generation += 1;
        d.online = online;
        d.paused = false;
        d.output.clear();
        self.record(
            node,
            if online { "connect" } else { "disconnect" },
            String::new(),
        )
    }
    pub fn advance(&mut self, until: Micros) -> io::Result<()> {
        if until < self.now {
            return Err(invalid("time cannot go backwards"));
        }
        while let Some((&key, _)) = self.events.iter().next() {
            if key.0 > until {
                break;
            }
            self.processed += 1;
            if self.processed > self.max_events {
                return Err(invalid("processed event budget exceeded"));
            }
            self.now = key.0;
            let event = self.events.remove(&key).unwrap();
            match event {
                Event::Line(node, generation, line, delivery) => {
                    if let Some(d) = self.devices.get_mut(&node) {
                        if d.online && d.generation == generation {
                            if d.output.len() + line.len() + 2 > 16384 {
                                return Err(invalid("virtual serial output overflow"));
                            }
                            // A fault-injected repeated receive notification also ends
                            // the current receive operation, as a real radio_rx does.
                            if delivery.is_some() && d.mode == Mode::Receive {
                                d.invalidate();
                            }
                            d.output.extend(line.bytes());
                            d.output.extend(b"\r\n");
                            let metrics = self.metrics.entry(node).or_default();
                            metrics.serial_output_peak =
                                metrics.serial_output_peak.max(d.output.len());
                            if let Some((started, length)) = delivery {
                                metrics.delivered_frames += 1;
                                metrics.delivered_bytes += length as u64;
                                metrics.delivery_latency_us.push(self.now - started);
                            }
                            self.record(node, "serial", line)?;
                        }
                    }
                }
                Event::RxTimeout(node, epoch) => {
                    let d = self.devices.get_mut(&node).unwrap();
                    if d.online && d.epoch == epoch && d.mode == Mode::Receive {
                        d.invalidate();
                        self.device_line(node, "radio_err".into())?;
                    }
                }
                Event::TxEnd(token) => {
                    let tx = self.tx.remove(&token).unwrap();
                    let d = &self.devices[&tx.source];
                    if !d.online || d.epoch != tx.epoch || d.mode != Mode::Transmit {
                        continue;
                    }
                    self.devices.get_mut(&tx.source).unwrap().invalidate();
                    self.record(
                        tx.source,
                        "tx_end",
                        if tx.watchdog { "watchdog" } else { "complete" }.into(),
                    )?;
                    self.device_line(
                        tx.source,
                        if tx.watchdog {
                            "radio_err"
                        } else {
                            "radio_tx_ok"
                        }
                        .into(),
                    )?;
                    if tx.watchdog {
                        continue;
                    }
                    for rx in &tx.receivers {
                        let d = &self.devices[&rx.node];
                        let link = self
                            .links
                            .iter()
                            .find(|l| l.from == tx.source && l.to == rx.node)
                            .cloned()
                            .unwrap();
                        let reason = if rx.collision {
                            Some("collision")
                        } else if !d.online || d.epoch != rx.epoch || d.mode != Mode::Receive {
                            Some("receiver unavailable")
                        } else if !link.enabled {
                            Some("link outage")
                        } else if self.chance(link.loss_per_mille) {
                            Some("injected loss")
                        } else {
                            None
                        };
                        if let Some(reason) = reason {
                            let metrics = self.metrics.entry(rx.node).or_default();
                            match reason {
                                "collision" => metrics.collisions += 1,
                                "injected loss" => metrics.injected_loss += 1,
                                _ => metrics.unavailable += 1,
                            }
                            self.record(rx.node, "drop", reason.into())?;
                            continue;
                        }
                        self.devices.get_mut(&rx.node).unwrap().invalidate();
                        let delay = link.delay_us
                            + if link.jitter_us > 0 {
                                self.random() % (link.jitter_us + 1)
                            } else {
                                0
                            };
                        let line = format!("radio_rx {}", hex::encode(&tx.data));
                        self.record(
                            rx.node,
                            "delivery",
                            format!("from {}: {} bytes", tx.source, tx.data.len()),
                        )?;
                        self.schedule(
                            self.now.saturating_add(delay),
                            Event::Line(
                                rx.node,
                                self.devices[&rx.node].generation,
                                line.clone(),
                                Some((tx.started, tx.data.len())),
                            ),
                        )?;
                        if self.chance(link.duplicate_per_mille) {
                            self.record(rx.node, "injected_duplicate", line.clone())?;
                            self.schedule(
                                self.now.saturating_add(delay + 1),
                                Event::Line(
                                    rx.node,
                                    self.devices[&rx.node].generation,
                                    line,
                                    Some((tx.started, tx.data.len())),
                                ),
                            )?;
                        }
                    }
                }
            }
        }
        self.now = until;
        Ok(())
    }
}
