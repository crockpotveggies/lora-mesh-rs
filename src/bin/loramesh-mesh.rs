#[cfg(unix)]
mod unix {
    pub fn main() {
        if loramesh::version_requested("loramesh-mesh") {
            return;
        }
        if let Err(e) = run() {
            eprintln!("mesh: {}", e);
            std::process::exit(1);
        }
    }
    fn run() -> Result<(), Box<dyn std::error::Error>> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args.len() != 1 || args[0] == "--help" {
            println!(
                "Usage: loramesh-mesh CONFIG.json\nProvision identities/state with loramesh-keygen; see docs/secure-mesh.md"
            );
            return Ok(());
        }
        let config = loramesh::mesh::daemon::load(std::path::Path::new(&args[0]))?;
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let signal = stop.clone();
        ctrlc::set_handler(move || signal.store(true, std::sync::atomic::Ordering::Release))?;
        loramesh::mesh::daemon::run(config, stop)?;
        Ok(())
    }
}
#[cfg(unix)]
fn main() {
    unix::main();
}
#[cfg(not(unix))]
fn main() {
    eprintln!("loramesh-mesh requires Unix; use loramesh-radio or loramesh-mesh-sim on Windows");
    std::process::exit(1);
}
