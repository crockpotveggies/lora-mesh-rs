# Performance evidence and stopping criterion

Measured on macOS-26.3-arm64-arm-64bit-Mach-O, rustc 1.95.0 (59807616e 2026-04-14), September 16, 2026.
The branch is `crockpot-revamp`. These are implementation comparisons within the
rewrite, not a measured speedup over physical 2020 hardware. The old fragmentation
protocol cannot supply a comparable reliable-delivery baseline.

## Simulated application performance

Each case transfers twelve 1500-byte IPv4/UDP packets (1472 application bytes each).
The producer maintains up to four queued packets, constrained by the same 120-second
initial-airtime admission budget as the daemon. Absolute packet lifetime is 120
seconds. SF is varied explicitly; BW=125 kHz, CR=4/5, preamble=8, CRC on, unrestricted
test airtime. Both data and ACKs experience the configured independent loss.

| Profile | Fragment bytes | Burst | Frame loss | Delivered | App bytes/sec | p50 seconds | p95 seconds |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| SF7 | 64 | 1 | 0% | 12/12 | 191.5 | 30.58 | 30.58 |
| SF7 | 209 | 1 | 0% | 12/12 | 363.0 | 16.05 | 16.05 |
| SF7 | 209 | 4 | 0% | 12/12 | 424.3 | 13.71 | 13.71 |
| SF7 | 209 | 8 | 0% | 12/12 | 436.5 | 13.32 | 13.32 |
| SF7 | 209 | 4 | 10% | 12/12 | 266.7 | 18.69 | 26.40 |
| SF9 | 209 | 4 | 0% | 12/12 | 144.2 | 40.44 | 40.44 |
| SF12 | 209 | 4 | 0% | 12/12 | 20.6 | 69.17 | 69.17 |
| SF12 | 209 | 4 | 10% | 10/12 | 12.1 | 82.61 | 113.95 |

Queue time is included in latency. Slow profiles can expire queued packets or
exhaust retries. A receiver can deliver a packet whose final ACK never reaches the
sender, so receiver-delivered and sender-acknowledged counts intentionally differ.
The full [54-case report](benchmarks/final.json) preserves these outcomes, resource
peaks and seeds. Repeated final runs produced identical protocol metrics.

## Optimization iterations

1. **Fragment capacity and ACK aggregation.** Within the same model, full 209-byte
   fragments and four-frame bursts improve loss-free SF7 goodput from
   191.5 to
   424.3 application bytes/sec.
   Eight-frame batches are slightly faster without loss, but hold the medium longer
   and delay receive opportunities. Four remains the default; both are configurable.
2. **Retry policy.** A short jitter window initially failed simultaneous-send tests.
   Full-burst backoff fixed contention but penalized one-way loss. The accepted policy
   uses shorter initial retries only after ACK progress and before competing peer
   data is observed. At SF7/10% loss this changes 224.5
   to 266.7 bytes/sec
   (18.8% improvement).
   The first unconstrained fast-retry attempt regressed bidirectional delivery and
   was rejected by tests. See the [conservative baseline](benchmarks/conservative-retries.json).
3. **Host event scheduling.** Replacing a 10 ms simulation clock cap with direct
   event jumps reduces median sweep execution from 276.0
   to 100.5 ms across five release runs
   (2.75× faster).
   All 54 protocol outcomes are identical between these clock implementations.
   Removing repeated admission attempts while an airtime-limited queue was full
   also prevents synthetic rejection counters from depending on clock granularity.
4. **Copy/allocation assessment.** Reassembly writes directly into one bounded packet
   buffer and moves that buffer into the output queue. Wire decoding borrows payloads;
   no per-fragment reassembly allocations or tail-copy chunking are used. Encoding
   and reassembling a 1500-byte packet costs a median
   616 ns in the host microbenchmark. Further
   complexity to remove these copies would have negligible effect on multi-second
   radio transfers, so no speculative unsafe or zero-copy buffer pool was added.

[Five-run host measurements](benchmarks/host-runs.json) contain wall time, CPU time,
peak RSS and microbenchmarks. Results include the benchmark's trace collection and
report construction; they are not daemon memory measurements.

## Actual release processes over PTYs

Two real `loramesh-link` processes exchanged six 1500-byte IPv4 packets through the
virtual LoStiks. Application goodput was 391.8
bytes/sec, p50 3.725 s and p95 3.752 s.
The p95 here is the maximum of six samples, so treat it as smoke-level timing evidence.
The measured run had no retransmissions or payload loss.

CPU usage, including startup/shutdown, was 0.64% / 0.65%
for the two idle daemons and 0.37% / 0.41% under load.
Peak per-process RSS was 6.12 MiB. Idle sampling lasts three seconds;
loaded sampling lasts 22.54 seconds, so initialization accounts
for a larger fraction of the idle percentage. The emulator is a separate process
and is excluded from these daemon measurements. [Raw results](benchmarks/daemon.json).

The separate Linux namespace suite passed real TUN ping, 2000-byte non-DF ping,
DF/MTU rejection, 4096-byte UDP echo and 8192-byte TCP echo. These validate IP behavior;
they are not RF benchmarks. Physical range, RSSI/SNR, interference and hardware
turnaround still require calibration.

## Compression decision

A [synthetic zlib level-1 probe](benchmarks/compressibility.json) confirms that repeated
bytes compress well while seeded random bytes expand. This deliberately contrasts
compressible test traffic with high-entropy traffic; neither represents a deployment
capture. Blind compression is therefore disabled. A future payload codec must signal
its format, preserve the uncompressed length/bounds, and save airtime after framing.

Removing a fixed 28-byte IPv4/UDP header would not reduce the eight-fragment count
of a 1500-byte packet with a 209-byte span. Some shorter packets could save airtime,
but safe header compression needs versioned shared context, explicit refresh and
recovery after restart/loss, and support for options/fragments/non-UDP traffic.
That complexity is not justified by these measurements. No implicit shared context
or compression mode is enabled.

## Correctness and coverage

The final macOS suite passes 61 tests. Linux ARM64 also runs the shared suite;
the separate namespace acceptance run passed twice, including real UDP/TCP payloads.
The protocol has 98.44% line coverage, the wire codec 100%, and the link simulator
93.57%, measured with `cargo llvm-cov --locked --all-targets`. The portable daemon
has 78.40% line coverage on macOS; Linux-specific TUN behavior is exercised by the
namespace suite rather than included in that macOS coverage number.
[Coverage detail](benchmarks/coverage.json). New library/binaries pass strict Clippy;
legacy daemon naming/dead-code warnings remain outside this rewrite's scope.

## Reproduce and continue

```sh
cargo build --release --locked --bins
target/release/loramesh-bench > target/final.json
target/release/loramesh-bench --conservative-retries > target/retry-baseline.json
target/release/loramesh-bench --fixed-clock > target/clock-baseline.json
python3 scripts/bench-daemon.py > target/process-performance.json
```

The benchmark's case JSON includes `fast_loss_retries` and `clock_step_us`, so both
baselines remain executable. `--fixed-clock` only changes simulator host scheduling.
For an individual replay, extract one report's `configuration` into a JSON file and
pass that file to `loramesh-bench`.

Stop criterion: correctness/coverage gates pass; finite queue/retry policies remain
observable; repeated sweeps are deterministic; the measured host cost is negligible
beside RF airtime; the remaining batch/profile choices trade latency, loss and range
rather than offering a universal speedup. Reopen optimization with actual device
captures, representative traffic, or a measured regression. Hardware performance
and an optimal field profile are not established by this work.
