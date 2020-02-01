use std::io;
use serialport::prelude::*;
use std::io::{BufReader, BufRead, Write};
use log::*;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::path::PathBuf;

#[derive(Clone)]
pub struct SerialIO {
    // BufReader can't be cloned.  Sigh.
    pub br: Arc<Mutex<BufReader<Box<dyn SerialPort>>>>,
    pub swrite: Arc<Mutex<Box<dyn SerialPort>>>,
    pub portname: PathBuf
}

impl SerialIO {

    /// Initialize the serial system, configuring the port.
    pub fn new(portname: PathBuf) -> io::Result<SerialIO> {
        let settings = SerialPortSettings {
            baud_rate: 57600,
            data_bits: DataBits::Eight,
            flow_control: FlowControl::None,
            parity: Parity::None,
            stop_bits: StopBits::One,
            timeout: Duration::new(60 * 60 * 24 * 365 * 20, 0),
        };
        let readport = serialport::open_with_settings(&portname, &settings)?;
        let writeport = readport.try_clone()?;
        
        Ok(SerialIO {br: Arc::new(Mutex::new(BufReader::new(readport))),
                    swrite: Arc::new(Mutex::new(writeport)),
                    portname: portname})
    }

    /// Read a line from the port.  Return it with EOL characters removed.
    /// None if EOF reached.
    pub fn readln(&mut self) -> io::Result<Option<String>> {
        let mut buf = String::new();
        let size = self.br.lock().unwrap().read_line(&mut buf)?;
        if size == 0 {
            debug!("{:?}: Received EOF from serial port", self.portname); 
            Ok(None)
        } else {
            let buf = String::from(buf.trim());
            trace!("{:?} SERIN: {}", self.portname, buf);
            Ok(Some(buf))
        }
    }

    /// Transmits a command with terminating EOL characters
    pub fn writeln(&mut self, mut data: String) -> io::Result<()> {
        trace!("{:?} SEROUT: {}", self.portname, data);
        data.push_str("\r\n");
        // Give the receiver a chance to process
        self.swrite.lock().unwrap().write_all(data.as_bytes())?;
        self.swrite.lock().unwrap().flush()
    }
}


