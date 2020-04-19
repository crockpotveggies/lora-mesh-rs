# LoRa Mesh

![](https://github.com/crockpotveggies/lora-mesh-rs/workflows/LoRa%20Mesh%20Tests/badge.svg)

LoRa has opened up a realm of possibilities for IoT and transmission of digital signals 
across long ranges. Currently it's very difficult to find open source mesh networking for LoRa that 
support IPv4 and application data. This project aims to provide a simple mesh network for LoRa devices 
that route IP traffic to a local interface, built entirely using [Rust](https://rust-lang.org/).

This is quite useful if you want to set up a network of devices and manage them remotely or use existing 
IP protocols to interact with your applications.

The mesh only supports 256 nodes, with expanded capacity on the roadmap. The mesh software  works out-of-the-box with [LoStik](https://ronoth.com/products/lostik).

This software is **not ready for production-use yet**.

## Running

Running the application requires root permissions. Standing up a node is as simple as:

```
sudo ./loramesh
```

This creates a node with ID `0` and a local network interface `loratun0` that you can use to send
and receive packets in the network.

You can configure the node by creating a `/etc/loramesh/conf.yml` file, a sample is included in the 
`conf/` directory of this repository. Configuration can also be passed as env, such as `LOMESH_DEBUG=true`.

### Network Topology

Each node deployed on a network **must have a unique ID between 0-255**.

Each network should only have one gateway. Theoretically because the IP address are currently hardcoded
to each node ID, like `172.16.0.<ID>`, then multiple gateways may not be an issue.

### Protocol

The protocol is very naive and asynchronous in nature. Only IPv4 packets are supported and are not guaranteed
delivery. It is recommended that users stick to UDP and assume lossy connections. 

### Transmissions

Users will still need to respect their local laws regarding radio transmissions.

## Known Issues

Software has only been tested on Linux X86_64 and raspberry pi.

All transmissions are single channel and while some safeguards have been taken to prevent collisions this
is more difficult as the network size increase.

Currently using LoRa Mesh for accessing the outside internet through a gateway is unsupported. You may be 
able to configure the gateway to route DNS queries and requests with custom software. Currently it functions
as a private network.

Gateways currently do not save their state, this could be an issue for unreliable nodes.

## Roadmap

- [x] LoStik interface
- [x] Local network tunnel
- [x] Bridge radio and tunnel
- [x] Packet chunking
- [x] Node discovery
- [x] Message protocol
- [x] Gateway DHCP
- [x] Multi-hop routing
- [ ] Network failure recovery
- [ ] Frame [lz4](https://docs.rs/crate/lz4-compress/0.1.1/source/src/compress.rs) compression
- [ ] RTS/CTS collision prevention
- [ ] Multiple LoRa device hardware
- [ ] Security and encryption
- [ ] Support 65,536 nodes


## Credits

Special acknowledgement to those who made this possible:

- John Goerzen creator of [LoRaPipe](https://github.com/jgoerzen/lorapipe) 
