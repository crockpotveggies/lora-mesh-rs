//! Hop authentication. Durable epochs prevent nonce reuse; receipts commit before delivery.
use crate::{link::wire, radio::protocol::invalid};
use chacha20poly1305::{KeyInit, Tag, XChaCha20Poly1305, XNonce, aead::AeadInPlace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read},
    path::Path,
};
#[cfg(unix)]
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};
use zeroize::Zeroizing;
pub const OVERHEAD: usize = 16;
pub const SPAN: usize = wire::MAX_SPAN - OVERHEAD;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub epoch: u64,
    pub high: u64,
    pub bits: u64,
}
impl Window {
    fn accepts(&self, epoch: u64, counter: u64) -> bool {
        epoch > self.epoch
            || epoch == self.epoch
                && (counter > self.high
                    || self.high - counter < 64 && self.bits & (1 << (self.high - counter)) == 0)
    }
    fn insert(&mut self, epoch: u64, counter: u64) {
        if epoch > self.epoch {
            self.epoch = epoch;
            self.high = counter;
            self.bits = 1;
        } else if counter > self.high {
            self.bits = if counter - self.high >= 64 {
                1
            } else {
                (self.bits << (counter - self.high)) | 1
            };
            self.high = counter;
        } else {
            self.bits |= 1 << (self.high - counter);
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    network: u32,
    node: u16,
    epoch: u64,
    fingerprint: String,
    receipts: BTreeMap<u16, Window>,
    pub announcements: BTreeMap<u16, (u64, u32)>,
}
pub struct Vault {
    network: u32,
    node: u16,
    state: State,
    ciphers: BTreeMap<u16, XChaCha20Poly1305>,
    counters: BTreeMap<u16, u64>,
    journal: Option<Journal>,
}
struct Journal {
    #[cfg(unix)]
    path: PathBuf,
    #[cfg(unix)]
    _lock: File,
}
#[cfg(unix)]
fn private_file(path: &Path) -> io::Result<File> {
    use std::os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawFd,
    };
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let m = f.metadata()?;
    if !m.is_file()
        || m.mode() & 0o077 != 0
        || m.uid() != unsafe { libc::geteuid() }
        || m.nlink() != 1
    {
        return Err(invalid(
            "secret/state file must be owned by this user, mode 0600, with one link",
        ));
    }
    let _ = f.as_raw_fd();
    Ok(f)
}

#[cfg(not(unix))]
fn private_file(_: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "persistent security storage requires Unix; Windows supports simulation only",
    ))
}
#[cfg(not(unix))]
impl Journal {
    fn save(&self, _: &State) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "persistent security storage requires Unix",
        ))
    }
}

