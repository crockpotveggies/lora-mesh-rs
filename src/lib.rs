//! Radio control and hardware-free test infrastructure shared by the daemon and simulator.
pub mod radio;
pub mod sim;

pub mod link;

pub mod mesh;

/// Common identity probe, usable without devices or deployment configuration.
pub fn version_requested(binary: &str) -> bool {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("{} {}", binary, env!("CARGO_PKG_VERSION"));
        true
    } else {
        false
    }
}
