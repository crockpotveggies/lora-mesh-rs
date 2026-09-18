use crate::hardware::LoStik;
use crate::settings::Settings;
use crate::stack::*;
use crate::stack::{Frame, NetworkTunnel};
use crossbeam_channel::Sender;
use log::*;
use packet::ip::v4::Packet;
use rand::prelude::ThreadRng;
use rand::{Rng, thread_rng};
use ratelimit_meter::{DirectRateLimiter, LeakyBucket};
use std::borrow::BorrowMut;
use std::net::Ipv4Addr;
use std::time::Duration;

pub struct MeshNode {
    /// The ID of this node
    id: u8,
    /// IP address of this node's tunnel
    ipaddr: Option<Ipv4Addr>,
    /// LoRa device for communication
    radio: LoStik,
    /// Local network interface for IP
    networktunnel: NetworkTunnel,
    /// Router instance
    router: MeshRouter,
    /// Options
    opt: Settings,
}

impl MeshNode {
    pub fn new(id: u8, mut networktunnel: NetworkTunnel, radio: LoStik, opt: Settings) -> Self {
        // If this node is a gateway, assign an IP address of 172.16.0.<id>.
        // Otherwise, we will wait for DHCP from a network gateway and
        // assign a default address.
        let mut ipaddr = None;
        if opt.isgateway {
            ipaddr = Some(Ipv4Addr::new(172, 16, 0, id));
            networktunnel.assignipaddr(&ipaddr.unwrap());
            networktunnel.routeipaddr(&ipaddr.unwrap(), &networktunnel.tunip.unwrap());
            info!(
                "Network gateway detected, added route to {}",
                ipaddr.unwrap().to_string()
            );
        }
        let router = MeshRouter::new(
            id,
            None,
            opt.maxhops.clone(),
            Duration::from_millis(opt.chunktimeout.clone()),
            opt.isgateway.clone(),
        );

        MeshNode {
            id,
            ipaddr,
            radio,
            networktunnel,
            router,
            opt,
        }
    }

