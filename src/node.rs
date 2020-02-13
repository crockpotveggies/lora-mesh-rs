use log::*;
use std::thread;
use std::time::Duration;
use crate::stack::{NetworkTunnel, Frame};
use crate::hardware::LoStik;
use crate::stack::MeshRouter;
use crate::stack::message::*;
use std::net::Ipv4Addr;
use crate::Opt;
use packet::ip::v4::Packet;
use ratelimit_meter::{DirectRateLimiter, LeakyBucket};
use crossbeam_channel::Sender;
use std::borrow::{Borrow, BorrowMut};
use hex;
use crate::stack::tun::ipassign;
use std::collections::HashMap;
use crate::stack::frame::recombine_chunks;

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
        // If this node is a gateway, assign an IP address of 172.16.0.<id>.
        // Otherwise, we will wait for DHCP from a network gateway and
        // assign a default address.
        let mut ipaddr = None;
        if opt.isgateway {
            ipaddr = Some(Ipv4Addr::new(172,16,0, id as u8));
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
        let mut router: MeshRouter;
        if self.opt.isgateway {
            router = MeshRouter::new(self.id, self.ipaddr, self.ipaddr, self.opt.maxhops as i32, Duration::from_millis(self.opt.timeout), self.opt.isgateway);
        }
        else {
            router = MeshRouter::new(self.id, self.ipaddr, None, self.opt.maxhops as i32, Duration::from_millis(self.opt.timeout), self.opt.isgateway);
        }
        // start i/o with local tunnel
        let (tunReceiver, tunSender) = self.networktunnel.split();
        // start radio i/o
        let (radioReceiver, radioSender) = self.radio.run();
        // rate limiters for different tasks
        let mut broadcastlimiter = DirectRateLimiter::<LeakyBucket>::new(nonzero!(1u32), Duration::from_secs(30));
        let mut mstlimiter = DirectRateLimiter::<LeakyBucket>::new(nonzero!(1u32), Duration::from_secs(240));

        // hashmap for storing incomplete chunks
        let mut rxchunks: HashMap<i8, Vec<Frame>> = HashMap::new();

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
                    self.handle_tun_ip(data, router.borrow_mut(), None, Some(&tunSender));
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
                    match Frame::from_bytes(&data) {
                        Err(e) => {
                            trace!("Received invalid radio frame, dropping");
                        },
                        Ok(mut frame) => {
                            // if this is a chunked packet, save the chunk
                            // in the hashmap and come back to it
                            if frame.txflag().more_chunks() {
                                match rxchunks.get_mut(&frame.sender()) {
                                    None => {
                                        let mut chunks = Vec::new();
                                        chunks.push(frame);
                                    },
                                    Some(mut chunks) => {
                                        chunks.push(frame);
                                    }
                                }
                            } else {
                                // do we need to recombine previous chunks?
                                match rxchunks.remove(&frame.sender()) {
                                    None => continue,
                                    Some(mut chunks) => {
                                        let header = frame.header();
                                        chunks.push(frame); // push final frame
                                        frame = recombine_chunks(chunks, header);
                                        continue;
                                    }
                                }
                                // TODO some things here depend if node is gateway
                                match frame.msgtype() {
                                    // received IP packet, handle it
                                    MessageType::IPPacket => {
                                        let packet = Packet::new(frame.payload()).expect("Could not parse IPv4 packet");
                                        self.handle_radio_ip(packet, frame, router.borrow_mut(), Some(&radioSender), None);
                                    },
                                    // process another node's broadcast
                                    MessageType::Broadcast => {
                                        match BroadcastMessage::from_frame(frame.borrow_mut()) {
                                            Err(e) => error!("Could not parse BroadcastMessage: {}", e),
                                            Ok(broadcast) => {
                                                // we aren't a gateway, we should rebroadcast this
                                                // if we haven't already
                                                if !self.opt.isgateway && !frame.route().contains(&self.id) {
                                                    frame.route_unshift(self.id.clone());
                                                    radioSender.send(frame.to_bytes());
                                                }
                                                // let our router handle the broadcast
                                                match router.handle_broadcast(broadcast, frame.route()) {
                                                    Err(e) => {
                                                        // ip address assignment failed, notify the source
                                                        let mut route: Vec<i8> = Vec::new();
                                                        if frame.route().len() > 0 {
                                                            route = frame.route().clone(); // this was multi-hop, send it back
                                                        } else {
                                                            route.push(frame.sender() as i8);
                                                        }
                                                        let bytes = e.to_frame(self.id, route).to_bytes();
                                                        radioSender.send(bytes);
                                                    },
                                                    Ok(ip) => {
                                                        match ip {
                                                            None => (), // no response, we know this node already
                                                            Some(ipaddr) => {
                                                                // tell the node of their new IP address
                                                                let mut route: Vec<i8> = Vec::new();
                                                                if frame.route().len() > 0 {
                                                                    route = frame.route().clone(); // this was multi-hop, send it back
                                                                } else {
                                                                    route.push(frame.sender() as i8);
                                                                }
                                                                let bits = IPAssignSuccessMessage::new(ipaddr).to_frame(self.id, route).to_bytes();
                                                                radioSender.send(bits);

                                                                // since we are a gateway, we must route the IP locally
                                                                ipassign(&self.networktunnel.interface, &ipaddr);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    },
                                    // we were successfully assigned an IP
                                    MessageType::IPAssignSuccess => {
                                        match frame.route_shift() {
                                            None => error!("Received invalid IP message with no destination"),
                                            Some(nexthop) => {
                                                if nexthop == self.id { // is it for us? drop if not
                                                    if frame.route().len() == 0 {
                                                        match IPAssignSuccessMessage::from_frame(frame.borrow_mut()) {
                                                            Err(e) => error!("Could not parse IPAssignSuccessMessage: {}", e),
                                                            Ok(message) => {
                                                                info!("Assigned new IP address {}", message.ipaddr.to_string());
                                                                self.handle_ip_assignment(message.ipaddr);
                                                            }
                                                        }
                                                    }
                                                    if frame.route().len() > 0 { // retransmit to next hop
                                                        radioSender.send(frame.to_bytes());
                                                    }
                                                }
                                            }
                                        }
                                    },
                                    // we sent a broadcast without IP, but got a failure
                                    MessageType::IPAssignFailure => {
                                        match frame.route_shift() {
                                            None => error!("Received invalid IP message with no destination"),
                                            Some(nexthop) => {
                                                if nexthop == self.id { // is it for us? drop if not
                                                    if frame.route().len() == 0 {
                                                        match IPAssignFailureMessage::from_frame(frame.borrow_mut()) {
                                                            Err(e) => error!("Could not parse IPAssignFailureMessage: {}", e),
                                                            Ok(message) => error!("Failed to be assigned IP: {}", message.reason)
                                                        }
                                                    }
                                                    if frame.route().len() > 0 { // retransmit to next hop
                                                        radioSender.send(frame.to_bytes());
                                                    }
                                                }
                                            }
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
            }

            // now handle any protocol tasks
            // such as broadcasts or route discovery
            if broadcastlimiter.check().is_ok() {
                debug!("Sending broadcast to nearby nodes");
                self.broadcast();
            }

            // clean up the mesh graph to optimize
            // routing and performance
            if mstlimiter.check().is_ok() {
                debug!("Applying minimum spanning tree to mesh router");
                router.min_spanning_tree();
            }
        }
    }

    /// Handle an IP assignment
    /// ensures a new local route is set up and node
    /// accepts new IP
    fn handle_ip_assignment(&mut self, ipaddr: Ipv4Addr) {
        self.ipaddr = Some(ipaddr);
        ipassign(&self.networktunnel.interface, &ipaddr);
    }

    /// Handle routing of a tunnel packet
    /// checks if packet was destinated for this node or if
    /// routing logic should be applied and forwarding necessary
    fn handle_tun_ip(&mut self, mut packet: Packet<Vec<u8>>, mut router: &mut MeshRouter, mut radioSender: Option<&Sender<Vec<u8>>>, mut tunSender: Option<&Sender<Vec<u8>>>) {
        // apply routing logic
        // if it cannot be routed, drop it
        if self.ipaddr.is_some() {
            if packet.destination().eq(&self.ipaddr.unwrap()) {
                debug!("Received packet from {}", packet.source());
                if !self.opt.debug {
                    // TODO route to tunnel during debug
                    // TODO why can't we get the raw buffer!?
                    tunSender.unwrap().send(Vec::from(packet.as_ref()));
                }
            }
            else {
                // look up a route for this destination IP
                // then send it in chunks if necessary
                match router.packet_route(&packet) {
                    None => {
                        trace!("Dropping packet to: {}", packet.destination());
                        drop(packet);
                    },
                    Some(route) => {
                        let mut message = IPPacketMessage::new(packet);
                        let chunks = message.to_frame(self.id.clone(), route).chunked(&self.opt.maxpacketsize);
                        for chunk in chunks {
                            radioSender.unwrap().send(chunk);
                        }
                    }
                }
            }
        }
    }

    /// Handle routing of an IP packet from radio
    /// checks if packet was destined for this node or if
    /// it should be passed to the next hop
    fn handle_radio_ip(&mut self, mut packet: Packet<Vec<u8>>, mut frame: Frame, mut router: &mut MeshRouter, mut radioSender: Option<&Sender<Vec<u8>>>, mut tunSender: Option<&Sender<Vec<u8>>>) {
        // apply routing logic
        // was this packet meant for us? if not, drop
        match frame.route_shift() {
            // there wasn't a next hop, something's wrong
            None => error!("Received an IP packet with no route"),
            Some(nexthop) => {
                if nexthop == self.id {
                    // are we the final destination?
                    match self.ipaddr {
                        None => {
                            // we can still forward it to another node id
                            if frame.route().len() > 0 {
                                // chunk it
                                let chunks = frame.chunked(&self.opt.maxpacketsize);
                                for chunk in chunks {
                                    radioSender.unwrap().send(chunk);
                                }
                            }
                        },
                        Some(localip) => {
                            if packet.destination().eq(&self.ipaddr.unwrap()) {
                                trace!("Received packet from {}", packet.source());
                                if !self.opt.debug {
                                    // TODO route to tunnel during debug
                                    // TODO why can't we get the raw buffer!?
                                    tunSender.unwrap().send(Vec::from(packet.as_ref()));
                                }
                            }
                            // packet wasn't meant for us, forward it
                            else {
                                // chunk it
                                // TODO move this to Frame
                                let chunks = frame.chunked(&self.opt.maxpacketsize);
                                for chunk in chunks {
                                    radioSender.unwrap().send(chunk);
                                }
                            }
                        }
                    }
                }
            }
        }
        // if it cannot be routed, drop it
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
        let mut route: Vec<i8> = Vec::new();
        route.push(self.id.clone());
        let mut frame = msg.to_frame(self.id, route);
        // dump
        self.radio.tx(&frame.to_bytes());
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