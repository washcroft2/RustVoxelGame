# 0001: Use BBR congestion control in quinn

Status: approved by the project owner, 2026-11-01.

## Context

The Phase 0 quinn spike (`docs/phase0/quinn.md`) found that with quinn's default Cubic
congestion control, bulk streaming at 2% packet loss and 150 ms RTT reaches only about
0.8 Mbit/s on a 20 Mbit/s link. That is the Phase 2 gate condition, and it would make joins and
chunk bursts slow. quinn's BBR reached about 14 Mbit/s in the same test.

quinn documents its BBR as "Experimental! Use at your own risk." Unpaced, it also fills the
bottleneck queue, which raised datagram latency from about 76 ms to 130–150 ms in the test.

## Decision

Use quinn's BBR congestion controller for game connections, together with the plan's
server-side terrain pacing. In the spike, pacing bulk traffic to about half the link rate kept
datagram latency within about 10 ms of the floor.

## Consequences

- Depends on an experimental component. Keep quinn pinned and re-test BBR on upgrades.
- Pacing is required, not optional. The server needs a pacing rate for each client; how it learns
  a safe rate on a real link is not yet worked out.
- The spike used localhost with an emulated link and one client. Before relying on this, test on
  real home connections and with many players sharing one upload link (Phase 2).
- If BBR misbehaves, falling back to Cubic with pacing is a configuration change.
