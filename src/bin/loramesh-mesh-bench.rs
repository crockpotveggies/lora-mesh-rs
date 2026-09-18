#[cfg(unix)]
mod unix {
    //! Crypto, durable replay, routing-cache and on-medium scenario benchmarks; clocks kept separate.
    use loramesh::{
        link::{simulation::packet, wire::Header},
        mesh::{
            security::{SPAN, Vault},
            simulation::{self, Case, pair_key},
        },
    };
    use std::{collections::BTreeMap, hint::black_box, time::Instant};
    pub fn main() -> Result<(), Box<dyn std::error::Error>> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        let mut runs = Vec::new();
        for input in [
            include_str!("../../scenarios/mesh/line.json"),
            include_str!("../../scenarios/mesh/triangle.json"),
            include_str!("../../scenarios/mesh/diamond.json"),
            include_str!("../../scenarios/mesh/hidden-terminal.json"),
        ] {
            let mut case: Case = serde_json::from_str(input)?;
            case.cache_routes = !args.iter().any(|a| a == "--uncached");
            if args.iter().any(|a| a == "--frequent-control") {
                case.announce_us = 10_000_000;
                case.neighbor_timeout_us = 40_000_000;
            }
            let start = Instant::now();
            let mut report = simulation::run(&case)?;
            let host_us = start.elapsed().as_micros();
            report.trace.clear();
            runs.push(serde_json::json!({"host_us":host_us,"report":report}));
        }
        let plain = Header {
            network: 1,
            source: 1,
            destination: 2,
            session: 1,
            sequence: 0,
            total: 193,
            index: 0,
            count: 1,
            span: SPAN as u8,
            ack: false,
            request_ack: true,
        }
        .encode(&packet(193, 0))?;
        let mut sender = Vault::memory(1, 1, 1, BTreeMap::from([(2, pair_key(1, 2))]))?;
        let mut receiver = Vault::memory(1, 2, 1, BTreeMap::from([(1, pair_key(1, 2))]))?;
        let start = Instant::now();
        for _ in 0..20_000 {
            let frame = sender.seal(black_box(&plain))?;
            black_box(receiver.open(black_box(&frame))?);
        }
        let crypto_roundtrip_ns = start.elapsed().as_nanos() / 20_000;
        // Unique private directory: no existing key/state files are touched by this benchmark.
        use std::os::unix::fs::DirBuilderExt;
        let path = std::env::temp_dir().join(format!(
            "lm-durable-bench-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(path.clone());
        let state = path.join("state.json");
        let keys = BTreeMap::from([(1, pair_key(1, 2))]);
        Vault::provision(&state, 1, 2, &keys)?;
        let mut receiver = Vault::persistent(&state, 1, 2, keys)?;
        let start = Instant::now();
        for _ in 0..100 {
            let frame = sender.seal(&plain)?;
            black_box(receiver.open(&frame)?);
        }
        let durable_roundtrip_ns = start.elapsed().as_nanos() / 100;
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"version":1,"crypto_roundtrip_ns":crypto_roundtrip_ns,"durable_roundtrip_ns":durable_roundtrip_ns,"runs":runs,"process_usage":loramesh::link::daemon::process_usage()})
            )?
        );
        Ok(())
    }
}
#[cfg(unix)]
fn main() {
    if let Err(e) = unix::main() {
        eprintln!("mesh benchmark: {}", e);
        std::process::exit(1);
    }
}
#[cfg(not(unix))]
fn main() {
    eprintln!(
        "loramesh-mesh-bench requires Unix; use loramesh-radio or loramesh-mesh-sim on Windows"
    );
    std::process::exit(1);
}
