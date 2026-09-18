#![no_main]
use libfuzzer_sys::fuzz_target;
use loramesh::{
    link::wire,
    mesh::{security::Vault, wire::Packet},
    radio::protocol::LineCodec,
};
use std::collections::BTreeMap;
use zeroize::Zeroizing;
fuzz_target!(|data: &[u8]| {
    let _ = wire::decode(data);
    let _ = wire::ipv4(data);
    if let Ok(packet) = Packet::decode(data) {
        assert_eq!(Packet::decode(&packet.encode().unwrap()).unwrap(), packet);
    }
    let mut codec = LineCodec::default();
    for byte in data {
        let _ = codec.push(*byte);
    }
    let mut receiver =
        Vault::memory(1, 2, 1, BTreeMap::from([(1, Zeroizing::new([7; 32]))])).unwrap();
    let _ = receiver.open(data);
});
