# Voxel Engine Implementation Plan

A Minecraft-like voxel engine in Rust: authoritative multiplayer, native on Linux, Windows and macOS.

## 1. Summary of key decisions

| Area | Decision | Main reason |
|---|---|---|
| Graphics | `wgpu` (Vulkan, DX12, Metal), custom renderer | One codebase across three platforms, with a HAL escape hatch |
| Engine base | Custom, not Bevy or another engine | The chunk store, scheduler and renderer are the product; a general engine would be in the way |
| Architecture | Client and server always separate; single-player runs an embedded server | One code path, so multiplayer can't rot |
| Transport | QUIC via `quinn` | Prioritised streams plus unreliable datagrams, encryption and congestion control included |
| Authority | Server-authoritative, with client prediction for movement and block edits | Cheat resistance and responsive feel |
| Chunks | Cubic 32³, palette-compressed | Power-of-two addressing; a good fit for bitmask meshing |
| World height | Large but bounded, configurable (default 4096 blocks) | Unbounded height greatly complicates skylight and generation for little gameplay value |
| Meshing | Binary greedy meshing, 12-byte quads, vertex pulling | Fast rebuilds and small GPU footprint |
| Far terrain | Server-built LOD tiles, clipmap on the client | View distances of several km; the client never needs the world seed |
| Persistence | `redb` transactions behind a `ChunkStore` trait | World-consistent saves with no custom crash recovery |
| Concurrency | Immutable chunk snapshots (`Arc` copy-on-write), two-phase block updates | Parallelism without locks in hot paths |
| Modding | Data packs, plus sandboxed WASM scripts on server and client | Server-sent mods are safe to run, and scripts stay out of hot loops |
| Circuits | Dedicated event-driven signal solver | Instant wires, no direction or position quirks, large builds stay cheap |
| Distribution | Unsigned private builds with a built-in updater | Family use needs no paid signing, but everyone must stay on one version |

**Project parameters:**

- One Minecraft-like game on a grid of cubic voxels, played privately with family. The engine serves this game only, so it has no general-purpose engine API to keep stable.
- Minecraft's core gameplay loop, expanded rather than replaced.
- Up to 30 players per world, with family-sized groups as the normal case.
- Bounded world height.
- Client mods are wanted, but they are the lowest-priority feature (section 10).
- AI agents write the code, directed by a human (section 12).
- Clean-room: no interoperability with Minecraft's protocol, mods or assets.
- Desktop only, with no browser, mobile or console targets.

### Game scope

Behaviour follows Minecraft closely, except for circuits, which keep the depth but drop the quirks (section 8). Each group is playable before the next one starts.

| Group | Contents | Phase |
|---|---|---|
| Survival core | Mining, ores, tools, combat, armour, crafting, smelting, inventory, chests, doors, torches, health, hunger, day/night, beds, farming, passive and hostile mobs, creative mode | 5 |
| Circuits | Wire, power sources, sensors, inverters, repeaters, comparators, observers, pistons, lamps, powered doors, hoppers, dispensers, rails and minecarts | 6 |
| World depth | Biomes, structures, villages and trading, breeding, enchanting, potions, weather, boats, maps, extra dimensions, bosses | 7 |
| Expansion | New systems and content that set this game apart | 9 |

Expansion depends on this structure: new blocks, items, mobs, recipes, biomes and circuit components should be data wherever possible, using the same pack path as the base game.

## 2. Targets

These are design goals to enforce in CI and playtests, not measurements. Revisit them after the Phase 0 spikes.

**Reference hardware**

The real machines that will play the game replace these placeholder specs in Phase 0.

- **A (main PCs):** the Linux and Windows PCs; assumed 8-core CPU and an RTX 3060-class GPU, 16 GB RAM
- **B (Apple):** the family MacBook, model not yet known; assumed Apple silicon at base M1 level or better, 16 GB
- **C (minimum):** the weakest machine the family will use; assumed 4-core laptop CPU, integrated GPU with Vulkan 1.2 or DX12

| Metric | Target |
|---|---|
| Frame rate, A | ≥144 fps at 1440p, 32-chunk radius (1024 blocks) plus LOD |
| Frame rate, B | ≥60 fps at native 1080p-equivalent, 24-chunk radius |
| Frame rate, C | ≥60 fps at 1080p, 12-chunk radius |
| Frame time p99 | ≤1.5× the median; no chunk-streaming hitches |
| Remesh, typical surface chunk, cubes only (re-measure on real terrain in Phase 1) | ≤100 µs median per core |
| Block edit to visible (local) | ≤2 frames |
| Terrain generation | ≥2,000 chunks/s on 8 cores |
| Client memory | ≤4 GB RAM, ≤2 GB VRAM at the A settings |
| Dedicated server | 30 players on 6 cores, tick p99 under budget (33 ms) |
| Hosting from the game | 8 players on hardware A while still meeting the host's frame target |
| Server upload | ≤10 Mbit/s with 30 players once initial chunk bursts finish, so home hosting is viable |
| Steady-state bandwidth | ≤100 kbit/s per player with 50 nearby entities, excluding chunks |
| Chunk wire size | ≤2 KB median for natural terrain |
| Join to playable | ≤3 s on a 50 ms link, 8-chunk radius |
| Circuit solve | 100k components with 10% switching per logic tick in ≤5 ms |
| Busy circuit build | 1,000 wires and 200 lamps toggling at 5 Hz: ≤500 kbit/s per client, frame targets still met |

## 3. Architecture

```
            ┌─────────────── client process ────────────────┐
 input ───► │ main thread: winit, sim prediction, render    │
            │ mesher pool ◄── chunk snapshots               │
            │ net thread (tokio/quinn)                      │
            └───────────────┬───────────────────────────────┘
                            │ QUIC, or in-memory channels in single-player
            ┌───────────────┴─────── server process ────────┐
            │ net thread (tokio/quinn)                      │
            │ tick thread + rayon: sim, block updates       │
            │ job pool: load, generate, light, encode       │
            │ save thread: compress + redb commit           │
            └───────────────────────────────────────────────┘
```

### Workspace layout

