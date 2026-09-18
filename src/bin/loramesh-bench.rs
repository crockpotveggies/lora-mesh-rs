//! Reproducible simulated goodput matrix; wall time measures simulator cost separately.
use loramesh::link::simulation::{Case, run};
use serde_json::json;
use std::{hint::black_box, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mut cases = Vec::new();
    for arg in args.iter().filter(|a| a.starts_with("--")) {
        if !["--fixed-clock", "--conservative-retries"].contains(&arg.as_str()) {
            return Err("unknown benchmark option".into());
        }
    }
    if let Some(path) = args.iter().find(|a| !a.starts_with("--")) {
        cases.push(serde_json::from_slice::<Case>(&std::fs::read(path)?)?);
    } else {
        for sf in [7, 9, 12] {
            for span in [64, 128, 209] {
                for batch in [1, 4, 8] {
                    for loss in [0, 100] {
                        let mut c = Case {
                            span,
                            batch,
                            loss_per_mille: loss,
                            packets: 12,
                            ..Case::default()
                        };
                        c.profile.sf = sf;
                        cases.push(c);
                    }
                }
            }
        }
    }
    let started = Instant::now();
    let mut reports = Vec::new();
    for mut case in cases {
        if args.iter().any(|a| a == "--fixed-clock") {
            case.clock_step_us = Some(10_000);
        }
        if args.iter().any(|a| a == "--conservative-retries") {
            case.fast_loss_retries = false;
        }
        reports.push(run(&case)?);
    }
    let simulation_wall_us = started.elapsed().as_micros();
    let packet = loramesh::link::simulation::packet(1500, 0);
    let started = Instant::now();
    for _ in 0..100_000 {
        black_box(loramesh::link::wire::ipv4(black_box(&packet)))?;
    }
    let ipv4_validation_ns = started.elapsed().as_nanos() / 100_000;
    let started = Instant::now();
    let mut receiver = loramesh::link::Endpoint::new(loramesh::link::Config {
        node: 2,
        peer: 1,
        ..Default::default()
    })?;
    for sequence in 0..10_000u32 {
        for (index, payload) in packet.chunks(209).enumerate() {
            let h = loramesh::link::wire::Header {
                network: 1,
                source: 1,
                destination: 2,
                session: 1,
                sequence,
                total: 1500,
                index: index as u8,
                count: 8,
                span: 209,
                ack: false,
                request_ack: index == 7,
            };
            receiver.on_frame(black_box(&h.encode(payload)?), 0);
        }
        black_box(
            receiver
                .receive_packet()
                .ok_or("benchmark delivery missing")?,
        );
    }
    let encode_reassemble_ns = started.elapsed().as_nanos() / 10_000;
    let usage = loramesh::link::daemon::process_usage();
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"version":1,"simulation_wall_us":simulation_wall_us,"ipv4_validation_ns":ipv4_validation_ns,"encode_reassemble_ns":encode_reassemble_ns,"process_usage":usage,"reports":reports})
        )?
    );
    Ok(())
}
