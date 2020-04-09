use std::convert::TryInto;
use std::net::Ipv4Addr;

pub fn parse_bool(byte: u8) -> std::io::Result<bool> {
    if byte as i8 == 0i8 { return Ok(false); }
    else if byte as i8 == 1i8 { return Ok(true); }
    panic!("Booleans should bubble an error when parsed incorrectly: {}", byte);
}

pub fn parse_byte(boolean: bool) -> u8 {
    if boolean { return 1i8 as u8; }
    else { return 0i8 as u8; }
}

pub fn to_octets(arr: &[u8]) -> [u8; 4] {
    arr.try_into().expect("Incorrect array length for IP octets")
}

pub fn parse_ipv4(arr: &[u8]) -> Ipv4Addr {
    Ipv4Addr::from(to_octets(arr))
}

pub fn parse_string(arr: &[u8]) -> Vec<u8> {
    Vec::from(arr)
}

pub fn composite_key(id1: &u8, id2: &u8) -> String {
    format!("{}-{}", id1, id2)
}