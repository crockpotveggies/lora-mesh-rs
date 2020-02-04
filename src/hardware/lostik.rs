use log::*;
use std::fs;
use std::io::{BufRead, BufReader, Error, ErrorKind};
use std::io;
use crossbeam_channel;
use crossbeam_channel::{Sender, Receiver};
use hex;
use std::thread;
use std::time::{Duration, Instant};
use format_escape_default::format_escape_default;
use std::path::PathBuf;
use ratelimit;

use crate::hardware::serial::SerialIO;
use crate::Opt;

pub fn mkerror(msg: &str) -> Error {
    Error::new(ErrorKind::Other, msg)
}

#[derive(Clone)]
pub struct LoStik {
    // Application options
    opt: Opt,
    
    ser: SerialIO,

    // Lines coming from the radio
    readerlinesrx: crossbeam_channel::Receiver<String>,

    // Writer for external channel
    rxoutsender: crossbeam_channel::Sender<Vec<u8>>,

    // Receiver for external channel, to be used by application
    rxout: crossbeam_channel::Receiver<Vec<u8>>,

    // Blocks to transmit
    txblockstx: crossbeam_channel::Sender<Vec<u8>>,
    txblocksrx: crossbeam_channel::Receiver<Vec<u8>>,
}

/// Reads the lines from the radio and sends them down the channel to
/// the processing bits.
fn readerlinesthread(mut ser: SerialIO, tx: crossbeam_channel::Sender<String>) {
    loop {
        let line = ser.readln().expect("Error reading line");
        if let Some(l) = line {
            tx.send(l).unwrap();
        } else {
            debug!("{:?}: EOF", ser.portname);
            return;
        }
    }
}

/// Assert that a given response didn't indicate an EOF, and that it
/// matches the given text.  Return an IOError if either of these
/// conditions aren't met.  The response type is as given by
/// ['ser::SerialIO::readln'].
pub fn assert_response(resp: String, expected: String) -> io::Result<()> {
    if resp == expected {
        Ok(())
    } else {
        Err(mkerror(&format!("Unexpected response: got {}, expected {}", resp, expected)))
    }
}

/// Loop for sending and receiving radio data
/// Uses the Token Bucket algorithm to limit the transmission slot so
/// we can ensure we have a healthy amount of time to receive
pub fn radioloop(mut radio: LoStik, txslot: Duration, rxQueue: Sender<Vec<u8>>, txQueue: Receiver<Vec<u8>>) {
    let mut limiter = ratelimit::Builder::new()
        .capacity(5) //number of tokens the bucket will hold
        .quantum(5) //add one token per interval
        .interval(txslot) //add quantum tokens every 1 second
        .build();

    // flag if radio is transmitting or not
    radio.rxstart();
    let mut isrx = true;
    let mut extratx: Option<Vec<u8>> = None;

    // check if we're allowed to transmit
    // if yes, put in transmit mode and send frames, if any
    // strategy is to always transmit within allowed rate limit
    // otherwise we ensure the radio is in receiving mode
    loop {
        // grab the first frame to transmit and flag txwait
        let mut next = txQueue.try_recv();
        if next.is_err() && !isrx && extratx.is_none() {
            radio.rxstart();
            isrx = true;
        }
        // only dump the extra frame if we are already not receiving
        // otherwise we'll sneak it in the next tx slot
        if !isrx && extratx.is_some() {
            radio.tx(&extratx.unwrap());
            extratx = None;
        }
        if limiter.try_wait().is_ok() && next.is_ok() {
            if isrx {
                radio.rxstop(); // we're okay to transmit, stop receiver
                isrx = false;
            }
            if extratx.is_some() { // sneak in our extra frame
                radio.tx(&extratx.unwrap());
                extratx = None;
            }
            radio.tx(&next.unwrap()); // grab the next frame and transmit
        }
        else {
            if next.is_ok() { // we were rate limited, save the extra frame
                extratx = Some(next.unwrap());
            }
            if !isrx {
                radio.rxstart(); // we're okay to receive again
                isrx = true;
            }
        }
    }
}

