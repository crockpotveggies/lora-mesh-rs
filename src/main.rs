use log::*;
use simplelog::*;
use std::io;

mod hardware;
mod node;
mod settings;
mod stack;

use crate::hardware::*;
use crate::node::*;
use crate::settings::*;
use crate::stack::*;

use std::sync::Arc;
use tun_tap::{Iface, Mode};

#[macro_use]
extern crate nonzero_ext;
extern crate config;
extern crate packet;
extern crate rand;

const TUN_DEFAULT_PREFIX: &str = "loratun%d";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let opt = Settings::new()?;

    if opt.debug {
        WriteLogger::init(LevelFilter::Trace, Config::default(), io::stderr())?;
    } else {
        WriteLogger::init(LevelFilter::Info, Config::default(), io::stderr())?;
    }
    info!("LoRa Mesh starting...");

    info!("Node ID is {}", opt.nodeid);
    let ls = LoStik::new(opt.clone())?;
    let iface = Arc::new(Iface::new(TUN_DEFAULT_PREFIX, Mode::Tun)?);
    let tun = NetworkTunnel::new(iface);

    let mut node: MeshNode = node::MeshNode::new(opt.nodeid, tun, ls, opt.clone());

    debug!("Running full network stack");
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let signal = stop.clone();
    ctrlc::set_handler(move || signal.store(true, std::sync::atomic::Ordering::Release))?;
    node.run(stop);
    Ok(())
}
