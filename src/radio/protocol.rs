use serde::{Deserialize, Serialize};
use std::io::{self, Error, ErrorKind};

pub const MAX_FRAME: usize = 255;
pub const MAX_LINE: usize = 600;
pub type Micros = u64;
pub fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

/// Bounded streaming framer. After an overlong line, discard through the next LF.
#[derive(Default, Debug)]
pub struct LineCodec {
    buffer: Vec<u8>,
    discarding: bool,
}
impl LineCodec {
    pub fn push(&mut self, byte: u8) -> Option<io::Result<String>> {
        if byte == b'\n' {
            if self.discarding {
                self.discarding = false;
                return None;
            }
            let mut bytes = std::mem::take(&mut self.buffer);
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            return Some(String::from_utf8(bytes).map_err(|_| invalid("non-UTF8 serial line")));
        }
        if self.discarding {
            return None;
        }
        if self.buffer.len() == MAX_LINE {
            self.buffer.clear();
            self.discarding = true;
            return Some(Err(invalid("serial line too long")));
        }
        self.buffer.push(byte);
        None
    }
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.discarding = false;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Ok,
    Busy,
    Invalid,
    RadioError,
    TxDone,
    Rx(Vec<u8>),
    Value(String),
}
impl Reply {
    pub fn parse(line: &str) -> io::Result<Self> {
        if line.len() > MAX_LINE {
            return Err(invalid("serial line too long"));
        }
        let line = line.trim();
        Ok(match line {
            "ok" => Self::Ok,
            "busy" => Self::Busy,
            "invalid_param" => Self::Invalid,
            "radio_err" => Self::RadioError,
            "radio_tx_ok" => Self::TxDone,
            _ if line.starts_with("radio_rx") => {
                let data = line
                    .strip_prefix("radio_rx ")
                    .ok_or_else(|| invalid("bad RX prefix"))?
                    .trim();
                if data.is_empty() || data.len() > MAX_FRAME * 2 {
                    return Err(invalid("bad RX length"));
                }
                Self::Rx(hex::decode(data).map_err(|_| invalid("invalid RX hex"))?)
            }
            _ if !line.is_empty()
                && line.is_ascii()
                && !line.bytes().any(|b| b.is_ascii_control()) =>
            {
                Self::Value(line.to_owned())
            }
            _ => return Err(invalid("invalid serial response")),
        })
    }
}

/// Supported LoRa profile; units are Hz and microseconds, CR is denominator (5..8).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RadioProfile {
    pub frequency: u32,
    pub sf: u8,
    pub bandwidth: u32,
    pub coding_rate: u8,
    pub preamble: u16,
    pub crc: bool,
    pub sync: u8,
}
impl Default for RadioProfile {
    fn default() -> Self {
        Self {
            frequency: 915_000_000,
            sf: 12,
            bandwidth: 125_000,
            coding_rate: 5,
            preamble: 8,
            crc: true,
            sync: 18,
        }
    }
}
impl RadioProfile {
    pub fn validate(&self) -> io::Result<()> {
        if !(7..=12).contains(&self.sf)
            || ![125_000, 250_000, 500_000].contains(&self.bandwidth)
            || !(5..=8).contains(&self.coding_rate)
            || self.preamble < 6
            || self.preamble > 4096
            || !(137_000_000..=1_020_000_000).contains(&self.frequency)
        {
            return Err(invalid("unsupported LoRa profile"));
        }
        Ok(())
    }
    /// Semtech explicit-header airtime equation, rounded up to a whole microsecond.
    pub fn airtime_us(&self, length: usize) -> io::Result<Micros> {
        self.validate()?;
        if length > MAX_FRAME {
            return Err(invalid("radio frame exceeds 255 bytes"));
        }
        let sf = i64::from(self.sf);
        let de = i64::from((1u64 << self.sf) * 1_000_000 / u64::from(self.bandwidth) >= 16_000);
        let numerator = 8 * length as i64 - 4 * sf + 28 + if self.crc { 16 } else { 0 };
        let denominator = 4 * (sf - 2 * de);
        let groups = if numerator <= 0 {
            0
        } else {
            (numerator + denominator - 1) / denominator
        };
        let payload_symbols = 8 + groups as u64 * u64::from(self.coding_rate);
        let quarter_symbols = (u64::from(self.preamble) + payload_symbols) * 4 + 17;
        let numerator = quarter_symbols * (1u64 << self.sf) * 1_000_000;
        let denominator = 4 * u64::from(self.bandwidth);
        Ok(numerator.div_ceil(denominator))
    }
    pub fn compatible(&self, other: &Self) -> bool {
        self.frequency == other.frequency
            && self.sf == other.sf
            && self.bandwidth == other.bandwidth
            && self.coding_rate == other.coding_rate
            && self.sync == other.sync
            && self.crc == other.crc
    }
    pub fn update(&mut self, field: &str, value: &str) -> io::Result<()> {
        let mut next = self.clone();
        match field {
            "sf" => {
                next.sf = value
                    .strip_prefix("sf")
                    .ok_or_else(|| invalid("bad SF"))?
                    .parse()
                    .map_err(|_| invalid("bad SF"))?
            }
            "bw" => {
                next.bandwidth = value
                    .parse::<u32>()
                    .map_err(|_| invalid("bad BW"))?
                    .checked_mul(1000)
                    .ok_or_else(|| invalid("bad BW"))?
            }
            "cr" => {
                next.coding_rate = value
                    .strip_prefix("4/")
                    .ok_or_else(|| invalid("bad CR"))?
                    .parse()
                    .map_err(|_| invalid("bad CR"))?
            }
            "prlen" => next.preamble = value.parse().map_err(|_| invalid("bad preamble"))?,
            "crc" => {
                next.crc = match value {
                    "on" => true,
                    "off" => false,
                    _ => return Err(invalid("bad CRC")),
                }
            }
            "freq" => next.frequency = value.parse().map_err(|_| invalid("bad frequency"))?,
            "sync" => {
                next.sync = u8::from_str_radix(value, 16).map_err(|_| invalid("bad sync word"))?
            }
            "mod" if value == "lora" => {}
            _ => return Err(invalid("unsupported radio field")),
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub fn get(&self, field: &str) -> Option<String> {
        Some(match field {
            "sf" => format!("sf{}", self.sf),
            "bw" => (self.bandwidth / 1000).to_string(),
            "cr" => format!("4/{}", self.coding_rate),
            "prlen" => self.preamble.to_string(),
            "crc" => if self.crc { "on" } else { "off" }.to_owned(),
            "freq" => self.frequency.to_string(),
            "sync" => format!("{:02X}", self.sync),
            "mod" => "lora".to_owned(),
            _ => return None,
        })
    }
}
