# Secure multihop mesh (milestones 5–6)

The replacement daemon is `loramesh-mesh`. It shares the production radio
controller, reliable link and forwarding engine with the simulator. This is an
experimental implementation with automated software acceptance; physical LoStik
calibration and an independent security audit remain outstanding.

## Build and try without hardware

Rust 2024, minimum Rust 1.85. Linux builds require `pkg-config`, `libudev-dev`;
TUN setup also requires `iproute2`. macOS supports simulator/serial/Unix-datagram
adapters, not TUN. Linux x86_64 and ARM64 are configured in CI.

```sh
cargo build --locked --release --bins
cargo test --locked --all-targets
cargo run --release --locked --bin loramesh-mesh-sim -- scenarios/mesh/diamond-repair.json --output target/mesh.json
cargo run --release --locked --bin loramesh-mesh-sim -- target/mesh.json --replay --output target/replay.json
cmp target/mesh.json target/replay.json
cargo test --locked --test mesh_process
```

The process test provisions fresh keys and runs three actual daemons over virtual
LoStik PTYs. On an isolated Linux host with root and `/dev/net/tun`, run
`sudo python3 scripts/test-secure-linux.py` after a debug build. It creates private
network namespaces, checks ping/TTL, IPv4 fragmentation, DF/MTU rejection, UDP
and TCP, and verifies workers have non-root UIDs, zero effective capabilities and
`no_new_privs`. It removes its namespaces afterward. Metrics/traces, never private
keys or state, are copied to `target/secure-tun-artifacts`.

## Provision and configure

Use a **new**, private directory and explicit node-to-IPv4 assignments. IDs are
nonzero u16, membership is capped at 64, and each node has at most 32 configured
immediate peers. The provisioning CLI currently accepts at most 32 members.

```sh
target/release/loramesh-keygen /secure/new-mesh \
  1=10.107.0.1 2=10.107.0.2 3=10.107.0.3 --links 1-2,2-3
```

This creates OS-random Ed25519 signing seeds, unique pairwise symmetric keys,
initial durable state and a complete `config.json` in each node directory.
It refuses existing directories/files and prints no secrets. Distribute only each
node's own directory through a secure channel; remove extra provisioning copies
before starting the deployed identities. Generated configs use harmless local
Unix datagram sockets and a placeholder serial path. Change `radio` to the USB
serial device, and set the same network number/profile on all peers. Configure
frequency, transmit power and `mesh.link.duty_per_mille` for the deployment's constraints;
the simulator's default profile/duty is a lab setting, not a regional policy.

For Linux, replace the generated adapter and add the service account's numeric
IDs (example IDs must be replaced with actual IDs):

```json
{
  "adapter": {"kind":"tun", "name":"mesh0", "address":"10.107.0.1", "peer_address":"10.107.0.2"},
  "run_as": {"uid":1001, "gid":1001, "groups":[20]}
}
```

These are fields to merge into the generated config, not a complete config.
The local address must match membership. `groups` is an explicit allowlist of
supplementary groups needed for the serial device. Give the service account
ownership of its directory (0700) and files (0600) and access to its serial device.
Keep the launch configuration in a root-controlled location when launching as root;
its absolute key/state paths can point into the private service directory.

Run `sudo target/release/loramesh-mesh /etc/loramesh/node.json`. Root is used only
for exclusive TUN creation, MTU 1476 and member /32 route setup. The daemon then
drops supplementary groups and real/effective/saved GID/UID, disables privilege
gain/core dumps, and only then opens secrets, persistent state and serial workers.
Root TUN operation without a non-root `run_as` is rejected. It never attaches to an
existing interface. USB reconnection runs unprivileged. SIGINT/SIGTERM stops cleanly;
closing the descriptor removes the nonpersistent interface. Datagram mode needs
neither root nor `run_as`.

Optional `mesh.static_routes` maps destination IDs to immediate peer IDs, e.g.
`{"3":2}` on node 1. Static routes take precedence while their peer is live.
Dynamic routing requires reciprocal signed edge advertisements, retains alternate
links, measures airtime/retry costs, and uses 20% hysteresis. Advertisements default
to 60 seconds with phased/jittered scheduling; neighbors expire after 240 seconds.
Exhausted reliable data sends can trigger earlier neighbor invalidation and at most
two bounded requeues. The diamond failure scenario tests this repair path.

## Security boundary and wire format

Every radio frame, including acknowledgments, uses RustCrypto
XChaCha20-Poly1305 with a distinct key for each configured peer pair. This provides
**hop-by-hop** confidentiality/authentication: relays are trusted and see plaintext
IPv4 traffic. Use application end-to-end encryption when relays must not see data.
Public routing announcements are also signed by their origin with Ed25519 so a
relay cannot impersonate another origin's topology advertisement. Authenticated
members can still lie about their own links or traffic; reciprocal edges constrain
route formation, not malicious-member behavior. Jamming, traffic analysis and
physical compromise are outside this boundary.

Secure radio version 2 is incompatible with the lab version 1 and legacy protocol:

