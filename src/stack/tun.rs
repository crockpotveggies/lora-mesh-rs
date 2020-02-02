use std::process::Command;
use std::thread;
use std::time::Duration;
extern crate tun_tap;
use tun_tap::{Iface, Mode};
use crate::TUN_DEFAULT_PREFIX;

pub struct NetworkTunnel {
    pub interface: Iface,
    pub ipaddr: String
}

impl NetworkTunnel {
    pub fn new(isgateway: bool) -> Self {
        let mut ipaddr = "10.107.1.3/24";
        let iface = Iface::new(TUN_DEFAULT_PREFIX, Mode::Tun).unwrap();
        eprintln!("Iface: {:?}", iface);
        // Configure the local kernel interface
        // If this node is a gateway, assign an IP address of 10.0.1.1
        // Otherwise, we will wait for DHCP from a network gateway
        ipassign(iface.name(), ipaddr);
        ipcmd("ip", &["link", "set", "dev", iface.name(), "up"]);
        println!("Created interface {} with IP addr {}", iface.name(), ipaddr);

        if isgateway {
            ipaddr = "10.0.1.1/32";
            iproute(iface.name(), ipaddr);
            println!("Network gateway detected, added route to {}", ipaddr);
        }

        NetworkTunnel {
            interface: iface,
            ipaddr: String::from(ipaddr)
        }
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

/// Assign IP address to interface
pub fn ipassign(tun: &str, addr: &str) {
    ipcmd("ip", &["addr", "add", addr, "dev", tun]);
}

pub fn iproute(tun: &str, addr: &str) {
    ipcmd("ip", &["route", "add", addr, "dev", tun]);
}