# AGENTS.md

A private Minecraft-like voxel game in Rust, for family play across Linux, Windows and macOS.

The design lives in `voxel-engine-plan.md`. Read the sections relevant to your task before starting. Section 12 covers how agents work here, and section 13 is the roadmap with phase gates.

## Checks

Run before every commit:

```
cargo xtask verify
```

Until `xtask` exists:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Also run `cargo xtask perf` when changing hot paths. Report what you actually ran. Metal and DX12 can't be validated from Linux, so say so rather than implying they were tested.

## Git

- Commit directly to the default branch, only when checks pass.
- Keep commits small and focused, with an imperative subject line.
- Never force-push or rewrite history.
- Parallel agents use separate worktrees and rebase onto the default branch before landing.

## Decisions

Make local implementation choices yourself and note any notable ones in the commit message. Stop and ask before:

- Changing the plan or the architecture rules below
- Changing a performance target or test baseline
- Changing the wire format or save format
- Changing gameplay or game design
- Adding a dependency not listed below
- Disabling, weakening or deleting a test or check to get a pass

Record approved significant decisions in `docs/decisions/`.

## Approved dependencies

- **Runtime:** `wgpu`, `winit`, `gilrs`, `quinn`, `rustls`, `tokio`, `rayon`, `glam`, `hecs`, `redb`, `zstd`, `mimalloc`, `bumpalo`, `wasmtime`, `kira`, `egui`, `egui-wgpu`, `egui-winit`, `cosmic-text`, `postcard`, `bytes`, `ed25519-dalek`, `rcgen`, `rand`, `offset-allocator`, `bytemuck`, `pollster`, `serde`, `serde_json`, `toml`, `image`, `blake3`, `ureq`, `zip`, `self-replace`, `rustc-hash`, `crossbeam-channel`, `wide`, `clap`, `directories`, `thiserror`, `anyhow`, `tracing`, `tracing-subscriber`
- **`profile` builds only:** `tracy-client`, `tracing-tracy`, `wgpu-profiler`
- **Tests and tools:** `proptest`, `criterion`, `gungraun`, `cargo-fuzz`, `cargo-deny`, `samply`

Keep default features off for these, because the defaults pull in a vulnerable older `zstd`, an extra crypto provider or unused codecs:

- `wasmtime`: enable only what's needed, and never `cache`
- `quinn` and `rustls`: use the `ring` provider
- `image`: `png` only
- `zip`: `deflate` only

Section 16 of the plan lists the tested versions and the two accepted `cargo-deny` notices. If an approved crate turns out to be unmaintained or unsuitable, ask rather than swapping it.

## Architecture rules

- Client and server are always separate. Single-player uses the embedded server, with no separate code path.
- The server is authoritative. Clients only predict movement and block edits.
- `server` never depends on `render`, `audio` or `winit`.
- `client` never depends on `worldgen`, and the seed never leaves the server.
- Only `render` imports `wgpu`.
- `mesh`, `light`, `proto` and `worldgen` stay pure, with no GPU or I/O.
- Tokio stays on net threads. Simulation code is not async.
- Worker threads use chunk snapshots rather than locks in hot paths.
- Block updates are two-phase, and circuits go through the solver in section 8.
- Game content is data in packs. Don't hard-code blocks, items or mobs in Rust.
- Scripts never run inside render, meshing, solver or block-update loops.

## Code

- Stable Rust, edition 2024, no nightly features.
- `#![forbid(unsafe_code)]` by default. `unsafe` only in small audited modules, with a `// SAFETY:` comment and Miri coverage where possible.
- Use `thiserror` for errors in library crates and `anyhow` only in `apps/` and `xtask`.
- Never panic on network, file or mod input. Only `unwrap` where a comment or the code makes the invariant clear.
- Validate lengths before allocating when decoding.
- Use `tracing` for logging and profiling spans, never `println!` in library crates. Spans in hot loops must compile out outside `profile` builds.
- Avoid allocation in per-frame and per-tick paths. Reuse buffers or use the worker arenas.
- Doc comments go on public items. Inline comments explain why (invariants, performance tricks), not what.
- Dependency versions are pinned. Check the local `cargo doc` output for the pinned version instead of relying on memory, especially for `wgpu`.

## Failures

Keep each failure as small as possible, following the table in section 11 of the plan:

- Bad client data disconnects that client and never stops the server.
- Corrupt chunks are backed up and never silently overwritten.
- Failed saves keep the last good save and warn players.
- GPU device loss rebuilds the renderer without losing game state.
- Mod errors disable that mod.
- Release builds use `panic = "abort"`. Don't use `catch_unwind` to continue after a bug, and don't attempt an emergency save.

## Tests

- New behaviour gets tests in the same commit.
- Where the plan names an independent reference (naive mesher, light recompute, slow circuit solver), test against it.
- Fix the code when a test fails. If you believe the test is wrong, ask.

## Scope

Do the task you were given. Note other problems you find, without fixing them or adding unrequested features.
