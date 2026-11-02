//! Renders to memory and checks pixels. These need a GPU or a software rasteriser;
//! with no adapter they print a note and pass, so a machine without one still builds.
//! They run for real on the Linux dev machine and on the Windows and macOS CI runners.

use std::path::Path;

use voxel_content::{Pack, Registry, TextureImage, load_texture_images};
use voxel_core::{BlockId, ChunkKey, ChunkPos, DimensionId, Face, LocalPos};
use voxel_mesh::{ChunkMesh, Mesher, Neighborhood};
use voxel_render::{Camera, Overlay, RenderConfig, RenderError, Renderer};
use voxel_world::ChunkData;
use voxel_worldgen::{Generator, NoiseParams, Terrain, TerrainBlocks};

const SIZE: u32 = 64;
const STONE: [u8; 3] = [125, 125, 125];

fn test_registry() -> (Registry, Vec<TextureImage>) {
    let block = |tex: &str| format!("[textures]\nall = \"{tex}\"\n");
    let pack = Pack::from_parts(
        "id = \"t\"\nversion = \"1\"\n",
        &[
            ("dirt".into(), block("t_dirt")),
            ("stone".into(), block("t_stone")),
        ],
        vec!["t_dirt".into(), "t_stone".into()],
    )
    .unwrap();
    let reg = Registry::build(&[pack]).unwrap();
    let flat = |rgb: [u8; 3]| TextureImage {
        width: 16,
        height: 16,
        rgba: (0..256)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect(),
    };
    // Registry order: t_dirt, t_stone.
    (reg, vec![flat([134, 96, 67]), flat(STONE)])
}

