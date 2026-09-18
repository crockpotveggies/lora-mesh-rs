# Simulator and radio-controller testing

The simulator and virtual LoStik are the test foundation for milestones 1 and 2.
They use the same radio controller and serial worker as the `loramesh` daemon.
These milestones do not implement the new reliable IPv4 wire protocol or routing.

## Quick start

From the repository root, without root privileges or radio hardware:

```sh
cargo test --locked --all-targets
cargo run --locked --bin loramesh-sim -- scenarios/two-node.json --output target/smoke.json
cargo run --locked --bin loramesh-sim -- target/smoke.json --replay --output target/replay.json
```

The two JSON reports are byte-identical. The smoke sends the five bytes `Hello`
through two production controllers at SF7/125 kHz. A failed expectation produces
an unsuccessful exit code. Successful and assertion-failure reports embed the
scenario, seed, controller metrics, medium metrics, and event trace. Event-budget
failures save the configuration and error so they can also be replayed. The CLI
limits input scenario/report files to 4 MiB.

To exercise **two separate production radio-probe processes** through real Unix
pseudo-terminals:

```sh
cargo build --locked --bins
cargo run --locked --bin loramesh-sim -- scenarios/two-node.json --pty-smoke --output target/pty-smoke.json
```

This runner starts a virtual laboratory, waits for each probe's readiness message,
transmits one known payload, verifies one reception, and cleans up its child
processes and temporary device paths. It fails on startup, delivery, duplicate-TX,
output-limit, or process timeout errors. The radio probe executable must be built
alongside the simulator. This fixed smoke uses SF7 and a bounded 15-second lab
lifetime; unlike deterministic mode, it does not execute scenario traffic or assert
scenario expectations. Its output is a real-time trace, not a replayable deterministic
report. Fault schedules still apply.

For interactive use:

```sh
cargo run --locked --bin loramesh-sim -- scenarios/virtual-lab.json --pty
```

The runner prints a map of node IDs to serial paths. In two terminals, substitute
the printed paths:

```sh
# Start the receiver first.
target/debug/loramesh-radio /printed/path/radio-2 --sf 7 --duration-ms 10000
# Then transmit from the other radio.
target/debug/loramesh-radio /printed/path/radio-1 --sf 7 --send 48656c6c6f --duration-ms 1000
```

The serial path can also be passed to the existing Linux daemon with
`LOMESH_RADIOPORT=/printed/path/radio-1`. That daemon still requires privileged
TUN setup and retains its legacy networking limitations. Use the radio probe for
milestone 1/2 tests; full TUN/network-namespace acceptance is now covered by
`scripts/test-linux-tun.py` and `scripts/test-secure-linux.py`.

PTY mode runs for the scenario's `duration_us`, in real time. Ctrl-C, SIGTERM,
normal completion, and handled errors clean up the laboratory. As with other
processes, SIGKILL cannot run cleanup code. A simulated reconnect creates a fresh
PTY and atomically replaces the same stable symlink; the production worker reopens
that path after its backoff. Scenario traffic and expectations are ignored in
interactive PTY mode because external clients drive the devices.

## Architecture

- `src/radio/protocol.rs`: bounded byte-stream framing, checked serial replies,
  validated LoRa profiles, and integer airtime calculations.
- `src/radio/controller.rs`: clock-independent state machine. It consumes lines,
  submissions, connection events and timer ticks, then emits I/O actions and events.
- `src/radio/runtime.rs`: single-owner production serial worker with 20 ms bounded
  reads, bounded channels, nonblocking delivery, and explicit shutdown/reconnection.
- `src/sim/device.rs`: independent device-side command grammar and firmware profiles.
- `src/sim/medium.rs`: ordered discrete events, directed links, airtime and reception.
- `src/sim/scenario.rs`: scenario validation, production-controller orchestration,
  expectations and reproducible reporting.
- `src/sim/pty.rs` and `process.rs`: real-time device endpoints and process smoke runner.

In deterministic mode, the clock jumps directly to the next event. Equal-time
ordering is stable; maps use node-ID ordering and scenario lists preserve their
order for ties. A fixed xorshift PRNG makes link-fault decisions repeatable without
relying on a dependency's random-number implementation. Serial output is delivered
to controllers in seven-byte chunks to exercise stream framing.

The virtual device does not reuse the host command validator. Hand-authored
conformance fixtures in `tests/fixtures` provide an independent reference, and
controller tests additionally use scripted replies instead of the emulator.
No actual firmware transcripts have been captured yet.

## Scenario format (version 1)

See the checked-in JSON files under `scenarios/` for complete examples.

| Field | Meaning |
| --- | --- |
| `version`, `name`, `seed` | Schema version, scenario label, deterministic fault seed |
| `duration_us`, `max_events` | Time and event/trace budgets; exhausting a budget is an error |
| `nodes` | Unique IDs, RN2903/RN2483 firmware profile, and radio profile |
| `links` | Directed reachability; reverse links must be listed explicitly |
| `traffic` | Submission time, source node and hexadecimal radio payload |
| `faults` | Timed disconnect/reconnect, reply suppression/delay, serial line injection, or link enable/disable |
| `expect` | Exact per-node receive counts, total completed transmissions and failed submissions |

Link fields include `loss_per_mille`, `duplicate_per_mille`, `delay_us`, `jitter_us`
and `enabled`. Defaults mean a lossless zero-delay link. A timed disabled link
models a burst outage; jitter can reorder otherwise successful deliveries.
Loss, duplicates, delay, and reordering are deliberate fault injection, distinct
from the modeled RF behavior. Unknown configuration fields are rejected.

