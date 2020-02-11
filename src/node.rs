use log::*;
use std::thread;
use std::time::Duration;
use crate::stack::{NetworkTunnel, Frame};
use crate::hardware::LoStik;
use crate::stack::MeshRouter;
use crate::stack::message::*;
use std::net::Ipv4Addr;
use crate::Opt;
use crate::stack::chunk::chunk_data;
use packet::ip::v4::Packet;
use ratelimit_meter::{DirectRateLimiter, LeakyBucket};
use crossbeam_channel::Sender;
use std::borrow::{Borrow, BorrowMut};
use hex;
use crate::stack::tun::ipassign;

pub struct MeshNode {
    /// The ID of this node
    id: i8,
    /// IP address of this node's tunnel
    ipaddr: Option<Ipv4Addr>,
    /// LoRa device for communication
    radio: LoStik,
    /// Local network interface for IP
    networktunnel: NetworkTunnel,
    /// Options
    opt: Opt
}

impl MeshNode {

    pub fn new(id: i8, mut networktunnel: NetworkTunnel, radio: LoStik, opt: Opt) -> Self {
        // If this node is a gateway, assign an IP address of 10.0.0.<id>.
        // Otherwise, we will wait for DHCP from a network gateway and
        // assign a default address.
        let mut ipaddr = None;
        if opt.isgateway {
            ipaddr = Some(Ipv4Addr::new(10,0,0, id as u8));
            networktunnel.routeipaddr(&ipaddr.unwrap());
            info!("Network gateway detected, added route to {}", ipaddr.unwrap().to_string());
        }

        MeshNode{
            id,
            ipaddr,
            radio,
            networktunnel,
            opt,
        }
    }