| Byte range | Contents |
| --- | --- |
| 0–29 | Existing link header (magic LM, version 2, flags, network, source, destination, data session/sequence, fragmentation fields) |
| 30–37 | Sender's durable epoch, big endian u64 |
| 38–45 | Per-peer transmission counter, big endian u64 |
| 46–61 | Poly1305 tag (16 bytes) |
| 62–254 | Encrypted fragment, up to 193 bytes |

All 46 header bytes are authenticated associated data. The 24-byte nonce is
`network || source || destination || sender_epoch || counter` (4+2+2+8+8 bytes).
Data sessions equal the sender epoch; ACK sessions identify the original data
sender's session. Each physical retry uses a new counter. Recovered link headers
are validated before committing acceptance. A fixture generated independently by
libsodium/PyNaCl checks the complete ciphertext/tag byte for byte.

The reassembled datagram begins with a 24-byte mesh header: magic MS/version 1,
announcement flag, origin/destination, origin epoch/sequence, remaining hop budget,
reserved zero byte and payload length. Data carries a validated IPv4 packet.
Announcements carry bounded neighbor/cost entries and a 64-byte Ed25519 signature
covering domain, network, origin, epoch, sequence and entries. Mutable hop budget
is excluded from that signature but authenticated on every radio hop.

Only the addressed next hop accepts a frame. Forwarding validates source/destination
IP assignments and decrements both IPv4 TTL/checksum and mesh hop budget. TTL
expiry currently drops with a metric; ICMP Time Exceeded generation is not yet
implemented. Duplicate caches, link queues, fragment assemblies, pending discovery,
LSAs and retry histories are all bounded. Small membership is intentional.

## State, restart and recovery

The state file is part of the identity, not disposable cache. Startup takes an
exclusive lock and atomically persists an incremented local epoch before sending.
Each authenticated receive persists a 64-counter replay window (or newer peer
epoch) before delivery/ACK; signed announcement freshness is durable too. Writes
use private temporary files, file fsync, rename and directory fsync. Missing,
corrupt, shared, symlinked or incorrectly owned/permitted state fails closed.
Disk errors terminate processing rather than accepting unrecorded ciphertext.
Storage must honor these durability operations; measure write endurance/latency
on the intended device.

**Never restore an old state snapshot, clone a running identity, or recreate lost
state using the same keys.** Local files cannot detect rollback of the entire disk.
After loss/rollback, generate fresh pairwise keys for every affected link and fresh
identity/config/state for affected nodes; distribute updated public membership and
peer keys together while stopped. Changing an existing key set with old state is
rejected. There is no automatic key exchange, online rotation or revocation service.
A clean restart preserves ciphertext/LSA replay rejection; application-level
exactly-once delivery across receiver restarts is not promised. Legitimate freshly
encrypted retries may redeliver an application packet after a restart.

## Testing, fuzzing and releases

`tests/mesh.rs`, `routing.rs`, `security.rs` and `mesh_process.rs` cover signed
origin checks, header tampering, wrong keys, old/reordered epochs/counters, durable
failure/restart, topology expiry, alternate repair, loops/hop limits, input/queue
bounds, deterministic replay and process interoperability. The original radio and
single-hop tests remain regression coverage.

```sh
cargo llvm-cov --locked --all-targets --json --output-path target/coverage.json
python3 scripts/check-link-coverage.py target/coverage.json
python3 scripts/seed-fuzz.py
cargo +nightly fuzz run parsers -- -max_total_time=30 -max_len=2048
cargo deny --no-default-features check advisories
python3 scripts/package-release.py
```

Install pinned CI versions of cargo-llvm-cov/cargo-fuzz/cargo-deny as needed.
Fuzzing uses libFuzzer and AddressSanitizer against serial framing, IPv4/link/mesh
parsers and secure-frame opening. Extend the corpus and run longer before releases.
CI uses native Linux x86_64/ARM64, macOS Apple Silicon/Intel and Windows runners,
plus a separate Rust 1.85 check. Windows tests cover the portable core; its package
contains only the radio/simulator tools.
See the [GitHub runner labels](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
and [Rust fuzz CI guide](https://rust-fuzz.github.io/book/cargo-fuzz/ci.html).

Packaging builds versioned native PKG/DMG, DEB and Windows Setup EXE installers,
plus portable archives, hashes and build/license metadata. Tag releases run the
full test/install pipeline before publishing GitHub assets. See [installation](installing.md)
and [release setup](releasing.md), including platform limitations and signing.

The old binary is retained only with `--features legacy --bin loramesh`; its old
dependencies are excluded from the default build/advisory boundary. Serialport 4
replaces 3; macOS USB serial and PTYs use one complete standard-57600-baud termios
configuration, avoiding the builder's intermediate settings and IOSSIOSPEED.
See the [LoStik compatibility report](radio-compatibility.md).
A small isolated Linux fd/ioctl adapter replaces old tun-tap/Tokio in the
new path. Strict, bounded IPv4 validation stays local rather than adding a large
packet framework. See [performance measurements](mesh-performance.md).
