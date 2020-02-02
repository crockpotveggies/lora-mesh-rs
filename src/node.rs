use crate::stack::tun::NetworkTunnel;
use crate::hardware::lostik::LoStik;

pub struct MeshNode {
    /// The ID of this node
    id: i32,
    /// LoRa device for communication
    device: LoStik,
    /// Local network interface for IP
    networktunnel: NetworkTunnel,
    /// Is this a network gateway?
    isgateway: bool
}

impl MeshNode {

    pub fn new(id: i32, device: LoStik, isgateway: bool) -> Self {
        let networktunnel = NetworkTunnel::new(isgateway);

        MeshNode{
            id,
            device,
            networktunnel,
            isgateway
        }
    }

    /// Main loop handles all networking, device, and routing
    pub fn run(&self) {
        let mut buffer = vec![0; 1504];
        loop {
            // Every read is one packet. If the buffer is too small, bad luck, it gets truncated.
            let size = self.networktunnel.interface.recv(&mut buffer).unwrap();
            assert!(size >= 4);
            println!("Packet: {:?}", &buffer[4..size]);
        }

    }

//    /// Handle messages once they're parsed
//    pub fn handlemessage

}