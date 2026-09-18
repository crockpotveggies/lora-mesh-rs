# LoRa Mesh

[![LoRa Mesh Tests](https://github.com/crockpotveggies/lora-mesh-rs/actions/workflows/test.yml/badge.svg)](https://github.com/crockpotveggies/lora-mesh-rs/actions/workflows/test.yml)

IPv4 over LoStik USB LoRa radios, written in Rust. The `crockpot-revamp` branch
replaces the original experimental mesh with a bounded reliable link, signed
multihop routing, authenticated encryption, and hardware-free testing.

This remains experimental software. Simulator and Linux TUN acceptance pass.
An RN2903 LoStik on macOS has passed repeated serial opens, low-power transmission,
secure-daemon operation and receive-watchdog recovery. Two-radio over-the-air IPv4
delivery, range/throughput calibration and independent security review remain pending.

## Start without hardware

```sh
cargo build --locked --bins
cargo test --locked --all-targets
cargo run --release --locked --bin loramesh-mesh-sim -- scenarios/mesh/diamond-repair.json --output target/mesh.json
cargo test --locked --test mesh_process
```

The simulator models shared half-duplex RF, airtime, loss, collisions, hidden
terminals, outages and device restarts. Virtual LoStik PTYs also run the actual
production daemons. Scenarios and seeds are embedded in reports for exact replay.

## Secure daemon

`loramesh-mesh` is the default binary. `loramesh-keygen` provisions per-node signing
keys, pairwise encryption keys and durable replay/nonce state. Linux TUN setup
runs briefly with privilege, then drops to a configured service account before
opening keys and radio workers. macOS supports serial and local datagram testing.

Read the [installation, configuration and recovery guide](docs/secure-mesh.md)
before provisioning. Do not discard or roll back state files with the same keys.
The current bounded design supports 64 configured members and 32 peers per node;
its provisioning CLI supports 32 members. IPv4 MTU is 1476 bytes. Encryption is
hop by hop: relays are trusted with packet plaintext.

## Development and evidence

- [Rewrite plan and milestone status](docs/rewrite-plan.md)
- [Secure multihop performance and coverage](docs/mesh-performance.md)
- [Single-hop performance](docs/performance.md)
- [Simulator and virtual device guide](docs/simulator.md)
- [Reliable link and TUN testing](docs/reliable-link.md)
- [LoStik compatibility fixes and physical test results](docs/radio-compatibility.md)

Rust 2024 / minimum compiler 1.85. Lockfiles are retained for reproducible builds.
CI is configured to test Linux x86_64/ARM64, macOS Apple Silicon/Intel and Windows
x86_64, then build native packages. Build/test output belongs in `target/`, packages
in `dist/`, and local node configuration, keys and durable state in an ignored
`.local/` directory or outside the checkout. Curated benchmark reports and test
fixtures remain versioned. Ignored identity/state files still need the recovery
handling described in the [secure mesh guide](docs/secure-mesh.md#state-restart-and-recovery).

## Installers and releases

Release tooling produces PKG/DMG installers for macOS, DEBs for Linux, a Windows
Setup EXE, and portable archives. Windows packages currently
include the radio probe and simulators; native IP networking remains Linux-only.
See the [installation/support guide](docs/installing.md).

`python3 scripts/package-release.py` builds native installers on your current OS.
A pushed `vVERSION` tag matching `Cargo.toml` runs the full checks and publishes all
platform artifacts, checksums and source to a GitHub release. See the
[release guide](docs/releasing.md) for versioning, signing and recovery.

The original 2020 implementation is available only through
`cargo run --features legacy --bin loramesh`. Its YAML configuration and obsolete
dependencies are compatibility material. `loramesh-link` uses the unauthenticated
single-hop lab protocol for regression tests. Neither is the secure deployment
path; they do not interoperate with secure radio version 2.

## Roadmap

The original checklist is preserved below, with status updated for the secure
replacement. Checked items indicate implemented functionality within the stated
scope, not production certification or complete physical-radio validation.

- [x] LoStik interface — bounded serial controller, reconnect/startup recovery and
  RN2903 hardware checks; RN2483 behavior is covered by the virtual device.
- [x] Local network tunnel — Linux TUN with explicit MTU, routes and privilege
  dropping. Native macOS and Windows network interfaces are not implemented.
- [x] Bridge radio and tunnel — secure IPv4 forwarding; Linux TUN and production
  daemon tests use virtual radios. Two-radio physical delivery remains to be tested.
- [x] Packet chunking — bounded fragmentation/reassembly, duplicate suppression,
  selective retries and aggregated acknowledgments.
- [x] Node discovery — authenticated neighbor and topology discovery among
  provisioned members. Automatic enrollment and IP assignment are not implemented.
- [x] Message protocol — versioned, validated and authenticated frames. Secure
  version 2 is incompatible with the original protocol and the version 1 lab link.
- [ ] Gateway DHCP — **removed from the secure replacement**; IDs, membership and
  IPv4 addresses are explicitly provisioned. The original checklist marked this done.
- [x] Multi-hop routing — signed link-state advertisements, airtime-aware route
  selection, bounded forwarding and alternate-path repair.
- [x] Network failure recovery — **newly implemented** bounded retransmission,
  neighbor expiry, route repair, serial reconnection and durable replay state.
- [ ] Frame [lz4](https://docs.rs/crate/lz4-compress/0.1.1/source/src/compress.rs)
  compression — compression was evaluated; no compression codec is enabled.
- [ ] RTS/CTS collision prevention — no RTS/CTS handshake. Airtime scheduling,
  backoff and retries mitigate contention; collisions remain possible.
- [ ] Multiple LoRa device hardware — LoStik RN2903/RN2483 command support exists;
  additional radio families/backends and physical RN2483 validation remain open.
- [x] Security and encryption — **newly implemented** hop-by-hop authenticated
  encryption, signed routing, durable replay protection and private provisioning.
  Relays see plaintext; independent security review remains open.
- [ ] Support 65,536 nodes — IDs use nonzero `u16`, but the implementation is capped
  at **64 members / 32 peers per node**, and keygen supports 32 members. This is a
  deliberate reduction from the original README's advertised 256-node capacity.

Additional work delivered during the revamp:

- [x] Deterministic simulator and virtual LoStiks, including production-process PTY tests.
- [x] Performance benchmarks, coverage gates, parser fuzzing and dependency checks.
- [x] Native installer tooling and GitHub Actions for tests and tagged release uploads.
- [ ] Two-radio physical IPv4 acceptance, field calibration and independent security review.
- [ ] Internet gateway/NAT integration; the supported network remains private IPv4.

See the [rewrite plan](docs/rewrite-plan.md) for milestone details and the
[platform support matrix](docs/installing.md#platform-support) for packaging limits.

## Credits

- John Goerzen, creator of [LoRaPipe](https://github.com/jgoerzen/lorapipe).