```
crates/
  core/       coordinates, IDs, math helpers, tick types (no heavy deps)
  world/      chunk storage, palette, fluid + light layers, edit log
  light/      block + sky propagation (pure, shared by client and server)
  content/    block/item/entity registries, content packs
  worldgen/   terrain + LOD-preview generation (server only)
  sim/        shared movement and collision, ECS components, tick logic
  proto/      wire types, versioning, encode/decode (no I/O)
  net/        quinn transport + in-memory transport behind one trait
  storage/    ChunkStore trait, redb backend
  mesh/       CPU meshers (pure functions, no GPU types)
  render/     wgpu renderer (the only crate that imports wgpu)
  audio/      kira wrapper
  script/     wasmtime host: shared sandbox, separate server and client APIs
  server/     library: tick loop, replication, chunk lifecycle
  client/     library: prediction, streaming, UI, game state
apps/
  game/       client binary, embeds the server for single-player and hosting
  dedicated/  headless server binary
  bot/        headless load-test client
  tools/      world inspector, dictionary trainer, replay viewer
packs/
  base/       the game's own blocks, items and assets, loaded as a content pack
xtask/        verify, perf and scenario commands (section 12)
```

Dependency rules, enforced in CI with `cargo-deny` and a graph check:

- `server` never depends on `render`, `audio` or `winit`.
- `client` never depends on `worldgen`, so the client can't hold the seed or generate terrain.
- `mesh` and `light` stay pure, which makes them easy to bench, fuzz and run in headless tools.

### Core dependencies

| Need | Crate | Note |
|---|---|---|
| GPU | `wgpu` | Pin the version; only `render` imports it |
| Window/input | `winit`, `gilrs` | Event loop stays on the main thread (required on macOS) |
| Transport | `quinn`, `rustls`, `tokio` | Tokio only on net threads; sim code is not async. `rustls` is direct for the pinned-certificate verifier. Use the `ring` provider, with default features off |
| Parallelism | `rayon`, custom job pool | See section 8 |
| Math | `glam` | |
| ECS | `hecs` | Isolated in `sim` |
| Storage | `redb` | Behind a trait |
| Compression | `zstd` | Trained dictionaries for chunks |
| Allocator | `mimalloc`, `bumpalo` | Per-worker arenas for meshing and generation |
| Script | `wasmtime` | Phase 10. Default features off, so the `cache` feature's older `zstd` stays out |
| Audio | `kira` | |
| Logging | `tracing`, `tracing-subscriber` | |
| Profiling | `tracy-client`, `tracing-tracy`, `wgpu-profiler` | Only in `profile` builds; `wgpu-profiler` must support the pinned `wgpu` |
| Dev UI | `egui`, `egui-wgpu`, `egui-winit` | Debug overlays; in-game UI is a small custom batcher. `egui-wgpu` is used only inside `render` |
| Text | `cosmic-text` | Handles more languages and input methods than simpler options |
| Errors | `thiserror`, `anyhow` | `anyhow` only in `apps/` and `xtask` |
| Wire data | `postcard`, `bytes` | Cold messages and shared chunk buffers |
| Identity | `ed25519-dalek`, `rcgen`, `rand` | Player keys, update signatures, self-signed server certificates |
| GPU | `offset-allocator`, `bytemuck`, `pollster` | Slab sub-allocation, GPU struct casts, blocking on wgpu's async setup. `offset-allocator` last released in 2024, so it's small enough to replace if needed |
| Packs and assets | `serde`, `serde_json`, `toml`, `image`, `blake3` | `image` with PNG only, which removes large unused codecs and a flagged dependency. `blake3` for content and cache hashes |
| Updates | `ureq`, `zip`, `self-replace` | HTTP client, deflate-only archives, replacing a running executable on Windows |
| Utilities | `rustc-hash`, `crossbeam-channel`, `wide`, `clap`, `directories` | Hashing, channels, SIMD, command-line arguments, per-OS save and config paths |
| Testing | `proptest`, `criterion`, `gungraun` | `gungraun` needs Valgrind, so it runs on Linux CI only. It replaces `iai-callgrind`, which has had no release since July 2025 |
| Dev tools | `cargo-fuzz`, `cargo-deny`, `samply` | Not linked into the game |

## 4. World data model

### Coordinates

- Blocks are `i32` horizontally, and entity positions are `f64`.
- Rendering is camera-relative, with chunk origins subtracted as integers before converting to `f32`. There is no precision loss far from the origin.
- Every chunk key includes a dimension ID, from Phase 1. Each dimension has its own height, generator, sky rules and storage table. Extra dimensions ship in Phase 7, but this is expensive to retrofit.

### Chunk layout

```rust
struct ChunkData {
    blocks: PalettedArray,        // uniform | 1/2/4/8/16 bits per block
    fluid:  Option<PalettedArray>,// separate layer: waterlogging needs no special states
    light:  Option<LightLayer>,   // sky + block nibbles; uniform fast paths
    opacity: Option<Box<OpacityMasks>>, // per-column u32 masks, maintained on write; None when uniform
    summary: ChunkSummary,        // non-air count, tickable flags, connectivity
    block_entities: SmallMap<u16, BlockEntityId>,
    version: u64,                 // bumps on every edit; drives replication and meshes
}
```

| Palette form | Blocks payload per 32³ chunk |
|---|---|
| Uniform | 0 bytes |
| 1 bit | 4 KB |
| 2 bits | 8 KB |
| 4 bits | 16 KB |
| 8 bits | 32 KB |
| 16 bits | 64 KB |

Design consequences:

- **Uniform chunks are free.** Air and solid stone cost no block memory, and most of a tall world is one or the other.
- **The fluid layer is separate from day one.** It is cheap now and painful to retrofit.
- **Chunk summaries allow O(1) skips.** A chunk without tickable blocks is never scanned, and a fully enclosed chunk is never meshed.

### Lookup

- The client and the server's per-player neighbourhoods use a toroidal clipmap array for O(1) neighbour access.
- The server's global chunk index is a two-level table of regions, then chunks. There is no hash lookup per block access.

### Concurrency model

- Chunks live in `Arc<ChunkData>`. Writers use `Arc::make_mut`, so a write copies only if a snapshot is still outstanding.
- Meshing and lighting jobs capture a 3×3×3 `Arc` neighbourhood. They never lock and never block the tick.
- Persistence and network encoding use the same snapshots.

## 5. Rendering

### Backend position

wgpu v30.0.1 (latest as of this writing) drives the feature choices below. The docs and changelog disagree on some Metal features, so **the baseline renderer relies on none of the disputed ones**.

| Feature | Vulkan | DX12 | Metal | Use in this plan |
|---|---|---|---|---|
| `multi_draw_indirect` | Native when count is available | Native when count is available | Likely emulated as a loop of `draw_indirect`; not settled by the docs | Baseline, measure in Phase 0 |
| `multi_draw_indirect_count` | Yes | Yes | No | Tier 2 only |
| Mesh shaders | Yes | Docs and changelog disagree | Docs and changelog disagree | Not used |
| Texture binding arrays | Yes | Yes | Yes | Optional, when layers run out |
| Storage buffer binding arrays | Yes | Docs say no | Docs and changelog disagree | Not used |
| Pipeline cache object | Yes | Not implemented | Not implemented | Use when available |
| Immediates (push constants) | Yes | Yes | Yes | Per-pass data |
| Timestamp queries inside passes | Yes | Yes | Generally not on tile-based GPUs | Pass-boundary timestamps only |

