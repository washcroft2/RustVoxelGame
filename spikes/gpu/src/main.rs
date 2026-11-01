//! Phase 0 GPU spike (plan sections 5 and 13). Throwaway.
//!
//! Standalone: prints a report and also writes it to `gpu-report.txt`, so the
//! same program can be handed to the Windows and macOS owners at a gate.
//!
//! 1. Which adapters and features wgpu reports on this machine.
//! 2. Does vertex pulling with `first_vertex` (quad base) and `first_instance`
//!    (chunk record) through direct, indirect and multi-draw-indirect calls
//!    produce exactly the expected pixels? How long do 10,000 draws take?
//! 3. Do greedy-meshed quads leave pinholes at T-junctions in 3D, and does a
//!    sub-pixel vertex expansion remove them?

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use glam::camera::rh::proj::directx::perspective;
use glam::camera::rh::view::look_at_mat4;
use glam::{Mat4, Vec3};
use spike_mesher::{Mesh, OpacityCols, mesh, scenes, t_junctions};
use wgpu::util::DeviceExt;

const INDIRECT_SHADER: &str = include_str!("indirect.wgsl");
const TERRAIN_SHADER: &str = include_str!("terrain.wgsl");

fn main() -> Result<()> {
    let mut report = String::new();
    let result = pollster::block_on(run(&mut report));
    if let Err(e) = &result {
        let _ = writeln!(report, "\nFAILED: {e:#}");
    }
    println!("{report}");
    std::fs::write("gpu-report.txt", &report).context("write gpu-report.txt")?;
    result
}

async fn run(out: &mut String) -> Result<()> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    writeln!(
        out,
        "os={} arch={}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )?;
    writeln!(out, "adapters wgpu can see:")?;
    for a in instance.enumerate_adapters(wgpu::Backends::all()).await {
        let i = a.get_info();
        writeln!(
            out,
            "  {} | {:?} | {:?} | driver {} {}",
            i.name, i.backend, i.device_type, i.driver, i.driver_info
        )?;
    }
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        })
        .await
        .context("no suitable adapter")?;
    let info = adapter.get_info();
    writeln!(
        out,
        "\nusing: {} ({:?}, {:?})",
        info.name, info.backend, info.device_type
    )?;
    let features = adapter.features();
    let limits = adapter.limits();
    let downlevel = adapter.get_downlevel_capabilities();
    let check = |name: &str, yes: bool| format!("  {name}: {}\n", if yes { "yes" } else { "NO" });
    out.push_str(&check(
        "INDIRECT_FIRST_INSTANCE",
        features.contains(wgpu::Features::INDIRECT_FIRST_INSTANCE),
    ));
    out.push_str(&check(
        "MULTI_DRAW_INDIRECT_COUNT",
        features.contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT),
    ));
    out.push_str(&check(
        "TEXTURE_BINDING_ARRAY",
        features.contains(wgpu::Features::TEXTURE_BINDING_ARRAY),
    ));
    out.push_str(&check(
        "TIMESTAMP_QUERY",
        features.contains(wgpu::Features::TIMESTAMP_QUERY),
    ));
    out.push_str(&check(
        "downlevel INDIRECT_EXECUTION",
        downlevel
            .flags
            .contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION),
    ));
    writeln!(
        out,
        "  max_storage_buffer_binding_size: {} MiB, max_buffer_size: {} MiB, max_texture_array_layers: {}",
        limits.max_storage_buffer_binding_size / (1 << 20),
        limits.max_buffer_size / (1 << 20),
        limits.max_texture_array_layers
    )?;

    let has_first_instance = features.contains(wgpu::Features::INDIRECT_FIRST_INSTANCE);
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            required_features: features & wgpu::Features::INDIRECT_FIRST_INSTANCE,
            required_limits: limits,
            ..Default::default()
        })
        .await
        .context("request_device")?;

    indirect_test(&device, &queue, has_first_instance, out)?;
    pinhole_test(&device, &queue, out)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 2: vertex pulling through first_vertex / first_instance
// ---------------------------------------------------------------------------

const CHUNKS: usize = 10_000;
const QUADS_PER_CHUNK: usize = 16;
const CELL: u32 = 16;
const GRID: u32 = 100;
const SIZE: u32 = GRID * CELL;

/// Same 16 colours as `indirect.wgsl`, in Rgba8Unorm bytes.
const PALETTE: [[u8; 3]; 16] = [
    [255, 0, 0],
    [0, 255, 0],
    [0, 0, 255],
    [255, 255, 0],
    [0, 255, 255],
    [153, 51, 0],
    [51, 153, 0],
    [0, 51, 153],
    [204, 204, 204],
    [102, 102, 102],
    [255, 153, 0],
    [153, 0, 153],
    [0, 153, 153],
    [255, 102, 102],
    [102, 255, 102],
    [102, 102, 255],
];

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Direct,
    IndirectLoop,
    MultiIndirect,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Direct => "10,000 x draw(first_vertex, first_instance)",
            Mode::IndirectLoop => "10,000 x draw_indirect",
            Mode::MultiIndirect => "1 x multi_draw_indirect(10,000)",
        }
    }
}

