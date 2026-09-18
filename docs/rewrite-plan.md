# LoRa Mesh rewrite plan

Status: milestones 1–6 are implemented, with simulator/virtual-device tests,
reliable IPv4 transport, signed multihop routing, authenticated encryption,
Linux TUN acceptance, coverage gates and recorded performance experiments.
Native packaging and tagged-release workflows are also implemented. Single-radio
RN2903 checks have passed; two-radio physical IPv4 acceptance, field calibration
and independent security review remain open. See [the simulator guide](simulator.md)
and [hardware results](radio-compatibility.md) for scope and validation limits.

Working branch: `crockpot-revamp`.
Baseline: `40ff4ab6c9926713cadbbce2eced79b392de7825` (April 2020).

## Objective

Keep the useful product: a USB radio turns a computer into a node on a private
IPv4 network. Rebuild the radio controller and packet protocol in Rust so that
delivery is predictable, resource usage is bounded, and performance can be
measured. Preserve the LoStik as the initial hardware target.

Optimize delivered application bytes per second and latency, as well as host CPU
and memory. Radio airtime is the main capacity constraint; a language edition or
async runtime change alone will not increase radio throughput.

## Scope and working assumptions

- Linux x86_64 and ARM64 are the initial runtime targets. Keep the protocol and
  simulation tests portable; macOS compilation does not establish TUN support.
- Establish a two-node, single-hop link before enabling multihop forwarding.
- Build the simulator and virtual LoStik before replacing the radio controller.
  Routine development and regression tests must run without physical radios.
- Keep IPv4/TUN as the application interface. First validate UDP and ICMP, then
  TCP behavior under loss, long round-trip times, and queue pressure.
- Use explicit static node and IP configuration initially. Dynamic assignment,
  automatic radio-profile negotiation, and Internet gateway/NAT support come later.
- Design a versioned replacement wire protocol. Do not assume compatibility with
  the 2020 frames; migrate communicating nodes together and reject unknown versions.
- Keep radio settings explicit. Do not silently change frequency, transmit power,
  or modulation during migration.
- Retain the current implementation while establishing the baseline and independent
  replacement modules. Remove legacy paths once replacement acceptance tests pass.

Hardware model/firmware, desired range, target traffic rate, typical packet sizes,
node count, topology, and deployment region still need confirmation before field
tuning. These do not block protocol, simulator, or serial-controller development.

## Proposed structure

Start with modules in one Cargo package; split crates only when reuse warrants it.

| Component | Responsibility |
| --- | --- |
| Protocol core | Checked frame codec, identifiers, fragmentation, reassembly, duplicate detection, hop limits |
| Link controller | Airtime accounting, bounded queues, receive windows, retry policy, deadlines |
| Radio adapter | LoStik command parser and state machine, deadlines, reset/reconnection |
| IP adapter | TUN lifecycle, IPv4 validation, MTU handling, local routes |
| Router | Neighbor state, route selection, link expiry and failure recovery |
| Simulator | Deterministic event clock, shared radio medium, topology, traffic and fault scenarios, replay and metrics |
| Virtual LoStik | Stateful command/response device with in-memory and pseudo-terminal transports, connected to the simulated medium |
| Application | Validated configuration, structured diagnostics, shutdown and health reporting |

The core receives events and produces actions without depending on a serial port,
privileged TUN device, wall-clock sleeps, or a particular async runtime. One owner
controls each physical radio. Queue limits must cover both bytes and estimated
airtime; control traffic must remain serviceable during data congestion.

## Milestone 0: establish the baseline

- [x] Verify the actual repository and compare it with the reviewed source.
- [x] Create `crockpot-revamp` and save this plan.
- [x] Run the original test suite in the actual checkout: four tests passed on
  Rust 1.95 on macOS, with 86 compiler warnings. Hardware and Linux TUN integration
  were unverified at baseline; Linux TUN acceptance now passes.
- [x] Preserve reproducible dependency resolution with a tracked `Cargo.lock`.
- [x] Convert review reproductions into regression tests for desired behavior as
  the relevant modules are fixed; do not make defective behavior the final contract.
- [x] Record legacy frame examples and independent serial command/response fixtures.