**Why not raw Vulkan through `ash`?** It would unlock mesh shaders and full bindless, but it means a translation layer on macOS and far more renderer code to own. Voxel terrain doesn't need those features to hit the targets. If a specific path proves insufficient, use wgpu's HAL access for that path instead of rewriting the renderer.

### Terrain vertex format

There are no vertex buffers. Each quad is 12 bytes in a storage buffer, and the vertex shader pulls it using `vertex_index / 4`.

```
word0: x:6  y:6  z:6  w:6  h:6  flags:2     // flags include AO diagonal flip
word1: material:16  ao:8 (4 × 2 bits)  extra:8
word2: light: 4 corners × (sky:4 + block:4)
```

- Face direction isn't stored per quad. Each chunk's quads are grouped by the six faces, and the draw record supplies the direction.
- Grouping gives chunk-level backface culling, which skips about half the quads before the GPU sees them.
- Sky and block light stay separate, so day/night changes need only a uniform update and no remesh.
- Later optimisation, only if profiling justifies it: an 8-byte quad with a per-chunk light/AO palette.

### Meshing

- Implement binary greedy meshing directly on 32³ chunks with a 1-block border. The `binary-greedy-meshing` crate is a useful reference, but it only supports 62³.
- Pipeline per axis: column masks → face masks (`col & !(col >> 1)`) → per-slice merge using `trailing_zeros` → emit.
- Quads merge only when material, AO and light all match.
- Cached opacity masks are maintained on write. The crate's author reports that this gives about a 4× speedup (self-reported, different chunk size).
- Separate passes produce opaque, cutout and translucent quad groups.
- Non-cube blocks (stairs, plants, torches) use a second path: prebuilt model templates emitted into a general vertex stream. Most terrain never touches this path.
- Blocks that change often (wire, lamps, repeaters) are flagged in content data and go into a small per-chunk overlay mesh. A running circuit rebuilds only the overlay, not the terrain mesh.
- **Known artefacts to handle in Phase 1:**
  - T-junction pinholes from greedy merging. Mitigate with a sub-pixel quad expansion in the vertex shader, and a clear colour matched to the fog.
  - AO interpolation anisotropy. Flip the quad diagonal based on AO values.

### GPU memory

- Quad data goes in storage-buffer slabs, sized at runtime from `max_storage_buffer_binding_size` (for example 256 MiB, in a growable slab list).
- Sub-allocate with an offset allocator. Each slab gets its own bind group and draw list.
- Uploads go through a staging ring with a per-frame byte budget, nearest chunks first. Budget overruns defer to the next frame, so streaming doesn't cause hitches.
- A freed range is reused only after the GPU has finished any frame that references it.
- Pool size estimate: 8k surface chunks × 1.5k quads × 12 bytes ≈ 150 MB. Validate with real terrain.

### Materials

- Texture arrays rather than an atlas: greedy quads tile with hardware repeat, and mipmaps don't bleed.
- A storage buffer holds the material table (per-face layer, animation, tint, flags).
- Fall back to several arrays via texture binding arrays if a pack exceeds the adapter's layer limit.

### Entity models

- Models are hierarchies of textured cuboids with keyframe animations, stored as data in packs.
- Rendering is instanced, with bone transforms in a storage buffer. Moving pistons and their blocks use the same path while they animate.

### Visibility tiers

Metal has no count draw, so GPU-compacted drawing isn't portable. An unused zero-instance slot still costs a real draw call if MDI is emulated. CPU compaction is therefore the baseline, and it's designed to be sufficient on its own.

**Tier 1, all platforms**

1. SIMD frustum test over SoA chunk bounds.
2. Cave culling: BFS through per-chunk face-connectivity bits (15 bits, computed at mesh time).
3. Face-group selection by camera position.
4. Write a compact indirect buffer per slab, and call `multi_draw_indirect`.

**Tier 2, Vulkan and DX12 when available**

- Add two-pass Hi-Z occlusion culling in a compute shader, with `multi_draw_indirect_count`.
- It's optional and is only enabled if it wins on benchmarks. Voxel terrain often already has good CPU-side cave culling.

**Portability checks for Phase 0:**

- Chunk records are addressed through `first_instance` in indirect draws, which requires `INDIRECT_FIRST_INSTANCE`. The fallback is individual draws with an immediate, at a higher CPU cost. Confirm on all reference GPUs.
- Vertex pulling needs the draw's base vertex to appear in `vertex_index`, including in indirect draws. DX12 handles this differently from Vulkan and Metal at the API level, so test that wgpu gives the same result on all three.

### Frame structure

A hand-ordered pass list, with no general render graph until one is needed:

1. Visibility and uploads
2. Shadow cascades (from Phase 4, optional per quality level)
3. Opaque and cutout terrain, entities
4. Sky, fog
5. Translucent: water and glass, chunks back to front, per-chunk quads re-sorted only when the camera crosses a chunk boundary
6. Post: bloom, tonemap, optional TAA or FXAA
7. UI

Voxel light handles nearly all local lighting, so no clustered light pipeline is planned. Camera rotation is applied per frame, not per tick, to keep input latency low.

### LOD

- Rings of tiles, each covering 2ⁿ chunks at 2ⁿ scale, in a clipmap around the player. Target: several km view distance on hardware A.
- Tiles use the same quad path with a per-tile scale, so there's no second terrain renderer.
- **The server builds tiles**, because the client has no seed:
  - Generated areas: downsample stored chunks, and mark tiles dirty on edit.
  - Ungenerated areas: `worldgen` produces a cheap surface-only preview without creating real chunks.
- Tiles are streamed at the lowest priority and cached on the client's disk, keyed by tile hash.
- Hide seams with skirts and a short distance-based dither crossfade.

## 6. Lighting

- 4-bit sky and 4-bit block light, propagated with a BFS flood. Removal uses the standard two-queue algorithm.
- Each column keeps a sky-top height, updated on edits and bounded by the configured world height.
- **Replication:**
  - Initial chunk data carries baked light. It compresses well, and clients avoid neighbour-order dependence on load.
  - For edits, clients run the same incremental `light` crate code. The fixed point of monotone flood propagation doesn't depend on update order, so client and server converge from the same block edits with no light traffic.
  - The server sends a column sky-top delta only when a change sits outside a client's loaded range.
  - Clients hold a guard ring of chunks one chunk beyond their render radius, with block deltas replicated but nothing drawn. Block light reaches at most 15 blocks, so an edit outside the ring can't affect any rendered chunk. Meshing needs these neighbours anyway.
