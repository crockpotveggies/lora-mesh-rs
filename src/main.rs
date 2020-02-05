/*
    Copyright (C) 2019  John Goerzen <jgoerzen@complete.org

    This program is free software: you can redistribute it and/or modify
    it under the terms of the GNU General Public License as published by
    the Free Software Foundation, either version 3 of the License, or
    (at your option) any later version.

    This program is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU General Public License for more details.

    You should have received a copy of the GNU General Public License
    along with this program.  If not, see <http://www.gnu.org/licenses/>.

*/
mod hardware;
mod stack;
mod node;

use simplelog::*;
use std::io;
use log::*;
use std::thread;

use crate::hardware::*;
use crate::node::*;
use crate::stack::*;

use std::path::PathBuf;
use structopt::StructOpt;
use std::time::Duration;

#[macro_use]
extern crate nonzero_ext;
extern crate packet;

const MESH_MAX_MESSAGE_LEN: usize = 200;
const TUN_DEFAULT_PREFIX: &str = "loratun%d";

#[derive(Debug, StructOpt, Clone)]
#[structopt(name = "loramesh", about = "Network mesh tool for LoRa", author = "Justin Long <crockpotveggies@users.github.com>")]
pub struct Opt {
    /// Activate debug mode
    // short and long flags (-d, --debug) will be deduced from the field's name
    #[structopt(short, long)]
    debug: bool,

    /// Set if node is a gateway to internet
    /* Turning this on will enable special networking features, including a
    DHCP server and will assign IP addresses to other nodes in the mesh. */
    #[structopt(long)]
    isgateway: bool,

    /// Pack as many bytes as possible into each TX frame, regardless of original framing
    #[structopt(long)]
    pack: bool,
    
    /// Radio initialization command file
    #[structopt(long, parse(from_os_str))]
    initfile: Option<PathBuf>,

    /// Maximum frame size sent to radio [10..250] (valid only for ping and kiss)
    #[structopt(long, default_value = "250")]
    maxpacketsize: usize,

    /// The size of the transmission slot used for transmission rate limiting
    #[structopt(long, default_value = "100")]
    txslot: u64,

    /// Amount of time (ms) to wait for end-of-transmission signal before transmitting
    /* The amount of time to wait before transmitting after receiving a
    packet that indicated more data was forthcoming.  The purpose of this is
    to compensate for a situation in which the "last" incoming packet was lost,
    to prevent the receiver from waiting forever for more packets before
    transmitting.  Given in ms. */
    #[structopt(long, default_value = "1000")]
    eotwait: u64,

    /// Timeout (ms) for synchronous messages
    /* Certain messages are synchronous and require a response, such as discovery
    and gateway requests. */
    #[structopt(long, default_value = "1000")]
    timeout: u64,

    /// Maximum number of hops a packet should travel
    #[structopt(long, default_value = "2")]
    maxhops: u32,

    /// The ID of this LoRa node
    /* This sets the ID of the node, similar to a MAC address. This must be
    between 1 and 255 otherwise the node will enter local test mode. It is recommended
    you set the gateway as 1. */
    #[structopt(short, long, default_value = "0")]
    nodeid: u32,
    
    #[structopt(parse(from_os_str))]
    /// Serial port to use to communicate with radio
    port: PathBuf,

    #[structopt(subcommand)]
    cmd: Command
}

#[derive(Debug, StructOpt, Clone)]
enum Command {
    /// Dump packets from local tunnel
    Dump,
    /// Node discovery without data link
    Discovery,
    /// Deploy node and enable data link
    Network,
}

fn main() {
    let opt: Opt = Opt::from_args();

    if opt.debug {
        WriteLogger::init(LevelFilter::Trace, Config::default(), io::stderr()).expect("Failed to init log");
    }
    info!("LoRa Mesh starting...");

    assert!(opt.nodeid <= 255, "Invalid node ID specified, it must be 255 or less.");
    info!("Node ID is {}", opt.nodeid);

    let tun = NetworkTunnel::new(opt.isgateway);

    let mut ls: LoStik = LoStik::new(opt.clone());
    let initfile = opt.initfile.clone();
    ls.init(initfile);


    let mut node: MeshNode = node::MeshNode::new(opt.nodeid as i8, tun, ls, opt.clone());

    match opt.cmd {
        Command::Dump => unsafe {
            debug!("Running tunnel dump");
            node.run_dump();
        }
        Command::Discovery => unsafe {
            debug!("Running network discovery");
            node.run_discovery();
        }
        Command::Network => unsafe {
            debug!("Running full network stack");
            node.run();
        }
    }
}
