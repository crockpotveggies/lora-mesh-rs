use simplelog::*;
use std::io;
use log::*;

mod hardware;
mod stack;
mod node;
mod settings;

use crate::settings::*;
use crate::hardware::*;
use crate::node::*;
use crate::stack::*;

use std::path::PathBuf;
use rand::prelude::ThreadRng;
use tun_tap::{Iface, Mode};
use std::sync::Arc;

#[macro_use]
extern crate nonzero_ext;
extern crate packet;
extern crate rand;
extern crate config;

const MESH_MAX_MESSAGE_LEN: usize = 200;
const TUN_DEFAULT_PREFIX: &str = "loratun%d";

fn main() {
    let opt: Settings = Settings::new().expect("Error loading settings");

    if opt.debug {
        WriteLogger::init(LevelFilter::Trace, Config::default(), io::stderr()).expect("Failed to init log");
    } else {
        WriteLogger::init(LevelFilter::Info, Config::default(), io::stderr()).expect("Failed to init log");
    }
    info!("LoRa Mesh starting...");

    assert!(opt.nodeid <= 255, "Invalid node ID specified, it must be 255 or less.");
    info!("Node ID is {}", opt.nodeid);
    let iface = Arc::new(Iface::new(TUN_DEFAULT_PREFIX, Mode::Tun).unwrap());
    let tun = NetworkTunnel::new(iface);

    let mut ls: LoStik = LoStik::new(opt.clone());
    let initfile = opt.radiocfg.clone();
    ls.init(initfile);


    let mut node: MeshNode = node::MeshNode::new(opt.nodeid, tun, ls, opt.clone());

    debug!("Running full network stack");
    node.run();
}
