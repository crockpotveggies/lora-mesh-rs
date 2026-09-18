#[cfg(unix)]
use loramesh::mesh::security::read_secret;
use loramesh::{
    link::{simulation::packet, wire::Header},
    mesh::security::{SPAN, Vault},
};
use std::collections::BTreeMap;
use zeroize::Zeroizing;
fn keys(peer: u16) -> BTreeMap<u16, Zeroizing<[u8; 32]>> {
    BTreeMap::from([(peer, Zeroizing::new([7; 32]))])
}
fn frame(sequence: u32) -> Vec<u8> {
    let p = packet(32, sequence);
    Header {
        network: 1,
        source: 1,
        destination: 2,
        session: 1,
        sequence,
        total: 32,
        index: 0,
        count: 1,
        span: SPAN as u8,
        ack: false,
        request_ack: true,
    }
    .encode(&p)
    .unwrap()
}
#[test]
fn authenticated_roundtrip_tampering_and_replay() {
    let mut a = Vault::memory(1, 1, 1, keys(2)).unwrap();
    let mut b = Vault::memory(1, 2, 1, keys(1)).unwrap();
    let plain = frame(0);
    let encrypted = a.seal(&plain).unwrap();
    assert_eq!(encrypted.len(), plain.len() + 16);
    assert_ne!(&encrypted[62..], &plain[46..]);
    for i in 0..encrypted.len() {
        let mut bad = encrypted.clone();
        bad[i] ^= 1;
        assert!(b.open(&bad).is_err(), "accepted tampering at {}", i);
    }
    assert_eq!(b.open(&encrypted).unwrap(), plain);
    assert!(b.open(&encrypted).is_err());
    let retry = a.seal(&plain).unwrap();
    assert_ne!(encrypted, retry);
    assert_eq!(b.open(&retry).unwrap(), plain);
}
#[test]
fn reordering_window_and_new_epoch() {
    let mut a = Vault::memory(1, 1, 1, keys(2)).unwrap();
    let mut b = Vault::memory(1, 2, 1, keys(1)).unwrap();
    let frames: Vec<_> = (0..70).map(|i| a.seal(&frame(i)).unwrap()).collect();
    b.open(&frames[69]).unwrap();
    b.open(&frames[68]).unwrap();
    assert!(b.open(&frames[0]).is_err());
    let mut a = a.restart().unwrap();
    let mut plain = frame(0);
    plain[12..20].copy_from_slice(&a.epoch().to_be_bytes());
    let new = a.seal(&plain).unwrap();
    b.open(&new).unwrap();
    assert!(b.open(&frames[67]).is_err());
}
#[test]
#[cfg(unix)]
fn durable_nonce_and_replay_restart_and_lock() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(
        dir.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let a = dir.path().join("a.json");
    let b = dir.path().join("b.json");
    Vault::provision(&a, 1, 1, &keys(2)).unwrap();
    Vault::provision(&b, 1, 2, &keys(1)).unwrap();
    assert!(Vault::provision(&a, 1, 1, &keys(2)).is_err());
    let mut sender = Vault::persistent(&a, 1, 1, keys(2)).unwrap();
    assert!(Vault::persistent(&a, 1, 1, keys(2)).is_err());
    let mut receiver = Vault::persistent(&b, 1, 2, keys(1)).unwrap();
    let old = sender.seal(&frame(0)).unwrap();
    receiver.open(&old).unwrap();
    assert!(receiver.announcement(1, 1, 3).unwrap());
    drop(receiver);
    let mut receiver = Vault::persistent(&b, 1, 2, keys(1)).unwrap();
    assert!(receiver.open(&old).is_err());
    assert!(!receiver.announcement(1, 1, 2).unwrap());
    drop(sender);
    let mut sender = Vault::persistent(&a, 1, 1, keys(2)).unwrap();
    let mut plain = frame(0);
    plain[12..20].copy_from_slice(&sender.epoch().to_be_bytes());
    let new = sender.seal(&plain).unwrap();
    assert_ne!(new, old);
    receiver.open(&new).unwrap();
    drop(sender);
    assert!(Vault::persistent(&a, 1, 9, keys(2)).is_err());
    let mut wrong = keys(2);
    wrong.get_mut(&2).unwrap()[0] ^= 1;
    assert!(Vault::persistent(&a, 1, 1, wrong).is_err());
}
#[test]
#[cfg(unix)]
fn missing_corrupt_state_and_key_permissions_fail_closed() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(
        dir.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let p = dir.path().join("state");
    assert!(Vault::persistent(&p, 1, 1, keys(2)).is_err());
    std::fs::write(&p, b"bad").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(Vault::persistent(&p, 1, 1, keys(2)).is_err());
    let key = dir.path().join("key");
    std::fs::write(&key, "07".repeat(32)).unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(*read_secret(&key).unwrap(), [7; 32]);
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(read_secret(&key).is_err());
    std::os::unix::fs::symlink(&key, dir.path().join("alias")).unwrap();
    assert!(read_secret(&dir.path().join("alias")).is_err());
}
#[test]
fn wrong_key_identity_plaintext_downgrade_and_bounds() {
    let mut a = Vault::memory(1, 1, 1, keys(2)).unwrap();
    let mut badkeys = keys(1);
    badkeys.get_mut(&1).unwrap()[0] = 9;
    let mut b = Vault::memory(1, 2, 1, badkeys).unwrap();
    let encrypted = a.seal(&frame(0)).unwrap();
    assert!(b.open(&encrypted).is_err());
    assert!(b.open(&frame(0)).is_err());
    for n in 0..62 {
        assert!(b.open(&encrypted[..n]).is_err());
    }
    assert!(Vault::memory(1, 1, 0, keys(2)).is_err());
    assert!(Vault::memory(1, 1, 1, BTreeMap::new()).is_err());
    assert!(a.seal(&[]).is_err());
}
#[test]
fn independent_libsodium_frame_vector() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/secure-v2.json")).unwrap();
    let plain = hex::decode(fixture["plaintext"].as_str().unwrap()).unwrap();
    assert_eq!(plain, frame(0));
    let mut a = Vault::memory(1, 1, 1, keys(2)).unwrap();
    let encrypted = a.seal(&plain).unwrap();
    assert_eq!(
        hex::encode(&encrypted),
        fixture["secure_frame"].as_str().unwrap()
    );
    let mut b = Vault::memory(1, 2, 1, keys(1)).unwrap();
    assert_eq!(b.open(&encrypted).unwrap(), plain);
}