- **Verification:** the bot suite compares chunk light hashes between client and server, excluding the guard ring, whose outer edge can legitimately be stale. Any other mismatch is a bug. Fall back to server light patches only if convergence fails in practice.

## 7. Multiplayer

### Transport mapping

| Channel | QUIC mechanism | Content |
|---|---|---|
| Control | Reliable stream, top priority | Login, content manifest, keepalive, disconnect |
| Events | Reliable ordered stream | Block deltas, inventory, chat, entity spawn/despawn |
| Terrain | Pool of reliable streams, per-priority | Chunks and LOD tiles |
| Input | Datagrams, client to server | Last N inputs per packet, so loss is covered by redundancy |
| State | Datagrams, server to client | Entity snapshots, ack of last processed input |

Notes:

- Block deltas and chunks can arrive in either order across streams. Each carries the chunk `version`, and the client buffers deltas until their base chunk is present.
- Chunk traffic can starve datagrams because they share a congestion window. Pace terrain from a budget derived from RTT and congestion state, and test this explicitly under loaded links.
- UDP socket buffers need tuning for high aggregate rates, since OS defaults are usually too small. Set them at startup and warn if the OS refuses.
- The single-player transport implements the same trait over in-process channels, with encoding skipped where possible.
- Hosting: the developer hosts from Canada through port forwarding, for family connecting from California, so internet play is the normal case. The dedicated binary is the main path, and hosting from inside the game stays available. QUIC runs over UDP, so the forwarded port must be UDP. The port and bind address are configurable, and the server logs its address and certificate fingerprint at startup.
- Circuit block changes are latest-state-wins. Each chunk keeps a dirty set that's sent at the client's allowed rate, so a congested link skips intermediate states and never builds a backlog. Light still converges, because it depends only on the final blocks. These updates carry the chunk `version`, so a stale circuit state can't overwrite a newer player edit arriving on the events stream.
- Keep datagrams to about 1,100 bytes, splitting snapshots across several. Links with a lower MTU, including VPNs like Tailscale, then work without relying on fragmentation.

### Identity and security

- Each player has an Ed25519 keypair. The server uses a self-signed certificate that the client pins on first connection, with a visible fingerprint.
- The server admits players through an allowlist of public keys, with no central accounts.
- All decoders are fuzzed, and every length is bounds-checked before allocation.
- Per-connection rate limits apply to edits, chat, chunk requests and login attempts.

### Protocol

- Hand-written compact encoding for hot messages (snapshots, deltas). `postcard` for cold messages.
- A protocol version is exchanged at login. Content is negotiated by manifest hash, and the server sends block-ID-to-name tables so IDs are never assumed.
- Missing content packs download from the server, content-addressed and hash-checked.

### Movement

1. The sim runs at a fixed 30 Hz, using shared code from the `sim` crate on both ends.
2. The client samples input, predicts locally, and sends tick-stamped inputs.
3. The server simulates each player from inputs, using a small per-client jitter buffer. It never accepts client positions as truth.
4. The server acks the last processed input with the resulting state.
5. On mismatch, the client rewinds and replays unacked inputs, smoothing visible corrections.

Cross-platform float differences only cause small corrections, because this isn't lockstep. Avoid `libm` transcendentals in movement code so that mispredictions stay rare.

### Block edits

- The client applies the edit optimistically and tags it with a sequence number.
- The server validates reach, line of sight, permissions and tool state, then acks the sequence or sends the authoritative block state.
- Rejected edits roll back. Predicted edits also feed client collision so that movement stays consistent.

### Entity replication

- Interest management uses a spatial grid per player, with a priority accumulator based on distance, relevance and time since last update.
- Snapshots go out at about 15 Hz, delta-encoded against the last client-acked baseline, with quantised positions and angles.
- Clients interpolate remote entities with a buffer of about 130 ms (two snapshots at 15 Hz) that adapts to jitter, so one lost snapshot doesn't cause a visible stall.
- Combat hit validation rewinds entity positions by client latency, capped at a maximum.

### Chunk streaming

- The server orders each client's queue by distance and view direction, and re-sorts as the player moves.
- Loading is limited vertically: a window around the player's height, plus the surface where it's in view. A full 4096-block column is 128 chunks, so loading whole columns at a 32-chunk radius would mean about 400k chunks per player. Generation follows the same demand, so deep and high chunks aren't created until needed.
- Each chunk version is encoded **once** and the bytes are shared between all clients (`Bytes`). It's cheap to build, and it saves most of the encoding work when players are close together.
- Encoding: palette and bit-packed layers, then `zstd` with a dictionary tied to the protocol version.
- Clients keep a disk cache. On entering range they send a `(pos, hash)`, and the server answers "use cache" when it matches, making reconnects nearly instant.
- Backpressure is per client. A slow client gets a smaller radius rather than unbounded server queues.

### Anti-cheat baseline

- Server-authoritative movement and reach checks.
- Seed never leaves the server.

## 8. Server simulation

### Tick order (30 Hz; block logic every 3rd tick, so 10 Hz)

1. Drain network input
2. Player and entity movement, serial at first; parallel over spatial cells only if profiling calls for it
3. Circuit solve, on logic ticks (below)
4. Block updates (two-phase, below), which merge with circuit output in the apply phase
5. Fluids and random ticks
6. Apply chunk lifecycle results from the job pool
7. Replication: build per-client snapshots
8. Queue dirty chunks for save

### Two-phase block updates

- **Read phase:** each active chunk, in parallel, reads the tick-start state and emits writes into per-chunk outboxes.
- **Apply phase:** writes merge in chunk and position order, with a deterministic priority rule for conflicts.
- Interior and boundary behaviour are identical by construction, so logic circuits don't behave differently across chunk borders. Results don't depend on thread timing.
- **Game logic consequence:** ordinary block behaviours see last tick's state and can't chain instantly in a single update. Instant signal travel belongs to the circuit solver below.
- **Why keep this with only 30 players:** load here scales with active blocks and fluids, not player count, so large builds still benefit. The model is also no more complex than a serial queue.

### Circuits

The goal is Redstone's depth (clocks, logic, memory, piston doors, item sorters) with rules that are easy to reason about. While a machine is fully loaded, its behaviour never depends on its position, direction or chunk borders.

**Model**

