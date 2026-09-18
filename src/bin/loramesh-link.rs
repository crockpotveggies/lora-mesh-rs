//! Versioned single-hop IPv4 daemon, independent of legacy mesh routing.
fn main() {
    if let Err(e) = run() {
        eprintln!("link: {}", e);
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 1 || args[0] == "--help" {
        println!(
            "Usage: loramesh-link CONFIG.json\nLinux TUN or Unix datagram packet adapter; see docs/reliable-link.md"
        );
        return Ok(());
    }
    #[cfg(unix)]
    {
        let config = loramesh::link::daemon::load(std::path::Path::new(&args[0]))?;
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let signal = stop.clone();
        ctrlc::set_handler(move || signal.store(true, std::sync::atomic::Ordering::Release))?;
        loramesh::link::daemon::run(config, stop)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        Err("requires Unix".into())
    }
}
