//! Compatibility adapter for the legacy IPv4 node; all serial control lives in the library.
use crate::settings::Settings;
use crossbeam_channel::{Receiver, Sender};
use loramesh::radio::{ControllerConfig, runtime::RadioHandle};
use std::{
    fs::File,
    io::{self, Read},
    time::Duration,
};

pub struct LoStik {
    handle: RadioHandle,
    pub txsender: Sender<Vec<u8>>,
}
impl LoStik {
    pub fn new(opt: Settings) -> io::Result<Self> {
        let mut config = ControllerConfig::default();
        config.receive_guard_us = opt.txslot * 1000;
        if let Some(path) = opt.radiocfg {
            let mut content = String::new();
            File::open(path)?.take(8193).read_to_string(&mut content)?;
            if content.len() > 8192 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "radio configuration too large",
                ));
            }
            config.initialization = content
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_owned)
                .collect();
        }
        let handle = RadioHandle::serial(opt.radioport, config)?;
        handle.wait_ready(Duration::from_secs(10))?;
        let txsender = handle.tx.clone();
        Ok(Self { handle, txsender })
    }
    pub fn run(&self) -> (Receiver<Vec<u8>>, Sender<Vec<u8>>) {
        (self.handle.rx.clone(), self.txsender.clone())
    }
    pub fn diagnostics(&self) {
        for event in self.handle.events.try_iter().take(256) {
            match event {
                loramesh::radio::Action::Diagnostic(message) => log::warn!("Radio: {}", message),
                loramesh::radio::Action::Failed(id, message) => {
                    log::warn!("Radio frame {} dropped: {}", id, message)
                }
                _ => {}
            }
        }
    }
}
