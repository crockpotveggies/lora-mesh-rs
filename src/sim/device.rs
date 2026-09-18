use crate::radio::protocol::{LineCodec, MAX_FRAME, Micros, RadioProfile};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Firmware {
    #[default]
    RN2903,
    RN2483,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Idle,
    Receive,
    Transmit,
}
#[derive(Debug)]
pub enum Effect {
    Line(String),
    Transmit(Vec<u8>),
    ReceiveTimer(Micros, u64),
}
/// Device-side command grammar intentionally does not use the controller's validator.
/// Firmware profile models this project's raw LoRa subset, not the LoRaWAN MAC.
pub struct Device {
    pub firmware: Firmware,
    pub profile: RadioProfile,
    pub mode: Mode,
    pub epoch: u64,
    pub generation: u64,
    stored_profile: RadioProfile,
    pub online: bool,
    pub paused: bool,
    pub watchdog_us: Micros,
    pub power: i8,
    pub codec: LineCodec,
    pub output: VecDeque<u8>,
    pub drop_replies: u32,
    pub reply_delay_us: Micros,
}
impl Device {
    pub fn new(firmware: Firmware, profile: RadioProfile) -> Self {
        Self {
            firmware,
            stored_profile: profile.clone(),
            profile,
            mode: Mode::Idle,
            epoch: 0,
            generation: 0,
            online: true,
            paused: false,
            watchdog_us: 60_000_000,
            power: 14,
            codec: LineCodec::default(),
            output: VecDeque::new(),
            drop_replies: 0,
            reply_delay_us: 0,
        }
    }
    pub fn invalidate(&mut self) {
        self.epoch += 1;
        self.mode = Mode::Idle;
        self.codec.reset();
    }
    fn version(&self) -> String {
        format!("{:?} 1.0.5 virtual", self.firmware)
    }
    pub fn command(&mut self, line: &str) -> Vec<Effect> {
        if !self.online {
            return vec![];
        }
        let w: Vec<_> = line.split_whitespace().collect();
        let response = match w.as_slice() {
            ["sys", "reset"] => {
                self.invalidate();
                self.generation += 1;
                self.profile = self.stored_profile.clone();
                self.watchdog_us = 60_000_000;
                self.power = 14;
                self.output.clear();
                self.paused = false;
                self.version()
            }
            ["sys", "get", "ver"] => self.version(),
            ["mac", "pause"] => {
                self.paused = true;
                "4294967245".into()
            }
            ["mac", "reset"] if self.firmware == Firmware::RN2903 => {
                self.invalidate();
                self.paused = false;
                "ok".into()
            }
            ["mac", "reset", band]
                if self.firmware == Firmware::RN2483 && ["433", "868"].contains(band) =>
            {
                self.invalidate();
                self.paused = false;
                "ok".into()
            }
            ["sys", "set", "pindig", pin, bit]
                if ["GPIO10", "GPIO11"].contains(pin) && ["0", "1"].contains(bit) =>
            {
                "ok".into()
            }
            ["radio", "get", "wdt"] => (self.watchdog_us / 1000).to_string(),
            ["radio", "get", "pwr"] => self.power.to_string(),
            ["radio", "get", field] => self
                .profile
                .get(field)
                .unwrap_or_else(|| "invalid_param".into()),
            ["radio", "set", field, value] => {
                if self.mode != Mode::Idle {
                    "busy".into()
                } else {
                    let valid = match *field {
                        "pwr" => value
                            .parse::<i8>()
                            .map(|p| {
                                let max = if self.firmware == Firmware::RN2903 {
                                    20
                                } else {
                                    14
                                };
                                if (-3..=max).contains(&p) {
                                    self.power = p;
                                    true
                                } else {
                                    false
                                }
                            })
                            .unwrap_or(false),
                        "wdt" => value
                            .parse::<u32>()
                            .map(|v| {
                                self.watchdog_us = u64::from(v) * 1000;
                                true
                            })
                            .unwrap_or(false),
                        "freq" => value
                            .parse::<u32>()
                            .map(|v| {
                                let legal = match self.firmware {
                                    Firmware::RN2903 => (902_000_000..=928_000_000).contains(&v),
                                    Firmware::RN2483 => {
                                        (433_050_000..=434_790_000).contains(&v)
                                            || (863_000_000..=870_000_000).contains(&v)
                                    }
                                };
                                legal && self.profile.update(field, value).is_ok()
                            })
                            .unwrap_or(false),
                        _ => self.profile.update(field, value).is_ok(),
                    };
                    if valid {
                        "ok".into()
                    } else {
                        "invalid_param".into()
                    }
                }
            }
            ["radio", "rx", "0"] => {
                if !self.paused || self.mode != Mode::Idle {
                    "busy".into()
                } else {
                    self.invalidate();
                    self.mode = Mode::Receive;
                    let mut out = vec![Effect::Line("ok".into())];
                    if self.watchdog_us > 0 {
                        out.push(Effect::ReceiveTimer(self.watchdog_us, self.epoch));
                    }
                    return out;
                }
            }
            ["radio", "rxstop"] => {
                if self.mode == Mode::Transmit {
                    "busy".into()
                } else {
                    self.invalidate();
                    "ok".into()
                }
            }
            ["radio", "tx", payload] => {
                if !self.paused || self.mode != Mode::Idle {
                    "busy".into()
                } else if payload.len() > MAX_FRAME * 2 || payload.is_empty() {
                    "invalid_param".into()
                } else {
                    match hex::decode(payload) {
                        Ok(data) => {
                            self.invalidate();
                            self.mode = Mode::Transmit;
                            return vec![Effect::Line("ok".into()), Effect::Transmit(data)];
                        }
                        Err(_) => "invalid_param".into(),
                    }
                }
            }
            _ => "invalid_param".into(),
        };
        vec![Effect::Line(response)]
    }
}
