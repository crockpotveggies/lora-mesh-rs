# Reliable single-hop IPv4 link

Milestones 3 and 4 introduce `loramesh-link`, retained as an unauthenticated
single-hop regression/lab tool. Milestones 5 and 6 add the [secure multihop
daemon](secure-mesh.md), which is the current deployment path. The original
`loramesh` executable requires `--features legacy`. These wire formats do not
interoperate. Dynamic address assignment and gateway/NAT remain outside scope.

## Run on Linux

```sh
cargo build --locked --release --bins
sudo target/release/loramesh-link conf/link-node1.json
```

Edit the sample serial path and explicitly choose frequency, power, modulation and
allowed airtime budget for your deployment. `duty_per_mille: 1000` is an unrestricted
**test setting**, not a regional compliance claim. The scheduler charges every
attempt and ACK, spacing transmissions to the configured fraction. This simple
spacing policy does not implement region-specific dwell time/channel hopping rules.
No profile is automatically selected from the benchmark results.

For node 2 swap `node`/`peer` and `address`/`peer_address`; keep network, modulation,
CRC, frequency, turnaround and airtime policy consistent. `power_dbm` is explicit;
firmware rejects unsupported power settings. A fresh random session is generated
at every daemon launch; the configuration's session value is only a core-test seed.

The daemon creates an exclusive point-to-point TUN interface with MTU 1500 and a
peer route. It refuses an existing interface name. Dropping its descriptor removes
the nonpersistent interface and routes, including on errors. SIGINT/SIGTERM stops
workers and writes metrics. Network setup currently requires privileges throughout
the process; privilege separation belongs to milestone 6.

IPv4 datagrams are validated (version, header length, total length and header
checksum); options, DF/MF, fragment offset, protocol and TTL are preserved. Linux
handles IP fragmentation and local path-MTU errors. IPv6 is rejected. The link
never truncates or silently rewrites an oversized IPv4 packet.

## Protocol and resource limits

[Wire v1](wire-v1.md) specifies the byte layout, selective bitmap ACKs, replay
window, authentication reservation and restart semantics. The default is 209 data
bytes per radio frame and a four-frame burst. ACKs preempt queued data. Receive
windows and airtime-sized jitter prevent synchronized transmitters from repeatedly
overlapping. After proven one-way progress, the first two retries use a smaller
ACK-sized window; repeated failures or competing peer data use full burst backoff.

Default limits are 16 queued packets / 24 KB / 120 seconds of estimated data
airtime; eight incomplete packets / 12 KB per peer (also the global cap because
this daemon has exactly one peer); 16 delivered packets; at most 16 pending ACK
records. The adapter has another 16-packet output queue. Limits include radio
header/tag overhead in airtime. Retry and control overhead are charged when sent;
queue admission's airtime estimate covers initial data transmissions.

Five consecutive retries without ACK progress are allowed. ACK progress resets
that counter, but every packet still has an absolute 120-second ingress deadline.
Queued packets can expire before transmission, especially at SF12. Select a longer
`lifetime_us` for bulk traffic or a shorter one for stale telemetry; this is an
explicit policy, not a promise of eventual delivery. Complete reassembly waits
within its existing memory/deadline bounds when output is full. Every rejection,
expiry, duplicate, retry and radio failure has a metric.

Delivery deduplication lasts for the receiving process lifetime, with a 64-sequence
window per session and eight sessions retained. Older sequences are rejected;
receiver restart can redeliver an in-flight packet. This is not durable exactly-once
messaging. No field in v1 authenticates a sender, even though 16 bytes are reserved
for a future authentication tag.

## Hardware-free validation

```sh
cargo test --locked --all-targets
cargo test --locked --test link_process
cargo build --locked --bins
sudo python3 scripts/test-linux-tun.py  # Linux only
```

The process test launches two actual daemons, each connected to a virtual LoStik
PTY, using Unix datagram sockets as unprivileged raw IPv4 adapters. The Linux
suite additionally creates isolated namespaces and real TUN devices, testing ping,
a 2000-byte non-DF ping, oversized DF rejection, 4096-byte UDP echo and 8192-byte
TCP echo. All subprocesses have deadlines; owned namespaces and PTYs are cleaned
on success and failure. Linux logs and simulator trace are saved under
`target/tun-artifacts`. Never use SIGKILL when you need graceful artifact capture.

The deterministic link tests use production endpoints, radio controllers and the
shared medium. They cover loss, duplicate/reordered fragments, burst outage,
asymmetric packet scheduling, bidirectional contention, session restart, invalid
frames, incomplete packet expiry, output congestion, selective retries and lost
final ACKs. Finite retry/age budgets intentionally permit drops under sustained
contention or blackholes.

## Performance and coverage

```sh
cargo run --release --locked --bin loramesh-bench > target/link-bench.json
cargo build --release --locked --bins
python3 scripts/bench-daemon.py > target/daemon-bench.json
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --version 0.9.1 --locked
cargo llvm-cov --locked --all-targets --json --output-path target/coverage.json
python3 scripts/check-link-coverage.py target/coverage.json
```

The benchmark sweeps SF7/9/12, fragment spans 64/128/209, batches 1/4/8, and 0/10%
independent frame loss. Every report embeds its complete `configuration`, seed,
link counters, queue/reassembly peaks, airtime, delivery loss and ingress-to-delivery
p50/p95. Pass a JSON configuration as `loramesh-bench CASE.json` to replay one case.
The load generator keeps up to four packets queued within the airtime budget; the 120-second lifetime is part of
the experiment and explains slow-profile losses. Application goodput excludes the
28-byte IPv4/UDP header and uses total elapsed virtual time. Frame loss applies to
ACKs as well as data. This is modeled performance, not measured RF range/capacity.

Virtual command processing is instantaneous; configured turnaround covers a fixed
allowance, not byte-accurate UART timing. PTY measurements exercise real process,
serial framing and scheduling overhead, but still use simulated RF airtime.
RSSI/SNR and link margin are not modeled or queried by this adapter. Hardware
captures and on-air profile comparisons remain necessary before choosing a range
or throughput profile.

`bench-daemon.py` reports actual release-process CPU time, peak RSS, loaded goodput
and latency separately from the simulator's execution speed. CPU/RSS include
startup and shutdown. Coverage thresholds require at least 95% of core lines,
98% of codec lines and 90% of link simulation lines; Linux/TUN behavior additionally
has the integration acceptance suite. Coverage is evidence, not a correctness proof.

See [the recorded benchmark results](performance.md) for optimization iterations,
tradeoffs and the stopping criterion.
