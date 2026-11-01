use std::collections::HashMap;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use voxel_content::TextureImage;
use voxel_core::{ChunkKey, ChunkPos, DimensionId};
use voxel_mesh::{ChunkMesh, LayerMesh};

use crate::RenderError;
use crate::camera::{Camera, Frustum};
use crate::gpu::Gpu;
use crate::pool::{Placement, QUAD_BYTES, QuadPool};
use crate::texture::create_texture_array;

const SHADER: &str = include_str!("terrain.wgsl");
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Chunk records are a fixed-size buffer; chunks beyond this in one frame are not drawn.
const MAX_RECORDS: usize = 1 << 16;
const CHUNK: f32 = 32.0;

/// How the swap chain is paced.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PresentPreference {
    /// Wait for the display (always available).
    Vsync,
    /// No tearing but the newest frame wins, where the system offers it.
    LowLatency,
    /// Present immediately and allow tearing, where the system offers it.
    Uncapped,
}

#[derive(Copy, Clone, Debug)]
pub struct RenderConfig {
    /// Size of each quad slab in bytes (clamped to what the adapter allows).
    pub slab_bytes: u64,
    /// Most chunk-mesh bytes written to the GPU in one frame, nearest chunks first.
    /// At least one mesh is always uploaded, so a large one can't starve.
    pub upload_budget_bytes: u64,
    pub present: PresentPreference,
    /// Linear RGB used for both the clear colour and the fog.
    pub sky: [f32; 3],
    pub fog_start: f32,
    pub fog_end: f32,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            slab_bytes: 64 << 20,
            upload_budget_bytes: 8 << 20,
            present: PresentPreference::LowLatency,
            sky: [0.32, 0.55, 0.95],
            fog_start: 200.0,
            fog_end: 480.0,
        }
    }
}

/// What the last frame did, for the F3 overlay and tests.
#[derive(Copy, Clone, Debug, Default)]
pub struct FrameStats {
    /// False when no frame could be acquired (minimised window, resize in progress).
    pub drawn: bool,
    pub resident_chunks: usize,
    pub visible_chunks: usize,
    pub draw_calls: usize,
    pub quads_drawn: u64,
    pub uploaded_chunks: usize,
    pub uploaded_bytes: u64,
    pub pending_uploads: usize,
    /// Mesh layers that did not fit in any slab.
    pub dropped_layers: usize,
    pub pool_quads_used: u64,
    pub pool_slabs: usize,
    pub pool_frees_waiting: u64,
    pub indirect: bool,
    /// Time spent in `render`, in milliseconds.
    pub cpu_ms: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Globals {
    view_proj: [f32; 16],
    eye: [f32; 4],
    fog: [f32; 4],
    misc: [f32; 4],
}

/// Arguments of one indirect draw, in the layout wgpu expects.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct DrawArgs {
    vertex_count: u32,
    instance_count: u32,
    first_vertex: u32,
    first_instance: u32,
}

#[derive(Copy, Clone)]
struct LayerGpu {
    place: Placement,
    /// Quad index of each face group's start, relative to the layer; `[6]` is the total.
    starts: [u32; 7],
}

struct Entry {
    key: ChunkKey,
    /// Block coordinates of the chunk's minimum corner.
    origin: [i32; 3],
    /// Opaque, cutout, translucent.
    layers: [Option<LayerGpu>; 3],
}

enum Output {
    Window {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    },
    Offscreen {
        color: wgpu::Texture,
    },
}

/// Per-frame scratch buffers, kept between frames so drawing allocates nothing in steady state.
#[derive(Default)]
struct Scratch {
    words: Vec<u32>,
    records: Vec<[f32; 4]>,
    /// (distance squared, entry index) of chunks that passed culling.
    visible: Vec<(f32, usize)>,
    /// `lists[layer][slab]`.
    lists: [Vec<Vec<DrawArgs>>; 3],
    args: Vec<DrawArgs>,
    /// (layer, slab, first draw in `args`, count)
    ranges: Vec<(usize, usize, usize, usize)>,
}

