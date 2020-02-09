use log::*;
use std::thread;
use std::time::Duration;
use crate::stack::{NetworkTunnel, Frame};
use crate::hardware::LoStik;
use crate::stack::MeshRouter;
use crate::stack::message::{BroadcastMessage, MessageType, ToFromFrame};
use std::net::Ipv4Addr;
use crate::Opt;
use crate::stack::chunk::chunk_data;
use packet::ip::v4::Packet;

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
            let ipaddr = Some(Ipv4Addr::new(10,0,0, id as u8));
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
                    match router.packet_route(&data) {
                        None => {
                            trace!("Dropping packet to: {}", data.destination());
                            drop(data);
                        },
                        Some(route) => {
                            // TODO sort through the routes
                            // chunk it
                            // TODO move this to Frame
                            let chunks = chunk_data(Vec::from(data.as_ref()), (self.opt.maxpacketsize).clone());
                            for chunk in chunks {
                                radioSender.send(chunk);
                            }
                            continue;
                        }
                    }
                },
            }

            // now handle packets coming from radio
            // parse the frame, determine if it goes to our tunnel
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
//                    Frame::parse()
                }
            }
        }
    }

    /// Run only network discovery
    pub fn run_discovery(&mut self) {
        loop {
            self.broadcast();

            self.radio.rxstart();
            thread::sleep(Duration::from_millis(1000));

        }
    }

    /// Send a broadcast packet to nearby nodes
    pub fn broadcast(&mut self) {
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
        let mut frame = msg.to_frame(self.id);
        // dump
        self.radio.tx(&frame.bits());
    }


    /// Main loop for local tunnel dump
    pub fn run_dump(&mut self) {
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

}