fn renderer(images: &[TextureImage], config: RenderConfig) -> Option<Renderer> {
    match Renderer::new_headless((SIZE, SIZE), images, config) {
        Ok(r) => Some(r),
        Err(RenderError::NoAdapter | RenderError::Device(_)) => {
            eprintln!("no graphics adapter: skipping");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

fn mesh_of(chunk: &ChunkData, reg: &Registry) -> ChunkMesh {
    let mut out = ChunkMesh::default();
    Mesher::new().mesh(&Neighborhood::new(chunk), reg, &mut out);
    out
}

fn key(x: i32, y: i32, z: i32) -> ChunkKey {
    ChunkKey::new(DimensionId::OVERWORLD, ChunkPos::new(x, y, z))
}

fn looking_at(from: [f64; 3], to: [f64; 3]) -> Camera {
    let f = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let len = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt();
    let (fx, fy, fz) = (f[0] / len, f[1] / len, f[2] / len);
    Camera {
        position: from,
        yaw: (-fx).atan2(-fz) as f32,
        pitch: fy.asin() as f32,
        ..Camera::default()
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// The byte value a texel of `albedo` ends up as after shading by `shade`.
fn shaded(albedo: [u8; 3], shade: f32) -> [u8; 3] {
    albedo.map(|a| {
        (linear_to_srgb(srgb_to_linear(f32::from(a) / 255.0) * shade) * 255.0).round() as u8
    })
}

fn pixel(img: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE + x) * 4) as usize;
    [img[i], img[i + 1], img[i + 2]]
}

fn near(a: [u8; 3], b: [u8; 3], tol: i32) -> bool {
    a.iter()
        .zip(b)
        .all(|(&x, y)| (i32::from(x) - i32::from(y)).abs() <= tol)
}

const SHADE: [f32; 6] = [0.7, 0.7, 1.0, 0.55, 0.85, 0.85];

#[test]
fn every_face_is_visible_from_outside_with_its_own_shade() {
    let (reg, images) = test_registry();
    let Some(mut r) = renderer(&images, RenderConfig::default()) else {
        return;
    };
    let mut chunk = ChunkData::empty();
    chunk.set_block(
        LocalPos::new(8, 8, 8),
        reg.block_id(&"t:stone".parse().unwrap()).unwrap(),
        &reg,
    );
    r.set_chunk(key(0, 0, 0), &mesh_of(&chunk, &reg));
    for face in Face::ALL {
        let n = face.normal();
        let from = [
            8.5 + 4.0 * f64::from(n[0]),
            8.5 + 4.0 * f64::from(n[1]),
            8.5 + 4.0 * f64::from(n[2]),
        ];
        let stats = r
            .render(&looking_at(from, [8.5; 3]), DimensionId::OVERWORLD, None)
            .unwrap();
        assert!(stats.drawn);
        let got = pixel(&r.read_pixels().unwrap(), SIZE / 2, SIZE / 2);
        let want = shaded(STONE, SHADE[face.index()]);
        assert!(near(got, want, 3), "{face:?}: got {got:?}, want {want:?}");
    }
}

#[test]
fn empty_sky_is_the_clear_colour_and_nothing_is_drawn() {
    let (_, images) = test_registry();
    let Some(mut r) = renderer(&images, RenderConfig::default()) else {
        return;
    };
    let stats = r
        .render(&Camera::default(), DimensionId::OVERWORLD, None)
        .unwrap();
    assert_eq!((stats.draw_calls, stats.visible_chunks), (0, 0));
    let sky = RenderConfig::default()
        .sky
        .map(|c| (linear_to_srgb(c) * 255.0).round() as u8);
    for (x, y) in [(0, 0), (SIZE - 1, SIZE - 1), (SIZE / 2, SIZE / 2)] {
        assert!(near(pixel(&r.read_pixels().unwrap(), x, y), sky, 2));
    }
}

#[test]
fn chunks_behind_the_camera_are_culled() {
    let (reg, images) = test_registry();
    let Some(mut r) = renderer(&images, RenderConfig::default()) else {
        return;
    };
    let stone = reg.block_id(&"t:stone".parse().unwrap()).unwrap();
    let mut chunk = ChunkData::empty();
    chunk.set_block(LocalPos::new(8, 8, 8), stone, &reg);
    let mesh = mesh_of(&chunk, &reg);
    r.set_chunk(key(0, 0, -3), &mesh); // in front: camera looks along -Z
    r.set_chunk(key(0, 0, 3), &mesh); // behind
    let cam = Camera {
        position: [8.0, 8.0, 8.0],
        ..Camera::default()
    };
    let s = r.render(&cam, DimensionId::OVERWORLD, None).unwrap();
    assert_eq!((s.resident_chunks, s.visible_chunks), (2, 1));
    // Another dimension's chunks are not drawn.
    let other = ChunkKey::new(DimensionId(1), ChunkPos::new(0, 0, -3));
    r.set_chunk(other, &mesh);
    let s = r.render(&cam, DimensionId::OVERWORLD, None).unwrap();
    assert_eq!((s.resident_chunks, s.visible_chunks), (3, 1));
}

#[test]
fn uploads_are_nearest_first_within_the_byte_budget() {
    let (reg, images) = test_registry();
    let mesh = {
        let mut c = ChunkData::empty();
        c.set_block(
            LocalPos::new(1, 1, 1),
            reg.block_id(&"t:stone".parse().unwrap()).unwrap(),
            &reg,
        );
        mesh_of(&c, &reg)
    };
    let one = 6 * 12; // a lone block is six quads
    // Room for two meshes per frame: 3 chunks need 2 frames, nearest first.
    let cfg = RenderConfig {
        upload_budget_bytes: one * 2 - 1,
        ..RenderConfig::default()
    };
    let Some(mut r) = renderer(&images, cfg) else {
        return;
    };
    for x in [5, 1, 3] {
        r.set_chunk(key(x, 0, 0), &mesh);
    }
    let cam = Camera {
        position: [0.0, 0.0, 0.0],
        ..Camera::default()
    };
    let s = r.render(&cam, DimensionId::OVERWORLD, None).unwrap();
    assert_eq!(
        (s.uploaded_chunks, s.pending_uploads, s.resident_chunks),
        (2, 1, 2)
    );
    let s = r.render(&cam, DimensionId::OVERWORLD, None).unwrap();
    assert_eq!(
        (s.uploaded_chunks, s.pending_uploads, s.resident_chunks),
        (1, 0, 3)
    );
}

#[test]
fn replaced_and_removed_meshes_release_their_space_once_the_gpu_is_done() {
    let (reg, images) = test_registry();
    let stone = reg.block_id(&"t:stone".parse().unwrap()).unwrap();
    let Some(mut r) = renderer(&images, RenderConfig::default()) else {
        return;
    };
    let mut chunk = ChunkData::empty();
    chunk.set_block(LocalPos::new(8, 8, 8), stone, &reg);
    let cam = looking_at([8.5, 8.5, 12.5], [8.5; 3]);
    for _ in 0..20 {
        r.set_chunk(key(0, 0, 0), &mesh_of(&chunk, &reg));
        r.render(&cam, DimensionId::OVERWORLD, None).unwrap();
        r.read_pixels().unwrap(); // waits for the GPU
    }
    // One more frame so the last frees are collected.
    let s = r.render(&cam, DimensionId::OVERWORLD, None).unwrap();
    assert_eq!(
        s.pool_quads_used, 6,
        "only the current mesh should still be allocated"
    );
    r.remove_chunk(key(0, 0, 0));
    r.read_pixels().unwrap();
    let s = r.render(&cam, DimensionId::OVERWORLD, None).unwrap();
    r.read_pixels().unwrap();
    let s2 = r.render(&cam, DimensionId::OVERWORLD, None).unwrap();
    assert_eq!((s.resident_chunks, s2.pool_quads_used), (0, 0));
}

#[test]
fn terrain_seen_from_above_has_no_pinholes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/base");
    let pack = Pack::read_dir(&dir).unwrap();
    let reg = Registry::build(std::slice::from_ref(&pack)).unwrap();
    let images = load_texture_images(&reg, &[pack]).unwrap();
    let Some(mut r) = renderer(
        &images,
        RenderConfig {
            fog_start: 1e6,
            fog_end: 2e6,
            ..RenderConfig::default()
        },
    ) else {
        return;
    };
    let blocks = TerrainBlocks::from_registry(&reg).unwrap();
    let gen_ = Generator::new(
        3,
        Terrain::Noise(NoiseParams {
            sea_level: -100,
            ..NoiseParams::default()
        }),
        blocks,
    );
    // Find a chunk row that has surface and mesh a 3x3 patch so borders are right.
    let cy = gen_.surface_height(48, 48).div_euclid(32);
    let chunks: Vec<ChunkData> = (-1..=1)
        .flat_map(|dz| (-1..=1).map(move |dx| (dx, dz)))
        .map(|(dx, dz)| gen_.chunk(key(1 + dx, cy, 1 + dz), &reg))
        .collect();
    let above: Vec<ChunkData> = (-1..=1)
        .flat_map(|dz| (-1..=1).map(move |dx| (dx, dz)))
        .map(|(dx, dz)| gen_.chunk(key(1 + dx, cy + 1, 1 + dz), &reg))
        .collect();
    let mut nb = Neighborhood::new(&chunks[4]);
    for (i, c) in chunks.iter().enumerate() {
        nb.set([i as i32 % 3 - 1, 0, i as i32 / 3 - 1], Some(c));
        nb.set([i as i32 % 3 - 1, 1, i as i32 / 3 - 1], Some(&above[i]));
    }
    let mut mesh = ChunkMesh::default();
    Mesher::new().mesh(&nb, &reg, &mut mesh);
    assert!(
        mesh.opaque.quads.len() > 100,
        "test terrain should be bumpy"
    );
    r.set_chunk(key(1, cy, 1), &mesh);

    // Nearly straight down at the middle of the chunk, from a few heights and offsets.
    let sky = RenderConfig::default()
        .sky
        .map(|c| (linear_to_srgb(c) * 255.0).round() as u8);
    let mut holes = 0;
    for (i, (ox, oy, oz)) in [
        (0.0, 30.0, 0.0),
        (3.0, 45.0, -2.0),
        (-4.0, 38.0, 5.0),
        (1.0, 60.0, 1.0),
    ]
    .into_iter()
    .enumerate()
    {
        let target = [
            48.0 + f64::from(i as u32) * 0.7,
            f64::from(gen_.surface_height(48, 48)),
            48.0,
        ];
        let cam = Camera {
            fov_y: 20f32.to_radians(),
            ..looking_at([48.0 + ox, target[1] + oy, 48.0 + oz], target)
        };
        r.render(&cam, DimensionId::OVERWORLD, None).unwrap();
        let img = r.read_pixels().unwrap();
        holes += img
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| near([p[0], p[1], p[2]], sky, 2))
            .count();
    }
    assert_eq!(holes, 0, "sky showed through the terrain");
}

#[test]
fn block_id_zero_is_air_in_the_test_registry() {
    let (reg, _) = test_registry();
    assert_eq!(
        reg.block_id(&"base:air".parse().unwrap()),
        Some(BlockId::AIR)
    );
}

/// A lone block of `id` in chunk (0,0,0), meshed.
fn lone_block(reg: &Registry, name: &str) -> ChunkMesh {
    let mut c = ChunkData::empty();
    c.set_block(
        LocalPos::new(8, 8, 8),
        reg.block_id(&name.parse().unwrap()).unwrap(),
        reg,
    );
    mesh_of(&c, reg)
}

#[test]
fn staging_buffers_are_reused_without_mixing_up_data() {
    let (reg, images) = test_registry();
    let Some(mut r) = renderer(&images, RenderConfig::default()) else {
        return;
    };
    let (stone, dirt) = (lone_block(&reg, "t:stone"), lone_block(&reg, "t:dirt"));
    let top_view = looking_at([8.5, 12.5, 8.5], [8.5; 3]);
    let want = |rgb: [u8; 3]| shaded(rgb, SHADE[Face::PosY.index()]);
    let mut checked = 0;
    // Hundreds of uploads through the ring, with the GPU allowed to run behind.
    for frame in 0..400 {
        let (mesh, rgb) = if frame % 3 == 0 {
            (&dirt, [134, 96, 67])
        } else {
            (&stone, STONE)
        };
        r.set_chunk(key(0, 0, 0), mesh);
        r.render(&top_view, DimensionId::OVERWORLD, None).unwrap();
        if frame % 7 == 0 {
            let got = pixel(&r.read_pixels().unwrap(), SIZE / 2, SIZE / 2);
            assert!(
                near(got, want(rgb), 3),
                "frame {frame}: got {got:?}, want {:?}",
                want(rgb)
            );
            checked += 1;
        }
    }
    assert!(checked > 50);
}

#[test]
fn a_mesh_larger_than_one_staging_buffer_uploads_whole() {
    let (reg, images) = test_registry();
    // A small budget gives a 1 MiB staging buffer; the mesh below is about 1.2 MB.
    let cfg = RenderConfig {
        upload_budget_bytes: 2 << 20,
        ..RenderConfig::default()
    };
    let Some(mut r) = renderer(&images, cfg) else {
        return;
    };
    let stone = reg.block_id(&"t:stone".parse().unwrap()).unwrap();
    let mut chunk = ChunkData::empty();
    for i in 0..32768 {
        let l = LocalPos::from_index(i).unwrap();
        if (l.x() + l.y() + l.z()).is_multiple_of(2) {
            chunk.set_block(l, stone, &reg);
        }
    }
    let mesh = mesh_of(&chunk, &reg);
    let bytes = mesh.quad_count() as u64 * 12;
    assert!(
        bytes > 1 << 20,
        "test mesh should exceed one staging buffer, got {bytes}"
    );
    r.set_chunk(key(0, 0, 0), &mesh);
    let cam = looking_at([16.0, 60.0, 16.0], [16.0, 16.0, 16.0]);
    let s = r.render(&cam, DimensionId::OVERWORLD, None).unwrap();
    assert_eq!(
        (s.uploaded_bytes, s.dropped_layers, s.pool_quads_used),
        (bytes, 0, mesh.quad_count() as u64)
    );
    let img = r.read_pixels().unwrap();
    let sky = RenderConfig::default()
        .sky
        .map(|c| (linear_to_srgb(c) * 255.0).round() as u8);
    assert!(
        !near(pixel(&img, SIZE / 2, SIZE / 2), sky, 2),
        "the big chunk should be visible"
    );
}

#[test]
fn staging_memory_stays_flat_over_many_uploads() {
    let (reg, images) = test_registry();
    let Some(mut r) = renderer(&images, RenderConfig::default()) else {
        return;
    };
    let (stone, dirt) = (lone_block(&reg, "t:stone"), lone_block(&reg, "t:dirt"));
    let cam = looking_at([8.5, 12.5, 8.5], [8.5; 3]);
    let run = |r: &mut Renderer, frames: usize| {
        for f in 0..frames {
            r.set_chunk(key(0, 0, 0), if f % 2 == 0 { &stone } else { &dirt });
            r.render(&cam, DimensionId::OVERWORLD, None).unwrap();
            r.wait_idle();
        }
    };
    run(&mut r, 20);
    let Some(before) = r.gpu_memory_bytes() else {
        eprintln!("this backend cannot report GPU memory: skipping the memory check");
        return;
    };
    run(&mut r, 300);
    let after = r.gpu_memory_bytes().unwrap();
    // A ring that never recycled would add a staging buffer (1 MiB or more) per frame.
    assert!(
        after < before + (16 << 20),
        "GPU memory grew from {before} to {after} bytes"
    );
}

#[test]
fn the_overlay_draws_in_gamma_space_over_the_scene() {
    let (_, images) = test_registry();
    let Some(mut r) = renderer(&images, RenderConfig::default()) else {
        return;
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/base");
    let font = Pack::read_dir(&dir)
        .unwrap()
        .read_font("monogram-extended")
        .unwrap();
    let ctx = egui::Context::default();
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "f".into(),
        std::sync::Arc::new(egui::FontData::from_owned(font)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("f".into());
    }
    ctx.set_fonts(fonts);
    ctx.set_pixels_per_point(1.0);
    let overlay = |t: f64| {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(64.0, 64.0),
            )),
            time: Some(t),
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            let rect = egui::Rect::from_min_size(egui::pos2(8.0, 8.0), egui::vec2(16.0, 16.0));
            ui.painter()
                .rect_filled(rect, 0.0, egui::Color32::from_gray(128));
        });
        Overlay {
            primitives: ctx.tessellate(out.shapes, out.pixels_per_point),
            textures_delta: out.textures_delta,
            pixels_per_point: out.pixels_per_point,
        }
    };
    r.render(
        &Camera::default(),
        DimensionId::OVERWORLD,
        Some(&mut overlay(0.0)),
    )
    .unwrap();
    let img = r.read_pixels().unwrap();
    // Opaque mid-grey must come out as 128 in the output bytes. Drawn into an sRGB target
    // without the gamma-space view it would come out near 188.
    assert!(
        near(pixel(&img, 16, 16), [128; 3], 2),
        "inside the rectangle: {:?}",
        pixel(&img, 16, 16)
    );
    let sky = RenderConfig::default()
        .sky
        .map(|c| (linear_to_srgb(c) * 255.0).round() as u8);
    assert!(
        near(pixel(&img, 48, 48), sky, 2),
        "outside it, the scene shows"
    );
}
