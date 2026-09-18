# Secure multihop performance and acceptance

Measured 2026-09-17 on this Apple ARM64 host, Rust 1.95.0 release builds.
[Raw five-run comparisons](benchmarks/mesh-m5-m6.json) and
[three-process measurements](benchmarks/mesh-process-m5-m6.json) accompany this report.
These are software/virtual-radio measurements, not physical radio throughput.

## Optimization loop

Each change was checked with the production forwarding engine over the simulated
shared medium. Initial tests exposed ACK starvation across peer queues; the node
now has one radio scheduler, global receive windows and priority for ready ACKs.
Signed control floods are coalesced, admission is bounded, changes are rate limited,
and origins phase/jitter their announcements. Cost estimates use a 32-attempt
sample and EWMA/20% hysteresis to avoid reacting to every collision.

The executable comparison `--frequent-control` uses 10-second announcements and
40-second neighbor expiry. The selected defaults are 60 and 240 seconds. Both
use seed 42, SF7/BW125 kHz, CR4/5, 915 MHz, CRC on, preamble 8, sync 18, duty 100%,
256-byte IPv4 traffic, identical topologies and 120 seconds of simulated time.
Duty 100% is a laboratory comparison setting, not a deployment recommendation.

| Scenario | Frequent-control delivery latency | Selected delivery latency | Frequent / selected total TX airtime |
| --- | ---: | ---: | ---: |
| Line (2 hops) | 55.728 s | 7.490 s | 47.366 / 7.628 s |
| Triangle | No delivery by deadline | 10.203 s | 67.310 / 29.361 s |
| Diamond (2 hops) | No delivery by deadline | 24.363 s | 92.592 / 35.994 s |
| Hidden terminals, 2 senders | 1.710 / 25.757 s | 0.686 / 7.178 s | 54.323 / 11.493 s |

Line latency fell 86.6%; its whole-scenario airtime fell 83.9%. Failed overloaded
baseline cases are retained in the raw results. Sparse traffic over a fixed
120-second scenario should not be interpreted as saturated link capacity.

Routing was then optimized independently: rebuild cached paths only when topology,
expiry or cost changes. Cache a Dijkstra calculation per previous first hop during
hysteresis evaluation. `--uncached` provides an executable reference implementation.
Deliveries and airtime remain identical; across the four scenarios, median host
execution was **20.543 ms cached vs 33.037 ms uncached** over five runs (37.8% less
time, 1.61× faster). Line route rebuilds dropped from 1,412 to 17; diamond rebuilds
from 8,692 to 27. Timing comparisons are short microbenchmarks, not universal ratios.

Alternate repair was subsequently tested by taking relay 2 offline at 80 seconds
and sending another diamond packet at 100 seconds. It arrived through relay 3 in
43.588 seconds with exactly one failover, well before passive neighbor expiry.
The normal diamond-repair scenario also verifies eventual topology convergence.

## Encryption and persistent replay cost

A maximum 255-byte secure radio frame carries up to 193 bytes of encrypted data.
The five-run median for seal + authenticated open + in-memory replay accounting
was **1.647 µs** (20,000 operations per run). Durable receiver accounting, including
atomic write, file fsync and directory fsync, was **8.861 ms** per round trip
(100 operations per run on this filesystem). The latter must be remeasured on the
deployment's SD card/flash. Persistence is deliberately not batched before ACKs:
doing so would weaken crash/replay behavior. Keys and temporary benchmark state
are deleted after each run.

The host crypto cost is small beside SF7/BW125 airtime (roughly 400 ms for a full
frame). Further allocation tricks are not justified by these measurements. The
remaining radio contention, hardware turnaround, routing cadence and storage
tradeoffs need deployment-specific workloads and physical-radio evidence.

## Real daemon processes over virtual serial

`python3 scripts/bench-secure-daemon.py` runs three release daemons, real durable
security files, PTY serial workers, and a shared virtual RF line. Three sequential
1476-byte IPv4 packets (1448 UDP payload bytes each) traverse two hops. The startup
routing/discovery cost is included in the loaded measurement.

The measured run delivered all three in 81.77 seconds: **53.12 UDP payload bytes/s**,
median packet latency 22.62 seconds and maximum 47.59 seconds. The raw report labels
that maximum as nearest-rank p95; with only three samples it is not a reliable tail
estimate. Dynamic control exchanges and retransmissions remain visible in metrics.
This is a different workload from the small-packet simulator or prior single-hop
benchmark and is not a like-for-like speedup claim.

Per-daemon CPU was 0.82–0.86% during a 3-second idle phase, and 0.24–0.27% while
loaded. Peak RSS was 6.38 MiB idle, 6.61–6.77 MiB loaded. CPU includes startup and
shutdown; short idle measurements exaggerate steady-state CPU. No polling spin or
unbounded queue growth was observed. Physical throughput is still unmeasured.

## Verification

- 72 default-target tests pass on macOS, plus 9 opt-in legacy tests.
- Linux ARM64 container tests and real three-namespace TUN acceptance pass: multihop
  ping/TTL, fragmented IPv4, DF/MTU rejection, UDP and TCP; worker UID/GID drop,
  zero capabilities and `no_new_privs` asserted.
- Seven secure topologies replay deterministically; additional tests cover earlier
  alternate repair, 32 authenticated restarts, malformed packets, signatures,
  replay windows, queue bounds, ciphertext mutation and durable write failures.
- Independent libsodium/PyNaCl fixture verifies XChaCha20-Poly1305 frame bytes.
- A 31-second AddressSanitizer/libFuzzer run completed 1,420,682 inputs without a
  crash. This is a bounded smoke run, not proof of parser/security correctness.
- Rust 1.85 check, Rust 1.95 formatting/all-target Clippy and default dependency
  advisory scan pass. Linux x86_64 CI is configured; it has not run locally.

| Module | Measured line coverage | CI minimum |
| --- | ---: | ---: |
| Reliable link engine | 98.43% | 95% |
| Link parser | 100.00% | 98% |
| Link simulation | 93.57% | 90% |
| Mesh forwarding | 93.97% | 92% |
| Mesh parser | 100.00% | 98% |
| Security/state | 95.12% | 94% |
| Routing | 98.68% | 97% |
| Mesh simulation | 94.00% | 90% |

Coverage is from macOS instrumented default-target tests. Linux privileged setup
is verified separately by namespace integration; line percentages alone do not
measure branch/security completeness. See the [secure mesh guide](secure-mesh.md)
for threat model, state rollback limitations, commands and operational recovery.

## Repeat

```sh
cargo build --locked --release --bins
# Run each five times on an otherwise quiet machine:
target/release/loramesh-mesh-bench
target/release/loramesh-mesh-bench --uncached
target/release/loramesh-mesh-bench --frequent-control
python3 scripts/bench-secure-daemon.py
cargo llvm-cov --locked --all-targets --json --output-path target/coverage.json
python3 scripts/check-link-coverage.py target/coverage.json
```
