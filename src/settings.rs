use config::{ConfigError, Config, File, Environment};
use std::path::PathBuf;
use serde::Deserialize;

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

    /// Maximum frame size sent to radio [10..250] (valid only for ping and kiss)
    pub maxpacketsize: usize,

    /// The size of the transmission slot, in milliseconds, used for transmission
    /// rate limiting
    /* The smaller the transmission slot, the more frequently transmissions will occur */
    pub txslot: u64,

    /// Amount of time (ms) to wait for end-of-transmission signal before transmitting
    /* The amount of time to wait before transmitting after receiving a
    packet that indicated more data was forthcoming.  The purpose of this is
    to compensate for a situation in which the "last" incoming packet was lost,
    to prevent the receiver from waiting forever for more packets before
    transmitting.  Given in ms. */
    pub eotwait: u64,

    /// Timeout (ms) to drop incomplete packet chunks
    pub chunktimeout: u64,

    /// Maximum number of hops a packet should travel
    pub maxhops: u8,
}

impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        let mut settings = config::Config::default();
        settings.set_default("nodeid", 0);
        settings.set_default("debug", false);
        settings.set_default("isgateway", false);
        settings.set_default("radioport", "/dev/ttyUSB0");
        settings.set_default::<Option<&str>>("radiocfg", None);
        settings.set_default("maxpacketsize", 200);
        settings.set_default("txslot", 200);
        settings.set_default("eotwait", 1000);
        settings.set_default("chunktimeout", 10000);
        settings.set_default("maxhops", 2);


        // local user settings file
        settings.merge(File::with_name("/etc/loramesh/conf.yml").required(false))?;

        // Add in settings from the environment (with a prefix of APP)
        settings.merge(config::Environment::with_prefix("LOMESH")).unwrap();

        settings.try_into()
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