Exit: reproducible build, known baseline, and an executable test list covering the
review findings. A hardware throughput baseline is a separate field measurement.

## Milestone 1: simulator and virtual device

Implemented. Deterministic scenario assertions and replay, PTY exchange/reconnect,
child-process smoke, and cleanup are covered by executable tests. The Linux CI
matrix is configured; Linux TUN acceptance now passes for both reliable single-hop
and secure multihop paths. Physical calibration remains pending. Raw radio and
IPv4 application metrics are reported separately.

Deliver two complementary tools that exercise the production protocol and radio
controller, rather than a separate implementation of the mesh stack:

1. A deterministic, in-process network simulator for fast protocol and scheduler
   tests. Use an injected clock, stable event ordering, and a seeded random source;
   advance directly to the next event instead of sleeping through radio airtime.
2. A virtual LoStik that exposes the command interface used by the real adapter.
   Offer an in-memory byte transport for deterministic tests and a pseudo-terminal
   endpoint that the actual daemon can open as its configured serial device. A PTY
   is sufficient initially; USB enumeration and electrical behavior are out of scope.

### Shared medium and scenarios

- Define a radio/clock boundary before implementing the replacement controller.
  Run the same protocol and scheduling logic with virtual and physical adapters.
- Model packet airtime from payload length and explicit modulation parameters,
  half-duplex operation, RX/TX transitions, receiver availability, and compatible
  channel/profile settings. TX completion must not imply reception by a peer.
- Start with explicit directed connectivity and a conservative collision model:
  overlapping compatible transmissions at a receiver fail. Represent asymmetric
  links and hidden terminals; document capture/interference effects not yet modeled.
- Support reproducible independent and burst loss, delayed delivery, duplication,
  reordering, node restart, link outage, and queue overload. Distinguish deliberate
  protocol fault injection from physically modeled radio behavior in reports.
- Store versioned scenario files containing topology, radio profiles, traffic,
  seed, fault schedule, duration, and expected outcomes. Include two-node, line,
  triangle, diamond, disconnected, and hidden-terminal scenarios.
- Produce machine-readable event traces and metrics: delivered application bytes,
  latency, loss, duplicate delivery, retransmissions, airtime, queue occupancy,
  reassembly memory, and recovery time. Save scenario and seed on every failure.
- Bound simulator events and memory; report a stalled or looping scenario instead
  of allowing CI to hang. Support replay and accelerated virtual time.

### Virtual LoStik behavior

- Implement the subset of RN2903/RN2483 commands actually used by the adapter:
  initialization and radio configuration, RX start/stop, TX, firmware queries, and
  LED commands. Keep firmware differences explicit in device profiles and reject
  unsupported commands rather than returning success for everything.
- Model immediate command acknowledgment separately from asynchronous `radio_rx`,
  TX completion, watchdog errors, and busy responses. Enforce legal state transitions
  and frame limits, and route transmitted payloads through the shared medium.
- Inject partial serial reads/writes, split or combined lines, malformed responses,
  absent/delayed acknowledgments, RX/stop races, EOF, and disconnect/reconnect. Handle
  PTY replacement after reconnect through the same device-path mechanism as the app.
- Base expected command behavior on vendor documentation and, when available,
  recorded device transcripts. Keep independent conformance fixtures so the driver
  and virtual device cannot silently agree on the same incorrect behavior.

### End-to-end test environment

- Provide a scenario runner that creates virtual devices, launches nodes, captures
  diagnostics, enforces timeouts, and cleans up processes and device endpoints.
- Run unprivileged in-memory tests on developer machines and in ordinary CI.
  Exercise the serial adapter through PTYs on supported Unix hosts.
- Add a Linux integration suite with isolated network namespaces and real TUN
  interfaces, one daemon and virtual radio per node. Test ping, UDP traffic, and
  TCP transfers through the full application without attaching physical radios.
  Confine privileged network setup to this suite and clean it up on failure.
- Use virtual time for deterministic in-process assertions; use bounded real time
  for PTY/process/TUN tests. Seeded faults alone do not make OS process scheduling
  deterministic, so capture full integration traces for diagnosis.
- Keep simulated link goodput separate from simulator execution speed and host
  performance measurements. Calibrate timing and loss assumptions against hardware
  later; passing simulation does not establish real-world range or RF performance.

