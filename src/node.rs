use log::*;
use std::thread;
use std::time::Duration;
use clokwerk::{Scheduler, TimeUnits};
use clokwerk::Interval::*;
use crate::stack::{NetworkTunnel, Frame};
use crate::hardware::LoStik;
use crate::stack::MeshRouter;
use crate::stack::message::{BroadcastMessage, MessageType, MessageHeader};
use std::net::Ipv4Addr;
use crate::Opt;
use crate::stack::chunk::chunk_data;

pub struct MeshNode {
    /// The ID of this node
    id: i8,
    /// LoRa device for communication
    radio: LoStik,
    /// Local network interface for IP
    networktunnel: NetworkTunnel,
    /// Mesh router instance on local node
    router: MeshRouter,
    /// Network scheduler
    scheduler: Scheduler,
    /// Options
    opt: Opt
}

impl MeshNode {

    pub fn new(id: i8, networktunnel: NetworkTunnel, radio: LoStik, opt: Opt) -> Self {
        let scheduler = Scheduler::new();
        let mut router = MeshRouter::new(opt.maxhops as i32, Duration::from_millis(opt.timeout));

        MeshNode{
            id,
            radio,
            networktunnel,
            router,
            scheduler,
            opt,
        }
    }

    /// Main loop, discover network and send/receive packets
    pub fn run(&mut self) {
        // start i/o with local tunnel
        let (tunReceiver, tunSender) = self.networktunnel.split();
        // start radio i/o
        let (radioReceiver, radioSender) = self.radio.run();

        // use token bucket algorithm to rate limit transmission
        loop {
            let r = tunReceiver.try_recv(); // forward tunnel packets
            match r {
                Ok(data) => {
                    // TODO IP layer and headers/flags on chunks
                    let chunks = chunk_data(data, (self.opt.maxpacketsize).clone());
                    for chunk in chunks {
                        radioSender.send(chunk);
                    }
                    continue;
                },
                Err(e) => {
                    if e.is_disconnected() {
                        r.unwrap(); // other threads crashed
                    }
                    // Otherwise - nothing to write, go on through.
                }
            }

            // if we don't need to transmit, enter rx
            if true {

            } else {
                self.radio.rxstart();
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
        let msgheader = MessageHeader { msgtype: MessageType::Broadcast, sender: self.id.clone() };
        let msg = BroadcastMessage {
            header: msgheader,
            isgateway: true,
            ipaddr: self.networktunnel.ipaddr
        };
        let mut packet = Frame::from_broadcast(msg);
        // dump
        self.radio.tx(&packet.bits());
    }


    /// Main loop for local tunnel dump
    pub fn run_dump(&mut self) {
        loop {
            // Read next packet from network tunnel
            let (receiver, _sender) = self.networktunnel.split();
            let r = receiver.recv();
            match r {
                Ok(data) => {
                    let size = data.len();
                    trace!("Packet: {:?}", &data[0..size]);
                },
                Err(_e) => {
                    // do nothing
                }
            }

        }
    }

}