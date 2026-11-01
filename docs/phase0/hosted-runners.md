# Phase 0: results from GitHub's hosted runners

CI (`.github/workflows/ci.yml`) builds the test bundle for Linux, Windows and macOS, and a manual
job unzips each bundle and runs its script the way a person would. That gave the first
Windows and macOS results. **These are shared virtual machines, not family hardware.** Numbers
are noisy and say nothing about real GPUs; treat them as a sanity check that the code and
scripts work on those systems. Raw reports: `reports/`.

## The machines

| | Windows runner | macOS runner |
|---|---|---|
| OS | Windows Server 2025 | macOS 26.6.2 |
| CPU | Xeon Platinum 8370C, 2 cores / 4 threads | Apple M1 (virtual) |
| RAM | 16 GB | 7.5 GB |
| GPU wgpu sees | Microsoft Basic Render Driver (DX12, **software**) | Apple Paravirtual device (Metal) |

## What it established

- **Everything builds and the tests pass** on both: format, clippy with warnings denied, and all
  tests, on Windows and macOS. This closes the plan's open item that the crates "compile on
  Windows and macOS".
- **The macOS programs are universal** (Intel + Apple silicon), ad-hoc signed, and run natively on
  Apple silicon when launched from a script.
- **Vertex pulling works on all three graphics APIs.** `first_vertex` and `first_instance`
  produced exactly the expected pixels through direct, indirect and multi-draw-indirect calls
  on Vulkan (real NVIDIA GPU), Metal (virtual) and DX12 (software), 0 wrong pixels each.
- **A real DX12 bug was found and fixed.** The terrain shader wrote to a vector component through
  a runtime index, which DX12's shader compiler rejects. Now rewritten; Vulkan output unchanged.
- **wasmtime works on Apple silicon and Windows**, including its JIT, in these binaries.
- **Pinholes behave the same on all three** (30 / 26 / 30 pixels with no expansion, 0 at 0.01
  block), so the rasteriser difference worry is smaller than feared, though this is software or
  virtual rendering.
- **The test scripts work**, after one fix each. Windows: an exit-code line was silently lost
  because of a batch quirk (`0>>` is a handle redirect). macOS: the closing "press Enter" prompt
  returned an error in the automated run.

## Feature support reported

| | Vulkan (RTX 2060 Super) | Metal (VM) | DX12 (software) |
|---|---|---|---|
| `INDIRECT_FIRST_INSTANCE` | yes | yes | yes |
| `MULTI_DRAW_INDIRECT_COUNT` | yes | **no** | yes |
| `TEXTURE_BINDING_ARRAY` | yes | yes | yes |
| `TIMESTAMP_QUERY` | yes | **no** | yes |
| downlevel `INDIRECT_EXECUTION` | yes | yes | yes |

The Metal gaps match the plan (no count draws; pass-boundary timestamps only), though a VM's
virtual GPU may support less than the real one.

## Draw call recording cost, 10,000 draws (CPU time to record, ms)

| Mode | Vulkan, real GPU | Metal VM | DX12 software |
|---|---|---|---|
| 10,000 direct draws | 0.93 | 0.62–0.78 | 1.36 |
| 10,000 `draw_indirect` | 2.2 | 1.4–2.1 | 2.5 |
| 1 `multi_draw_indirect` | **0.27** | **0.92–1.30** | **0.44** |

On Vulkan and DX12 the multi-draw is the cheapest way to issue 10,000 draws. On Metal it is
cheaper than a loop of `draw_indirect` but about 1.5 times the cost of plain direct draws, which
fits the plan's expectation that Metal emulates it. At roughly 1 ms for 10,000 draws it is
affordable (a 60 fps frame is 16.7 ms), but this is a virtual M1, so **the real MacBook still has
to confirm it.** GPU execution time is not meaningful here (software and virtual).

## Other programs on the hosted machines (shared, noisy)

| Measure | Linux, Ryzen 7 5700X | Windows runner | macOS M1 VM |
|---|---|---|---|
| Mesher, smooth terrain | 62 µs | 55 µs | 40 µs |
| Mesher, rough terrain | 190 µs | 208 µs | 155 µs |
| Circuits: 100k components, 10% switching, median / p99 | 1.06 / 1.7 ms | 1.2 / 3.2 ms | 1.6 / 10.6 ms |
| wasmtime guest-to-host call | 4.8 ns | 3.1 ns | 2.3 ns |
| wasmtime epoch interrupt (1 ms tick), median | ~1.0 ms | 1.5 ms | 5.4 ms |
| redb bulk write, chunks/s (noisy profile) | 46k | 59k | 27k |
| redb autosave commit, median / p99 (noisy profile) | 21 / 104 ms | 29 / 48 ms | 37 / 129 ms |

- The mesher and circuit targets hold on these slower shared machines, which is mildly
  reassuring for the weakest-machine case.
- The macOS VM's tails are poor (circuit max 28 ms, epoch interrupt up to 7.5 ms). That is
  probably virtualisation noise, not Apple silicon, but only the real MacBook can say.
- The quinn conclusions held on both: Cubic about 0.8 Mbit/s at 2% loss, BBR 13–15 Mbit/s, and
  pacing to 50% keeps datagrams near the latency floor. Datagram p99 was noisier on the VMs.

## Still not established

- **Real hardware on Windows and Mac:** DX12 on an actual GPU, Metal on the MacBook, real
  performance of everything above.
- **First-launch behaviour for unsigned downloads:** SmartScreen on Windows, Gatekeeper on macOS.
  CI runs the programs from a script, so none of that was exercised. The instruction sheets in the
  bundles are untested on a person's machine.
- **Self-update** (`self-replace`) prompts.
- The Linux runner has no GPU, so no GPU test ran there.