Exit: a one-command, hardware-free smoke scenario creates two virtual radios and
passes a payload through their command interfaces. Identical in-process scenarios
and seeds produce identical traces. Independent tests verify airtime vectors,
half-duplex receive exclusion, collision handling, serial event ordering, fault
injection, resource bounds, and cleanup. The runner and PTY transport are ready
to exercise the replacement daemon; full IPv4/TUN acceptance has since passed in
milestones 3 and 6.

## Milestone 2: robust codecs and radio control

Implemented. The daemon now uses the shared production controller/serial worker.
Tests cover parsing, interleaved events, bounded queues, timeouts, EOF, reconnect,
partial writes, shutdown, and malformed legacy mesh inputs. Hardware conformance
still requires device captures; the emulator implements the adapter's command subset.

- Extract a pure serial-line parser. Correct the `radio_rx` prefix offset and
  handle documented spacing, empty/truncated input, invalid hex, and bounded lengths.
- Parse command replies and asynchronous radio events into distinct types. Do not
  allow LED or configuration commands to consume received packets or TX completion.
- Implement explicit initializing, receiving, stopping-receive, transmitting,
  recovering, and disconnected states, with command-specific deadlines.
- Check TX completion and propagate initialization/I/O failures. Recover from
  unplug/replug, EOF, busy responses, watchdog errors, and missing responses.
- Eliminate duplicate scheduling and repeated rate-limit consumption. Wait for
  input or deadlines instead of continuously polling empty channels.
- Reject malformed frames without panics and validate configuration ranges.

Exit: deterministic virtual-device tests verify interleaved replies/events, timeout
recovery, shutdown, and exactly one initial transmission per scheduled frame.
PTY tests exercise the production serial adapter against the virtual LoStik.
All malformed-input cases return errors without crashing or allocating unboundedly.

## Milestone 3: reliable two-node IPv4 link

Implemented in `loramesh-link`; see [wire v1](wire-v1.md) and the
[reliable-link guide](reliable-link.md). Production-process PTY tests and Linux
namespace/TUN ping, fragmentation, UDP and TCP acceptance have passed locally.
Physical LoStik acceptance and durable receiver-restart deduplication remain
explicit limitations.

- Specify the new frame format before implementation: version, network identity,
  source/destination, packet sequence/session identity, fragment index/count or
  offset/total length, hop limit, flags, and authentication overhead.
- Derive fragment capacity from the complete radio frame budget. Validate lengths
  and bounds before storing fragments; define sequence-wrap and restart behavior.
- Reassemble by fragment identity, detect duplicates, expire incomplete packets,
  and cap per-peer and global buffered bytes. Deliver each complete packet once.
- Add bounded selective retransmission with acknowledgment aggregation where useful.
  Derive retry deadlines from airtime and turnaround; make reliability policy
  explicit so stale UDP telemetry can expire rather than retry indefinitely.
- Implement bounded ingress/egress queues with control priority, fairness, and
  observable drop reasons. Avoid blocking the radio event consumer on a full queue.
- Integrate TUN with explicit MTU and oversized-packet behavior. Preserve IPv4
  semantics and test fragmentation/DF/path-MTU behavior rather than assuming that
  every IP packet fits a single radio frame.

Exit: simulated two-node IPv4 delivery passes loss, duplication, reordering,
overload, restart, and timeout tests. Linux network-namespace tests use the virtual
LoStiks and real daemon to verify actual TUN packet flow, ping, UDP, and TCP.
Two physical LoStiks then verify on-air behavior using equivalent traffic scenarios.

## Milestone 4: airtime and performance

Implemented and measured with the [benchmark suite](performance.md): profile,
fragment and batch sweeps; bounded airtime scheduling; selective-retry tuning;
event-driven simulation; process CPU/RSS and end-to-end latency/goodput. Hardware
link margin, RSSI/SNR and regional policy calibration remain field work. Compression
is evaluated but not enabled without representative application traffic/context.

- Record radio profile, frame size, TX airtime, queue delay, retransmissions,
  received signal metrics where supported, and application goodput.
- Schedule airtime rather than a fixed number of frames per second. Reserve time
  for reception and acknowledgments and account for the deployment's radio limits.