    /// Main loop, discover network and send/receive packets
    pub fn run(&mut self, stop: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        // random number generator for frame IDs
        let mut rng = thread_rng();

        // update the router if we are a gateway
        if self.opt.isgateway {
            self.router.handle_ip_assignment(&self.ipaddr.unwrap());
            self.router.handle_gateway_assignment(&self.ipaddr.unwrap());
        }

        // start i/o with local tunnel
        let tunreader = self.networktunnel.run();
        // start radio i/o
        let (rxreader, txsender) = self.radio.run();
        // rate limiters for different tasks
        let mut broadcastlimiter = DirectRateLimiter::<LeakyBucket>::new(
            nonzero!(1u32),
            Duration::from_secs(rng.gen_range(40, 80)),
        );
        let mut mstlimiter =
            DirectRateLimiter::<LeakyBucket>::new(nonzero!(1u32), Duration::from_secs(240));

        // hashmap for storing incomplete chunks
        let mut assemblies = crate::stack::assembly::LegacyAssemblies::new(Duration::from_millis(
            self.opt.chunktimeout,
        ));

        while !stop.load(std::sync::atomic::Ordering::Acquire) {
            // Block until either input is ready or a maintenance deadline needs attention.
            let mut inputs = crossbeam_channel::Select::new();
            inputs.recv(&tunreader);
            inputs.recv(&rxreader);
            let _ = inputs.ready_timeout(Duration::from_millis(100));
            self.radio.diagnostics();
            assemblies.expire(std::time::Instant::now());
            // handle packets coming from tunnel
            // pull the next packet from the receiver, process it, and determine if we
            // need to forward it to the radio
            let r = tunreader.try_recv();
            match r {
                Err(e) => {
                    if e.is_disconnected() {
                        r.unwrap(); // other threads crashed
                        panic!("Network tunnel crashed: {}", e);
                    }
                    // Otherwise - nothing to write, go on through.
                }
                Ok(data) => {
                    // apply routing logic
                    // if it cannot be routed, drop it
                    self.handle_tun_ip(rng, data, &txsender);
                }
            }

            // now handle packets coming from radio
            // parse the frame, and match against message type to
            // determine if it goes to our tunnel
            // or if it is routed to another node
            let r = rxreader.try_recv();
            match r {
                Err(e) => {
                    if e.is_disconnected() {
                        r.unwrap(); // other threads crashed
                        panic!("Network tunnel crashed: {}", e);
                    }
                    // Otherwise - nothing to write, go on through.
                }
                Ok(data) => {
                    match Frame::from_bytes(&data) {
                        Err(e) => {
                            debug!("Dropping radio frame {}", e);
                        }
                        Ok(mut frame) => {
                            trace!(
                                "Received frame txflag {} frameid {} sender {} routes {}",
                                &frame.txflag().to_u8(),
                                &frame.frameid(),
                                &frame.sender(),
                                &frame.routeoffset()
                            );
                            let _sender = frame.sender();
                            let _frameid = frame.frameid();
                            let mut frame = match assemblies.push(frame, std::time::Instant::now())
                            {
                                Ok(Some(frame)) => frame,
                                Ok(None) => continue,
                                Err(e) => {
                                    warn!("Dropping invalid/over-budget packet: {}", e);
                                    continue;
                                }
                            };
                            // TODO some things here depend if node is gateway
                            match frame.msgtype() {
                                // received IP packet, handle it
                                MessageType::IPPacket => {
                                    debug!("Recieved IP packet from {}", &frame.sender());
                                    match IPPacketMessage::from_frame(&mut frame) {
                                        Err(e) => {
                                            error!("Dropping invalid IPv4 packet message {}", e);
                                        }
                                        Ok(msg) => {
                                            let packet = msg.packet();
                                            self.handle_radio_ip(packet, frame, &txsender);
                                        }
                                    }
                                }
                                // process another node's broadcast
                                MessageType::Broadcast => {
                                    match BroadcastMessage::from_frame(frame.borrow_mut()) {
                                        Err(e) => error!("Could not parse BroadcastMessage: {}", e),
                                        Ok(broadcast) => {
                                            debug!(
                                                "Received broadcast from {} {:?}",
                                                &frame.sender(),
                                                broadcast.clone().ipaddr
                                            );
                                            // we aren't a gateway, we should rebroadcast this
                                            if !self.opt.isgateway
                                                && !frame.route().contains(&self.id)
                                            {
                                                frame.route_unshift(self.id.clone());
                                                if let Err(e) = txsender.try_send(frame.to_bytes())
                                                {
                                                    warn!(
                                                        "Radio queue full or disconnected: {}",
                                                        e
                                                    );
                                                }
                                            }
                                            // we need an IP to operate properly
                                            if self.ipaddr.is_some() {
                                                // add route to IP if new observation and we aren't a gateway
                                                if &frame.sender() != &self.id
                                                    && !self.opt.isgateway
                                                {
                                                    if broadcast.ipaddr.is_some() {
                                                        let ip = broadcast.ipaddr.unwrap().clone();
                                                        match self
                                                            .router
                                                            .node_observe_get(&frame.sender())
                                                        {
                                                            Some(_) => {}
                                                            None => {
                                                                info!(
                                                                    "Broadcast received from node {}, routing IP {}",
                                                                    &frame.sender(),
                                                                    &ip.to_string()
                                                                );
                                                                self.networktunnel.routeipaddr(
                                                                    &ip,
                                                                    &self.ipaddr.unwrap(),
                                                                );
                                                                // TODO should we put broadcast handler here and refactor gateway logic?
                                                            }
                                                        }
                                                    }
                                                };
                                                // let our router handle the broadcast and add route to IP if we are a gateway
                                                match self
                                                    .router
                                                    .handle_broadcast(broadcast, frame.route())
                                                {
                                                    Err(e) => {
                                                        error!(
                                                            "Failed to assign IP to broadcast from {}",
                                                            &frame.sender()
                                                        );
                                                        // ip address assignment failed, notify the source
                                                        let mut route: Vec<u8> = Vec::new();
                                                        if frame.route().len() > 0 {
                                                            route = frame.route().clone();
                                                        // this was multi-hop, send it back
                                                        } else {
                                                            route.push(frame.sender());
                                                        }
                                                        let bytes = e
                                                            .to_frame(
                                                                rng.gen_range(1u8, 244u8),
                                                                self.id,
                                                                route,
                                                            )
                                                            .to_bytes();
                                                        if let Err(e) = txsender.try_send(bytes) {
                                                            warn!(
                                                                "Radio queue full or disconnected: {}",
                                                                e
                                                            );
                                                        }
                                                    }
                                                    Ok(ip) => {
                                                        match ip {
                                                            None => (), // no response, we know this node already
                                                            Some((ipaddr, isnew)) => {
                                                                info!(
                                                                    "Sending IP {} to node {}",
                                                                    ipaddr.to_string(),
                                                                    frame.sender()
                                                                );

                                                                // tell the node of their new IP address
                                                                let mut route: Vec<u8> = Vec::new();
                                                                if frame.route().len() > 0 {
                                                                    route = frame.route().clone();
                                                                // this was multi-hop, send it back
                                                                } else {
                                                                    route.push(frame.sender());
                                                                }
                                                                let bits =
                                                                    IPAssignSuccessMessage::new(
                                                                        ipaddr,
                                                                    )
                                                                    .to_frame(
                                                                        rng.gen_range(1u8, 244u8),
                                                                        self.id,
                                                                        route,
                                                                    )
                                                                    .to_bytes();
                                                                if let Err(e) =
                                                                    txsender.try_send(bits)
                                                                {
                                                                    warn!(
                                                                        "Radio queue full or disconnected: {}",
                                                                        e
                                                                    );
                                                                }

                                                                // since we are a gateway, we must route the IP locally
                                                                if isnew {
                                                                    info!(
                                                                        "Broadcast received from node {}, assigned new IP {}",
                                                                        &frame.sender(),
                                                                        &ipaddr.to_string()
                                                                    );
                                                                    self.networktunnel.routeipaddr(
                                                                        &ipaddr,
                                                                        &self.ipaddr.unwrap(),
                                                                    );
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                // we were successfully assigned an IP
                                MessageType::IPAssignSuccess => {
                                    match frame.route_shift() {
                                        None => error!(
                                            "Received invalid IP message with no destination"
                                        ),
                                        Some(nexthop) => {
                                            if nexthop == self.id {
                                                // is it for us? drop if not
                                                if frame.route().len() == 0 {
                                                    match IPAssignSuccessMessage::from_frame(
                                                        frame.borrow_mut(),
                                                    ) {
                                                        Err(e) => error!(
                                                            "Could not parse IPAssignSuccessMessage: {}",
                                                            e
                                                        ),
                                                        Ok(message) => {
                                                            info!(
                                                                "Received new IP address {} from gateway {}",
                                                                &message.ipaddr.to_string(),
                                                                &frame.sender()
                                                            );
                                                            self.handle_ip_assignment(
                                                                message.ipaddr,
                                                            );
                                                        }
                                                    }
                                                }
                                                if frame.route().len() > 0 {
                                                    // retransmit to next hop
                                                    if let Err(e) =
                                                        txsender.try_send(frame.to_bytes())
                                                    {
                                                        warn!(
                                                            "Radio queue full or disconnected: {}",
                                                            e
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                // we sent a broadcast without IP, but got a failure
                                MessageType::IPAssignFailure => {
                                    match frame.route_shift() {
                                        None => error!(
                                            "Received invalid IP message with no destination"
                                        ),
                                        Some(nexthop) => {
                                            if nexthop == self.id {
                                                // is it for us? drop if not
                                                if frame.route().len() == 0 {
                                                    match IPAssignFailureMessage::from_frame(
                                                        frame.borrow_mut(),
                                                    ) {
                                                        Err(e) => error!(
                                                            "Could not parse IPAssignFailureMessage: {}",
                                                            e
                                                        ),
                                                        Ok(message) => error!(
                                                            "Failed to be assigned IP: {}",
                                                            message.reason
                                                        ),
                                                    }
                                                }
                                                if frame.route().len() > 0 {
                                                    // retransmit to next hop
                                                    if let Err(e) =
                                                        txsender.try_send(frame.to_bytes())
                                                    {
                                                        warn!(
                                                            "Radio queue full or disconnected: {}",
                                                            e
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                // handle route discovery
                                // TODO: refactor out old message architecture
                                MessageType::RouteDiscovery => {}
                                MessageType::RouteSuccess => {}
                                MessageType::RouteFailure => {}
                                MessageType::TransmitRequest => {}
                                MessageType::TransmitConfirm => {}
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
                self.router.min_spanning_tree();
            }
        }
    }

    /// Handle an IP assignment
    /// ensures a new local route is set up and node
    /// accepts new IP
    // TODO recover if new IP is different than old
    fn handle_ip_assignment(&mut self, ipaddr: Ipv4Addr) {
        if self.ipaddr.is_none() {
            self.ipaddr = Some(ipaddr);
            self.networktunnel.assignipaddr(&ipaddr);
            self.networktunnel
                .routeipaddr(&ipaddr, &self.networktunnel.tunip.unwrap());
            self.router.handle_ip_assignment(&ipaddr);
        }
    }

    /// Handle routing of a tunnel packet
    /// checks if packet was destinated for this node or if
    /// routing logic should be applied and forwarding necessary
    fn handle_tun_ip(
        &mut self,
        mut framerng: ThreadRng,
        packet: Packet<Vec<u8>>,
        txsender: &Sender<Vec<u8>>,
    ) {
        // apply routing logic
        // if it cannot be routed, drop it
        if self.ipaddr.is_some() {
            if packet.destination().eq(&self.ipaddr.unwrap()) {
                debug!("Received packet from {}", packet.source());
                if !self.opt.debug {
                    // TODO route to tunnel during debug
                    self.networktunnel.send(packet);
                }
            } else {
                // look up a route for this destination IP
                // then send it in chunks if necessary
                match self.router.packet_route(&packet) {
                    None => {
                        trace!("Dropping packet to: {}", packet.destination());
                        drop(packet);
                    }
                    Some(route) => {
                        let message = IPPacketMessage::new(packet);
                        let chunks = message
                            .to_frame(framerng.gen_range(1, 244) as u8, self.id.clone(), route)
                            .chunked(&self.opt.maxpacketsize);
                        for chunk in chunks {
                            trace!("Sending chunk");
                            if let Err(e) = txsender.try_send(chunk) {
                                warn!("Radio queue full or disconnected: {}", e);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Handle routing of an IP packet from radio
    /// checks if packet was destined for this node or if
    /// it should be passed to the next hop
    fn handle_radio_ip(
        &mut self,
        packet: Packet<Vec<u8>>,
        frame: Frame,
        txsender: &Sender<Vec<u8>>,
    ) {
        // apply routing logic
        // was this packet meant for us? if not, drop
        match self.ipaddr {
            None => {
                self.handle_ip_nexthop(packet, frame, txsender);
            }
            Some(ipaddr) => {
                if packet.destination().eq(&ipaddr) {
                    trace!(
                        "Forwarding IP packet from {} to local network",
                        packet.source()
                    );
                    self.networktunnel.send(packet);
                } else {
                    trace!("Forwarding IP packet from {} to next hop", packet.source());
                    self.handle_ip_nexthop(packet, frame, txsender);
                }
            }
        }
    }

    /// retransmit or drop an IP packet bound for another destination
    fn handle_ip_nexthop(
        &mut self,
        packet: Packet<Vec<u8>>,
        mut frame: Frame,
        txsender: &Sender<Vec<u8>>,
    ) {
        match frame.route_shift() {
            // there wasn't a next hop, something's wrong
            None => error!(
                "Received an IP packet from {} with no route",
                &frame.sender()
            ),
            Some(nexthop) => {
                if nexthop == self.id {
                    warn!("Dropping invalid self-directed forwarding route");
                    return;
                }

                // we can still forward it to another node id
                if frame.route().len() > 0 {
                    // chunk it
                    let chunks = frame.chunked(&self.opt.maxpacketsize);
                    for chunk in chunks {
                        if let Err(e) = txsender.try_send(chunk) {
                            warn!("Radio queue full or disconnected: {}", e);
                        }
                    }
                } else {
                    error!(
                        "Dropping IP packet from {} to {}: no route available",
                        &packet.source(),
                        &packet.destination()
                    );
                }
            }
        }
    }

    /// Send a broadcast packet to nearby nodes
    fn broadcast(&mut self) {
        // prepare broadcast
        if self.radio.txsender.is_empty() {
            let mut ipOffset = 0;
            if self.ipaddr.is_some() {
                ipOffset = 4;
            }
            let msg = BroadcastMessage {
                header: None,
                isgateway: self.opt.isgateway.clone(),
                ipOffset,
                ipaddr: self.ipaddr,
            };
            let mut route: Vec<u8> = Vec::new();
            route.push(self.id.clone());
            let mut frame = msg.to_frame(1u8, self.id, route);
            // dump
            if let Err(e) = self.radio.txsender.try_send(frame.to_bytes()) {
                warn!("Radio queue full or disconnected: {}", e);
            }
        }
    }
}
