# Phase 0 spike: circuit solver

Code: `spikes/circuit` (throwaway). Benchmark: `cargo run --release -p spike-circuit`.
Tests: `cargo test --release -p spike-circuit`.

## What was built

An event-driven solver on a 3D grid following section 8, with a slow reference solver to
check it against.

- **Nets.** Wire blocks are 6-connected and compile into nets by flood fill. Strength is
  0–15 and falls by 1 per block. Editing wire rebuilds only the nets it touches.
- **Components.** Sources (constant power to all adjacent wire), inverters, repeaters
  (delay 1–4 ticks) and lamps. Inverters and repeaters read the wire behind them and drive the
  wire in front. Outputs are binary (0 or 15). Gate outputs change only through a timing
  wheel, at least one tick later, so every feedback loop passes through a delay.
- **Tick.** Apply due output changes, solve only dirty nets (a multi-source flood in 15
  strength buckets), then re-read only the components next to wire cells that changed.
- **Reference solver** (`naive.rs`). Each tick it recomputes every wire strength from scratch
  by relaxing to a fixed point and re-reads every component. It shares no solving code with
  the real solver.

Gaps in section 8 that I filled with the simplest choice, which you may want to revisit:
6-connected wire (so stacked wire merges, and there are no up/down staircase rules);
binary component outputs; lamps react in the same tick as the wire; sources are levers only.

## Correctness evidence

- 11 tests pass. Golden circuits: strength decay and cutoff at 16 blocks, the stronger of two
  sources winning, a period-2 inverter clock, a 3-inverter ring, exact repeater delays 1–4, and
  cutting and healing a net.
- The real solver matches the reference tick for tick on fixed seeds and on 600 proptest cases
  (random circuits, with and without random wire placements, removals and lever toggles
  between ticks).
- A translated or quarter-turned copy of a random circuit behaves identically, including
  moves across 32-block boundaries (300 cases). This holds by construction here, since the
  solver has no chunk concept; real chunk-border behaviour is not modelled.
- I checked that the comparison can fail by temporarily injecting three bugs (gates ignoring
  input changes, wire removal not reporting the cleared cell, rebuilt nets not marked dirty).
  Each was caught by several tests, and each change was reverted.
- A guard test confirms the random circuits do something. Even with stamped-in clocks only
  about 29% of circuit-ticks change anything and 10 of 40 circuits are still active at the end,
  so the random comparison is real but not exhaustive. My first generator produced under 10%
  activity, which is why the guard exists.
- While writing tests I made two mistakes of my own: a wrong expected value in the
  stronger-source test (arithmetic error; the solver was right) and a reference-solver bug
  that panicked on wire blocks. Both were fixed before anything was committed.

## Performance (AMD Ryzen 7 5700X, one thread, release build)

**1. The section 2 target: 100k components, 10% switching per logic tick, ≤ 5 ms.**
10,000 units of 10 components; every tenth unit is a live clock rippling through eight
repeaters to a lamp. Exactly 10,000 components (10.0%) switch every tick. About 9,000 nets are
solved and 13,000 wire cells change per tick.

| Step time | Median | p99 | Max |
|---|---|---|---|
| 100k components, 10% switching, run 1 | 1.07 ms | 1.9 ms | 3.3 ms |
| same, run 2 | 1.08 ms | 1.3 ms | 1.4 ms |

Resident memory for that world was about 56 MB. The median is stable; the tail varies between runs. Result: the target is met with room to
spare on this machine, on this workload.

**2. One huge net (not in section 2).** A single 100,000-cell wire with 10,000 sources and 100
toggled per tick: median 1.04 ms, max 2.0 ms. Cost follows the lit area of the dirty net, about
10 ns per cell, so many such nets dirty in the same tick would add up.

**3. Editing wire costs time proportional to the whole net it touches**, even for a trivial
edit such as extending the end:

| Net size | Extend end | Remove end | Cut middle | Heal middle |
|---|---|---|---|---|
| 100 | 2.4 µs | 2.4 µs | 2.6 µs | 2.4 µs |
| 1,000 | 18 µs | 35 µs | 19 µs | 20 µs |
| 10,000 | 170 µs | 174 µs | 190 µs | 184 µs |
| 100,000 | 1.9 ms | 1.9 ms | 1.7 ms | 1.7 ms |

This is the simple "dissolve and relabel" approach. It is fine for ordinary builds. Very
large shared nets would feel it if many edits land in one tick. Known improvements, not
built: merge the smaller net into the larger on placement, and search from both sides on a
cut so the cost follows the smaller side.

## Not covered, and what this does not show

- **Whether the rules feel right to players.** The plan's risk is that the circuit rules feel
  wrong. This spike only shows the rules can be implemented, are deterministic, and are fast.
  Judging feel needs a person to build things with it, which belongs with the Phase 6 gate.
- The workload is synthetic and made of small nets. Real builds will differ.
- Single thread only. Nets are independent within a tick so parallel solving should work, but
  it wasn't needed and wasn't tried.
- Not modelled: solid blocks carrying power, comparators, observers, pistons, hoppers,
  signal strengths other than 0 and 15 from components, chunk loading and unloading, light
  updates from lamps, and replication to clients.
- Memory is about 13 bytes per grid cell plus component and net tables; a 4096-high world
  would need a sparse or per-chunk layout rather than this dense array.
- Linux and this CPU only.