pub struct Renderer {
    gpu: Gpu,
    output: Output,
    size: (u32, u32),
    color_format: wgpu::TextureFormat,
    depth_view: wgpu::TextureView,
    pipeline_opaque: wgpu::RenderPipeline,
    pipeline_translucent: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    globals: wgpu::Buffer,
    records: wgpu::Buffer,
    indirect: wgpu::Buffer,
    /// Draw entries `indirect` can hold.
    indirect_capacity: usize,
    texture_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    pool: QuadPool,
    entries: Vec<Entry>,
    index: HashMap<ChunkKey, usize>,
    pending: HashMap<ChunkKey, ChunkMesh>,
    config: RenderConfig,
    scratch: Scratch,
}

impl Renderer {
    /// A renderer that draws into a texture you can read back, with no window. Used by tests,
    /// screenshots and headless tools.
    pub fn new_headless(
        size: (u32, u32),
        images: &[TextureImage],
        config: RenderConfig,
    ) -> Result<Self, RenderError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let gpu = Gpu::new(instance, None)?;
        let size = (size.0.max(1), size.1.max(1));
        let color = offscreen_texture(&gpu.device, size);
        Self::build(
            gpu,
            Output::Offscreen { color },
            OFFSCREEN_FORMAT,
            size,
            images,
            config,
        )
    }

    /// A renderer that draws into a window. `target` is the window (an `Arc` of a winit window works).
    pub fn new_window(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        size: (u32, u32),
        images: &[TextureImage],
        config: RenderConfig,
    ) -> Result<Self, RenderError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance
            .create_surface(target)
            .map_err(|e| RenderError::Surface(e.to_string()))?;
        let gpu = Gpu::new(instance, Some(&surface))?;
        let size = (size.0.max(1), size.1.max(1));
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| RenderError::Surface("the surface supports no formats".into()))?;
        let mut sc = surface
            .get_default_config(&gpu.adapter, size.0, size.1)
            .ok_or_else(|| {
                RenderError::Surface("the adapter cannot draw to this surface".into())
            })?;
        sc.format = format;
        sc.present_mode = choose_present_mode(config.present, &caps.present_modes);
        surface.configure(&gpu.device, &sc);
        Self::build(
            gpu,
            Output::Window {
                surface,
                config: sc,
            },
            format,
            size,
            images,
            config,
        )
    }

    fn build(
        gpu: Gpu,
        output: Output,
        color_format: wgpu::TextureFormat,
        size: (u32, u32),
        images: &[TextureImage],
        mut config: RenderConfig,
    ) -> Result<Self, RenderError> {
        let device = &gpu.device;
        let limit = gpu.max_storage_binding.min(device.limits().max_buffer_size);
        config.slab_bytes = (config.slab_bytes.min(limit) / QUAD_BYTES).max(1) * QUAD_BYTES;

        let texture_view = create_texture_array(device, &gpu.queue, images, gpu.max_array_layers)?;
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("block sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 1, // anisotropic filtering needs linear magnification, which would blur blocks
            ..Default::default()
        });

        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty,
            count: None,
        };
        let buffer = |ty| wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let storage = buffer(wgpu::BufferBindingType::Storage { read_only: true });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain bind group layout"),
            entries: &[
                entry(0, buffer(wgpu::BufferBindingType::Uniform)),
                entry(1, storage),
                entry(2, storage),
                entry(
                    3,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                ),
                entry(
                    4,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
        });

        let (pipeline_opaque, pipeline_translucent) = make_pipelines(device, &bgl, color_format)?;
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let records = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk records"),
            size: (MAX_RECORDS * 16) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let indirect_capacity = 4096;
        let indirect = indirect_buffer(device, indirect_capacity);
        let depth_view = depth_texture(device, size);
        let pool = QuadPool::new(config.slab_bytes);
        Ok(Self {
            gpu,
            output,
            size,
            color_format,
            depth_view,
            pipeline_opaque,
            pipeline_translucent,
            bgl,
            globals,
            records,
            indirect,
            indirect_capacity,
            texture_view,
            sampler,
            pool,
            entries: Vec::new(),
            index: HashMap::new(),
            pending: HashMap::new(),
            config,
            scratch: Scratch::default(),
        })
    }

    /// Adapter, backend and device type, for logs and the F3 overlay.
    pub fn gpu_description(&self) -> String {
        self.gpu.describe()
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn color_format(&self) -> wgpu::TextureFormat {
        self.color_format
    }

    /// Chunks with meshes on the GPU (not counting ones still waiting to upload).
    pub fn resident_chunks(&self) -> usize {
        self.entries.len()
    }

    pub fn resize(&mut self, size: (u32, u32)) {
        let size = (size.0.max(1), size.1.max(1));
        if size == self.size {
            return;
        }
        self.size = size;
        self.depth_view = depth_texture(&self.gpu.device, size);
        match &mut self.output {
            Output::Window { surface, config } => {
                config.width = size.0;
                config.height = size.1;
                surface.configure(&self.gpu.device, config);
            }
            Output::Offscreen { color } => *color = offscreen_texture(&self.gpu.device, size),
        }
    }

    /// Queues a chunk's mesh for upload, replacing any earlier one. An empty mesh removes the chunk.
    /// Uploads happen in `render`, nearest chunks first, within the per-frame byte budget.
    pub fn set_chunk(&mut self, key: ChunkKey, mesh: &ChunkMesh) {
        self.pending.insert(key, mesh.clone());
    }

    /// Drops a chunk's mesh, queued or resident.
    pub fn remove_chunk(&mut self, key: ChunkKey) {
        self.pending.remove(&key);
        self.remove_entry(key);
    }

    fn remove_entry(&mut self, key: ChunkKey) {
        let Some(i) = self.index.remove(&key) else {
            return;
        };
        let old = self.entries.swap_remove(i);
        if i < self.entries.len() {
            self.index.insert(self.entries[i].key, i);
        }
        for layer in old.layers.into_iter().flatten() {
            self.pool.free_later(layer.place);
        }
    }

    /// Copies one chunk's mesh into the pool and swaps it in. Returns the bytes written.
    fn upload(&mut self, key: ChunkKey, mesh: &ChunkMesh, dropped: &mut usize) -> u64 {
        self.remove_entry(key);
        let origin = key.pos.origin();
        let mut entry = Entry {
            key,
            origin: [origin.x, origin.y, origin.z],
            layers: [None; 3],
        };
        let mut bytes = 0;
        for (slot, layer) in [&mesh.opaque, &mesh.cutout, &mesh.translucent]
            .into_iter()
            .enumerate()
        {
            match self.upload_layer(layer) {
                Ok(Some((gpu, n))) => {
                    entry.layers[slot] = Some(gpu);
                    bytes += n;
                }
                Ok(None) => {}
                Err(()) => *dropped += 1,
            }
        }
        if entry.layers.iter().any(Option::is_some) {
            self.index.insert(key, self.entries.len());
            self.entries.push(entry);
        }
        bytes
    }

    #[allow(clippy::result_unit_err)]
    fn upload_layer(&mut self, layer: &LayerMesh) -> Result<Option<(LayerGpu, u64)>, ()> {
        let quads = layer.quads.len();
        if quads == 0 {
            return Ok(None);
        }
        let quads = u32::try_from(quads).map_err(|_| ())?;
        let (device, bgl, globals, records, view, sampler) = (
            &self.gpu.device,
            &self.bgl,
            &self.globals,
            &self.records,
            &self.texture_view,
            &self.sampler,
        );
        let mut make = |bytes: u64| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("quad slab"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bg = make_bind_group(device, bgl, globals, &buffer, records, view, sampler);
            (buffer, bg)
        };
        let place = self.pool.alloc(quads, &mut make).ok_or(())?;
        let words = &mut self.scratch.words;
        words.clear();
        words.extend(layer.quads.iter().flat_map(|q| [q.w0, q.w1, q.w2]));
        let data: &[u8] = bytemuck::cast_slice(words);
        self.gpu.queue.write_buffer(
            &self.pool.slabs[place.slab].buffer,
            u64::from(place.offset) * QUAD_BYTES,
            data,
        );
        Ok(Some((
            LayerGpu {
                place,
                starts: layer.face_ranges,
            },
            data.len() as u64,
        )))
    }

    fn process_uploads(&mut self, camera_chunk: ChunkPos, stats: &mut FrameStats) {
        if self.pending.is_empty() {
            return;
        }
        let dist2 = |k: &ChunkKey| {
            let d = |a: i32, b: i32| (i64::from(a) - i64::from(b)).pow(2);
            d(k.pos.x, camera_chunk.x) + d(k.pos.y, camera_chunk.y) + d(k.pos.z, camera_chunk.z)
        };
        let mut order: Vec<(i64, ChunkKey)> = self.pending.keys().map(|k| (dist2(k), *k)).collect();
        order.sort_unstable_by_key(|&(d, k)| (d, k.pos.x, k.pos.y, k.pos.z));
        for (_, key) in order {
            if stats.uploaded_bytes >= self.config.upload_budget_bytes {
                break;
            }
            let Some(mesh) = self.pending.remove(&key) else {
                continue;
            };
            let mut dropped = 0;
            stats.uploaded_bytes += self.upload(key, &mesh, &mut dropped);
            stats.uploaded_chunks += 1;
            stats.dropped_layers += dropped;
        }
    }

    /// Culls resident chunks of `dim` and builds the draw lists. Returns the number of draws.
    fn build_draws(
        &mut self,
        camera: &Camera,
        dim: DimensionId,
        frustum: &Frustum,
        stats: &mut FrameStats,
    ) {
        let origin = camera.block_origin();
        let eye = camera.eye_in_block();
        let s = &mut self.scratch;
        s.visible.clear();
        for (i, e) in self.entries.iter().enumerate() {
            if e.key.dim != dim {
                continue;
            }
            let rel = Vec3::new(
                (i64::from(e.origin[0]) - i64::from(origin[0])) as f32,
                (i64::from(e.origin[1]) - i64::from(origin[1])) as f32,
                (i64::from(e.origin[2]) - i64::from(origin[2])) as f32,
            );
            if frustum.intersects_aabb(rel, rel + Vec3::splat(CHUNK)) {
                let c = rel + Vec3::splat(CHUNK / 2.0) - eye;
                s.visible.push((c.length_squared(), i));
            }
        }
        // Nearest first helps early depth rejection; translucent walks the list backwards.
        s.visible.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        s.visible.truncate(MAX_RECORDS);
        stats.visible_chunks = s.visible.len();

        s.records.clear();
        let slabs = self.pool.slabs.len();
        for layer in &mut s.lists {
            layer.resize_with(slabs, Vec::new);
            layer.iter_mut().for_each(Vec::clear);
        }
        let push = |s: &mut Scratch, vi: usize, layer: usize, record: u32| {
            let e = &self.entries[s.visible[vi].1];
            let Some(l) = e.layers[layer] else { return };
            let rel = s.records[record as usize];
            let o = Vec3::new(rel[0], rel[1], rel[2]);
            for dir in 0..6usize {
                let count = l.starts[dir + 1] - l.starts[dir];
                if count == 0 || !face_visible(dir, eye, o) {
                    continue;
                }
                s.lists[layer][l.place.slab].push(DrawArgs {
                    vertex_count: 6 * count,
                    instance_count: 1,
                    first_vertex: 6 * (l.place.offset + l.starts[dir]),
                    first_instance: record << 3 | dir as u32,
                });
            }
        };
        for vi in 0..s.visible.len() {
            let e = &self.entries[s.visible[vi].1];
            s.records.push([
                (i64::from(e.origin[0]) - i64::from(origin[0])) as f32,
                (i64::from(e.origin[1]) - i64::from(origin[1])) as f32,
                (i64::from(e.origin[2]) - i64::from(origin[2])) as f32,
                0.0,
            ]);
        }
        for vi in 0..s.visible.len() {
            push(s, vi, 0, vi as u32);
            push(s, vi, 1, vi as u32);
        }
        // Back to front for blending.
        for vi in (0..s.visible.len()).rev() {
            push(s, vi, 2, vi as u32);
        }
    }

    /// Draws one frame: uploads queued meshes within budget, culls, draws, presents.
    pub fn render(&mut self, camera: &Camera, dim: DimensionId) -> Result<FrameStats, RenderError> {
        let started = Instant::now();
        let mut stats = FrameStats {
            indirect: self.gpu.indirect_first_instance,
            ..Default::default()
        };
        let _ = self.gpu.device.poll(wgpu::PollType::Poll);
        self.pool.collect();
        let cam_chunk = voxel_core::BlockPos::new(
            camera.block_origin()[0],
            camera.block_origin()[1],
            camera.block_origin()[2],
        )
        .chunk();
        self.process_uploads(cam_chunk, &mut stats);

        let frame = match &self.output {
            Output::Window { surface, .. } => match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t)
                | wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some(t),
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    None
                }
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                    // The window changed under us; reconfigure and try again next frame.
                    if let Output::Window { surface, config } = &self.output {
                        surface.configure(&self.gpu.device, config);
                    }
                    None
                }
                _ => return Err(RenderError::NoFrame),
            },
            Output::Offscreen { .. } => None,
        };
        if matches!(self.output, Output::Window { .. }) && frame.is_none() {
            stats.resident_chunks = self.entries.len();
            stats.pending_uploads = self.pending.len();
            stats.cpu_ms = started.elapsed().as_secs_f32() * 1e3;
            return Ok(stats);
        }

        let aspect = self.size.0 as f32 / self.size.1 as f32;
        let view_proj = camera.view_proj(aspect);
        let frustum = Frustum::from_view_proj(view_proj);
        self.build_draws(camera, dim, &frustum, &mut stats);

        // Flatten the per-slab lists into one indirect buffer.
        let s = &mut self.scratch;
        s.args.clear();
        s.ranges.clear();
        for (layer, per_slab) in s.lists.iter().enumerate() {
            for (slab, list) in per_slab.iter().enumerate() {
                if !list.is_empty() {
                    s.ranges.push((layer, slab, s.args.len(), list.len()));
                    s.args.extend_from_slice(list);
                }
            }
        }
        stats.draw_calls = s.args.len();
        stats.quads_drawn = s.args.iter().map(|a| u64::from(a.vertex_count / 6)).sum();
        if s.args.len() > self.indirect_capacity {
            self.indirect_capacity = s.args.len().next_power_of_two();
            self.indirect = indirect_buffer(&self.gpu.device, self.indirect_capacity);
        }
        let queue = &self.gpu.queue;
        if !s.args.is_empty() {
            queue.write_buffer(&self.indirect, 0, bytemuck::cast_slice(&s.args));
        }
        if !s.records.is_empty() {
            queue.write_buffer(&self.records, 0, bytemuck::cast_slice(&s.records));
        }
        let eye = camera.eye_in_block();
        let half_pixel = (camera.fov_y * 0.5).tan() / self.size.1 as f32;
        let globals = Globals {
            view_proj: view_proj.to_cols_array(),
            eye: [eye.x, eye.y, eye.z, self.config.fog_start],
            fog: [
                self.config.sky[0],
                self.config.sky[1],
                self.config.sky[2],
                self.config.fog_end,
            ],
            misc: [half_pixel, 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

        let color_view = match (&self.output, &frame) {
            (_, Some(f)) => f.texture.create_view(&wgpu::TextureViewDescriptor {
                format: Some(self.color_format),
                ..Default::default()
            }),
            (Output::Offscreen { color }, None) => color.create_view(&Default::default()),
            (Output::Window { .. }, None) => return Err(RenderError::NoFrame),
        };
        let mut enc = self.gpu.device.create_command_encoder(&Default::default());
        {
            let [r, g, b] = self.config.sky.map(f64::from);
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("terrain"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for translucent in [false, true] {
                pass.set_pipeline(if translucent {
                    &self.pipeline_translucent
                } else {
                    &self.pipeline_opaque
                });
                for &(layer, slab, first, count) in &self.scratch.ranges {
                    if (layer == 2) != translucent {
                        continue;
                    }
                    pass.set_bind_group(0, &self.pool.slabs[slab].bind_group, &[]);
                    if self.gpu.indirect_first_instance {
                        pass.multi_draw_indirect(&self.indirect, (first * 16) as u64, count as u32);
                    } else {
                        // Without INDIRECT_FIRST_INSTANCE, issue the same draws one by one.
                        for a in &self.scratch.args[first..first + count] {
                            pass.draw(
                                a.first_vertex..a.first_vertex + a.vertex_count,
                                a.first_instance..a.first_instance + 1,
                            );
                        }
                    }
                }
            }
        }
        queue.submit([enc.finish()]);
        self.pool.frame_submitted(queue);
        if let Some(f) = frame {
            queue.present(f);
        }

        stats.drawn = true;
        stats.resident_chunks = self.entries.len();
        stats.pending_uploads = self.pending.len();
        let (used, waiting) = self.pool.usage();
        stats.pool_quads_used = used;
        stats.pool_frees_waiting = waiting;
        stats.pool_slabs = self.pool.slabs.len();
        stats.cpu_ms = started.elapsed().as_secs_f32() * 1e3;
        Ok(stats)
    }

    /// Blocks until the GPU has finished everything submitted so far. For benchmarks and
    /// screenshots; normal frames never wait.
    pub fn wait_idle(&self) {
        let _ = self.gpu.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// Copies the last frame of a headless renderer to the CPU as tightly packed RGBA8
    /// (sRGB-encoded), `width * height * 4` bytes.
    pub fn read_pixels(&self) -> Result<Vec<u8>, RenderError> {
        let Output::Offscreen { color } = &self.output else {
            return Err(RenderError::NotHeadless);
        };
        let (w, h) = self.size;
        let row = (w * 4).next_multiple_of(256);
        let buf = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.gpu.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.gpu.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| RenderError::Readback(e.to_string()))?;
        let mapped = slice
            .get_mapped_range()
            .map_err(|e| RenderError::Readback(e.to_string()))?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let start = (y * row) as usize;
            out.extend_from_slice(&mapped[start..start + (w * 4) as usize]);
        }
        drop(mapped);
        buf.unmap();
        Ok(out)
    }
}

