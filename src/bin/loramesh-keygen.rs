#[cfg(unix)]
mod unix {
    //! Explicit provisioning. Refuses existing directories; never prints or overwrites keys.
    use loramesh::mesh::{Config, Member, daemon::DaemonConfig, security::Vault};
    use std::{collections::BTreeMap, fs, io::Write, path::Path};
    use zeroize::Zeroizing;
    fn private_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    }
    pub fn main() {
        if loramesh::version_requested("loramesh-keygen") {
            return;
        }
        if let Err(e) = run() {
            eprintln!("key provisioning: {}", e);
            std::process::exit(1);
        }
    }
    fn run() -> Result<(), Box<dyn std::error::Error>> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args.first().map(|a| a == "--help").unwrap_or(false) {
            println!(
                "Usage: loramesh-keygen NEW_DIRECTORY NODE=IP NODE=IP [...] [--links 1-2,2-3]"
            );
            return Ok(());
        }
        if args.len() < 3 {
            return Err(
                "Usage: loramesh-keygen NEW_DIRECTORY NODE=IP NODE=IP [...] [--links 1-2,2-3]"
                    .into(),
            );
        }
        let mut members = BTreeMap::new();
        let mut secrets = BTreeMap::new();
        let mut edges = None;
        let mut i = 1;
        while i < args.len() {
            if args[i] == "--links" {
                i += 1;
                edges = Some(args.get(i).ok_or("missing links")?.clone());
            } else {
                let (id, ip) = args[i].split_once('=').ok_or("expected NODE=IP")?;
                let id: u16 = id.parse()?;
                if id == 0 || members.contains_key(&id) {
                    return Err("invalid or duplicate node ID".into());
                }
                let mut seed = Zeroizing::new([0; 32]);
                getrandom::fill(seed.as_mut()).map_err(|e| std::io::Error::other(e.to_string()))?;
                let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
                members.insert(
                    id,
                    Member {
                        address: ip.parse()?,
                        verifying_key: hex::encode(signing.verifying_key().to_bytes()),
                    },
                );
                secrets.insert(id, seed);
            }
            i += 1;
        }
        if !(2..=32).contains(&members.len()) {
            return Err("provision 2..32 members".into());
        }
        let mut adjacency: BTreeMap<u16, std::collections::BTreeSet<u16>> =
            members.keys().map(|id| (*id, Default::default())).collect();
        let pairs = if let Some(edges) = edges {
            edges
                .split(',')
                .map(|edge| {
                    let (a, b) = edge.split_once('-').ok_or("invalid edge")?;
                    Ok((a.parse::<u16>()?, b.parse::<u16>()?))
                })
                .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?
        } else {
            members
                .keys()
                .flat_map(|a| {
                    members
                        .keys()
                        .filter(move |b| *b > a)
                        .map(move |b| (*a, *b))
                })
                .collect()
        };
        let mut keys: BTreeMap<u16, BTreeMap<u16, Zeroizing<[u8; 32]>>> =
            members.keys().map(|id| (*id, BTreeMap::new())).collect();
        for (a, b) in pairs {
            if a == b || !members.contains_key(&a) || !members.contains_key(&b) {
                return Err("invalid edge member".into());
            }
            let mut key = Zeroizing::new([0; 32]);
            getrandom::fill(key.as_mut()).map_err(|e| std::io::Error::other(e.to_string()))?;
            adjacency.get_mut(&a).unwrap().insert(b);
            adjacency.get_mut(&b).unwrap().insert(a);
            keys.get_mut(&a).unwrap().insert(b, key.clone());
            keys.get_mut(&b).unwrap().insert(a, key);
        }
        use std::os::unix::fs::DirBuilderExt;
        let root = Path::new(&args[0]);
        fs::DirBuilder::new().mode(0o700).create(root)?;
        let root = root.canonicalize()?;
        for (&id, member) in &members {
            let dir = root.join(id.to_string());
            fs::DirBuilder::new().mode(0o700).create(&dir)?;
            private_write(
                &dir.join("signing.key"),
                Zeroizing::new(hex::encode(*secrets[&id])).as_bytes(),
            )?;
            let mut peer_keys = BTreeMap::new();
            for (peer, key) in &keys[&id] {
                let path = dir.join(format!("peer-{}.key", peer));
                private_write(&path, Zeroizing::new(hex::encode(**key)).as_bytes())?;
                peer_keys.insert(*peer, path);
            }
            let mesh = Config {
                members: members.clone(),
                peers: adjacency[&id].iter().copied().collect(),
                link: loramesh::link::Config {
                    node: id,
                    ..Default::default()
                },
                ..Default::default()
            };
            mesh.validate()?;
            Vault::provision(&dir.join("state.json"), mesh.link.network, id, &keys[&id])?;
            let config = DaemonConfig {
                radio: "/dev/REPLACE_WITH_RADIO".into(),
                power_dbm: 14,
                mesh,
                adapter: loramesh::link::daemon::Adapter::Datagram {
                    bind: dir.join("packets.sock"),
                    peer: dir.join("client.sock"),
                },
                signing_key: dir.join("signing.key"),
                peer_keys,
                state: dir.join("state.json"),
                run_as: None,
                metrics: Some(dir.join("metrics.json")),
            };
            private_write(
                &dir.join("config.json"),
                &serde_json::to_vec_pretty(&config)?,
            )?;
            println!(
                "provisioned node {} ({}) at {}",
                id,
                member.address,
                dir.display()
            );
        }
        Ok(())
    }
}
#[cfg(unix)]
fn main() {
    unix::main();
}
#[cfg(not(unix))]
fn main() {
    eprintln!("loramesh-keygen requires Unix; use loramesh-radio or loramesh-mesh-sim on Windows");
    std::process::exit(1);
}