Scenarios cover two-node, line, triangle, diamond, disconnected, hidden-terminal,
and reconnect cases. The topology scenarios test radio connectivity and interference;
**they do not yet test mesh forwarding**. No simulated node forwards arbitrary
received payloads automatically.

## Model and measurements

The medium requires a receiver to be listening from the beginning of a transmission
through its completion. It models half-duplex operation, exact profile matching,
receiver interruption, watchdogs, and same-profile overlapping transmissions at a
receiver. Hidden terminals can collide even when they cannot hear one another.
A completed TX command says nothing about whether another device received it.

Airtime uses the Semtech explicit-header LoRa equation, automatic low-data-rate
optimization at long symbol periods, and integer rounding to microseconds. Supported
profiles are SF7–SF12, 125/250/500 kHz, CR4/5–4/8, configurable preamble/CRC/sync.
Independent test vectors include a 207-byte SF12/125 kHz frame at 7,544,832 us and
a 207-byte SF7/125 kHz frame at 327,936 us.

The current conservative RF model has no propagation-distance model, capture
effect, adjacent-channel interference, SF cross-interference, detailed RSSI/SNR,
antenna or terrain simulation. Configured delay/jitter is a delivery fault, not a
physical propagation model. Abruptly interrupted transmissions retain their
scheduled airtime accounting as a conservative upper bound. The virtual firmware
only implements commands used by the adapter; unsupported commands return errors.
It restores the scenario's stored profile on reset. USB enumeration and electrical
behavior are outside the PTY model.

Reports distinguish controller receipts from medium deliveries, and expose:

- Transmitted/received/failed frame counts and received payload bytes.
- Peak controller queue depth and virtual serial output occupancy.
- Per-node airtime, collision/loss/unavailable-receiver drops, and delivery latencies.
- Ordered serial commands, responses, state changes, faults, and payload traces.

Medium latency runs from TX start to delivery of the virtual serial response,
including configured injected delay. It excludes time waiting in the host queue.
Duplicate injection counts as another delivery. Payload-byte rates are raw test
payload goodput, not IPv4 application throughput. Reassembly and link-retry metrics
are reported by the link and secure mesh runners described below. Simulator
wall-clock execution speed is not on-air throughput or a range prediction.

## Bounds and recovery

Serial lines are capped at 600 bytes; frames at 255 bytes. Overlong lines discard
through the next newline. The production worker recovers the session on malformed
stream framing and re-arms RX after a malformed RX response. Unknown/late events
cannot count as successful TX completion in the wrong state.

The controller defaults to at most 32 queued/active frames and 120 seconds of queued
airtime, with queue expiry after 120 seconds. Incoming channels are also bounded;
a full consumer queue drops and counts the event instead of blocking radio control.
Command acknowledgments default to a two-second deadline; TX completion gets the
calculated airtime plus that margin. Failed or ambiguous TX is reported once and
never automatically retransmitted. The future link layer owns retry policy.

Recovery closes the transport, waits a bounded backoff, reopens the configured path,
clears stale serial buffers, resets the module, applies configuration, reads back
airtime parameters, and resumes RX. The receive guard provides listening time after
TX; it is not a regulatory duty-cycle implementation. Default daemon settings remain
SF12/125 kHz/CR4/5, with a one-second receive guard from `txslot`. Power defaults to
20 dBm for RN2903 and 14 dBm for RN2483. Initialization files must use the supported
command subset; invalid commands fail validation before opening the serial device.

The retained legacy mesh parser now rejects malformed enum tags, truncated control
messages, invalid UTF-8 error messages, and invalid IP assignments. Legacy incomplete
assemblies are capped at 64 packets, 1,500 bytes and 150 chunks each, with an actual
expiry timer. The default assembly timeout is now 120 seconds, because a 1,500-byte
packet at SF12 can require over a minute with receive guards. Existing explicit
10-second settings are now enforced and may need increasing. This bounds state but
cannot make the old unordered fragment format
reliable; use the replacement reliable link or secure mesh protocol instead.

## Validation status

Automated tests run without radios on the development macOS host: deterministic
scenario/replay, scripted command conformance, partial reads/writes, controller
faults, PTY byte exchange, device replacement, child-process smoke, and cleanup.
The CI workflow runs the suite on macOS and Linux. Linux CI has been configured,
not observed running during this local change. Physical-radio calibration,
Linux TUN integration, reliable fragmentation, and multihop routing remain pending.


## Secure multihop scenarios

Milestones 5–6 add `loramesh-mesh-sim` and `scenarios/mesh/`: line, triangle,
diamond, hidden-terminal, disconnected, restart and diamond-repair. These run the
production secure Node, link/controller code and shared medium with deterministic
**test-only** keys. Production keygen uses OS randomness and durable state.

```sh
cargo run --release --locked --bin loramesh-mesh-sim -- scenarios/mesh/line.json --output target/secure.json
cargo run --release --locked --bin loramesh-mesh-sim -- target/secure.json --replay --output target/secure-replay.json
cmp target/secure.json target/secure-replay.json
```

Reports include per-node/per-peer metrics, routes, TTL, control traffic, radio
airtime, collisions, latency, throughput and exact replay inputs. `faults` uses the
existing medium fault format; `restarts` contains `{ "at_us": 70000000, "node": 2 }`.
Simulated restart advances the epoch while retaining replay state. Process tests
exercise real durable files separately. See [secure operation](secure-mesh.md)
and [multihop performance](mesh-performance.md) for limitations and commands.
