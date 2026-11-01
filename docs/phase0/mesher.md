# Phase 0 spike: binary greedy mesher

Code: `spikes/mesher` (throwaway). Benchmark: `cargo run --release -p spike-mesher`.
Tests: `cargo test --release -p spike-mesher`.

## What it is

- 32³ chunk with a 1-block border (34³ `u16` array, 0 = air). Cube-only and opaque.
- Per-column `u64` opacity masks on each axis; face masks from `col & !(col >> 1)` and
  `col & !(col << 1)`; per-depth planes; greedy merge with width along bit runs.
- Output is the plan's 12-byte quad (`word0`: x,y,z,w,h as 6 bits each plus 2 flag bits;
  `word1`: material 16 bits, AO 8 bits; `word2` unused), grouped by the six directions.
- Merge key is material + AO (4 corners x 2 bits). Light is not in the key here.
- The AO diagonal-flip flag is derived from the AO bits.

## Correctness

An independent naive mesher (`naive.rs`: one unit face per exposed side, AO read straight
from the 3D array) is compared with the greedy output expanded back to unit cells. The test
requires no overlaps, identical coverage, identical material and AO per cell, and a quad
count no larger than the naive one. It runs on fixed scenes and on 600 proptest cases
(random noise at 0–100% density with 1–6 materials, and random boxes).

I checked the test can fail by temporarily breaking the AO corner order and the material
merge rule; both made it fail. Both changes were reverted.

## Performance (AMD Ryzen 7 5700X, one thread, release build, 3000 runs each)

Median / p99 of the greedy mesh with the opacity masks already built.

| Case | Median | p99 | Quads | Unit faces (naive) | Coplanar T-junctions |
|---|---|---|---|---|---|
| terrain (smooth) | 64 µs | 68 µs | 2,477 | 2,719 | 31 |
| terrain (rough) | 193 µs | 207 µs | 6,821 | 6,918 | 116 |
| flat | 16 µs | 19 µs | 1 | 1,024 | 0 |
| boxes | 75 µs | 79 µs | 978 | 5,423 | 818 |
| solid / empty | 7 µs | 8 µs | 0 | 0 | 0 |
| noise 50%, 4 materials | 1.5 ms | 1.5 ms | 49,068 | 49,176 | 116 |
| checkerboard (worst) | 1.5 ms | 1.6 ms | 98,304 | 98,304 | 0 |

Building the opacity masks from scratch costs a further 40–50 µs. The plan assumes they are
maintained on write, so that is not in the figures above.

Disabling AO entirely took the rough terrain from 193 µs to 130 µs, so AO is about a third of
the time. Cost scales at roughly 15–30 ns per emitted quad.

## Reading

- The section 2 target is "remesh, 32³ cube-only chunk, ≤100 µs median per core". It does not
  say what chunk. The smooth surface chunk meets it (64 µs); my rough one does not (193 µs).
  Which is typical depends on real terrain, which doesn't exist until Phase 3. I did not
  change the target.
- The rough terrain emits about 6.8k quads, over four times the plan's 1.5k estimate for a
  surface chunk (section 5). Even the smooth one emits 2.5k. Either the estimate is
  optimistic or my generated terrain is too stepped; real worldgen output will settle it.
- Greedy merging helps little on stepped terrain (2,477 quads vs 2,719 unit faces) because
  AO in the key splits runs on every slope. Large flat or boxy areas merge well.
- Not tried: SIMD AO, a smaller key, skipping AO on faces that are fully lit or unoccluded.
  AO is the obvious place to look if the target is missed on real terrain.

## T-junctions

Greedy merging does produce them: the counts above are quad corners that fall strictly inside
a coplanar neighbour's edge. They are rare on smooth terrain and common on boxy shapes. They
cannot be avoided by the mesher. The plan's mitigation is a sub-pixel quad expansion in the
vertex shader plus a fog-matched clear colour. That is NOT tested here; it needs the GPU
spike, which should render these meshes and read back the image to look for pinholes. Edges
shared between perpendicular planes are also not counted.

## Not covered

Light in the merge key, non-cube blocks, separate opaque/cutout/translucent passes, chunk
borders supplied from real neighbours, multi-threaded throughput, other CPUs (only this
machine has been measured), and the Mac and Windows machines.