impl LoStik {
    pub fn new(mut opt: Opt) -> LoStik {
        let (readerlinestx, readerlinesrx) = crossbeam_channel::unbounded();
        let (txblockstx, txblocksrx) = crossbeam_channel::bounded(2);
        let (rxoutsender, rxout) = crossbeam_channel::unbounded();

        let ser = SerialIO::new(opt.port.clone()).expect("Failed to initialize serial port");
        let ser2 = ser.clone();
        thread::spawn(move || readerlinesthread(ser2, readerlinestx));

        return LoStik { opt, ser, rxoutsender, rxout, readerlinesrx, txblockstx, txblocksrx};
    }

    pub fn run(&self) -> (Receiver<Vec<u8>>, Sender<Vec<u8>>) {
        // set up channels for sending and receiving packets
        let (inboundSender, inboundReceiver) = crossbeam_channel::unbounded();
        let (outboundSender, outboundReceiver) = crossbeam_channel::unbounded();

        let mut ls2 = self.clone();
        let txslot = self.opt.txslot.clone();
        thread::spawn(move || radioloop(ls2, Duration::from_millis(txslot), inboundSender, outboundReceiver));

        return (inboundReceiver, outboundSender);
    }

    /// apply radio settings using init file
    pub fn init(&mut self, mut initfile: Option<PathBuf>) -> io::Result<()> {
        // First, send it an invalid command.  Then, consume everything it sends back
        self.ser.writeln(String::from("INVALIDCOMMAND"))?;

        // Give it a chance to do its thing.
        thread::sleep(Duration::from_secs(1));

        // Consume all data.
        while let Ok(_) = self.readerlinesrx.try_recv() {
        }

        debug!("Configuring radio");
        let default = vec![
            "sys get ver",
            "mac reset",
            "mac pause",
            "radio get mod",
            "radio get freq",
            "radio get pwr",
            "radio get sf",
            "radio get bw",
            "radio get cr",
            "radio get wdt",
            "radio set pwr 20",
            "radio set sf sf12",
            "radio set bw 125",
            "radio set cr 4/5",
            "radio set wdt 60000"];

        let initlines: Vec<String> = if let Some(file) = initfile {
            let f = fs::File::open(file)?;
            let reader = BufReader::new(f);
            reader.lines().map(|l| l.unwrap()).collect()
        } else {
            default.iter().map(|l| String::from(*l)).collect()
        };

        for line in initlines {
            if line.len() > 0 {
                self.ser.writeln(line)?;
                self.oninit()?;
            }
        }
        Ok(())
    }

    fn oninit(&mut self) -> io::Result<()> {
        let line = self.readerlinesrx.recv().unwrap();
        if line == "invalid_param" {
            Err(mkerror("Bad response from radio during initialization"))
        } else {
            Ok(())
        }
    }

    fn onrx(&mut self, msg: String, readqual: bool) -> io::Result<()> {
        if msg.starts_with("radio_rx ") {
            if let Ok(mut decoded) = hex::decode(&msg.as_bytes()[10..]) {
                trace!("DECODED: {}", format_escape_default(&decoded));
                self.rxoutsender.send(decoded).unwrap();
            } else {
                return Err(mkerror("Error with hex decoding"));
            }
        }
        // Might get radio_err here.  That's harmless.
        Ok(())
    }

    pub fn rxstart(&mut self) -> io::Result<()> {
        // Enter read mode

        self.ser.writeln(String::from("radio rx 0"))?;
        let mut response = self.readerlinesrx.recv().unwrap();

        // For some reason, sometimes we get a radio_err here, then an OK.  Ignore it.
        if response == String::from("radio_err") {
            response = self.readerlinesrx.recv().unwrap();
        }
        assert_response(response, String::from("ok"))?;
        Ok(())
    }

    pub fn rxstop(&mut self) -> io::Result<()> {
        self.ser.writeln(String::from("radio rxstop"))?;
        let checkresp = self.readerlinesrx.recv().unwrap();
        if checkresp.starts_with("radio_rx ") {
            // We had a race.  A packet was coming in.  Decode and deal with it,
            // then look for the 'ok' from rxstop.  We can't try to read the quality in
            // this scenario.
            self.onrx(checkresp, false)?;
            self.readerlinesrx.recv().unwrap();  // used to pop this into checkresp, but no need now.
        }

        // Now, checkresp should hold 'ok'.
        //  It might not be; I sometimes see radio_err here.  it's OK too.
        // assert_response(checkresp, String::from("ok"))?;
        Ok(())
    }

    pub fn tx(&mut self, data: &[u8])  {
        self.txblockstx.send(data.to_vec()).unwrap();
    }

}