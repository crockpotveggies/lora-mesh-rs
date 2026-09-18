//! Accelerated link benchmark through production radio controllers and virtual LoStiks.
use super::{Config, Endpoint, Metrics};
use crate::{
    radio::{Action, Controller, LineCodec, RadioProfile},
    sim::{
        device::{Device, Firmware},
        medium::{Link, Medium},
        scenario::controller_config,
    },
};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, io};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Case {
    pub seed: u64,
    pub packets: usize,
    pub packet_size: usize,
    pub span: usize,
    pub batch: usize,
    pub profile: RadioProfile,
    pub loss_per_mille: u16,
    pub duplicate_per_mille: u16,
    pub delay_us: u64,
    pub jitter_us: u64,
    pub duty_per_mille: u16,
    pub bidirectional: bool,
    pub duration_us: u64,
    pub outage_us: Option<(u64, u64)>,
    pub fast_loss_retries: bool,
    pub clock_step_us: Option<u64>,
}
impl Default for Case {
    fn default() -> Self {
        Self {
            seed: 42,
            packets: 20,
            packet_size: 1500,
            span: super::MAX_SPAN,
            batch: 4,
            profile: RadioProfile {
                sf: 7,
                ..RadioProfile::default()
            },
            loss_per_mille: 0,
            duplicate_per_mille: 0,
            delay_us: 0,
            jitter_us: 0,
            duty_per_mille: 1000,
            bidirectional: false,
            duration_us: 3_600_000_000,
            outage_us: None,
            fast_loss_retries: true,
            clock_step_us: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub configuration: Case,
    pub elapsed_us: u64,
    pub delivered: usize,
    pub offered: usize,
    pub application_goodput_bytes_sec: f64,
    pub p50_latency_us: u64,
    pub p95_latency_us: u64,
    pub packet_loss: f64,
    pub metrics: Vec<Metrics>,
    pub medium_airtime_us: u64,
    pub collisions: u64,
}
/// Valid IPv4/UDP datagram with a stable payload marker and zero UDP checksum.
pub fn packet(size: usize, marker: u32) -> Vec<u8> {
    assert!((32..=1500).contains(&size));
    let mut p = vec![0; size];
    p[0] = 0x45;
    p[2..4].copy_from_slice(&(size as u16).to_be_bytes());
    p[8] = 64;
    p[9] = 17;
    p[12..16].copy_from_slice(&[10, 107, 0, 1]);
    p[16..20].copy_from_slice(&[10, 107, 0, 2]);
    p[20..22].copy_from_slice(&1234u16.to_be_bytes());
    p[22..24].copy_from_slice(&5678u16.to_be_bytes());
    p[24..26].copy_from_slice(&((size - 20) as u16).to_be_bytes());
    p[28..32].copy_from_slice(&marker.to_be_bytes());
    for (i, b) in p[32..].iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(73);
    }
    checksum(&mut p);
    p
}
pub fn checksum(p: &mut [u8]) {
    p[10] = 0;
    p[11] = 0;
    let ihl = usize::from(p[0] & 15) * 4;
    let mut sum = p[..ihl]
        .chunks_exact(2)
        .map(|w| u32::from(u16::from_be_bytes([w[0], w[1]])))
        .sum::<u32>();
    while sum >> 16 != 0 {
        sum = (sum & 65535) + (sum >> 16);
    }
    p[10..12].copy_from_slice(&(!(sum as u16)).to_be_bytes());
}
fn actions(
    index: usize,
    actions: Vec<Action>,
    medium: &mut Medium,
    radio: &mut Controller,
    link: &mut Endpoint,
) -> io::Result<()> {
    let mut pending: VecDeque<_> = actions.into();
    let id = index as u16 + 1;
    while let Some(action) = pending.pop_front() {
        match action {
            Action::Write(line) => medium.bytes(id, format!("{}\r\n", line).as_bytes())?,
            Action::Reconnect => pending.extend(radio.connected(medium.now)),
            Action::Received(bytes) => link.on_frame(&bytes, medium.now),
            Action::Transmitted(_) => link.transmitted(medium.now, true),
            Action::Failed(_, _) => link.transmitted(medium.now, false),
            _ => {}
        }
    }
    Ok(())
}
pub fn run(case: &Case) -> io::Result<Report> {
    if case.packets == 0
        || case.packets > 1000
        || case
            .clock_step_us
            .map(|s| s == 0 || s > 1_000_000)
            .unwrap_or(false)
        || !(32..=1500).contains(&case.packet_size)
        || case.loss_per_mille > 1000
        || case.duplicate_per_mille > 1000
        || case.delay_us > 60_000_000
        || case.jitter_us > 60_000_000
        || case.duration_us == 0
        || case.duration_us > 3_600_000_000
        || case
            .outage_us
            .map(|(start, end)| start >= end || end > case.duration_us)
            .unwrap_or(false)
    {
        return Err(crate::radio::protocol::invalid(
            "invalid link benchmark case",
        ));
    }
    case.profile.validate()?;
    let initial_airtime = (0..case.packet_size)
        .step_by(case.span.max(1))
        .map(|offset| {
            case.profile
                .airtime_us(super::HEADER + (case.packet_size - offset).min(case.span))
        })
        .collect::<io::Result<Vec<_>>>()?
        .iter()
        .sum::<u64>();
    if initial_airtime == 0 || initial_airtime > Config::default().queue_airtime_us {
        return Err(crate::radio::protocol::invalid(
            "benchmark packet exceeds admission airtime budget",
        ));
    }
    let generator_window = (Config::default().queue_airtime_us / initial_airtime).min(4) as usize;
    let mut medium = Medium::new(case.seed, 1_000_000);
    for id in 1..=2 {
        medium
            .devices
            .insert(id, Device::new(Firmware::RN2903, case.profile.clone()));
        medium.links.push(Link {
            from: id,
            to: 3 - id,
            loss_per_mille: case.loss_per_mille,
            duplicate_per_mille: case.duplicate_per_mille,
            delay_us: case.delay_us,
            jitter_us: case.jitter_us,
            enabled: true,
        });
    }
    let mut radios = Vec::new();
    let mut links = Vec::new();
    let mut codecs = [LineCodec::default(), LineCodec::default()];
    for i in 0..2 {
        let mut rc = controller_config(&case.profile);
        rc.receive_guard_us = 0;
        radios.push(Controller::new(rc)?);
        links.push(Endpoint::new(Config {
            node: i + 1,
            peer: 2 - i,
            session: case.seed.wrapping_add(u64::from(i) + 1).max(1),
            profile: case.profile.clone(),
            span: case.span,
            batch: case.batch,
            duty_per_mille: case.duty_per_mille,
            fast_loss_retries: case.fast_loss_retries,
            ..Config::default()
        })?);
    }
    for i in 0..2 {
        actions(
            i,
            vec![Action::Reconnect],
            &mut medium,
            &mut radios[i],
            &mut links[i],
        )?;
    }
    let mut offered = [0usize; 2];
    let mut born = [vec![0u64; case.packets], vec![0u64; case.packets]];
    let mut latencies = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut finished = false;
    for _ in 0..1_000_000 {
        medium.advance(medium.now)?;
        if let Some((start, end)) = case.outage_us {
            for l in &mut medium.links {
                l.enabled = !(start <= medium.now && medium.now < end);
            }
        }
        for i in 0..2 {
            for byte in medium.drain(i as u16 + 1, 4096) {
                if let Some(line) = codecs[i].push(byte) {
                    let a = radios[i].on_line(&line?, medium.now);
                    actions(i, a, &mut medium, &mut radios[i], &mut links[i])?;
                }
            }
            links[i].tick(medium.now);
            // Closed-loop load: keep four packets queued; record ingress-to-delivery latency.
            while (i == 0 || case.bidirectional)
                && offered[i] < case.packets
                && links[i].queued() < generator_window
                && medium.now >= 100_000
            {
                let n = offered[i];
                if links[i]
                    .enqueue(packet(case.packet_size, n as u32), medium.now)
                    .is_ok()
                {
                    born[i][n] = medium.now;
                    offered[i] += 1;
                } else {
                    break;
                }
            }
            if radios[i].state() == crate::radio::State::Receiving && radios[i].queued() == 0 {
                if let Some(bytes) = links[i].poll_transmit(medium.now) {
                    if radios[i].enqueue(0, bytes, medium.now).is_err() {
                        links[i].transmitted(medium.now, false);
                    }
                }
            } else {
                links[i].tick(medium.now);
            }
            let a = radios[i].tick(medium.now);
            actions(i, a, &mut medium, &mut radios[i], &mut links[i])?;
            while let Some(p) = links[i].receive_packet() {
                let n = u32::from_be_bytes([p[28], p[29], p[30], p[31]]) as usize;
                if n >= case.packets
                    || !seen.insert((i, n))
                    || p != packet(case.packet_size, n as u32)
                {
                    return Err(crate::radio::protocol::invalid(
                        "corrupt or duplicate delivery",
                    ));
                }
                latencies.push(medium.now - born[1 - i][n]);
            }
        }
        if offered[0] == case.packets
            && (!case.bidirectional || offered[1] == case.packets)
            && links.iter().all(|l| l.queued() == 0)
            && radios.iter().all(|r| r.queued() == 0)
        {
            finished = true;
            break;
        }
        if medium.now >= case.duration_us {
            finished = true;
            break;
        }
        let pending = medium.devices.values().any(|d| !d.output.is_empty());
        let next = if pending {
            medium.now
        } else {
            std::iter::once(Some(100_000))
                .chain(std::iter::once(medium.next_event()))
                .chain(
                    case.outage_us
                        .into_iter()
                        .flat_map(|(s, e)| [Some(s), Some(e)]),
                )
                .chain(radios.iter().map(Controller::next_deadline))
                .chain(links.iter().map(Endpoint::next_deadline))
                .flatten()
                .filter(|d| *d > medium.now)
                .min()
                .unwrap_or(case.duration_us)
                .min(
                    medium
                        .next_event()
                        .unwrap_or(case.duration_us)
                        .max(medium.now),
                )
        };
        let next = case
            .clock_step_us
            .map(|step| next.min(medium.now + step))
            .unwrap_or(next);
        medium.advance(next.min(case.duration_us))?;
    }
    if !finished {
        return Err(crate::radio::protocol::invalid(
            "link scenario event budget exhausted",
        ));
    }
    latencies.sort_unstable();
    let percentile = |pct: usize| {
        latencies
            .get((latencies.len() * pct).div_ceil(100).saturating_sub(1))
            .copied()
            .unwrap_or(0)
    };
    let delivered = latencies.len();
    let offered = offered.iter().sum::<usize>();
    let metrics = links.into_iter().map(|l| l.metrics).collect::<Vec<_>>();
    Ok(Report {
        configuration: case.clone(),
        elapsed_us: medium.now,
        delivered,
        offered,
        application_goodput_bytes_sec: (delivered * (case.packet_size - 28)) as f64 * 1_000_000.0
            / medium.now.max(1) as f64,
        p50_latency_us: percentile(50),
        p95_latency_us: percentile(95),
        packet_loss: 1.0 - delivered as f64 / offered.max(1) as f64,
        medium_airtime_us: medium.metrics.values().map(|m| m.airtime_us).sum(),
        collisions: medium.metrics.values().map(|m| m.collisions).sum(),
        metrics,
    })
}
