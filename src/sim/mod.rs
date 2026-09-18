//! Deterministic packet-level model, not an RF propagation or range predictor.
pub mod device;
pub mod medium;
#[cfg(unix)]
pub mod pty;
pub mod scenario;
pub use medium::{Medium, Trace};
pub use scenario::{Report, Scenario, run};

#[cfg(unix)]
pub mod process;
