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

use std::path::PathBuf;
use structopt::StructOpt;

const MESH_MAX_MESSAGE_LEN: usize = 200;
const TUN_DEFAULT_PREFIX: &str = "loratun%d";

#[derive(Debug, StructOpt)]
#[structopt(name = "loramesh", about = "Network mesh tool for LoRa", author = "Justin Long <crockpotveggies@users.github.com>")]
struct Opt {
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
    #[structopt(long, default_value = "100")]
    maxpacketsize: usize,

    /// Maximum time to transmit at once before giving a chance to receive (in ms). 0=infinite
    #[structopt(long, default_value = "0")]
    txslot: u64,

    /// Amount of time (ms) to pause before transmitting a packet
    /* The
    main purpose of this is to give the other radio a chance to finish
    decoding the previous packet, send it to the OS, and re-enter RX mode.
    A secondary purpose is to give the duplex logic a chance to see if
    anything else is coming in.  Given in ms.
     */
    #[structopt(long, default_value = "120")]
    txwait: u64,

    /// Amount of time (ms) to wait for end-of-transmission signal before transmitting
    /* The amount of time to wait before transmitting after receiving a
    packet that indicated more data was forthcoming.  The purpose of this is
    to compensate for a situation in which the "last" incoming packet was lost,
    to prevent the receiver from waiting forever for more packets before
    transmitting.  Given in ms. */
    #[structopt(long, default_value = "1000")]
    eotwait: u64,
    
    #[structopt(parse(from_os_str))]
    /// Serial port to use to communicate with radio
    port: PathBuf,

    #[structopt(subcommand)]
    cmd: Command
}

#[derive(Debug, StructOpt)]
enum Command {
    /// Dump packets from local tunnel
    Dump,
    /// Transmit ping requests
    Ping,
    /// Pipe data across radios
    Pipe,
    /// Pipe KISS data across the radios
    Kiss,
    /// Receive ping requests and transmit pongs
    Pong,

}

fn main() {
    let opt = Opt::from_args();

    if opt.debug {
        WriteLogger::init(LevelFilter::Trace, Config::default(), io::stderr()).expect("Failed to init log");
    }
    info!("lora starting");

    let maxpacketsize = opt.maxpacketsize;

    let (mut ls, radioreceiver) = lostik::LoStik::new(opt.debug, opt.txwait, opt.eotwait, maxpacketsize, opt.pack, opt.txslot, opt.port);
    ls.configure(opt.initfile).expect("Failed to configure radio");

//    let mut ls2 = ls.clone();
//    thread::spawn(move || ls2.run().expect("Failure in readerthread"));

    let node = node::MeshNode::new(ls, opt.isgateway);
    node.run();
}
