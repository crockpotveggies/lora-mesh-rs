use log::*;
use std::process::Command;
use std::thread;
use std::time::Duration;
extern crate tun_tap;
use tun_tap::{Iface, Mode};
use crate::TUN_DEFAULT_PREFIX;
use std::net::Ipv4Addr;
use crossbeam_channel;
use crossbeam_channel::{Receiver, Sender};

pub struct NetworkTunnel {
    pub interface: Iface,
    pub ipaddr: Option<Ipv4Addr>,
}

fn tunloop(tun: &Iface, sender: Sender<Vec<u8>>, receiver: Receiver<Vec<u8>>) {
    loop {
        let mut buffer = vec![0; 1504];
        // Read next packet from network tunnel
        let size = tun.recv(&mut buffer).unwrap();
        assert!(size >= 4);
        trace!("Packet: {:?}", &buffer[4..size]);

        // Forward packet to lora
        sender.send(buffer);

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

        // Configure the local kernel interface with a kernel
        // IP and we will route and capture traffic through it
        let mut iaddr = Ipv4Addr::new(10,107,1,3);
        ipassign(iface.name(), &iaddr);
        ipcmd("ip", &["link", "set", "dev", iface.name(), "up"]);
        println!("Created interface {} with IP addr {}", iface.name(), iaddr.to_string());

        // If this node is a gateway, assign an IP address of 10.0.1.1.
        // Otherwise, we will wait for DHCP from a network gateway and
        // assign a default address.
        let mut nodeaddr = None;
        if isgateway {
            nodeaddr = Some(Ipv4Addr::new(10,0,0,1));
            iproute(iface.name(), &nodeaddr.unwrap());
            println!("Network gateway detected, added route to {}", nodeaddr.unwrap().to_string());
        }

        NetworkTunnel {
            interface: iface,
            ipaddr: nodeaddr
        }
    }

    /// Main loop, pulls packets from network interface and vice-versa
    pub fn run(&self) -> (Receiver<Vec<u8>>, Sender<Vec<u8>>) {
        // set up channels for sending and receiving packets
        let (inboundSender, inboundReceiver) = crossbeam_channel::unbounded();
        let (outboundSender, outboundReceiver) = crossbeam_channel::unbounded();

        thread::spawn(move || tunloop(&self.interface, inboundSender, outboundReceiver));

        return (inboundReceiver, outboundSender);
    }

    /// Set the IP address of this node
    /* This performs a kernel ip route which allows us to capture
    traffic from local interface. */
    pub fn setipaddr(&mut self, addr: &Ipv4Addr) {
        iproute(self.interface.name(), addr);
        self.ipaddr = Some(addr.clone());
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