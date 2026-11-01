# Phase 0 spike: GPU indirect draws, vertex pulling and T-junction pinholes

Code: `spikes/gpu` (throwaway). Run: `cargo run --release -p spike-gpu`. It prints a report and
writes it to `gpu-report.txt` in the current directory, so the same program can be built once
and given to the Windows and macOS owners (see "Still to do"). wgpu 30.0.1 with the `vulkan`,
`metal` and `dx12` backends compiled in, glam 0.34.2, bytemuck, pollster.

**Only Vulkan on one NVIDIA GPU was tested.** Nothing here says anything about DX12 or Metal.

## Adapter (this machine)

NVIDIA GeForce RTX 2060 SUPER, Vulkan, driver 615.78.08. Reported by wgpu: `INDIRECT_FIRST_INSTANCE`,
`MULTI_DRAW_INDIRECT_COUNT`, `TEXTURE_BINDING_ARRAY`, `TIMESTAMP_QUERY` and downlevel
`INDIRECT_EXECUTION` all supported. `max_storage_buffer_binding_size` 2047 MiB, `max_buffer_size`
4095 MiB, 2048 texture array layers. The only adapter wgpu enumerated was the Vulkan one.

## Vertex pulling with first_vertex and first_instance

Each chunk is one draw of 16 quads (96 vertices). `first_vertex` = 6 x the chunk's first quad (the
quad bases are shuffled so a wrong base shows up), and `first_instance` = the chunk's index into a
storage buffer of chunk records. The vertex shader reads quads with `vertex_index / 6` and the
record with `instance_index`. 10,000 chunks fill a 1600x1600 image, compared pixel for pixel with
a CPU rendering of the same data.

| Mode | CPU record time per frame | Submit and wait | Wrong pixels |
|---|---|---|---|
| 10,000 x `draw(first_vertex, first_instance)` | 0.93 ms | 0.45 ms | 0 |
| 10,000 x `draw_indirect` | 2.23 ms | 0.42 ms | 0 |
| 1 x `multi_draw_indirect(10,000)` | 0.27 ms | 0.21 ms | 0 |

Medians of 100 frames. Reading:
- On Vulkan with this driver, `vertex_index` includes `first_vertex` and `instance_index`
  includes the indirect `first_instance`, in all three modes. This is the behaviour section 5
  needs; the DX12 difference the plan worries about is exactly what this program checks there.
- `multi_draw_indirect` is much cheaper to record than a loop of `draw_indirect` (about 8x here),
  and direct draws are in between. So the plan's choice of MDI is right on Vulkan.
- Whether Metal's emulated MDI is as cheap is the open question, and is the main thing the
  MacBook run needs to answer.
- The scene has little geometry (160k quads), so these times are mostly per-draw overhead, which
  is the point of the test. They say nothing about shading or fill cost.

## T-junction pinholes

The mesher spike's terrain meshes are drawn in 3D (all six face directions, perspective camera,
depth test, magenta clear colour) from 40 pseudo-random poses looking nearly straight down at the
middle of the chunk, so every pixel should land on terrain. A magenta pixel is a pinhole. The
vertex shader can push every quad corner outward by `eps` blocks along both tangents.

Pinhole pixels over 40 poses at 512x512 (10.5 million pixels), then the number of poses affected:

| Scene | Quads | T-junctions | eps 0 | 0.0001 | 0.0005 | 0.002 | 0.01 |
|---|---|---|---|---|---|---|---|
| terrain (smooth) | 2,477 | 31 | 30 / 22 | 1 / 1 | 2 / 2 | 1 / 1 | 0 / 0 |
| terrain (rough) | 6,821 | 116 | 5 / 5 | 10 / 8 | 3 / 3 | 0 / 0 | 0 / 0 |

Reading:
- Pinholes do occur with no expansion (22 of 40 poses on the smooth terrain), but they are
  tiny: 30 pixels in 10.5 million.
- A small expansion does not remove them cleanly. 0.0001 left some on the rough terrain and was
  no better than nothing there. At 0.01 block, none appeared in this sample. This is one GPU, one
  set of poses, and a near-vertical view; I would not call 0.01 a safe value from this alone.
- A fixed world-space expansion is the wrong shape. 0.01 block is invisible at a distance but
  will be several pixels when a block fills the screen, where it makes the neighbouring
  coplanar quad poke over the material boundary. The expansion probably needs to scale with
  distance so it stays a fraction of a pixel. That was not built or tested.
- Pinholes depend on the rasteriser, so the same test must run on DX12 and Metal.

## Still to do

- Run the program on the Windows PC (DX12 or Vulkan) and the MacBook (Metal) and read their
  `gpu-report.txt`: first_vertex and first_instance correctness, the three draw modes' costs
  (especially Metal's MDI), which features are supported, and the pinhole counts.
- A distance-scaled expansion and a close-up test for material-boundary bleed.
- GPU-side timing. Only CPU record and submit-plus-wait wall time were measured.
- Texture arrays, large slabs, staging-ring uploads and the full frame structure belong to Phase 1.
