# Single-hop wire protocol v1

All integers are big endian. Frames are at most 255 bytes. Nodes must migrate
both ends together; this format is incompatible with the 2020 protocol.

| Offset | Bytes | Meaning |
| --- | --- | --- |
| 0 | 2 | Magic `LM` |
| 2 | 1 | Version 1 |
| 3 | 1 | Flags: bit 0 ACK, bit 1 request ACK; other bits zero |
| 4 | 4 | Network ID |
| 8 | 2 | Source |
| 10 | 2 | Destination |
| 12 | 8 | Random nonzero sender session (ACK echoes data session) |
| 20 | 4 | Packet sequence, starts at zero; never wraps |
| 24 | 2 | Original IPv4 packet length (20–1500) |
| 26 | 1 | Fragment index (zero for ACK) |
| 27 | 1 | Fragment count (1–32) |
| 28 | 1 | Hop limit, exactly 1 in this single-hop implementation |
| 29 | 1 | Fragment span, 48–209 bytes |
| 30 | 16 | Reserved authentication tag; MUST be zero in v1 |
| 46 | variable | Data fragment, or four-byte received-fragment bitmap for ACK |

Count must equal ceil(total/span). All nonfinal fragments have exactly span
bytes; the final fragment has the remaining bytes. ACK bits outside count are
invalid. Network, source, destination, session, sequence, count, span, total,
flags and payload lengths are checked before allocating reassembly state.

The tag reservation is included in every capacity/airtime calculation. It does
**not** provide authentication; v1 requires a trusted test network. Radio CRC
must be enabled. Encryption and replay protection against attackers are milestone 6.

Selective repeat uses one active packet per direction, a configurable bounded
burst, and an aggregate bitmap ACK. The burst's final frame requests an ACK.
The receiver also sends a delayed bitmap if that final frame is lost. Retry
windows include data/ACK airtime, serial turnaround and configured duty spacing.
Only missing fragments are retried; the last fragment probes when a final ACK
is lost. Consecutive retries without ACK progress, absolute packet age, queued bytes/airtime
and reassembly are bounded. ACK progress resets the consecutive retry counter.
ACKs have priority and are coalesced per packet. A receive window follows every
burst; deterministic session/node jitter breaks symmetric retry collisions.

Each peer has at most eight session replay windows retained for the lifetime of
the receiver. Each window tracks the newest 64 sequences; older packets are
rejected, never redelivered. New sessions beyond the cap are rejected, requiring
an operator restart. A sender creates a random session on process restart and
refuses sequence exhaustion. Exactly-once delivery applies within a receiver
process lifetime; receiver restart loses replay state and can redeliver an
in-flight packet. Persistent replay state is deferred with authenticated sessions.

IPv4 bytes (including options, fragmentation fields, TTL and checksums) are
preserved. The link does not perform IP routing, decrement TTL or IP reassembly.
An explicit TUN MTU of 1500 lets Linux fragment non-DF traffic and return local
EMSGSIZE for oversized DF traffic. Oversized or malformed ingress is rejected.
