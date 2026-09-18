# LoStik compatibility corrections

Compared with original `master` at `40ff4ab`. The previous physical session showed
USB disappearance and serial I/O errors; their cause has not been isolated.

## Driver behavior

- On macOS, open the descriptor directly and apply raw 8N1/no-flow-control
  settings and standard 57,600 baud together, using `cfsetispeed`, `cfsetospeed`,
  and one `tcsetattr`. This avoids `IOSSIOSPEED` and serialport 4's intermediate
  reconfiguration at an inherited speed. Exclusive access and bounded serial I/O
  still use the current serial library. The port remains raw 8N1 with no flow control. PTYs use
  this same path, with a test that reads back both speeds.
- Start each session with `INVALIDCOMMAND`, drain replies for one second, then
  query `sys get ver`. Discard malformed and partial old input during this
  window. The deadline is event-driven, so shutdown remains responsive.
- Reset the MAC instead of rebooting the entire module. RN2903 uses `mac reset`;
  RN2483 selects `mac reset 433` or `mac reset 868` from the configured frequency
  (868 when none is configured). Normal initialization and profile readback follow.
- Preserve startup retries until the caller's readiness deadline. On timeout,
  include the most recent diagnostic. Opening, EOF, and initialization errors
  no longer prematurely consume readiness.
- Allow one lingering `radio_err` before the TX command acknowledgment, keeping
  the existing deadline. A second error, missing acknowledgment, or error after
  acknowledgment still fails the transmission. Only `radio_tx_ok` counts as success.

Command replies continue to pace normal operation. This change does not add the
old LED round trips or an unbounded `tcdrain` to the worker. No per-packet sleep
was introduced. Set `LORAMESH_RADIO_TRACE=1` to log actions before they execute,
including initialization failures before readiness. The one-second compatibility delay applies only to session startup.

## Validation

The complete macOS suite passed 78 tests, including secure multihop IPv4 delivery
through production daemon processes over PTYs. Targeted Linux container tests
passed; Windows cross-compilation passed. Strict Clippy passed.

Regression tests cover startup noise (including oversized unterminated input),
transient open/EOF failures, interruptible settling, delayed initialization
responses, both RN2483 bands, lingering RX errors before TX acknowledgments,
real TX failures, and bounded missing-ack recovery.
`scenarios/lingering-receive-error.json` exercises the delayed-ack race in the
independent virtual radio, and the usual scenario suite checks deterministic replay.

Coverage now has CI gates for the radio controller (90%), protocol (90%), and
serial worker (85%), alongside the existing link and mesh gates.

The [before/after benchmark](benchmarks/radio-compatibility.json) uses release
production daemons, three PTYs, and three 1476-byte IPv4 packets across two hops.
Goodput was 54.52 B/s before and 55.91 B/s with the final correction; median
packet latency was 21.74 s and 18.81 s. Loaded process CPU totaled 1.141 s before
and 1.038 s after. The intermediate revision measured 53.14 B/s and is also
retained in the report.
These are single runs, not a statistically established speedup or regression.
Startup is excluded from elapsed throughput measurement but included in CPU
counters. Physical RF throughput is not measured by this benchmark.

## Physical verification

After the device reappeared, the first compatibility revision still encountered
USB disappearance during startup. A minimal OS-level open/configuration check
succeeded without sending UART commands. After replacing the macOS builder path
with a single complete termios configuration, four consecutive receive-only
open/initialize/close cycles passed, with the USB node remaining present.

The previously failing single-radio acceptance test now passes on the RN2903:

- A short 915 MHz, SF7/BW125, 2 dBm probe transmission completed and returned to RX.
- The secure daemon initialized and completed a radio transmission with zero
  reported radio failures.
- An injected IPv4 packet for the absent peer expired as expected, without a
  false delivery report.

The final receive-only soak ran for 65 seconds after readiness (66.87 seconds
including startup): the 60-second watchdog fired, receiving restarted, and there
were zero reconnects or USB disappearances. The [hardware summary](benchmarks/lostik-compatibility.json)
records these results. Final measured line coverage was 92.90% for the controller,
91.52% for the protocol, and 86.77% for the serial worker; all CI gates passed.
These results support the compatibility correction but do not isolate which
previous OS operation caused the disappearances or prove long-term reliability.
Only one physical radio is available: over-the-air reception, end-to-end IPv4
radio delivery, and physical throughput remain unverified.

Local ignored evidence is in `target/hardware/`: `compat-reopens.json`,
`compat-single-radio.json`, `compat-watchdog-soak.json`, the corresponding logs,
`compat-coverage.json`, and `compat-usb-system.log`. Early failed attempts are
retained as `compat-receive-diagnostic.json` and its log.
