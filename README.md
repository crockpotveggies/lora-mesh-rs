# LoRa Mesh

LoRa has opened up a realm of possibilities for Internet of Things and 
transmission of digital signals across long ranges. Currently it's very difficult
to find open source mesh networking for LoRa that supports IPv4 and application data.
This project aims to provide a simple mesh network for LoRa devices that route IP
traffic to a local interface, built entirely using [Rust](https://rust-lang.org/).

This is quite useful if you want to set up a network of devices and manage them remotely
or use existing IP protocols to interact with your applications.

The mesh only supports 256 nodes, with expanded capacity on the roadmap. The mesh software 
works out-of-the-box with [LoStik](https://ronoth.com/products/lostik).

## Roadmap

- [x] LoStik interface
- [x] Local network tunnel
- [x] Bridge radio and tunnel
- [x] Packet chunking
- [x] Node discovery
- [x] Message protocol
- [x] Gateway DHCP
- [ ] Multi-hop routing (spanning tree?)
- [ ] RTS/CTS collision prevention
- [ ] Multiple LoRa devices
- [ ] Support 65,536 nodes

## Credits
Special acknowledgement to those who made this possible:

- John Goerzen creator of [LoRaPipe](https://github.com/jgoerzen/lorapipe) 