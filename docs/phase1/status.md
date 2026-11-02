# Phase 1 status

Where Phase 1 ("Core and local rendering", plan section 13) stands. Numbers were measured on one
machine (Ryzen 7 5700X, RTX 2060 Super, Vulkan, Linux; see `docs/phase0/machines.md`) unless
stated. Nothing here has run on real Windows or macOS hardware.

## Items

| Plan item | State |
|---|---|
| Workspace and CI on all three platforms | Done. Hosted runners build and test all three. |
| `core`, `world`, `content`, `mesh`, basic `render` | Done (crates under `crates/`). |
| Base content from `packs/base`, namespaced IDs | Done. Air is built into the engine as `base:air`. Textures are generated flat-colour placeholders. |
| Dimension IDs in chunk keys | Done (`ChunkKey`). |
| Test terrain | Done: flat and gradient-noise hills (`worldgen`). |
| Free camera | Done (`apps/game`). Keyboard and mouse handling was written but not exercised by hand. |
| F3 overlay | Done. egui for display only, in the pack's CC0 pixel font. |
| Tracy integration | Done for CPU spans (`--features profile`). GPU timestamps are not done, see below. |
| Staging ring | Done, using wgpu's `StagingBelt`. |
| Slab allocator | Done (`offset-allocator`, with frees held until the GPU is finished). |
| Tier 1 frustum culling | Done on the CPU. The plan says SIMD over SoA bounds; this is plain scalar code over a dense array, which is far inside budget (see below). |
| Texture arrays | Done, with CPU-built mipmaps. |
| `cargo xtask verify` dependency rules | Done. |
| `cargo xtask perf` | Done, against the section 2 targets. No regression baselines yet (they need the owner's approval). |

## Measurements

Release build, radius 16, generated hills, seed 1.

| What | Result | Target (section 2) |
|---|---|---|
| Load everything in range | 0.33 s, 4,036 chunks, 0.54 M quads | none |
| Frame time, 2560x1440, flythrough at 40 blocks/s | mean 0.64 ms (1,571 fps), median 0.40, p99 2.8, max 4.1 | 144 fps or better |
| Remesh, surface chunk, one core | median 48-53 us, p90 83-87, p99 92-119 | 100 us median |
| Chunks drawn / indirect draws per frame | about 650 / about 1,000 | none |
| Upload backlog during the flight | none (peak 0 waiting) | none |

Remesh over three seeds and the flat world gave medians of 48-50 us. A one-quad chunk costs about
50 us, so most of the cost is a fixed one (gathering the padded columns), not the quads. This
terrain is smooth: the median chunk has about 107 quads, against the plan's estimate of about
1,500 for real terrain. The figure has to be measured again once Phase 3 worldgen has caves, ores
and trees.

GPU memory at radius 16 is about 88 MB (quad pool 7 MB).

## Open questions for the owner

1. **Frame time spread.** The target "p99 at most 1.5x the median" is not enforced by
   `cargo xtask perf` yet. Uncapped on this GPU a frame takes 0.4 ms, so a 2.8 ms hitch is a 7x
   ratio while using under half of a 144 Hz frame (6.9 ms). The ratio is not meaningful at that
   speed. Options: measure with the frame rate capped at the target (so frames are paced, and
   the question becomes whether work ever exceeds the budget), or keep the ratio and measure on
   a slower GPU where frames are longer. The target stays as written until you decide.
2. **Staging ring.** Built from wgpu's `StagingBelt` rather than hand-written. It is a ring of
   reusable mapped buffers written directly and recycled when the GPU is done, which is what the
   plan describes. Say so if you wanted a hand-written one.
3. **GPU timing in Tracy.** `wgpu-profiler` 0.28's Tracy support needs `tracy-client` 0.18, but
   `tracing-tracy` needs 0.19, and the two cannot be linked together. GPU pass timings in
   profile builds would need either a newer `wgpu-profiler`, or me feeding its results to Tracy
   by hand. Not needed for the gate.

## Not done, and why

- **Real hardware.** No Windows PC, MacBook or weakest-machine runs. CI renders the same tests on
  Windows software rendering (WARP) and a virtual Mac GPU, and on Linux with lavapipe, but those
  say nothing about speed or about real drivers.
- **Phase 1 gate on machines A and B.** Needs the family machines; see above.
- **Non-cube blocks, the overlay mesh for fast-changing blocks, cave-culling bits, light in
  quads, translucent sorting within a chunk.** Not in the Phase 1 list; later phases.
- **Texture orientation and mip bleeding** have only been seen on flat-colour textures. Check
  them when real art arrives.

## Things to know

- `mesh` and `worldgen` depend on `content`, which also holds the code that reads pack files.
  The dependency rules pass because they look at crates, not code; splitting the registry from
  the file loading would make the pure crates cleaner.
- The client generates its own terrain for now. Phase 2 moves that to the embedded server.