#[test]
#[cfg(unix)]
fn durable_write_failure_never_accepts_or_advances_replay() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = dir.path().join("state.json");
    Vault::provision(&path, 1, 2, &keys(1)).unwrap();
    let mut receiver = Vault::persistent(&path, 1, 2, keys(1)).unwrap();
    let mut sender = Vault::memory(1, 1, 1, keys(2)).unwrap();
    let cipher = sender.seal(&frame(0)).unwrap();
    let blocker = path.with_extension("next");
    std::fs::create_dir(&blocker).unwrap();
    assert!(receiver.open(&cipher).is_err());
    assert!(receiver.announcement(1, 1, 1).is_err());
    std::fs::remove_dir(blocker).unwrap();
    assert_eq!(receiver.open(&cipher).unwrap(), frame(0));
    assert!(receiver.announcement(1, 1, 1).unwrap());
}
#[test]
fn duplicate_keys_and_vault_identity_are_rejected() {
    assert!(
        Vault::memory(
            1,
            1,
            1,
            BTreeMap::from([(2, Zeroizing::new([7; 32])), (3, Zeroizing::new([7; 32]))])
        )
        .is_err()
    );
    let vault = Vault::memory(1, 1, 1, keys(2)).unwrap();
    assert!(vault.matches(1, 1, &[2]));
    assert!(!vault.matches(2, 1, &[2]));
    assert!(!vault.matches(1, 2, &[2]));
    assert!(!vault.matches(1, 1, &[3]));
}

#[cfg(not(unix))]
#[test]
fn windows_storage_is_explicitly_unsupported() {
    let error =
        loramesh::mesh::security::read_secret(std::path::Path::new("unused.key")).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
}