fn indirect_test(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    has_first_instance: bool,
    out: &mut String,
) -> Result<()> {
    writeln!(
        out,
        "\n== vertex pulling + first_vertex/first_instance, {CHUNKS} draws x {QUADS_PER_CHUNK} quads, {SIZE}x{SIZE} target"
    )?;

    // Chunks get irregular quad bases so a wrong first_vertex shows up as wrong pixels.
    let mut order: Vec<usize> = (0..CHUNKS).collect();
    let mut s = 0x9E37_79B9_7F4A_7C15u64;
    for i in (1..order.len()).rev() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        order.swap(i, (s % (i as u64 + 1)) as usize);
    }
    let mut quads = vec![0u32; CHUNKS * QUADS_PER_CHUNK * 3];
    let mut recs = vec![[0f32; 4]; CHUNKS];
    let mut args = vec![[0u32; 4]; CHUNKS];
    for i in 0..CHUNKS {
        let base = order[i] * QUADS_PER_CHUNK;
        for q in 0..QUADS_PER_CHUNK {
            let (qx, qy) = ((q % 4) as u32, (q / 4) as u32);
            quads[(base + q) * 3] = (qx * 4) | (qy * 4) << 6 | 4 << 18 | 4 << 24;
            quads[(base + q) * 3 + 1] = material(i, qx, qy);
        }
        recs[i] = [
            ((i as u32 % GRID) * CELL) as f32,
            ((i as u32 / GRID) * CELL) as f32,
            0.0,
            0.0,
        ];
        args[i] = [6 * QUADS_PER_CHUNK as u32, 1, 6 * base as u32, i as u32];
    }

    let quad_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("quads"),
        contents: bytemuck::cast_slice(&quads),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let rec_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("recs"),
        contents: bytemuck::cast_slice(&recs),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let screen = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("screen"),
        contents: bytemuck::cast_slice(&[SIZE as f32, SIZE as f32, 0.0, 0.0]),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let arg_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("args"),
        contents: bytemuck::cast_slice(&args),
        usage: wgpu::BufferUsages::INDIRECT,
    });

    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            storage(0),
            storage(1),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: quad_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: rec_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: screen.as_entire_binding(),
            },
        ],
    });
    let pipeline = make_pipeline(device, &bgl, INDIRECT_SHADER, false);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());

    let modes: &[Mode] = if has_first_instance {
        &[Mode::Direct, Mode::IndirectLoop, Mode::MultiIndirect]
    } else {
        writeln!(
            out,
            "INDIRECT_FIRST_INSTANCE is not supported here: skipping the two indirect modes"
        )?;
        &[Mode::Direct]
    };
    writeln!(
        out,
        "{:<46} {:>12} {:>14} {:>10}",
        "mode", "record (ms)", "submit+wait", "wrong px"
    )?;
    for &mode in modes {
        let mut record = Vec::new();
        let mut gpu = Vec::new();
        for frame in 0..120 {
            let t0 = Instant::now();
            let mut enc = device.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bg, &[]);
                match mode {
                    Mode::Direct => {
                        for a in &args {
                            pass.draw(a[2]..a[2] + a[0], a[3]..a[3] + 1);
                        }
                    }
                    Mode::IndirectLoop => {
                        for i in 0..CHUNKS as u64 {
                            pass.draw_indirect(&arg_buf, i * 16);
                        }
                    }
                    Mode::MultiIndirect => pass.multi_draw_indirect(&arg_buf, 0, CHUNKS as u32),
                }
            }
            let cmd = enc.finish();
            let t1 = Instant::now();
            queue.submit([cmd]);
            device.poll(wgpu::PollType::wait_indefinitely())?;
            let t2 = Instant::now();
            if frame >= 20 {
                record.push(t1 - t0);
                gpu.push(t2 - t1);
            }
        }
        let wrong = count_wrong_pixels(device, queue, &target)?;
        writeln!(
            out,
            "{:<46} {:>12.2} {:>14.2} {:>10}",
            mode.name(),
            median(&mut record).as_secs_f64() * 1e3,
            median(&mut gpu).as_secs_f64() * 1e3,
            wrong
        )?;
        ensure!(
            wrong == 0,
            "{}: {wrong} pixels differ from the CPU reference",
            mode.name()
        );
    }
    writeln!(
        out,
        "wrong px must be 0: it means every draw got the right quads (first_vertex) and chunk record (first_instance)."
    )?;
    Ok(())
}

fn material(chunk: usize, qx: u32, qy: u32) -> u32 {
    (chunk as u32 * 7 + qx * 3 + qy * 5) % 16
}

