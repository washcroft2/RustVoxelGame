# 0004: Hosted CI for all three systems; no self-hosted runners

Status: approved by the project owner, 2026-11-01.

## Context

Sections 11 to 13 planned self-hosted runners on the Linux and Windows PCs to cover real-GPU
Vulkan and DX12 performance. That needs a Rust toolchain on the family PCs, which the project
owner ruled out: the other users are non-technical. The repository is public, so GitHub-hosted
runners for Linux, Windows and macOS are free. Phase 0 showed they build, test and run the
programs (`docs/phase0/hosted-runners.md`).

## Decision

Use GitHub-hosted runners for all three systems. Run real-GPU performance checks locally on the
Linux PC. Check real Windows and Mac hardware by having their owners run the test bundle at
phase gates. No family machine needs Rust installed.

## Consequences

- No automatic real-GPU performance regression check on CI. Performance regressions are caught
  locally, or at gates for Windows and Mac.
- Hosted runners are virtual (software or virtual GPUs), so their timings are only a sanity check.
- The repository must stay public, or hosted macOS minutes become a cost (see the plan's earlier
  pricing notes).
