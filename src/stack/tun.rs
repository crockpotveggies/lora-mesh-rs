use std::process::Command;
use std::thread;
use std::time::Duration;
extern crate tun_tap;
use tun_tap::{Iface, Mode};
use crate::TUN_DEFAULT_PREFIX;
use std::net::Ipv4Addr;

pub struct NetworkTunnel {
    pub interface: Iface,
    pub ipaddr: Ipv4Addr
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
        let mut nodeaddr = Ipv4Addr::new(192,168,0,1);
        if isgateway {
            nodeaddr = Ipv4Addr::new(10,0,0,1);
            iproute(iface.name(), &nodeaddr);
            println!("Network gateway detected, added route to {}", nodeaddr.to_string());
        }

        NetworkTunnel {
            interface: iface,
            ipaddr: nodeaddr
        }
    }

    /// Set the IP address of this node
    /* This performs a kernel ip route which allows us to capture
    traffic from local interface. */
    pub fn setipaddr(&mut self, addr: &Ipv4Addr) {
        iproute(self.interface.name(), addr);
        self.ipaddr = addr.clone();
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