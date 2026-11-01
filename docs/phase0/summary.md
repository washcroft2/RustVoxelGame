# Phase 0 summary

State at the end of the Linux-side work. Detail is in the per-spike files in this directory.
Most numbers were measured on one Linux machine (see `machines.md`). Windows and macOS have
only run on GitHub's hosted virtual machines (`hosted-runners.md`), not on family hardware.

## Spikes

| Item (section 13) | Status | Headline result |
|---|---|---|
| `xtask` | Partly done | `cargo xtask verify` works. `perf`, `bundle`, `scenario`, `assets`, `model-sheet` do not exist yet |
| Binary greedy mesher | Done on Linux | Matches a naive reference exactly. 64 µs median on smooth terrain, 193 µs on rough; target is 100 µs (`mesher.md`) |
| quinn datagrams vs bulk | Done on Linux | Pacing bulk to half the link keeps datagrams near the latency floor. Cubic gets ~0.8 Mbit/s at 2% loss (`quinn.md`) |
| redb with chunk blobs | Done on Linux | Fast; autosave commits tens of ms. btrfs only, synthetic blobs (`redb.md`) |
| Circuit solver | Done on Linux | 100k components at 10% switching: ~1.1 ms vs 5 ms target. Whether the rules feel right is untested (`circuits.md`) |
| MDI + `first_instance` | Real GPU: Vulkan only | Correct on Vulkan (real GPU), Metal (VM) and DX12 (software). Metal MDI costs ~1.5x direct draws to record (`gpu.md`, `hosted-runners.md`) |
| wgpu feature status | Real GPU: Vulkan only | RTX 2060 Super has everything. Metal VM lacks count draws and timestamps, as the plan expected (`hosted-runners.md`) |
| wasmtime on the MacBook | Works on Apple silicon VM | Host calls 2–5 ns, JIT and epoch interrupt work on Linux, Windows and an M1 VM. First-launch and self-update on the real MacBook untested (`wasmtime.md`, `hosted-runners.md`) |
| CI, macOS builds, test bundles | Done, untested on people's machines | GitHub Actions builds and tests all three OSes; bundles for each run end to end on hosted machines. Self-hosted runners dropped (decision 0004) (`hosted-runners.md`) |
| Machine specs | One of N | `machines.md` |

## Findings that touch the plan

These are for you to weigh at the gate. I have not changed the plan, targets or baselines.

1. **Bulk streaming at the Phase 2 gate's 2% loss is slow with quinn's default congestion
   control** (about 0.8 Mbit/s at 150 ms RTT). Join time and the chunk-burst design depend on
   this. BBR avoids it but quinn documents its BBR as experimental. Pacing works either way.
2. **The remesh target (≤100 µs) doesn't say which chunk.** It is met on a smooth surface chunk and
   missed on a rough one. Section 5's estimate of ~1.5k quads per surface chunk looks low next
   to 2.5k–6.8k in my synthetic terrain; real worldgen will settle it.
3. **T-junction pinholes happen and the fix needs care.** A fixed corner expansion that works at
   a distance will show at close range; it likely needs to scale with distance.
4. **Wire edits cost time proportional to the whole net** (1.9 ms at 100k cells). Fine for
   ordinary builds; a known weakness for very large shared nets.
5. **The circuit model has gaps the plan doesn't decide:** wire connectivity (I used 6-connected),
   binary component outputs, and stacked wire merging.
6. **Compression and wire-size targets can't be judged yet**; they need real terrain.

None of these contradicts a decision in section 1 so far, but the section 1 decisions are
unconfirmed until runs on real Windows and Mac hardware are in.

## Decisions made at the gate

Recorded in `docs/decisions/`; the plan is edited to match.

- **0001** Use BBR congestion control in quinn, with server-side pacing.
- **0002** The 100 µs remesh target applies to a typical surface chunk; number unchanged, to be
  re-measured on real terrain in Phase 1.
- **0003** Vertical wire connectivity is an open question until Phase 6.
- **0004** Hosted CI for all three systems; no self-hosted runners.
- **Not recorded as decisions:** quads per surface chunk (note only), and the T-junction fix
  (enlarge quads by about one pixel, as the reference mesher does; depth hole-filling stays an
  option for later and is independent of the rest of the game).

Still open, deliberately carried: real DX12 and Metal hardware, first launch of unsigned
downloads, self-update, the remaining machine specs, `xtask perf` and its baselines (Phase 1),
and the dependency-rule and fuzz steps of `verify` (Phase 1).

## What I need from other people or machines

- **Windows PC:** download the Windows bundle from the latest CI run, unzip it, double-click
  `run.bat`, and send back `voxel-report.txt`. No installs.
- **MacBook owner:** download the macOS bundle, unzip it, right-click `run.command` and choose
  Open, and send back `voxel-report.txt`. How macOS reacts is part of what is being tested.
- **Machine specs** for every machine that will play.
- **Your call on the findings above**, especially 1 and 2.

## Housekeeping notes

- `cargo xtask verify` now takes several minutes because the debug-mode property tests in the
  mesher and circuit spikes are slow (~45 s and ~65 s). They are throwaway; I have not changed
  them.
- Spike crates live under `spikes/` as workspace members. Delete or promote them at the gate.
- CI uses `actions/checkout@v4`, `cache@v4` and `upload-artifact@v4`, which GitHub says target a
  deprecated Node version and will need bumping.