    /// Main loop, discover network and send/receive packets
    pub fn run(&mut self) {
        // instantiate the router
        let mut router = MeshRouter::new(self.id, self.ipaddr, self.opt.maxhops as i32, Duration::from_millis(self.opt.timeout));
        // start i/o with local tunnel
        let (tunReceiver, tunSender) = self.networktunnel.split();
        // start radio i/o
        let (radioReceiver, radioSender) = self.radio.run();
        // rate limiters for different tasks
        let mut broadcastlimiter = DirectRateLimiter::<LeakyBucket>::new(nonzero!(1u32), Duration::from_secs(30));

        loop {
            // handle packets coming from tunnel
            // pull the next packet from the receiver, process it, and determine if we
            // need to forward it to the radio
            let r = tunReceiver.try_recv();
            match r {
                Err(e) => {
                    if e.is_disconnected() {
                        r.unwrap(); // other threads crashed
                        panic!("Network tunnel crashed: {}", e);
                    }
                    // Otherwise - nothing to write, go on through.
                },
                Ok(data) => {
                    // apply routing logic
                    // if it cannot be routed, drop it
                    self.handle_ip_packet(data, Vec::new(), router.borrow_mut(), None, Some(&tunSender));
                },
            }

            // now handle packets coming from radio
            // parse the frame, and match against message type to
            // determine if it goes to our tunnel
            // or if it is routed to another node
            let r = radioReceiver.try_recv();
            match r {
                Err(e) => {
                    if e.is_disconnected() {
                        r.unwrap(); // other threads crashed
                        panic!("Network tunnel crashed: {}", e);
                    }
                    // Otherwise - nothing to write, go on through.
                },
                Ok(data) => {
                    match Frame::parse(&data) {
                        Err(e) => {
                            trace!("Received invalid radio frame, dropping");
                        },
                        Ok(mut frame) => {
                            // TODO some things here depend if node is gateway
                            match frame.msgtype() {
                                // received IP packet, handle it
                                MessageType::IPPacket => {
                                    let packet = Packet::new(frame.payload()).expect("Could not parse IPv4 packet");
                                    self.handle_ip_packet(packet, data.clone(), router.borrow_mut(), Some(&radioSender), None);
                                },
                                // process another node's broadcast
                                // TODO broadcast should be routed through nodes if needs IP assignment
                                MessageType::Broadcast => {
                                    match BroadcastMessage::from_frame(frame.borrow_mut()) {
                                        Err(e) => error!("Could not parse BroadcastMessage: {}", e),
                                        Ok(broadcast) => {
                                            // TODO IP assign responses should be routed through nodes if not gateway
                                            match router.handle_broadcast(broadcast) {
                                                Err(e) => {
                                                    let mut route: Vec<i8> = Vec::new();
                                                    route.push(frame.sender() as i8);
                                                    let frame = e.to_frame(self.id, route).bits();
                                                    radioSender.send(frame);
                                                },
                                                Ok(ip) => {
                                                    match ip {
                                                        None => (), // no resposne
                                                        Some(ipaddr) => {
                                                            if self.opt.isgateway {
                                                                let mut route = Vec::new();
                                                                route.push(frame.sender() as i8);
                                                                let bits = IPAssignSuccessMessage::new(ipaddr).to_frame(self.id, route).bits();
                                                                radioSender.send(bits);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                },
                                // we were successfully assigned an IP
                                MessageType::IPAssignSuccess => {
                                    match IPAssignSuccessMessage::from_frame(frame.borrow_mut()) {
                                        Err(e) => error!("Could not parse IPAssignSuccessMessage: {}", e),
                                        Ok(message) => {
                                            info!("Assigned new IP address {}", message.ipaddr.to_string());
                                            self.handle_ip_assign(message.ipaddr);
                                        }
                                    }
                                },
                                // we sent a broadcast without IP, but got a failure
                                MessageType::IPAssignFailure => {
                                    match IPAssignFailureMessage::from_frame(frame.borrow_mut()) {
                                        Err(e) => error!("Could not parse IPAssignFailureMessage: {}", e),
                                        Ok(message) => error!("Failed to be assigned IP: {}", message.reason)
                                    }
                                },
                                // handle route discovery
                                MessageType::RouteDiscovery => {},
                                MessageType::RouteSuccess => {},
                                MessageType::RouteFailure => {},
                                MessageType::TransmitRequest => {},
                                MessageType::TransmitConfirm => {},
                            }
                        }
                    }
                }
            }

            // now handle any protocol tasks
            // such as broadcasts or route discovery
            if broadcastlimiter.check().is_ok() {
                debug!("Sending broadcast to nearby nodes");
                self.broadcast();
            }
        }
    }

    /// Handle an IP assignment
    /// ensures a new local route is set up and node
    /// accepts new IP
    fn handle_ip_assign(&mut self, ipaddr: Ipv4Addr) {
        self.ipaddr = Some(ipaddr);
        ipassign(&self.networktunnel.interface, &ipaddr);
    }

    /// Handle routing of a packet
    /// checks if packet was destinated for this node or if
    /// routing logic should be applied and forwarding necessary
    fn handle_ip_packet(&mut self, mut packet: Packet<Vec<u8>>, bits: Vec<u8>, mut router: &mut MeshRouter, mut radioSender: Option<&Sender<Vec<u8>>>, mut tunSender: Option<&Sender<Vec<u8>>>) {
        // apply routing logic
        // if it cannot be routed, drop it
        if self.ipaddr.is_some() {
            if packet.destination().eq(&self.ipaddr.unwrap()) {
                trace!("Received packet from {}", packet.source());
                if !self.opt.debug {
                    // TODO route to tunnel during debug
                    // TODO why can't we get the raw buffer!?
                    tunSender.unwrap().send(bits);
                }
            }
            else {
                match router.packet_route(&packet) {
                    None => {
                        trace!("Dropping packet to: {}", packet.destination());
                        drop(packet);
                    },
                    Some(route) => {
                        // TODO sort through the routes
                        // chunk it
                        // TODO move this to Frame
                        let chunks = chunk_data(Vec::from(packet.as_ref()), (self.opt.maxpacketsize).clone());
                        for chunk in chunks {
                            radioSender.unwrap().send(chunk);
                        }
                    }
                }
            }
        }
    }

    /// Send a broadcast packet to nearby nodes
    fn broadcast(&mut self) {
        // prepare broadcast
        let mut ipOffset = 0i8;
        if self.ipaddr.is_some() {
            ipOffset = 4i8;
        }
        let msg = BroadcastMessage {
            header: None,
            isgateway: true,
            ipOffset,
            ipaddr: self.ipaddr
        };
        let mut frame = msg.to_frame(self.id, Vec::new());
        // dump
        self.radio.tx(&frame.bits());
    }


    /// Main loop for local tunnel dump
    pub fn run_tunnel_dump(&mut self) {
        loop {
            // Read next packet from network tunnel
            let (receiver, _sender) = self.networktunnel.split();
            let r = receiver.recv();
            match r {
                Ok(data) => {
                    let packet = data.as_ref();
                    let size = packet.len();
                    trace!("Packet: {:?}", &packet[0..size]);
                },
                Err(_e) => {
                    // do nothing
                }
            }
        }
    }


    /// Main loop for radio tunnel dump
    pub fn run_radio_dump(&mut self) {
        // start radio i/o
        let (radioReceiver, radioSender) = self.radio.run();

        loop {
            let r = radioReceiver.try_recv();
            match r {
                Err(e) => {
                    if e.is_disconnected() {
                        r.unwrap(); // other threads crashed
                        panic!("Network tunnel crashed: {}", e);
                    }
                    // Otherwise - nothing to write, go on through.
                },
                Ok(data) => {
                    trace!("Received frame:\n{}", hex::encode(data));
                }
            }
        }
    }

}