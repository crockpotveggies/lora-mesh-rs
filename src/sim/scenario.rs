use super::{
    device::{Device, Firmware},
    medium::{Link, Medium, Trace},
};
use crate::radio::{
    Action, Controller, ControllerConfig, LineCodec,
    protocol::{Micros, RadioProfile, invalid},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSpec {
    pub id: u16,
    #[serde(default)]
    pub firmware: Firmware,
    #[serde(default)]
    pub profile: RadioProfile,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Traffic {
    pub at_us: Micros,
    pub from: u16,
    pub payload: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Fault {
    Disconnect {
        at_us: Micros,
        node: u16,
    },
    Reconnect {
        at_us: Micros,
        node: u16,
    },
    DropReplies {
        at_us: Micros,
        node: u16,
        count: u32,
    },
    DelayReplies {
        at_us: Micros,
        node: u16,
        delay_us: Micros,
    },
    Serial {
        at_us: Micros,
        node: u16,
        line: String,
    },
    Link {
        at_us: Micros,
        from: u16,
        to: u16,
        enabled: bool,
    },
}
impl Fault {
    pub fn at(&self) -> Micros {
        match *self {
            Self::Disconnect { at_us, .. }
            | Self::Reconnect { at_us, .. }
            | Self::DropReplies { at_us, .. }
            | Self::DelayReplies { at_us, .. }
            | Self::Serial { at_us, .. }
            | Self::Link { at_us, .. } => at_us,
        }
    }
    pub fn apply(&self, medium: &mut Medium) -> io::Result<()> {
        match self {
            Self::Disconnect { node, .. } => medium.connection(*node, false),
            Self::Reconnect { node, .. } => medium.connection(*node, true),
            Self::DropReplies { node, count, .. } => {
                medium
                    .devices
                    .get_mut(node)
                    .ok_or_else(|| invalid("unknown fault node"))?
                    .drop_replies = *count;
                Ok(())
            }
            Self::DelayReplies { node, delay_us, .. } => {
                medium
                    .devices
                    .get_mut(node)
                    .ok_or_else(|| invalid("unknown fault node"))?
                    .reply_delay_us = *delay_us;
                Ok(())
            }
            Self::Serial { node, line, .. } => medium.inject_line(*node, line.clone(), 0),
            Self::Link {
                from, to, enabled, ..
            } => {
                let l = medium
                    .links
                    .iter_mut()
                    .find(|l| l.from == *from && l.to == *to)
                    .ok_or_else(|| invalid("unknown fault link"))?;
                l.enabled = *enabled;
                Ok(())
            }
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Expect {
    pub received: BTreeMap<u16, u64>,
    pub transmitted: Option<u64>,
    pub failed: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub version: u32,
    pub name: String,
    pub seed: u64,
    pub duration_us: Micros,
    pub max_events: usize,
    pub nodes: Vec<NodeSpec>,
    pub links: Vec<Link>,
    #[serde(default)]
    pub traffic: Vec<Traffic>,
    #[serde(default)]
    pub faults: Vec<Fault>,
    #[serde(default)]
    pub expect: Expect,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeMetrics {
    pub received: u64,
    pub received_bytes: u64,
    pub transmitted: u64,
    pub failed: u64,
    pub queue_peak: usize,
    pub received_payloads: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Report {
    pub version: u32,
    pub scenario: String,
    pub configuration: serde_json::Value,
    pub seed: u64,
    pub duration_us: Micros,
    pub nodes: BTreeMap<u16, NodeMetrics>,
    pub trace: Vec<Trace>,
    pub medium: BTreeMap<u16, super::medium::MediumMetrics>,
    pub failures: Vec<String>,
}
impl Scenario {
    pub fn validate(&self) -> io::Result<()> {
        if self.version != 1
            || self.nodes.is_empty()
            || self.nodes.len() > 128
            || self.duration_us == 0
            || self.duration_us > 3_600_000_000
            || self.max_events == 0
            || self.max_events > 1_000_000
            || self.traffic.len() > 100_000
            || self.faults.len() > 100_000
            || self.name.len() > 128
        {
            return Err(invalid("unsupported scenario version or resource limits"));
        }
        let mut ids = BTreeSet::new();
        for node in &self.nodes {
            if !ids.insert(node.id) {
                return Err(invalid("duplicate node id"));
            }
            node.profile.validate()?;
            let freq = node.profile.frequency;
            let legal = match node.firmware {
                Firmware::RN2903 => (902_000_000..=928_000_000).contains(&freq),
                Firmware::RN2483 => {
                    (433_050_000..=434_790_000).contains(&freq)
                        || (863_000_000..=870_000_000).contains(&freq)
                }
            };
            if !legal {
                return Err(invalid(
                    "profile frequency incompatible with virtual firmware",
                ));
            }
        }
        let mut pairs = BTreeSet::new();
        for link in &self.links {
            if !ids.contains(&link.from)
                || !ids.contains(&link.to)
                || link.from == link.to
                || !pairs.insert((link.from, link.to))
                || link.loss_per_mille > 1000
                || link.duplicate_per_mille > 1000
                || link.delay_us > 60_000_000
                || link.jitter_us > 60_000_000
            {
                return Err(invalid("invalid directed link"));
            }
        }
        for packet in &self.traffic {
            if !ids.contains(&packet.from)
                || packet.at_us >= self.duration_us
                || packet.payload.is_empty()
                || packet.payload.len() > 510
                || hex::decode(&packet.payload).is_err()
            {
                return Err(invalid("invalid traffic"));
            }
        }
        for fault in &self.faults {
            if fault.at() >= self.duration_us {
                return Err(invalid("fault after scenario end"));
            }
            let node = match fault {
                Fault::Disconnect { node, .. }
                | Fault::Reconnect { node, .. }
                | Fault::DropReplies { node, .. } => Some(node),
                Fault::DelayReplies { node, delay_us, .. } => {
                    if *delay_us > 60_000_000 {
                        return Err(invalid("reply delay too large"));
                    }
                    Some(node)
                }
                Fault::Serial { node, line, .. } => {
                    if line.len() > 600 || line.contains(['\r', '\n']) {
                        return Err(invalid("invalid serial fault"));
                    }
                    Some(node)
                }
                Fault::Link { from, to, .. } => {
                    if !pairs.contains(&(*from, *to)) {
                        return Err(invalid("unknown fault link"));
                    }
                    None
                }
            };
            if node.map(|n| !ids.contains(n)).unwrap_or(false) {
                return Err(invalid("unknown fault node"));
            }
        }
        if self.expect.received.keys().any(|id| !ids.contains(id)) {
            return Err(invalid("unknown expectation node"));
        }
        Ok(())
    }
    pub fn medium(&self) -> io::Result<Medium> {
        self.validate()?;
        let mut medium = Medium::new(self.seed, self.max_events);
        medium.links = self.links.clone();
        medium.links.sort_by_key(|l| (l.from, l.to));
        for node in &self.nodes {
            medium
                .devices
                .insert(node.id, Device::new(node.firmware, node.profile.clone()));
        }
        Ok(medium)
    }
}
pub fn controller_config(profile: &RadioProfile) -> ControllerConfig {
    let mut config = ControllerConfig {
        receive_guard_us: 10_000,
        initialization: vec![
            "mac pause".into(),
            "radio set mod lora".into(),
            "radio set wdt 60000".into(),
        ],
        ..ControllerConfig::default()
    };
    for field in ["freq", "sf", "bw", "cr", "prlen", "crc", "sync"] {
        config.initialization.push(format!(
            "radio set {} {}",
            field,
            profile.get(field).unwrap()
        ));
    }
    config
}
fn actions(
    node: u16,
    actions: Vec<Action>,
    medium: &mut Medium,
    controllers: &mut BTreeMap<u16, Controller>,
    metrics: &mut BTreeMap<u16, NodeMetrics>,
) -> io::Result<()> {
    let mut pending: std::collections::VecDeque<_> = actions.into();
    while let Some(action) = pending.pop_front() {
        match action {
            Action::Write(line) => {
                medium.bytes(node, format!("{}\r\n", line).as_bytes())?;
            }
            Action::Reconnect => {
                if medium.devices[&node].online {
                    pending.extend(controllers.get_mut(&node).unwrap().connected(medium.now));
                } else {
                    pending.extend(
                        controllers
                            .get_mut(&node)
                            .unwrap()
                            .disconnected(medium.now, "virtual device offline"),
                    );
                }
            }
            Action::Close => {}
            Action::Received(data) => {
                let m = metrics.get_mut(&node).unwrap();
                m.received += 1;
                m.received_bytes += data.len() as u64;
                m.received_payloads.push(hex::encode(&data));
                medium.record(node, "host_rx", hex::encode(data))?;
            }
            Action::Transmitted(id) => {
                metrics.get_mut(&node).unwrap().transmitted += 1;
                medium.record(node, "host_tx", id.to_string())?;
            }
            Action::Failed(id, reason) => {
                metrics.get_mut(&node).unwrap().failed += 1;
                medium.record(node, "host_failed", format!("{}: {}", id, reason))?;
            }
            Action::Diagnostic(message) => medium.record(node, "diagnostic", message)?,
            Action::State(state) => medium.record(node, "state", format!("{:?}", state))?,
        }
    }
    Ok(())
}
pub fn run(scenario: &Scenario) -> io::Result<Report> {
    let mut medium = scenario.medium()?;
    let mut controllers = BTreeMap::new();
    let mut codecs = BTreeMap::new();
    let mut metrics = BTreeMap::new();
    for node in &scenario.nodes {
        controllers.insert(node.id, Controller::new(controller_config(&node.profile))?);
        codecs.insert(node.id, LineCodec::default());
        metrics.insert(node.id, NodeMetrics::default());
    }
    let ids: Vec<_> = controllers.keys().copied().collect();
    // Stable file order breaks ties between faults/traffic at the same timestamp.
    let mut faults = scenario.faults.clone();
    faults.sort_by_key(Fault::at);
    let mut fi = 0;
    let mut traffic = scenario.traffic.clone();
    traffic.sort_by_key(|t| t.at_us);
    let mut ti = 0;
    for &id in &ids {
        actions(
            id,
            vec![Action::Reconnect],
            &mut medium,
            &mut controllers,
            &mut metrics,
        )?;
    }
    let mut iterations = 0;
    loop {
        iterations += 1;
        if iterations > scenario.max_events {
            return Err(invalid("scenario stalled or event budget exceeded"));
        }
        while fi < faults.len() && faults[fi].at() <= medium.now {
            faults[fi].apply(&mut medium)?;
            if let Fault::Disconnect { node, .. } = faults[fi] {
                codecs.get_mut(&node).unwrap().reset();
                let a = controllers
                    .get_mut(&node)
                    .unwrap()
                    .disconnected(medium.now, "injected disconnect");
                actions(node, a, &mut medium, &mut controllers, &mut metrics)?;
            }
            fi += 1;
        }
        while ti < traffic.len() && traffic[ti].at_us <= medium.now {
            let packet = &traffic[ti];
            let id = packet.from;
            if let Err(e) = controllers.get_mut(&id).unwrap().enqueue(
                ti as u64,
                hex::decode(&packet.payload).unwrap(),
                medium.now,
            ) {
                actions(
                    id,
                    vec![Action::Failed(ti as u64, e.to_string())],
                    &mut medium,
                    &mut controllers,
                    &mut metrics,
                )?;
            }
            let m = metrics.get_mut(&id).unwrap();
            m.queue_peak = m.queue_peak.max(controllers[&id].queued());
            ti += 1;
        }
        medium.advance(medium.now)?;
        for &id in &ids {
            // Force partial byte reads, rather than handing complete replies to the controller.
            let bytes = medium.drain(id, 7);
            for byte in bytes {
                if let Some(line) = codecs.get_mut(&id).unwrap().push(byte) {
                    let controller = controllers.get_mut(&id).unwrap();
                    let a = match line {
                        Ok(line) => controller.on_line(&line, medium.now),
                        Err(e) => {
                            codecs.get_mut(&id).unwrap().reset();
                            controller.disconnected(medium.now, &e.to_string())
                        }
                    };
                    actions(id, a, &mut medium, &mut controllers, &mut metrics)?;
                }
            }
            let a = controllers.get_mut(&id).unwrap().tick(medium.now);
            actions(id, a, &mut medium, &mut controllers, &mut metrics)?;
        }
        if medium.now >= scenario.duration_us {
            break;
        }
        let buffered = medium.devices.values().any(|d| !d.output.is_empty());
        let next = if buffered {
            medium.now
        } else {
            [
                medium.next_event(),
                controllers
                    .values()
                    .filter_map(Controller::next_deadline)
                    .min(),
                faults.get(fi).map(Fault::at),
                traffic.get(ti).map(|t| t.at_us),
                Some(scenario.duration_us),
            ]
            .iter()
            .filter_map(|v| *v)
            .min()
            .unwrap()
            .max(medium.now)
        };
        medium.advance(next.min(scenario.duration_us))?;
    }
    for &id in &ids {
        let a = controllers.get_mut(&id).unwrap().shutdown();
        actions(id, a, &mut medium, &mut controllers, &mut metrics)?;
    }
    let mut failures = vec![];
    for (node, expected) in &scenario.expect.received {
        if metrics[node].received != *expected {
            failures.push(format!(
                "node {}: expected {} received, got {}",
                node, expected, metrics[node].received
            ));
        }
    }
    let transmitted = metrics.values().map(|m| m.transmitted).sum::<u64>();
    let failed = metrics.values().map(|m| m.failed).sum::<u64>();
    if scenario
        .expect
        .transmitted
        .map(|n| n != transmitted)
        .unwrap_or(false)
    {
        failures.push(format!(
            "expected {:?} transmissions, got {}",
            scenario.expect.transmitted, transmitted
        ));
    }
    if scenario.expect.failed.map(|n| n != failed).unwrap_or(false) {
        failures.push(format!(
            "expected {:?} failures, got {}",
            scenario.expect.failed, failed
        ));
    }
    Ok(Report {
        version: 1,
        scenario: scenario.name.clone(),
        configuration: serde_json::to_value(scenario)?,
        seed: scenario.seed,
        duration_us: scenario.duration_us,
        nodes: metrics,
        trace: medium.trace,
        medium: medium.metrics,
        failures,
    })
}