- Signal strength is 0–15 and drops by 1 per wire block.
- Connected wire compiles into nets, updated incrementally when a player edits wire.
- Solid blocks can carry power one step, as in Minecraft: wire or a component powers the block, and components touching that block read it. This is what lets a lever on a wall or a torch on a block work. The solver treats each powered block as a small extra node.
- Components (inverter, repeater, comparator, observer) always delay by at least one logic tick. Every feedback loop therefore passes through a delay, so a solve always terminates.
- Sensors turn world state into signals: comparators reading containers, pressure plates, daylight sensors, observers watching blocks. Each subscribes to the changes it cares about, so nothing polls the world and idle sensors cost nothing.

**Each logic tick (10 Hz)**

1. Components with changed outputs mark the nets they drive as dirty.
2. Dirty nets solve instantly, with a multi-source flood from their drivers setting each wire's level.
3. Components read the settled nets and compute their next state, using a timing wheel for longer delays.
4. Outputs that change blocks (lamps, doors, pistons) go into the block-update outboxes.

**Performance:** this is event-driven, so idle circuits cost nothing. Separate circuit groups can solve in parallel, since each reads only the previous state.

**Persistence and chunk loading**

- Saves store component state and pending delays. Nets and wire levels are rebuilt on load.
- A circuit stops when its chunk isn't block-ticking, and any net crossing that border holds its last value. A chunk-loader block keeps permanent machines running.

**Pistons**

- Moves are planned in the read phase and applied atomically, including attached blocks and block entities, up to a push limit.
- If two moves target the same cell, they all fail. That rule doesn't depend on update order.
- Clients animate moving blocks through the entity model path.

**Differences from Minecraft:** direction-dependent update order, quasi-connectivity and zero-tick tricks don't exist, so Minecraft builds relying on them won't work. Most ordinary repeater and logic designs should carry over, since timing also runs at 10 Hz. The Phase 0 prototype and Phase 6 golden circuits test this.

**Open questions (from the Phase 0 prototype):**

- How wire connects vertically. The prototype joined wire in all six directions, so stacked wire merges. Minecraft uses step-up and step-down rules instead. Decide when it can be played with, in Phase 6.
- Solid blocks carrying power (above) and component outputs between 0 and 15, which comparators need, are not yet prototyped. The prototype used on/off outputs only.
- Whether the rules feel right to players is untested and can only be judged by building with them.

**Expansion:** components are archetypes with data parameters, so new gates or signal types can be added as data where an archetype fits. Scripts can't run inside the solver.

**Testing**

- Golden circuits: clock, RS latch, full adder, memory cell, piston door, item sorter.
- Property test: translating or rotating a circuit, or moving it across chunk borders, must not change its behaviour.
- Reference check: compare against a slow, naive solver on random circuits.

**Lamp load:** lamps trigger light updates, which can be far more expensive than the signal change. Clients and the server coalesce light work per chunk with a time budget, so a light update may land a frame or two late rather than cost a dropped frame.

### Chunk lifecycle

States: `Unloaded → Loading → Generating → Decorated → Lit → Ready`

- Ticket-based reference counts decide what's loaded, and at what level (data only, entity-ticking, block-ticking).
- Decoration needs generated neighbours, and lighting needs decorated neighbours. The dependency graph is driven by the job scheduler.
- The job pool has priority lanes and cancellation. Players moving away cancel stale generation, and rayon lacks both features, so rayon is reserved for fork-join work inside a tick.

### Terrain generation

- Stages: climate (2D noise), density (3D noise), surface, caves, then features and structures.
- Sample 3D density on a coarse grid and interpolate, which saves most of the noise cost.
- SIMD noise with AVX2 on x86-64 (runtime-detected) and NEON on ARM.
- Cross-chunk features write through a pending-feature store, so generation order can't change the outcome.
- Output is a pure function of `(seed, position, generator version)`. Golden tests cover this.

### Fluids, entities, AI

- Fluids use a cellular model with per-chunk queues and a tick budget. Over-budget work defers instead of stalling the tick.
- All physics is swept AABB against the voxel grid, shared by players and mobs.
- Pathfinding runs on the job pool with time-sliced searches, using chunk-level hierarchy for longer routes.

### Persistence

- A save thread takes `Arc` snapshots of dirty chunks, player data and entity state, compresses them, and commits **one redb transaction per save cycle** (5–30 s).
- Since players and chunks commit together, a crash can't duplicate items or leave a torn world.
- The `ChunkStore` trait isolates this choice. **Gate at Phase 3:** if write throughput or file growth fails benchmarks with realistic blob sizes, swap in a custom region-file backend.
- Each chunk carries a format version, with explicit upgrade code and test fixtures from the first release.

## 9. Client threading and frame flow

- **Main thread:** winit events, prediction, visibility, wgpu submission.
- **Net thread:** quinn, decode, handoff through bounded channels.
- **Mesher pool:** `N-2` workers, nearest-first with cancellation by chunk version. Edits near the player jump the queue, and their results apply in the same frame if ready.
- **Frame pacing:** present mode is user-selectable, defaulting to low-latency where available. Test explicitly on macOS (ProMotion), Wayland, X11 and Windows flip model.
- Avoid heap churn in hot paths: per-worker bump arenas, reused buffers, and CI-tracked allocations per frame via a counting allocator.

## 10. Modding

Three layers, built in this order. Each is useful without the next, and client scripts can be cut without affecting the rest.

| Layer | Runs on | Can do | Trust |
|---|---|---|---|
| Data packs | Both | Blocks, items, recipes, textures, models, sounds, UI layouts | Data only; parsers are fuzzed |
| Server scripts (WASM) | Server | Gameplay rules through event hooks | Installed by the server owner |
| Client scripts (WASM) | Client | HUD and UI, particles, input actions, entity animation, messages to their server-side counterpart | Sandboxed, treated as untrusted |

### Client script rules

- **Sources:** a server can send scripts when a player joins, or a player can install them locally. Server-sent code is remote code, so it needs a consent prompt per server and hash pinning.
- **Sandbox:** no file, network or process access. Each mod gets scoped key-value storage and a message channel to its own server-side part.
- **No native mods and no custom shaders.** Shaders can hang or crash GPU drivers, and native code can't be sandboxed. Shader support could be revisited later with validation.
- **Budgets:** memory limits and epoch interruption apply to each mod. A mod that repeatedly overruns its per-frame budget gets disabled with a visible message.
- **Retained API:** scripts update retained UI trees and particle emitters, and the engine draws them. Render, meshing and block-update loops never call into scripts, so an installed mod can't slow them down directly.
- **Cheating:** a client mod can only see what the client already has, but a modified client can't be detected reliably in any case. Security never depends on client mods behaving. Server authority stays the protection.