- Benchmark explicit SF/BW profiles on both peers. Choose profiles from measured
  delivery and link margin, not advertised peak bitrate.
- Benchmark fragment sizes and small bounded batches against latency and loss.
- Evaluate IP/UDP header compression only with explicit shared context and recovery.
  Enable payload compression only when it saves enough bytes on real traffic.
- Reduce unnecessary buffer copies and allocations after measuring host costs.

Exit: repeatable before/after results report application goodput, p50/p95 latency,
loss, idle/loaded CPU, and peak memory. Report calculated airtime separately from
measured performance. Do not promise a fixed speedup before hardware measurements.

## Milestone 5: multihop routing

Implemented: signed reciprocal link-state routing, airtime/retry costs, 20%
hysteresis, bounded flooding/forwarding, static routes, alternate-path repair,
TTL/checksum handling and seven replayable secure topology scenarios. See the
[secure mesh guide](secure-mesh.md) and [measured results](mesh-performance.md).

- Enforce next-hop ownership, hop limits, duplicate suppression, and bounded
  discovery forwarding. Reject self/invalid routes without panicking.
- Correct neighbor inference and retain alternate links. Remove periodic destructive
  minimum-spanning-tree pruning of the topology.
- Expire stale neighbors/routes and define repair after a relay disappears.
- Select routes using measured delivery and airtime costs, with hysteresis to avoid
  constant route changes. Start with static routes to isolate forwarding correctness.
- Test line, triangle, diamond, disconnected, and hidden-terminal topologies with
  competing senders; include per-hop airtime and control overhead in comparisons.

Exit: only intended relays forward unicast traffic, loops terminate, broken paths
recover, and total transmissions remain bounded under duplicate discovery traffic.

## Milestone 6: secure operation and modernization

Implemented: XChaCha20-Poly1305 frames, Ed25519 origin advertisements, durable
epoch/replay state, private provisioning, privilege separation, Rust 2024/MSRV
1.85, modern serial/native Linux TUN adapters, coverage gates, fuzz target,
advisory checks and native release packaging. Local macOS and Linux ARM64
acceptance passed; x86_64 execution is configured in CI but has not run locally.
Physical-radio calibration and independent security review remain pending.

- Use established authenticated encryption with authenticated routing metadata,
  replay protection, and a nonce/session design that remains safe across restarts.
  Reserve its wire overhead in milestone 3; complete security before field deployment.
- Validate peer identity and configuration; bound all state created by radio input.
- Isolate privileged interface/route setup from ongoing radio packet processing.
- Move to Rust 2024 with a declared minimum supported compiler and a locked build.
- Evaluate maintained TUN, serial, and IP parsing dependencies behind the adapters;
  keep dependency/runtime choices separate from the protocol's public contract.
- Add Linux x86_64/ARM64 CI coverage, formatting/lints, parser fuzzing, dependency
  checks, release packaging, configuration examples, and recovery documentation.

Exit: reproducible releases, safe restart/key behavior, useful operational errors,
and a documented installation that does not require the entire daemon to retain
unnecessary privileges.

## Verification and evidence

The initial review used a temporary checkout of the same baseline: four original
unit tests passed on Rust 1.95, and eight isolated probes demonstrated existing
defects. Those probes included a replay of the scheduler's decision logic and a
serial response in the documented single-space format; they were not hardware tests.

Preserve the following cases in the replacement test suite: malformed enum tags,
empty routes, truncated broadcast payloads, header-inclusive frame size limits,
missing/duplicate/reordered fragments, single-space radio responses, duplicate
TX scheduling, and relay-versus-origin neighbor inference.

Physical-radio acceptance remains pending access to two radios. Full TUN integration
has passed in an isolated Linux container; simulator/core tests and supported PTY tests need no hardware.
Document device firmware and configuration with each result so comparisons remain
meaningful. The injectable clock/transport boundaries, independent serial fixtures,
virtual smoke scenarios, receive parser, and radio command state machine are now
implemented. The new wire format, reliable fragmentation and two-node IPv4 delivery
now use this environment. Secure multihop forwarding also uses the same simulator,
with real three-process and Linux TUN acceptance. Next work is physical-radio
calibration, deployment-specific airtime policy and independent security review.
