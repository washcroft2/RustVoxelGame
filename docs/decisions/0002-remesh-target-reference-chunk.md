# 0002: The remesh target applies to a typical surface chunk

Status: approved by the project owner, 2026-11-01.

## Context

Section 2 set "Remesh, 32³ cube-only chunk: ≤100 µs median per core" without saying which chunk.
The Phase 0 mesher spike (`docs/phase0/mesher.md`) took 64 µs on smooth hills and 193 µs on
rough, bumpy terrain. Light is not yet in the merge key, so real chunks will cost more.

## Decision

Keep the 100 µs number. Define it as a typical surface chunk, cubes only. Re-measure on real
terrain in Phase 1, when the gate checks "remesh within target". The number is not relaxed.

## Consequences

- The Phase 1 gate needs a stated reference chunk, taken from real worldgen output.
- If real chunks miss the target, the next step is a decision about the number or the mesher;
  ambient occlusion in the merge key is the first place to look (about a third of the time).