### Early work that keeps mods possible

These add little work and avoid a costly retrofit:

- **Phase 1:** the base game loads from `packs/base` through the same path as any mod, and all IDs are namespaced names.
- **Phase 2:** the protocol includes a content manifest and namespaced mod message channels.
- **Phase 4:** materials, block models and particles are data, not hard-coded Rust.
- **Phase 5:** the game's UI is a retained widget tree, and input goes through an action map, so scripts can use both.

## 11. Engineering practices

- **Rust:** stable toolchain, edition 2024, no nightly in shipped crates. `cargo-fuzz` and Miri need a nightly toolchain, so those steps of `verify` run on Linux only and don't affect the code that ships.
- **Unsafe:** `#![forbid(unsafe_code)]` by default. Allowed only in small audited modules (palette bit-packing, SIMD), each covered by Miri where possible.
- **Release builds:** thin or fat LTO, `codegen-units = 1`, `panic = "abort"`, and PGO for release artifacts after Phase 8.
- **wgpu upgrades:** v28–v30 each brought breaking API changes, and more are queued in trunk. Keep wgpu inside `render`, pin the version, and schedule upgrades deliberately.

### Error handling

A failure should affect as little as possible, and nothing should risk a family world.

| Failure | Response |
|---|---|
| Bad data or protocol error from one client | Disconnect that client; the server keeps running |
| Corrupt chunk on disk | Back up the bad data, log it, and refuse to overwrite it without an explicit decision |
| Save failure, such as a full disk | Warn players, retry, and keep the last good save |
| GPU device lost | Rebuild the renderer without losing game state |
| Out of GPU memory | Reduce render distance and show a message |
| Mod error | Disable that mod and tell the player |
| Bug (a panic) | Write the crash log from the panic hook, then exit |

A panic means the game state can't be trusted, so there's no emergency save. Transactional saves limit the loss to the last 5–30 s, and the dedicated server restarts automatically.

### Profiling

- **`profile` builds** enable Tracy through a cargo feature. Tracy can accept network connections when active, so family releases don't include it.
- **Spans:** hot loops use spans that compile out at build level, because a disabled `tracing` span still costs a small check. Coarser spans can stay in all builds.
- **CPU sampling:** use `samply` to find costs in code without spans.
- **GPU timing:** `wgpu-profiler` sends pass-boundary timestamps to Tracy.
- **Family releases** keep the F3 overlay and local frame-time reports.

### Testing

| Type | Scope |
|---|---|
| Property tests (`proptest`) | Palette round-trips, mesher face coverage vs brute force, light vs from-scratch recompute |
| Fuzzing (`cargo-fuzz`) | All wire decoders, save-file parsers |
| Golden tests | Worldgen output, wire format, render screenshots with tolerance |
| Network simulation | Loss, reorder, latency and jitter shim on the transport trait |
| Bot load tests | Headless clients walking, digging and building at target player counts |
| Benchmarks | `criterion` locally, instruction-count benches in CI for stable regression detection |
| Soak test | Multi-hour bot run, checking for memory growth and desync |

### Assets

- **Textures and audio** come from free sources, preferring CC0. `packs/base/ASSETS.toml` records each file's source, author and licence, and `xtask verify` fails on any missing entry.
- **Don't use "free" texture packs derived from Minecraft's textures.** They carry Mojang's rights, whatever the download page says.
- **Style consistency** is the main risk with mixed sources. `cargo xtask assets` normalises textures to one resolution and shared palette, and audio to consistent loudness and format.
- **Models** are written by agents in the cuboid format. `cargo xtask model-sheet` renders each one from several angles to a PNG, so the agent can check its own work and a person can approve it.

### CI and delivery

