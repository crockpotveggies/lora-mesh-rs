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

pub struct NetworkTunnel {
    pub interface: String,
    pub ipaddr: Option<Ipv4Addr>,
    /// receiver for packets coming from tun
    pub inboundReceiver: Receiver<Vec<u8>>,
    /// sends packets to tun
    pub outboundSender: Sender<Vec<u8>>
}

fn tunloop(tun: Iface, sender: Sender<Vec<u8>>, receiver: Receiver<Vec<u8>>) {
    debug!("Network tunnel thread started...");

    loop {
        let mut buffer = vec![0; 1504];
        // Read next packet from network tunnel
        let size = tun.recv(&mut buffer).unwrap();
        assert!(size >= 4);
        trace!("Packet of size {}:\n {:?}", size, &buffer[4..size]);

        // Forward packet to lora
        sender.send(Vec::from(&buffer[0..size]));

        // send anything, if necessary
        let r = receiver.try_recv();
        match r {
            Ok(data) => {
                tun.send(&data);
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
    pub fn new(isgateway: bool) -> Self {
        let iface = Iface::new(TUN_DEFAULT_PREFIX, Mode::Tun).unwrap();
        eprintln!("Iface: {:?}", iface);

        let tunname = String::from(iface.name().clone());

        // Configure the local kernel interface with a kernel
        // IP and we will route and capture traffic through it
        let mut iaddr = Ipv4Addr::new(10,107,1,3);
        ipassign(tunname.as_str(), &iaddr);
        ipcmd("ip", &["link", "set", "dev", tunname.as_str(), "up"]);
        println!("Created interface {} with IP addr {}", tunname, iaddr.to_string());

        // set up channels for sending and receiving packets
        let (inboundSender, inboundReceiver) = crossbeam_channel::unbounded();
        let (outboundSender, outboundReceiver) = crossbeam_channel::unbounded();
        thread::spawn(move || tunloop(iface, inboundSender, outboundReceiver));

        // If this node is a gateway, assign an IP address of 10.0.1.1.
        // Otherwise, we will wait for DHCP from a network gateway and
        // assign a default address.
        let mut nodeaddr = None;
        if isgateway {
            nodeaddr = Some(Ipv4Addr::new(10,0,0,1));
            iproute(tunname.as_str(), &nodeaddr.unwrap());
            println!("Network gateway detected, added route to {}", nodeaddr.unwrap().to_string());
        }

        NetworkTunnel {
            interface: tunname,
            ipaddr: nodeaddr,
            inboundReceiver,
            outboundSender
        }
    }

    /// Set the IP address of this node
    /* This performs a kernel ip route which allows us to capture
    traffic from local interface. */
    pub fn setipaddr(&mut self, addr: &Ipv4Addr) {
        iproute(self.interface.as_str(), addr);
        self.ipaddr = Some(addr.clone());
    }

    /// Return a sender and receiver for tunnel I/O
    pub fn split(&mut self) -> (Receiver<Vec<u8>>, Sender<Vec<u8>>) {
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

/// Kernel assign IP address to interface
pub fn ipassign(tun: &str, addr: &Ipv4Addr) {
    ipcmd("ip", &["addr", "add", &addr.to_string(), "dev", tun]);
}

/// Kernel route IP traffic to interface
pub fn iproute(tun: &str, addr: &Ipv4Addr) {
    assert!(addr.is_private(), "Refusing to route mesh traffic to non-private IP.");
    ipcmd("ip", &["route", "add", &addr.to_string(), "dev", tun]);
}