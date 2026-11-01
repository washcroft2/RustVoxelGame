# Phase 0 spike: wasmtime

Code: `spikes/wasmtime` (throwaway). Run: `cargo run --release -p spike-wasmtime`.
Needs the `wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`).

## Build

wasmtime 50.0.1 with `default-features = false, features = ["runtime", "cranelift", "std"]`.
Builds on Linux; `cargo deny check` is clean (one duplicate `hashbrown` warning); no `zstd`
in the tree. Windows and macOS builds are not yet tested.

## Linux results (x86_64, RTX 2060 Super box, release build, median of 15)

| Measure | Epoch off | Epoch on |
|---|---|---|
| Compile, 743-byte module | 2.0 ms | 2.0 ms |
| Instantiate | 17 µs | 17 µs |
| Guest to host call | 4.3 ns | 4.7 ns |
| Host to guest call | 16.6 ns | 16.6 ns |
| 200M-iteration pure guest loop | 133 ms | 229 ms |

Epoch interruption with a 1 ms ticker and a 1-tick deadline stopped a `loop {}` guest in
1.02 / 1.04 / 1.09 ms (min / median / max over 30 runs), always as `Trap::Interrupt`.

Reading:
- Crossing the boundary is cheap. The host function here is trivial, so real calls that read
  world state will cost more; this is the floor, not the budget.
- Epoch checks add about 70% to a worst-case tight loop with no calls. Real scripts do less
  pure looping, so treat this as an upper bound and re-measure with a realistic script.
- Interruption latency is one tick plus scheduling noise, so the tick period sets the bound.
- The compile time is for a trivial module. Real mods need their own measurement.

## Still to do (needs the MacBook, and Windows for the DX12 box)

- Same benchmark on macOS (Apple silicon or Intel) and Windows.
- Whether Cranelift JIT runs in an ad-hoc-signed `.app` without extra entitlements.
- First-launch behaviour for the unsigned build, and whether a `self-replace` update
  triggers repeated prompts.

These go in the `cargo xtask bundle` report once that command exists.
