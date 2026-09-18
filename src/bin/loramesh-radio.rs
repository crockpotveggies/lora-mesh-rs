//! Hardware/PTY probe using exactly the daemon's production radio worker, without TUN.
use loramesh::radio::{ControllerConfig, runtime::RadioHandle};
use std::time::{Duration, Instant};
fn main() {
    if loramesh::version_requested("loramesh-radio") {
        return;
    }
    if let Err(e) = run() {
        eprintln!("radio probe: {}", e);
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let signal = stop.clone();
    ctrlc::set_handler(move || signal.store(true, std::sync::atomic::Ordering::Release))?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" {
        println!(
            "Usage: loramesh-radio PORT [--send HEX] [--duration-ms N] [--sf N] [--frequency HZ] [--power DBM]"
        );
        return Ok(());
    }
    let mut duration = 10_000u64;
    let mut payload = None;
    let mut config = ControllerConfig::default();
    let mut i = 1;
    while i < args.len() {
        let value = args.get(i + 1).ok_or("missing option value")?;
        match args[i].as_str() {
            "--send" => {
                let p = hex::decode(value)?;
                if p.is_empty() || p.len() > 255 {
                    return Err("payload must be 1..255 bytes".into());
                }
                payload = Some(p);
            }
            "--duration-ms" => duration = value.parse()?,
            "--frequency" => config
                .initialization
                .push(format!("radio set freq {}", value.parse::<u32>()?)),
            "--power" => config
                .initialization
                .push(format!("radio set pwr {}", value.parse::<i8>()?)),
            "--sf" => config
                .initialization
                .push(format!("radio set sf sf{}", value)),
            _ => return Err("unknown option".into()),
        }
        i += 2;
    }
    if duration == 0 || duration > 3_600_000 {
        return Err("duration must be 1..3600000 ms".into());
    }
    let mut radio = RadioHandle::serial(args[0].clone().into(), config)?;
    if let Err(error) = radio.wait_ready(Duration::from_secs(5)) {
        // Initialization events identify the last attempted command even if readiness fails.
        for event in radio.events.try_iter() {
            eprintln!("{:?}", event);
        }
        radio.shutdown();
        return Err(error.into());
    }
    let must_transmit = payload.is_some();
    let mut transmitted = false;
    println!("ready");
    if let Some(p) = payload {
        radio.tx.try_send(p)?;
    }
    let until = Instant::now() + Duration::from_millis(duration);
    while Instant::now() < until && !stop.load(std::sync::atomic::Ordering::Acquire) {
        crossbeam_channel::select! {
            recv(radio.rx)->packet=>if let Ok(packet)=packet{println!("rx {}",hex::encode(packet));},
            recv(radio.events)->event=>if let Ok(event)=event{
                if matches!(event,loramesh::radio::Action::Transmitted(_)){transmitted=true;}
                if let loramesh::radio::Action::Failed(_,ref message)=event{return Err(message.clone().into());}
                eprintln!("{:?}",event);
            },
            default(Duration::from_millis(20))=>{},
        }
    }
    radio.shutdown();
    if must_transmit && !transmitted {
        return Err("transmission did not complete before deadline".into());
    }
    Ok(())
}
