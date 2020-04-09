use crate::{MESH_MAX_MESSAGE_LEN};
use enumn::N;
use std::net::Ipv4Addr;
use crate::stack::{Frame, MessageType};
use crate::stack::frame::{FrameHeader, ToFromFrame};
use crate::stack::util::{parse_ipv4};

/// Notify node of their new IP address.
pub struct IPAssignSuccessMessage {
    pub header: Option<FrameHeader>,
    pub ipaddr: Ipv4Addr
}

impl IPAssignSuccessMessage {
    pub fn new(ipaddr: Ipv4Addr) -> Self {
        return IPAssignSuccessMessage{ header: None, ipaddr}
    }
}

impl ToFromFrame for IPAssignSuccessMessage {
    fn from_frame(f: &mut Frame) -> std::io::Result<Box<Self>> {
        let header = f.header();
        let data = f.payload();
        let octets = &data[0..data.len()];
        let ipaddr = parse_ipv4(octets);

        Ok(Box::new(IPAssignSuccessMessage {
            header: Some(header),
            ipaddr
        }))
    }

    fn to_frame(&self, frameid: u8, sender: u8, route: Vec<u8>) -> Frame {
        // cast the route
        let route: Vec<u8> = route.clone().iter().map(|i| i.clone() as u8).collect();
        let routeoffset = route.len() as u8;

        // write the payload
        let mut data: Vec<u8> = Vec::new();
        let octets = self.ipaddr.octets();
        octets.iter().for_each(|oct| data.push(oct.clone()));

        Frame::new(
            0i8 as u8,
            frameid,
            MessageType::IPAssignSuccess as u8,
            sender as u8,
            routeoffset as u8,
            route,
            data
        )
    }
}

/// Assigning IP to node failed, tell them.
pub struct IPAssignFailureMessage {
    pub header: Option<FrameHeader>,
    pub reason: String
}

impl IPAssignFailureMessage {
    pub fn new(reason: String) -> Self {
        return IPAssignFailureMessage{ header: None, reason}
    }
}

impl ToFromFrame for IPAssignFailureMessage {
    fn from_frame(f: &mut Frame) -> std::io::Result<Box<Self>> {
        let header = f.header();
        let reason = String::from_utf8(f.payload()).expect("Could not parse UTF-8 message");

        Ok(Box::new(IPAssignFailureMessage {
            header: Some(header),
            reason
        }))
    }

    fn to_frame(&self, frameid: u8, sender: u8, route: Vec<u8>) -> Frame {
        // cast the route
        let route: Vec<u8> = route.clone().iter().map(|i| i.clone() as u8).collect();
        let routeoffset = route.len() as u8;

        let payload = &self.reason;

        Frame::new(
            0u8,
            frameid,
            MessageType::IPAssignFailure as u8,
            sender as u8,
            routeoffset,
            route,
            payload.clone().into_bytes()
        )
    }
}