pub fn read_secret(path: &Path) -> io::Result<Zeroizing<[u8; 32]>> {
    let mut text = Zeroizing::new(String::new());
    private_file(path)?.take(129).read_to_string(&mut text)?;
    let mut key = Zeroizing::new([0u8; 32]);
    hex::decode_to_slice(text.trim(), key.as_mut())
        .map_err(|_| invalid("key must be 32 bytes encoded as hex"))?;
    Ok(key)
}
#[cfg(unix)]
impl Journal {
    fn lock(path: &Path) -> io::Result<Self> {
        use std::os::unix::{
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawFd,
        };
        let parent = path
            .parent()
            .ok_or_else(|| invalid("state requires a parent directory"))?;
        let m = fs::symlink_metadata(parent)?;
        if !m.is_dir() || m.mode() & 0o077 != 0 || m.uid() != unsafe { libc::geteuid() } {
            return Err(invalid(
                "state directory must be owned by this user and mode 0700",
            ));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path.with_extension("lock"))?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::other("state is already in use"));
        }
        Ok(Self {
            path: path.to_owned(),
            _lock: lock,
        })
    }
    fn save(&self, state: &State) -> io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        let tmp = self.path.with_extension("next");
        match fs::remove_file(&tmp) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(&serde_json::to_vec(state)?)?;
        file.sync_all()?;
        fs::rename(&tmp, &self.path)?;
        File::open(self.path.parent().unwrap())?.sync_all()?;
        Ok(())
    }
}
impl Vault {
    fn state(
        network: u32,
        node: u16,
        keys: &BTreeMap<u16, Zeroizing<[u8; 32]>>,
    ) -> io::Result<State> {
        if keys.is_empty() || keys.len() > 32 || keys.contains_key(&node) {
            return Err(invalid("invalid peer key set"));
        }
        let values: Vec<_> = keys.values().collect();
        for (i, key) in values.iter().enumerate() {
            if values[..i].contains(key) {
                return Err(invalid("pairwise keys must be distinct"));
            }
        }
        let mut hash = Sha256::new();
        hash.update(network.to_be_bytes());
        hash.update(node.to_be_bytes());
        for (peer, key) in keys {
            hash.update(peer.to_be_bytes());
            hash.update(**key);
        }
        Ok(State {
            version: 2,
            network,
            node,
            epoch: 0,
            fingerprint: hex::encode(hash.finalize()),
            receipts: keys.keys().map(|p| (*p, Window::default())).collect(),
            announcements: BTreeMap::new(),
        })
    }
    /// Explicit one-time provisioning. Never recreate lost state with the same keys.
    #[cfg(unix)]
    pub fn provision(
        path: &Path,
        network: u32,
        node: u16,
        keys: &BTreeMap<u16, Zeroizing<[u8; 32]>>,
    ) -> io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        let journal = Journal::lock(path)?;
        let state = Self::state(network, node, keys)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(&serde_json::to_vec(&state)?)?;
        file.sync_all()?;
        File::open(journal.path.parent().unwrap())?.sync_all()?;
        Ok(())
    }
    #[cfg(unix)]
    pub fn persistent(
        path: &Path,
        network: u32,
        node: u16,
        keys: BTreeMap<u16, Zeroizing<[u8; 32]>>,
    ) -> io::Result<Self> {
        let journal = Journal::lock(path)?;
        let mut bytes = Vec::new();
        private_file(path)?.take(32769).read_to_end(&mut bytes)?;
        if bytes.len() > 32768 {
            return Err(invalid("oversized state file"));
        }
        let mut state: State = serde_json::from_slice(&bytes)?;
        let expected = Self::state(network, node, &keys)?;
        if state.version != 2
            || state.node != node
            || state.network != network
            || state.fingerprint != expected.fingerprint
            || state.receipts.keys().ne(keys.keys())
            || state.announcements.len() > 64
        {
            return Err(invalid(
                "state/key identity mismatch: rotate keys rather than reset state",
            ));
        }
        state.epoch = state
            .epoch
            .checked_add(1)
            .ok_or_else(|| invalid("epoch exhausted: rotate keys"))?;
        journal.save(&state)?;
        Self::build(state, keys, Some(journal))
    }
    /// Deterministic simulation only. Production must use persistent().
    pub fn memory(
        network: u32,
        node: u16,
        epoch: u64,
        keys: BTreeMap<u16, Zeroizing<[u8; 32]>>,
    ) -> io::Result<Self> {
        if epoch == 0 {
            return Err(invalid("zero epoch"));
        }
        let mut state = Self::state(network, node, &keys)?;
        state.epoch = epoch;
        Self::build(state, keys, None)
    }
    fn build(
        state: State,
        keys: BTreeMap<u16, Zeroizing<[u8; 32]>>,
        journal: Option<Journal>,
    ) -> io::Result<Self> {
        let ciphers = keys
            .into_iter()
            .map(|(p, k)| (p, XChaCha20Poly1305::new_from_slice(k.as_ref()).unwrap()))
            .collect();
        Ok(Self {
            network: state.network,
            node: state.node,
            state,
            ciphers,
            counters: BTreeMap::new(),
            journal,
        })
    }
    /// Advance the durable epoch while preserving replay receipts (also used for simulator restarts).
    pub fn restart(mut self) -> io::Result<Self> {
        let mut next = self.state.clone();
        next.epoch = next
            .epoch
            .checked_add(1)
            .ok_or_else(|| invalid("epoch exhausted"))?;
        self.commit(next)?;
        self.counters.clear();
        Ok(self)
    }
    pub fn matches(&self, network: u32, node: u16, peers: &[u16]) -> bool {
        self.network == network
            && self.node == node
            && self.ciphers.len() == peers.len()
            && peers.iter().all(|p| self.ciphers.contains_key(p))
    }
    pub fn epoch(&self) -> u64 {
        self.state.epoch
    }
    fn commit(&mut self, next: State) -> io::Result<()> {
        if let Some(j) = &self.journal {
            j.save(&next)?;
        }
        self.state = next;
        Ok(())
    }
    fn nonce(aad: &[u8]) -> [u8; 24] {
        let mut nonce = [0; 24];
        nonce[..8].copy_from_slice(&aad[4..12]);
        nonce[8..].copy_from_slice(&aad[30..46]);
        nonce
    }
    pub fn seal(&mut self, plain: &[u8]) -> io::Result<Vec<u8>> {
        let (h, payload) = wire::decode(plain)?;
        if h.network != self.network || h.source != self.node || plain.len() + OVERHEAD > 255 {
            return Err(invalid("invalid secure transmission identity/size"));
        }
        let cipher = self
            .ciphers
            .get(&h.destination)
            .ok_or_else(|| invalid("unknown peer"))?;
        let counter = self.counters.entry(h.destination).or_default();
        *counter = counter
            .checked_add(1)
            .ok_or_else(|| invalid("frame counter exhausted"))?;
        let mut out = Vec::with_capacity(plain.len() + OVERHEAD);
        out.extend_from_slice(&plain[..30]);
        out[2] = 2;
        out.extend_from_slice(&self.state.epoch.to_be_bytes());
        out.extend_from_slice(&counter.to_be_bytes());
        let nonce = Self::nonce(&out);
        let mut ciphertext = payload.to_vec();
        let tag = cipher
            .encrypt_in_place_detached(XNonce::from_slice(&nonce), &out, &mut ciphertext)
            .map_err(|_| invalid("encryption failed"))?;
        out.extend_from_slice(&tag);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }
    pub fn open(&mut self, frame: &[u8]) -> io::Result<Vec<u8>> {
        if frame.len() < 62 || frame.len() > 255 || &frame[..3] != b"LM\x02" {
            return Err(invalid("invalid secure frame"));
        }
        let network = u32::from_be_bytes(frame[4..8].try_into().unwrap());
        let source = u16::from_be_bytes(frame[8..10].try_into().unwrap());
        let destination = u16::from_be_bytes(frame[10..12].try_into().unwrap());
        if network != self.network || destination != self.node {
            return Err(invalid("foreign secure frame"));
        }
        let cipher = self
            .ciphers
            .get(&source)
            .ok_or_else(|| invalid("unconfigured sender"))?;
        let epoch = u64::from_be_bytes(frame[30..38].try_into().unwrap());
        let counter = u64::from_be_bytes(frame[38..46].try_into().unwrap());
        if epoch == 0 || counter == 0 || !self.state.receipts[&source].accepts(epoch, counter) {
            return Err(invalid("replayed secure frame"));
        }
        let mut payload = frame[62..].to_vec();
        let nonce = Self::nonce(frame);
        cipher
            .decrypt_in_place_detached(
                XNonce::from_slice(&nonce),
                &frame[..46],
                &mut payload,
                Tag::from_slice(&frame[46..62]),
            )
            .map_err(|_| invalid("authentication failed"))?;
        let mut out = Vec::with_capacity(frame.len() - OVERHEAD);
        out.extend_from_slice(&frame[..30]);
        out[2] = 1;
        out.extend_from_slice(&[0; 16]);
        out.extend_from_slice(&payload);
        let (h, _) = wire::decode(&out)?;
        if !h.ack && h.session != epoch {
            return Err(invalid("packet session differs from authenticated epoch"));
        }
        let mut next = self.state.clone();
        next.receipts
            .get_mut(&source)
            .unwrap()
            .insert(epoch, counter);
        self.commit(next)?;
        Ok(out)
    }
    /// Persist origin-signed announcement freshness before applying or forwarding it.
    pub fn announcement(&mut self, origin: u16, epoch: u64, sequence: u32) -> io::Result<bool> {
        if self
            .state
            .announcements
            .get(&origin)
            .map(|old| *old >= (epoch, sequence))
            .unwrap_or(false)
        {
            return Ok(false);
        }
        if !self.state.announcements.contains_key(&origin) && self.state.announcements.len() >= 64 {
            return Err(invalid("announcement identity cap"));
        }
        let mut next = self.state.clone();
        next.announcements.insert(origin, (epoch, sequence));
        self.commit(next)?;
        Ok(true)
    }
}
