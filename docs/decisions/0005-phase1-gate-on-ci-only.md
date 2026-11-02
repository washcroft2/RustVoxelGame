# 0005: Phase 1 gate closed on CI and the Linux PC; real-hardware runs skipped

Status: approved by the project owner, 2026-11-02.

## Context

The Phase 1 gate (plan section 13) asks for a 16-chunk radius at the target frame rate on
machines A and B, remesh within target, and the same behaviour on all three systems. Checking
that on the Windows PC and the MacBook means getting other people to run a test bundle, which
the project owner finds troublesome. The same choice was made at the Phase 0 gate.

## Decision

Close Phase 1 on the evidence available without family machines:

- CI on hosted Linux, Windows and macOS runners, where the render tests must find a graphics
  adapter and draw (software or virtual GPUs; `VOXEL_REQUIRE_GPU`).
- Performance on the Linux PC (RTX 2060 Super), through `cargo xtask perf`.

The Windows PC and MacBook gate runs are not done for Phase 1.

## Consequences

- Frame rate on the Windows PC and on the MacBook (machine B) is unmeasured. Real DX12 and Metal
  drivers have not run the renderer. Problems specific to them could surface late, when the game
  first reaches those machines.
- The remesh figure (48-53 us) is from generated hills and still needs re-measuring on Phase 3
  worldgen.
- This applies to Phase 1. It is not assumed for later gates; each gate asks again, and the
  family playtests at the Phase 2, 4, 5, 6 and 7 gates are a different kind of check.
- Still open and not part of this decision: how to measure the frame-time spread target
  (`docs/phase1/status.md`, question 1).
