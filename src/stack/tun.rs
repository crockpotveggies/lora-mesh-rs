use log::*;
use std::process::Command;
use std::thread;
extern crate tun_tap;
use tun_tap::{Iface, Mode};
use crate::TUN_DEFAULT_PREFIX;
use std::net::Ipv4Addr;
use crossbeam_channel;
use crossbeam_channel::{Receiver, Sender};
use packet::ip::v4::Packet;
use std::sync::Arc;

pub struct NetworkTunnel {
    pub tunname: String,
    pub interface: Arc<Iface>,
    pub tunip: Option<Ipv4Addr>,
    /// receiver for packets coming from tun
    pub inboundSender: Sender<Packet<Vec<u8>>>,
    pub inboundReceiver: Receiver<Packet<Vec<u8>>>
}

fn tunloop(iface: Arc<Iface>, sender: Sender<Packet<Vec<u8>>>) {
    info!("Network tunnel started...");

    loop {
        let mut buffer = vec![0; 1504];
        // Read next packet from network tunnel
        let size = iface.recv(&mut buffer).unwrap();
        assert!(size >= 4);
        trace!("Network packet of size {}", size);

        // Forward packet to node/radio
        match Packet::new(Vec::from(&buffer[4..size])) {
            Err(e) => error!("Received invalid IP packet {}", e), // unsupported protocol
            Ok(ippacket) => { sender.send(ippacket); }
        }
    }
}

impl NetworkTunnel {
    pub fn new(iface: Arc<Iface>) -> Self {
        trace!("Iface: {:?}", iface);

        let tunname = String::from(iface.name().clone());

        // Configure the local kernel interface with a kernel
        // IP and we will route and capture traffic through it
        let iaddr = Ipv4Addr::new(10,107,1,3);
        ipassign(tunname.as_str(), &iaddr);
        ipcmd("ip", &["link", "set", "dev", tunname.as_str(), "up"]);
        info!("Created interface {} with IP addr {}", tunname, iaddr.to_string());

        // set up channels for sending and receiving packets
        let (inboundSender, inboundReceiver) = crossbeam_channel::unbounded();

        NetworkTunnel {
            tunname: tunname,
            interface: iface,
            tunip: Some(iaddr),
            inboundSender,
            inboundReceiver
        }
    }

    /// Start the network tunnel thread
    pub fn run(&self) -> Receiver<Packet<Vec<u8>>> {
        let sender = self.inboundSender.clone();
        let iface = Arc::clone(&self.interface);
        thread::spawn(move || tunloop(iface, sender) );
        return self.inboundReceiver.clone();
    }

    /// Send packet on tunnel
    pub fn send(&mut self, packet: Packet<Vec<u8>>) {
        self.interface.send(packet.as_ref()).map(|res| trace!("Network tunnel sent {} bytes", &res) );
    }

    /// Add IP address to this tunnel's interface
    /* This performs a kernel ip route which allows us to capture
    traffic from local interface. */
    pub fn assignipaddr(&mut self, ipaddr: &Ipv4Addr) {
        ipassign(self.tunname.as_str(), ipaddr);
    }

    /// Set up a route to an IP through this node
    /* This performs a kernel ip route which allows us to capture
    traffic from local interface. */
    pub fn routeipaddr(&mut self, dest: &Ipv4Addr, via: &Ipv4Addr) {
        iproute(self.tunname.as_str(), dest, via);
    }
}

/// Run a shell command and panic if it fails
pub fn ipcmd(cmd: &str, args: &[&str]) {
    let ecode = Command::new("ip")
        .args(args)
        .spawn()
        .unwrap()
        .wait()
        .unwrap();
    assert!(ecode.success(), "Failed to execte `{}` arg `{}` with code `{}`", cmd, args[0], ecode.to_string());
}

/// Kernel route IP traffic to interface
pub fn iproute(tun: &str, dest: &Ipv4Addr, via: &Ipv4Addr) {
    trace!("Adding tunnel ip route dest {} via {}", &dest.to_string(), &via.to_string());
    assert!(dest.is_private(), "Refusing to route mesh traffic to non-private IP.");
    ipcmd("ip", &["route", "add", &dest.to_string(), "via", &via.to_string(), "dev", tun]);
}

/// Kernel assign IP address to interface
pub fn ipassign(tun: &str, addr: &Ipv4Addr) {
    ipcmd("ip", &["addr", "add", &addr.to_string(), "dev", tun]);
}