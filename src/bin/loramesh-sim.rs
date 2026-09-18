use loramesh::sim::{Scenario, run};
use std::{
    fs,
    io::{self, Read},
    path::Path,
};
fn main() {
    if loramesh::version_requested("loramesh-sim") {
        return;
    }
    if let Err(e) = execute() {
        eprintln!("simulation failed: {}", e);
        std::process::exit(1);
    }
}
fn execute() -> Result<(), Box<dyn std::error::Error>> {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let signal = stop.clone();
    ctrlc::set_handler(move || signal.store(true, std::sync::atomic::Ordering::Release))?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help") {
        println!(
            "Usage: loramesh-sim SCENARIO.json [--output REPORT.json] [--replay] [--pty | --pty-smoke]\nWithout --pty, runs production controllers in deterministic virtual time.\n--pty exposes virtual serial paths for external daemons, in real time; scenario traffic/expectations are not applied."
        );
        return Ok(());
    }
    let mut input = String::new();
    fs::File::open(&args[0])?
        .take(4 * 1024 * 1024 + 1)
        .read_to_string(&mut input)?;
    if input.len() > 4 * 1024 * 1024 {
        return Err("scenario exceeds 4 MiB".into());
    }
    let scenario: Scenario = if args.iter().any(|a| a == "--replay") {
        let report: serde_json::Value = serde_json::from_str(&input)?;
        serde_json::from_value(
            report
                .get("configuration")
                .ok_or("report has no configuration")?
                .clone(),
        )?
    } else {
        serde_json::from_str(&input)?
    };
    scenario.validate()?;
    let mut output = None;
    let mut pty = false;
    let mut smoke = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--replay" => {}
            "--pty" => pty = true,
            "--pty-smoke" => smoke = true,
            "--output" => {
                index += 1;
                output = Some(args.get(index).ok_or("missing output path")?.clone());
            }
            _ => return Err("unknown option".into()),
        }
        index += 1;
    }
    if pty && smoke {
        return Err("choose one PTY mode".into());
    }
    if smoke {
        #[cfg(unix)]
        {
            let program = std::env::current_exe()?.with_file_name("loramesh-radio");
            let trace = loramesh::sim::process::smoke(&scenario, &program, &stop)?;
            if let Some(path) = output {
                write_report(&path, &serde_json::to_vec_pretty(&trace)?)?;
            }
            println!(
                "PTY smoke passed: two production processes, one transmitted and received payload"
            );
            return Ok(());
        }
        #[cfg(not(unix))]
        return Err("PTY mode requires Unix".into());
    }
    if pty {
        #[cfg(unix)]
        {
            let lab = loramesh::sim::pty::PtyLab::start(&scenario)?;
            println!("{}", serde_json::to_string_pretty(&lab.paths)?);
            eprintln!(
                "Virtual radios active for {} seconds; scheduled traffic and expectations apply only to deterministic mode.",
                scenario.duration_us as f64 / 1_000_000.0
            );
            while lab.running() && !stop.load(std::sync::atomic::Ordering::Acquire) {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let trace = lab.trace();
            let encoded = serde_json::to_vec_pretty(&trace)?;
            if let Some(path) = output {
                write_report(&path, &encoded)?;
            }
            if let Some(error) = lab.error() {
                return Err(error.into());
            }
            return Ok(());
        }
        #[cfg(not(unix))]
        return Err("PTY mode requires Unix".into());
    }
    let report = match run(&scenario) {
        Ok(report) => report,
        Err(error) => {
            let diagnostic =
                serde_json::json!({"version":1,"configuration":scenario,"error":error.to_string()});
            let encoded = serde_json::to_vec_pretty(&diagnostic)?;
            if let Some(path) = output {
                write_report(&path, &encoded)?;
            } else {
                eprintln!("{}", String::from_utf8(encoded)?);
            }
            return Err(error.into());
        }
    };
    let encoded = serde_json::to_vec_pretty(&report)?;
    if let Some(path) = output {
        write_report(&path, &encoded)?;
    } else {
        println!("{}", String::from_utf8(encoded)?);
    }
    if !report.failures.is_empty() {
        return Err(report.failures.join("; ").into());
    }
    Ok(())
}
fn write_report(path: &str, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(path, bytes)
}