fn count_wrong_pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tex: &wgpu::Texture,
) -> Result<usize> {
    let data = read_texture(device, queue, tex, SIZE, SIZE)?;
    let mut wrong = 0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let chunk = ((y / CELL) * GRID + x / CELL) as usize;
            let (qx, qy) = ((x % CELL) / 4, (y % CELL) / 4);
            let want = PALETTE[material(chunk, qx, qy) as usize];
            let at = ((y * SIZE + x) * 4) as usize;
            if data[at..at + 4] != [want[0], want[1], want[2], 255] {
                wrong += 1;
            }
        }
    }
    Ok(wrong)
}

// ---------------------------------------------------------------------------
// Test 3: pinholes at T-junctions
// ---------------------------------------------------------------------------

const VIEW: u32 = 512;
const POSES: usize = 40;

fn pinhole_test(device: &wgpu::Device, queue: &wgpu::Queue, out: &mut String) -> Result<()> {
    writeln!(
        out,
        "\n== pinholes: greedy-meshed terrain seen from {POSES} random camera poses, {VIEW}x{VIEW}"
    )?;
    writeln!(
        out,
        "background is magenta; every pixel of the view should land on terrain, so magenta = pinhole"
    )?;
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let pipeline = make_pipeline(device, &bgl, TERRAIN_SHADER, true);
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: VIEW,
            height: VIEW,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: VIEW,
            height: VIEW,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let (color_view, depth_view) = (
        color.create_view(&Default::default()),
        depth.create_view(&Default::default()),
    );
    let globals = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 80,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // Fixed pseudo-random poses looking nearly straight down at the middle of the chunk.
    let mut s = 0xDEAD_BEEF_1234_5678u64;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f32 / (1u64 << 53) as f32
    };
    let poses: Vec<Mat4> = (0..POSES)
        .map(|_| {
            let eye = Vec3::new(
                16.0 + rnd() * 8.0 - 4.0,
                45.0 + rnd() * 10.0,
                16.0 + rnd() * 8.0 - 4.0,
            );
            let target = Vec3::new(16.0 + rnd() * 6.0 - 3.0, 14.0, 16.0 + rnd() * 6.0 - 3.0);
            perspective(25f32.to_radians(), 1.0, 0.1, 200.0) * look_at_mat4(eye, target, Vec3::Y)
        })
        .collect();

    let epsilons = [0.0f32, 0.0001, 0.0005, 0.002, 0.01];
    write!(out, "{:<18} {:>6} {:>6}", "scene", "quads", "tjunc")?;
    for e in epsilons {
        write!(out, " {:>12}", format!("eps {e}"))?;
    }
    writeln!(out, "   (pinhole pixels / poses affected)")?;

    for (name, voxels) in [
        ("terrain (smooth)", scenes::terrain_smooth()),
        ("terrain (rough)", scenes::terrain()),
    ] {
        let mut m = Mesh::default();
        mesh(&voxels, &OpacityCols::from_voxels(&voxels), &mut m);
        let words: Vec<u32> = m.quads.iter().flat_map(|q| [q.w0, q.w1, q.w2]).collect();
        let quad_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&words),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: quad_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: globals.as_entire_binding(),
                },
            ],
        });
        write!(
            out,
            "{:<18} {:>6} {:>6}",
            name,
            m.quads.len(),
            t_junctions(&m)
        )?;
        for eps in epsilons {
            let (mut holes, mut affected) = (0usize, 0usize);
            for vp in &poses {
                let mut uniform = [0f32; 20];
                uniform[..16].copy_from_slice(&vp.to_cols_array());
                uniform[16] = eps;
                queue.write_buffer(&globals, 0, bytemuck::cast_slice(&uniform));
                let mut enc = device.create_command_encoder(&Default::default());
                {
                    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &color_view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 1.0,
                                    g: 0.0,
                                    b: 1.0,
                                    a: 1.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &depth_view,
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
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &bg, &[]);
                    for dir in 0..6u32 {
                        let (a, b) = (m.face_ranges[dir as usize], m.face_ranges[dir as usize + 1]);
                        if b > a {
                            pass.draw(6 * a..6 * b, dir..dir + 1);
                        }
                    }
                }
                queue.submit([enc.finish()]);
                let data = read_texture(device, queue, &color, VIEW, VIEW)?;
                let n = data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|p| p[..3] == [255, 0, 255])
                    .count();
                holes += n;
                affected += usize::from(n > 0);
            }
            write!(out, " {:>12}", format!("{holes}/{affected}"))?;
        }
        writeln!(out)?;
    }
    writeln!(
        out,
        "eps is the outward push of each quad corner, in blocks."
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn make_pipeline(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    wgsl: &str,
    depth: bool,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(bgl)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: depth.then(|| wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Copies an Rgba8 texture to the CPU. `width * 4` must be a multiple of 256.
fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tex: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    ensure!(
        (width * 4).is_multiple_of(256),
        "row size must be 256-byte aligned"
    );
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(width * height * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([enc.finish()]);
    let slice = buf.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
    device.poll(wgpu::PollType::wait_indefinitely())?;
    let data = slice.get_mapped_range()?.to_vec();
    buf.unmap();
    Ok(data)
}

fn median(v: &mut [Duration]) -> Duration {
    v.sort();
    v[v.len() / 2]
}
