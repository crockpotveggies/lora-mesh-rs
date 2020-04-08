use log::*;
use std::process::Command;
use std::thread;
use std::time::Duration;
extern crate tun_tap;
use tun_tap::{Iface, Mode};
use crate::TUN_DEFAULT_PREFIX;
use std::net::Ipv4Addr;
use crossbeam;
use crossbeam_channel;
use crossbeam_channel::{Receiver, Sender};
use packet::ip::v4::Packet;

pub struct NetworkTunnel {
    pub interface: String,
    pub tunip: Option<Ipv4Addr>,
    /// receiver for packets coming from tun
    pub inboundReceiver: Receiver<Packet<Vec<u8>>>,
    /// sends packets to tun
    pub outboundSender: Sender<Vec<u8>>
}

fn tunloop(tun: Iface, sender: Sender<Packet<Vec<u8>>>, receiver: Receiver<Vec<u8>>) {
    info!("Network tunnel started...");

    loop {
        let mut buffer = vec![0; 1504];
        // Read next packet from network tunnel
        let size = tun.recv(&mut buffer).unwrap();
        assert!(size >= 4);
        trace!("Network packet of size {}", size);

        // Forward packet to node/radio
        match Packet::new(Vec::from(&buffer[4..size])) {
            Err(e) => debug!("Received invalid IP packet"), // unsupported protocol
            Ok(ippacket) => {
                sender.send(ippacket);
            }
        }

        // send anything, if necessary
        let r = receiver.try_recv();
        match r {
            Ok(data) => {
                tun.send(data.as_ref());
                continue;
            },
            Err(e) => {
                if e.is_disconnected() {
                    // other threads crashed
                    r.unwrap();
                }
                // Otherwise - nothing to write, go on through.
            }
        }
    }
}

impl NetworkTunnel {
    pub fn new() -> Self {
        let iface = Iface::new(TUN_DEFAULT_PREFIX, Mode::Tun).unwrap();
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
        let (outboundSender, outboundReceiver) = crossbeam_channel::unbounded();
        thread::spawn(move || tunloop(iface, inboundSender, outboundReceiver));

        NetworkTunnel {
            interface: tunname,
            tunip: Some(iaddr),
            inboundReceiver,
            outboundSender
        }
    }

    /// Set up a route to an IP through this node
    /* This performs a kernel ip route which allows us to capture
    traffic from local interface. */
    pub fn routeipaddr(&mut self, dest: &Ipv4Addr, via: &Ipv4Addr) {
        iproute(self.interface.as_str(), dest, via);
    }

    /// Return a sender and receiver for tunnel I/O
    pub fn split(&mut self) -> (Receiver<Packet<Vec<u8>>>, Sender<Vec<u8>>) {
        return (self.inboundReceiver.clone(), self.outboundSender.clone())
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
    assert!(dest.is_private(), "Refusing to route mesh traffic to non-private IP.");
    ipcmd("ip", &["route", "add", &dest.to_string(), "via", &via.to_string(), "dev", tun]);
}

/// Kernel assign IP address to interface
pub fn ipassign(tun: &str, addr: &Ipv4Addr) {
    ipcmd("ip", &["addr", "add", &addr.to_string(), "dev", tun]);
}