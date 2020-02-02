use crate::stack::tun::NetworkTunnel;
use crate::hardware::lostik::LoStik;

pub struct MeshNode {
    /// LoRa device for communication
    device: LoStik,
    /// Local network interface for IP
    networktunnel: NetworkTunnel,
    /// Is this a network gateway?
    isgateway: bool
}

impl MeshNode {

    pub fn new(device: LoStik, isgateway: bool) -> Self {
        let tun = NetworkTunnel::new(isgateway);
        MeshNode{
            device: device,
            networktunnel: tun,
            isgateway
        }
    }

    pub fn run(&self) {
        let mut buffer = vec![0; 1504];
        loop {
            // Every read is one packet. If the buffer is too small, bad luck, it gets truncated.
            let size = self.networktunnel.interface.recv(&mut buffer).unwrap();
            assert!(size >= 4);
            println!("Packet: {:?}", &buffer[4..size]);
        }

    }

}