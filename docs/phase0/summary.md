# Phase 0 summary

State at the end of the Linux-side work. Detail is in the per-spike files in this directory.
Everything below was measured on one Linux machine (see `machines.md`). Nothing has run on
Windows or macOS.

## Spikes

| Item (section 13) | Status | Headline result |
|---|---|---|
| `xtask` | Partly done | `cargo xtask verify` works. `perf`, `bundle`, `scenario`, `assets`, `model-sheet` do not exist yet |
| Binary greedy mesher | Done on Linux | Matches a naive reference exactly. 64 µs median on smooth terrain, 193 µs on rough; target is 100 µs (`mesher.md`) |
| quinn datagrams vs bulk | Done on Linux | Pacing bulk to half the link keeps datagrams near the latency floor. Cubic gets ~0.8 Mbit/s at 2% loss (`quinn.md`) |
| redb with chunk blobs | Done on Linux | Fast; autosave commits tens of ms. btrfs only, synthetic blobs (`redb.md`) |
| Circuit solver | Done on Linux | 100k components at 10% switching: ~1.1 ms vs 5 ms target. Whether the rules feel right is untested (`circuits.md`) |
| MDI + `first_instance` | Vulkan only | Correct and cheap on Vulkan. DX12 and Metal untested (`gpu.md`) |
| wgpu feature status | Vulkan only | All features present on the RTX 2060 Super. DX12 and Metal untested (`gpu.md`) |
| wasmtime on the MacBook | Linux half done | Host calls ~5 ns, epoch interrupt works to ~1 ms. Mac untested (`wasmtime.md`) |
| Linux and Windows runners, macOS builds, MacBook bundle | Not started | Needs your decisions; see below |
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
unconfirmed until the Windows and macOS runs are in.

## What I need from other people or machines

- **Windows PC:** run the prebuilt `spike-gpu` and the other spike programs; send back the output
  files. No installs for the user, if the build comes from CI.
- **MacBook owner:** open a bundle and send back its report. Someone still has to build that
  bundle on a Mac, which means hosted macOS CI.
- **A GitHub repository** (public is fine) so CI can build Windows and macOS packages.
- **Machine specs** for every machine that will play.
- **Your call on the findings above**, especially 1 and 2.

## Housekeeping notes

- `cargo xtask verify` now takes several minutes because the debug-mode property tests in the
  mesher and circuit spikes are slow (~45 s and ~65 s). They are throwaway; I have not changed
  them.
- Spike crates live under `spikes/` as workspace members. Delete or promote them at the gate.