/// A face group can be seen only from its outer side. Offsets are camera minus chunk origin.
fn face_visible(dir: usize, eye: Vec3, origin: Vec3) -> bool {
    let c = (eye - origin).to_array()[dir / 2];
    // Faces of positive-direction groups lie on planes 1..=32 into the chunk, negative on 0..=31.
    if dir.is_multiple_of(2) {
        c > 1.0
    } else {
        c < CHUNK - 1.0
    }
}

fn choose_present_mode(
    want: PresentPreference,
    available: &[wgpu::PresentMode],
) -> wgpu::PresentMode {
    let has = |m| available.contains(&m);
    match want {
        PresentPreference::Vsync => wgpu::PresentMode::Fifo,
        PresentPreference::LowLatency if has(wgpu::PresentMode::Mailbox) => {
            wgpu::PresentMode::Mailbox
        }
        PresentPreference::Uncapped if has(wgpu::PresentMode::Immediate) => {
            wgpu::PresentMode::Immediate
        }
        PresentPreference::Uncapped if has(wgpu::PresentMode::Mailbox) => {
            wgpu::PresentMode::Mailbox
        }
        _ => wgpu::PresentMode::Fifo,
    }
}

fn offscreen_texture(device: &wgpu::Device, size: (u32, u32)) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen color"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OFFSCREEN_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn depth_texture(device: &wgpu::Device, size: (u32, u32)) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn indirect_buffer(device: &wgpu::Device, entries: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("indirect draws"),
        size: (entries * 16) as u64,
        usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn make_bind_group(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    quads: &wgpu::Buffer,
    records: &wgpu::Buffer,
    texture: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("terrain bind group"),
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: quads.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: records.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(texture),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn make_pipelines(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> Result<(wgpu::RenderPipeline, wgpu::RenderPipeline), RenderError> {
    // wgpu panics on validation errors by default; catch them so they come back as errors.
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("terrain"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("terrain"),
        bind_group_layouts: &[Some(bgl)],
        immediate_size: 0,
    });
    let make = |fragment: &str, translucent: bool| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(fragment),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(!translucent),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: translucent.then_some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let pair = (make("fs_opaque", false), make("fs_translucent", true));
    if let Some(err) = pollster::block_on(scope.pop()) {
        return Err(RenderError::Pipeline(err.to_string()));
    }
    Ok(pair)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A face group may be skipped only if none of its planes can face the camera.
    #[test]
    fn face_groups_are_skipped_exactly_when_no_plane_faces_the_camera() {
        for dir in 0..6usize {
            let axis = dir / 2;
            let positive = dir % 2 == 0;
            // Planes of a positive group sit at 1..=32 into the chunk, a negative one at 0..=31.
            let planes: Vec<f32> = if positive {
                (1..=32).map(|p| p as f32).collect()
            } else {
                (0..32).map(|p| p as f32).collect()
            };
            for step in -80..=1200 {
                let c = step as f32 * 0.05 - 4.0;
                let mut eye = Vec3::splat(0.5);
                eye[axis] = c;
                let any_faces_camera = planes.iter().any(|&p| if positive { c > p } else { c < p });
                assert_eq!(
                    face_visible(dir, eye, Vec3::ZERO),
                    any_faces_camera,
                    "dir {dir}, camera at {c}"
                );
            }
        }
    }
}
