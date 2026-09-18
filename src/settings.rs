use config::{ConfigError, File};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Clone)]
pub struct Settings {
    /// The ID of this LoRa node
    /* This sets the ID of the node, similar to a MAC address. This must be
    between 1 and 255 otherwise the node will enter local test mode. It is recommended
    you set the gateway as 1. */
    pub nodeid: u8,

    /// Activate debug mode
    // short and long flags (-d, --debug) will be deduced from the field's name
    pub debug: bool,

    /// Set if node is a gateway to internet
    /* Turning this on will enable special networking features, including a
    DHCP server and will assign IP addresses to other nodes in the mesh. */
    pub isgateway: bool,

    /// Local device port for radio
    pub radioport: PathBuf,

    /// Radio initialization command file
    pub radiocfg: Option<PathBuf>,

    /// Maximum legacy payload chunk size [10..250]; the complete frame is capped at 255 bytes.
    pub maxpacketsize: usize,

    /// Minimum receive window after each completed transmission, in milliseconds.
    /// This is a listening guard, not a regulatory duty-cycle limit.
    pub txslot: u64,

    /// Timeout (ms) to drop incomplete packet chunks
    pub chunktimeout: u64,

    /// Maximum number of hops a packet should travel
    pub maxhops: u8,
}

impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        let mut settings = config::Config::default();
        settings.set_default("nodeid", 0)?;
        settings.set_default("debug", false)?;
        settings.set_default("isgateway", false)?;
        settings.set_default("radioport", "/dev/ttyUSB0")?;
        settings.set_default::<Option<&str>>("radiocfg", None)?;
        settings.set_default("maxpacketsize", 200)?;
        settings.set_default("txslot", 1000)?;
        settings.set_default("chunktimeout", 120000)?;
        settings.set_default("maxhops", 2)?;

        // local user settings file
        settings.merge(File::with_name("/etc/loramesh/conf.yml").required(false))?;

        // Add in settings from the environment (with a prefix of APP)
        settings.merge(config::Environment::with_prefix("LOMESH"))?;

        let settings: Self = settings.try_into()?;
        settings
            .validate()
            .map_err(|e| ConfigError::Message(e.to_string()))?;
        Ok(settings)
    }
    pub fn validate(&self) -> std::io::Result<()> {
        if !(10..=250).contains(&self.maxpacketsize)
            || self.txslot == 0
            || self.txslot > 60000
            || self.chunktimeout == 0
            || self.chunktimeout > 3600000
            || self.maxhops == 0
            || self.maxhops > 32
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid packet, timing, or hop limit",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[test]
fn settings_load() {
    let opt: Settings = Settings::new().expect("Error loading settings");

    assert_eq!(&opt.nodeid, &0);
    assert_eq!(&opt.isgateway, &false);
    assert_eq!(&opt.radioport.to_str().unwrap(), &"/dev/ttyUSB0");
    assert_eq!(&opt.maxpacketsize, &200usize);
    assert_eq!(&opt.maxhops, &2);
    assert_eq!(&opt.radiocfg, &None);
}
