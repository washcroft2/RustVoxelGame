# Phase 0 spike: quinn datagrams against bulk streams

Code: `spikes/quinn` (throwaway). Run: `cargo run --release -p spike-quinn` (about 3 minutes).
quinn 0.11.12 (`runtime-tokio`, `rustls-ring`, default features off), rustls 0.23.46 with
`ring`, rcgen 0.14.10 for a self-signed certificate.

## Setup

Server and client run in one process, joined through a userspace UDP proxy (`link.rs`) in
place of `tc netem`, which needs root. The proxy applies, per direction: 75 ms one-way delay
(RTT 150 ms), ±3 ms jitter, a 20 Mbit/s rate cap with a 150 ms drop-tail queue shared by all
connections, and optional random loss. Delivery is strictly in order.

Load: 30 Hz x 200 B datagrams server to client for 12 s (stand-in for entity updates), with and
without a bulk stream of 16 KB writes (stand-in for the terrain burst). Latency is one-way, so
the floor is about 76 ms. Pacing means the server limits its own write rate.

## Results

**2% loss each way** (the Phase 2 gate condition):

| Scenario | Datagrams delivered | p50 | p99 | Bulk throughput |
|---|---|---|---|---|
| datagrams only | 97.0% | 76 ms | 80 ms | none |
| same connection, Cubic, unpaced | 98.9% | 101 ms | 190 ms | **0.8 Mbit/s** |
| same connection, BBR, unpaced | 95.3% | 139 ms | 227 ms | 14.5 Mbit/s |
| same connection, BBR, paced 50% | 98.6% | 79 ms | 86 ms | 9.8 Mbit/s |
| second connection, Cubic, unpaced | 98.9% | 76 ms | 80 ms | 0.7 Mbit/s |
| second connection, BBR, unpaced | 98.1% | 129 ms | 227 ms | 15.3 Mbit/s |
| second connection, BBR, paced 50% | 98.6% | 79 ms | 85 ms | 9.7 Mbit/s |

**0% loss:**

| Scenario | Datagrams delivered | p50 | p99 | Bulk throughput |
|---|---|---|---|---|
| datagrams only | 100% | 76 ms | 80 ms | none |
| same connection, Cubic, unpaced | 100% | 180 ms | 228 ms | 18.3 Mbit/s |
| second connection, Cubic, unpaced | 100% | 181 ms | 227 ms | 18.3 Mbit/s |
| same connection, BBR, unpaced | 98.9% | 131 ms | 228 ms | 15.7 Mbit/s |
| second connection, BBR, unpaced | 100% | 182 ms | 223 ms | 17.7 Mbit/s |
| same connection, BBR, paced 50% | 100% | 79 ms | 85 ms | 9.9 Mbit/s |
| second connection, BBR, paced 50% | 100% | 78 ms | 85 ms | 9.9 Mbit/s |

## Reading

1. **Bulk streaming at 2% loss with Cubic is very slow.** About 0.8 Mbit/s on a 20 Mbit/s link,
   which is what loss-based congestion control predicts at 150 ms RTT and 2% loss. At that
   rate a join with thousands of chunks takes tens of seconds. This bears on the Phase 2 gate
   (2% loss, 150 ms) and the join-time target. It is a property of the congestion controller,
   not a bug in the test (the same code reaches 18.3 Mbit/s on the clean link).
2. **BBR avoids that** (14–15 Mbit/s at 2% loss) but fills the shared queue when unpaced,
   which raises datagram latency to p50 130–140 ms and drops about 5–7k packets at the queue.
   quinn documents its BBR as "Experimental! Use at your own risk", so adopting it needs its own
   evaluation. That is a decision for you, and I have not made it.
3. **Pacing the bulk stream to about half the link rate keeps datagrams healthy** in every
   configuration: p50 within 3 ms of the floor, p99 within about 10 ms, and about 10 Mbit/s of
   bulk throughput. This supports the plan's per-client terrain pacing. The catch is that the
   server must know the link rate to pace against it; this test used the known 20 Mbit/s.
4. **A second connection helps only in one case.** With Cubic and loss, bulk sharing a
   connection with datagrams delayed them (p50 +25 ms, p99 +110 ms, no queue involved), while a
   second connection did not. But when the bulk traffic fills the bottleneck queue (clean link,
   or BBR), a second connection is no better, because both share the queue. So the plan's
   fallback of a separate terrain connection does not replace pacing.
5. **Datagram delivery** stays near the link's own loss rate in every case; I saw no extra
   datagram loss from sharing a connection with bulk.

## Not covered

- Only server to client traffic and one client. Client to server input datagrams, many clients
  sharing one upload link (the 10 Mbit/s home-hosting case), and congestion across players.
- Real network paths: loss here is independent random loss, whereas real loss is bursty; real
  jitter is correlated; real queues differ. All traffic crosses localhost through the proxy.
- Cubic and BBR parameters were quinn's defaults. Other tuning (initial window, pacing in the
  transport, flow-control windows) was not explored.
- How a server would learn the safe pacing rate on a real link.
- Other operating systems.
- The first version of the proxy reordered packets and produced misleading Cubic results; it
  was fixed before the numbers above, and the clean-link run confirms Cubic fills the link.