- **CI:** GitHub-hosted runners for Linux, Windows and macOS build, lint and test every push. The repository is public, so hosted minutes are free. Hosted runners have no real GPU (Windows uses a software renderer, macOS a virtual one), so they check correctness and that the programs run, not performance. Real-GPU performance checks run locally on the Linux PC with `cargo xtask perf`. Real Windows and Mac hardware is checked with the test bundles at phase gates. No family machine needs Rust installed.
- **MacBook testing:** it's a family member's laptop, so it isn't a CI runner. `cargo xtask bundle` creates a package that its owner opens to run the scenarios and produce a report to send back. It runs at phase gates only, so Metal problems can stay hidden between gates.
- **Headless GPU tests** use lavapipe or WARP.
- **Packaging:** plain archives or an AppImage for Linux, a zip for Windows, an ad-hoc-signed universal `.app` for macOS (runs on Apple silicon and Intel, so the MacBook model doesn't need to be known), and the dedicated server binary. Without paid signing, macOS and Windows show a security warning on first launch, so include short instructions.
- **Updates:** a built-in updater downloads releases over HTTPS from an unlisted static location and checks them against a public key embedded in the game, with the private signing key kept off CI. Avoid a private GitHub repository as the download source, because family clients would then need credentials. Clients and servers must match on protocol version, and the join screen explains any mismatch.
- **Saves:** family worlds must survive updates. Format versions, upgrade code and test fixtures start with the first family release, and the game backs up a world before upgrading it.
- **Diagnostics:** no telemetry. Crash logs and frame-time reports save locally and can be sent by hand.

## 12. Implementation by AI agents

With agents writing the code, the limit is verification rather than typing speed. An agent can produce plausible code faster than anyone can check it, so every gate needs evidence that a machine can produce, plus a short list of things only a person can judge.

### Machine-checkable commands

| Command | Checks | Output |
|---|---|---|
| `cargo xtask verify` | Format, clippy with warnings denied, tests, `cargo-deny`, crate dependency rules, short fuzz runs | Pass or fail |
| `cargo xtask perf` | Benchmarks against committed baselines and section 2 targets | JSON report |
| `cargo xtask scenario <name>` | Scripted flythrough or bot scenario on real GPU and network shim | Frame-time histogram, screenshots, desync and correction counts |
| `cargo xtask assets` | Normalises and checks textures and audio, including licence entries | Processed assets, or a failure |
| `cargo xtask model-sheet` | Renders models from several angles | PNG contact sheet |
| `cargo xtask bundle` | Builds the MacBook test package | Package that produces a report |

Feel has to be made measurable where possible. Track prediction correction size, rubber-band events, input-to-present latency, and frame-time spikes during streaming.

### Guarding against agent failure modes

| Failure mode | Control |
|---|---|
| Weakening tests or targets to get a pass | Targets and baselines live in protected files; changes need human approval |
| Tests that repeat the implementation's own mistake | Compare against independent references: naive mesher, from-scratch light recompute, brute-force collision |
| Subtle bugs in `unsafe`, GPU synchronisation or wire parsing | Miri, wgpu validation layers, fuzzing, and a review pass by a separate agent |
| Invented or outdated APIs in fast-moving crates | Pin versions, and have agents consult `cargo doc` output for the pinned version |
| Architecture drifting between sessions | Decision records and `AGENTS.md` in the repo; dependency rules enforced by `verify` |
| Features nobody asked for | Each task lists its scope and acceptance commands; anything else is a new task |

### Working method

- Freeze shared types first (`core`, `world`, `proto`), then agents can work in parallel on separate crates using git worktrees, rebasing and landing directly on the default branch once checks pass.
- With no pull request step, separate-agent reviews happen after commits land, at least once per phase gate and for any `unsafe`, GPU synchronisation or wire parsing work.
- Keep tasks to one crate or a clear interface, with acceptance criteria in the form of commands.
- Keep the pure crates (`mesh`, `light`, `proto`, `worldgen`) free of GPU and I/O dependencies, since those are the easiest for agents to test completely.

### What needs a human

- **Real hardware.** Linux can't validate Metal or DX12, and a software rasteriser says nothing about performance. The Windows PC's owner runs the test bundle at phase gates, as the MacBook's owner does.
- **The MacBook owner** runs the test bundle remotely at phase gates, when convenient. A late report doesn't hold up work on other platforms, but that gate stays open for macOS until the report arrives.
- **Family playtests** at the Phase 2, 4, 5, 6 and 7 gates for movement feel, latency and fun. Agents can't judge these.
- **Art direction and final asset choices.** Agents can collect and normalise free assets and write models, but a person decides what looks right.
- **Expansion design.** Agents can propose ideas, but what makes this game different is your call.

## 13. Roadmap

Sizes are relative (S, M, L), not calendar estimates, because I don't have a credible basis for predicting how long agents will take. The likely limit is how quickly gates can be checked on real hardware and by playtest. Each phase ends with a gate; if a gate fails, fix it before moving on.

Networking comes before generation and rendering scale on purpose. Retrofitting authority and prediction is the most common way this kind of project fails.

### Phase 0: Risk spikes (S)

Throwaway code answering the questions that could change the architecture.

- Render 10k draws through `multi_draw_indirect` and `first_instance` on all three OSes, and measure Metal's cost.
- Prototype a 32³ binary greedy mesher, plus quad format and T-junction handling.
- Test quinn datagrams competing with bulk streams on a lossy, loaded link.
- Benchmark redb with realistic chunk blobs.
- Confirm wgpu's actual Metal and DX12 feature status on real hardware.
- Run an ad-hoc-signed build on the MacBook, including wasmtime and a test of first launch and self-update, and measure host-call overhead and epoch interruption inside a frame. This is the one spike that needs the remote owner's time, so it shouldn't block the others.
- Prototype the circuit solver against the section 2 target, including net rebuilds during edits.
- Set up `xtask`, hosted CI for all three systems, and the test bundles for the Windows PC and the MacBook.
- Record the actual specs of every machine that will play, and replace the reference hardware in section 2.

**Gate:** written results, with each decision in section 1 confirmed or revised. Targets adjusted to measured data.

### Phase 1: Core and local rendering (M)

- Workspace and CI on all three platforms
- `core`, `world`, `content`, `mesh`, basic `render`
- Base game content loaded from `packs/base`, with namespaced IDs
- Dimension IDs in chunk keys
- Flat or noise test terrain, free camera, F3 overlay, Tracy integration
- Staging ring, slab allocator, Tier 1 frustum culling, texture arrays

**Gate:** 16-chunk radius at target frame rate on A and B, and remesh within target. Same binary behaviour on all three OSes.

### Phase 2: Multiplayer vertical slice (L)

- `proto`, `net`, `server`, `sim`; embedded and dedicated server
- Login, allowlist, content manifest, mod message channels, chunk streaming, block edits with prediction and rollback
- Per-client backpressure and terrain pacing
- Connection rate limits, QUIC address validation and fuzzed login decoders, because a port-forwarded server is reachable by anyone before the allowlist check
- Movement prediction and reconciliation, remote-player interpolation
- Network simulation shim and first bots

**Gate:** 28 bots and 2 human players on one server, with Linux, Windows and the MacBook all joining, at 150 ms latency and 2% loss, with no visible desync and acceptable feel in playtests. Includes one real session between Canada and California.

### Phase 3: Persistent real world (M)

- Chunk lifecycle, tickets, job pool with priorities and cancellation
- `worldgen` terrain, caves and features; `light` crate and replication strategy
- `storage` with transactional saves, kill-9 crash tests, and corrupt-chunk and save-failure handling
- Disk cache on the client, zstd dictionaries

**Gate:** generation rate on target, a crash at any moment leaves a consistent world, and client/server light hashes match over a long bot run.

### Phase 4: Rendering at scale (L)

- Cave culling, face-group draws, compact indirect path, 32-chunk radius
- Water and translucency, model blocks, cutout pass, day/night, sky, fog
- Materials, block models and particles defined as data
- Overlay meshes for frequently changing blocks, instanced entity models
- LOD tiles from server to client
- Post effects and shadows; Tier 2 Hi-Z only if benchmarks justify it
- GPU device-lost recovery and out-of-memory handling

**Gate:** all frame targets met on A, B and C while flying through terrain at speed, with no streaming hitches at p99.

### Phase 5: Survival core, first family release (L)

- Two-phase block updates, fluids, random ticks
- ECS entities, item drops, mobs, pathfinding
- The survival core content from section 1, with the first free textures, audio and agent-written models
- Audio; retained widget UI and input action map, including IME-capable chat, menus, settings, key rebinding and a server list
- Player models, first-person hand and held items
- Packaging, updater, hosting from inside the game, save versioning and backups
- All decoders fuzzed for the first release, with bad client data handled as in section 11

**Gate:** the family can install, update and play on one world across all their machines. The survival loop is fun, and the tick budget holds with 30 players and thousands of entities.

### Phase 6: Circuits (L)

- Net compiler, solver, timing wheel and component set from section 1
- Pistons, hoppers, dispensers, rails and minecarts
- Latest-state-wins replication, overlay meshes in use, coalesced light work
- Golden circuits, invariance tests and the reference solver

**Gate:** golden circuits pass, moving a circuit never changes its behaviour, the circuit targets in section 2 are met, and a family member can build a working piston door.

### Phase 7: World depth (L)

- The world depth content from section 1, including extra dimensions
- New terrain and structure generators, tuned at each step by playing on the family world

**Gate:** a full playthrough from new world to final boss is possible, and an existing world keeps working after this update, with new terrain only in newly generated chunks.

### Phase 8: Hardening (S)

- 30-bot load tests with heavy building, fluids and circuits; profile-driven fixes
- Fault-injection tests covering the failure table in section 11
- Longer fuzz runs, soak tests, admin tools, PGO

**Gate:** server targets met, a 24-hour soak has no leak or crash, and bandwidth is within target.

### Phase 9: Expansion (size depends on design)

- Write a short design for each new system, and prefer data-driven additions.
- Add engine features only when data can't express the idea.

**Gate:** decided per feature, and always includes a family playtest.

### Phase 10: Mods (M, can be cut)

1. Public data pack format with hash-addressed download
2. Server scripting, with event hooks, resource limits and a capability API
3. Client scripting, under the rules in section 10

**Gate:** a test mod with server and client parts installs from a server join, runs inside its budget, and can't reach files or the network when it tries.

## 14. Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Metal draw-call cost from emulated MDI | Medium | High on macOS | Measure in Phase 0. CPU compaction limits draw count. Merge adjacent ranges. HAL path to Metal indirect command buffers as a last resort |
| wgpu API churn or backend bugs | High | Medium | Pin versions, isolate in `render`, keep a real-GPU test matrix, contribute fixes upstream |
| Chunk bursts disturb realtime datagrams | Medium | High | Terrain pacing; Phase 0 test; a separate connection for terrain if needed |
| Client and server light diverge | Medium | Medium | Same crate, hash checks in bots, light patches as fallback |
| Greedy mesh artefacts | High | Low | Known mitigations, screenshot tests |
| Circuit rules feel wrong to players | Low | Medium | Decided: no Redstone quirks, as the players aren't Redstone specialists. Prototype in Phase 0; golden circuits; family build test at the Phase 6 gate |
| Lamp-heavy or clock-heavy builds overload light, meshing or the network | Medium | Medium | Overlay meshes, coalesced light, latest-state replication, dedicated scenario |
| Mixed free assets look inconsistent | High | Medium | Normalising pipeline, a human art pass, and a limited number of sources |
| Metal bugs stay hidden between MacBook test runs | Medium | Medium | Daily hosted macOS tests, test bundle at each gate, wgpu validation layers on other backends |
| Update breaks a family world | Medium | High | Save fixtures in CI, automatic backup before upgrade |
| redb write amplification or file growth | Medium | Medium | Trait boundary, Phase 3 gate, region-file fallback |
| LOD complexity grows out of control | Medium | Medium | Reuse the terrain path, cut scope to a far-field heightmap if needed |
| Agent code passes checks but is subtly wrong | High | High | Independent reference tests, protected targets, separate review pass (section 12) |
| Game feel can't be judged by metrics | High | Medium | Family playtests at the Phase 2, 4, 5, 6 and 7 gates |
| Minecraft-parity content is very large | High | Medium | Three playable groups in order; ship and play each before starting the next |
| Client script sandbox escape or abuse | Low | High | Keep wasmtime updated, no ambient capabilities, consent per server; client scripts can be cut |
| wasmtime or the updater blocked by macOS security rules | Low | Medium | Tested on the MacBook in Phase 0; the fallbacks are server-only scripts and manual updates |

## 15. Deferred on purpose

- Mesh shaders: revisit once wgpu's cross-backend support is settled and benchmarks show a benefit
- Ray-traced global illumination, voxel cone tracing
- Custom shaders from mods
- NAT traversal and relay servers for hosting without port forwarding

## 16. What was and wasn't verified

Checked on 2026-11-01:

- wgpu's latest release (v30.0.1) and the feature-support wording in its docs and changelog. Parts of those pages were only read as summaries.
- quinn's current release (0.11.12) and its datagram support.
- Reported `binary-greedy-meshing` timings: 50–340 µs on 62³ chunks. These are self-reported by the author and come from different hardware and versions.
- All library crates in section 3 resolved together in a test project on Rust 1.98.1, with one `wgpu` version (30.0.1) shared by `egui-wgpu` and `wgpu-profiler`, and compiled with `cargo check` on Linux. That confirms they build together, not that they behave correctly.
- `cargo-deny` on that project, using the feature settings in section 3: no known vulnerabilities and licences clean. Default features had pulled in a vulnerable older `zstd` through wasmtime's cache.
- Two unmaintained-crate notices remain, with no fix available upstream. Record both as explicit exceptions in `deny.toml`:
  - `ttf-parser`, through `cosmic-text` and winit's Linux window decorations. Acceptable, because the game only loads fonts it ships.
  - `atomic-polyfill`, through `postcard`. Doesn't appear in the dependency tree for any of the game's four build targets.

Tested versions: `wgpu` 30.0.1, `winit` 0.30.13, `quinn` 0.11.12, `rustls` 0.23.46, `tokio` 1.54.1, `egui` 0.37.0, `wasmtime` 50.0.1, `redb` 4.4.0, `hecs` 0.11.2, `kira` 0.12.6, `zstd` 0.14.3, `glam` 0.34.2, `cosmic-text` 0.19.0, `wgpu-profiler` 0.28.0, `tracy-client` 0.19.0, `tracing-tracy` 0.12.0.

Not verified, to be settled in Phase 0:

- Plain `multi_draw_indirect` behaviour on each backend. The docs imply emulation when the count feature is missing, without a per-backend statement.
- Mesh shader and storage-buffer binding array support on Metal and DX12, where docs and changelog conflict. The plan does not depend on them.
- `INDIRECT_FIRST_INSTANCE` on target GPUs.
- That the crates compile on Windows and macOS. Dependencies were resolved for those targets but only compiled on Linux.
- The dev tools `cargo-fuzz` and `samply`, whose crates.io listings were checked but which weren't run. `samply`'s last release was February 2025.
- wasmtime's behaviour in an ad-hoc-signed macOS build, and its per-frame overhead.
- Current macOS and Windows first-launch behaviour for unsigned apps, and whether self-updates avoid repeated prompts.
- The circuit design, which comes from reasoning and has no prototype or play testing behind it yet.
- All performance targets and memory estimates, which are unmeasured.

## 17. Open questions

1. **Which MacBook is it?** This isn't blocking, since universal builds run on either chip type. The answer only affects the macOS performance targets, and the first test bundle report will reveal it.
2. **What are the expansion ideas?** Needed by Phase 9. It helps to know any big ones sooner, since a few may need engine support earlier.
3. **How should wire connect vertically?** See the circuit open questions in section 8. Needed by Phase 6.
