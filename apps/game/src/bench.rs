//! A scripted flythrough with no window: streams the world around a moving camera, draws to
//! memory, and reports frame times. Stands in for `cargo xtask scenario` until that exists.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use voxel_core::{BlockPos, DimensionId};
use voxel_render::{Camera, RenderConfig, Renderer};

use crate::overlay::{DebugInfo, DebugOverlay};
use crate::stream::{StreamConfig, Streamer};
use crate::world::World;

pub struct BenchOptions {
    pub radius: i32,
    pub seconds: f64,
    pub size: (u32, u32),
    /// Blocks per second the camera flies at.
    pub speed: f64,
    pub screenshot: Option<PathBuf>,
    /// Draw the debug overlay on the screenshot.
    pub overlay: bool,
}

fn percentile(sorted: &[f32], p: f32) -> f32 {
    sorted
        .get(((sorted.len() as f32 - 1.0) * p).round() as usize)
        .copied()
        .unwrap_or(0.0)
}

pub fn run(world: World, opts: BenchOptions) -> Result<()> {
    let mut renderer = Renderer::new_headless(opts.size, &world.images, RenderConfig::default())
        .context("creating a headless renderer")?;
    println!("graphics: {}", renderer.gpu_description());
    let (min_y, max_y) = world.vertical_chunks();
    let workers =
        std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(2).max(1));
    let mut streamer = Streamer::new(
        StreamConfig {
            radius: opts.radius,
            min_y,
            max_y,
            workers,
        },
        DimensionId::OVERWORLD,
        world.generator,
        Arc::clone(&world.registry),
    );
    let ground = f64::from(world.generator.surface_height(0, 0));
    let mut camera = Camera {
        position: [0.5, ground + 25.0, 0.5],
        yaw: -std::f32::consts::FRAC_PI_2, // looking along +X
        pitch: -0.25,
        ..Camera::default()
    };
    let tick = |streamer: &mut Streamer, renderer: &mut Renderer, camera: &Camera| -> Result<_> {
        let o = camera.block_origin();
        streamer.update(BlockPos::new(o[0], o[1], o[2]).chunk(), renderer);
        let s = renderer.render(camera, DimensionId::OVERWORLD, None)?;
        renderer.wait_idle();
        Ok(s)
    };

    // Load everything in range first, so the flight measures steady-state streaming.
    let t0 = Instant::now();
    let mut frames = 0;
    let mut last = Default::default();
    while t0.elapsed().as_secs_f64() < 300.0 {
        last = tick(&mut streamer, &mut renderer, &camera)?;
        frames += 1;
        if streamer.settled() && last.pending_uploads == 0 {
            break;
        }
    }
    let st = streamer.stats();
    println!(
        "settled in {:.2} s ({frames} frames): {} chunks loaded, {} meshed, {} resident on the GPU, {:.2}M quads, remesh median {:.0} us",
        t0.elapsed().as_secs_f64(),
        st.loaded,
        st.meshed,
        last.resident_chunks,
        st.quads as f64 / 1e6,
        st.remesh_median_us
    );
    println!(
        "draw: {} chunks visible, {} indirect draws, {:.2}M quads this frame",
        last.visible_chunks,
        last.draw_calls,
        last.quads_drawn as f64 / 1e6
    );

    // Fly in a straight line at a fixed pace, so every run covers the same ground.
    let (dt, mut times) = (1.0 / 60.0, Vec::new());
    let (mut peak_pending, mut peak_in_flight) = (0, 0);
    let start = Instant::now();
    let mut sim = 0.0;
    while start.elapsed().as_secs_f64() < opts.seconds {
        sim += dt;
        camera.position[0] = 0.5 + opts.speed * sim;
        let t = Instant::now();
        let s = tick(&mut streamer, &mut renderer, &camera)?;
        times.push(t.elapsed().as_secs_f32() * 1e3);
        peak_pending = peak_pending.max(s.pending_uploads);
        peak_in_flight = peak_in_flight.max(streamer.stats().in_flight);
        last = s;
    }
    times.sort_by(f32::total_cmp);
    let mean = times.iter().sum::<f32>() / times.len().max(1) as f32;
    println!(
        "flythrough {:.1} s at {:.0} blocks/s, {} frames, {}x{}: mean {mean:.2} ms ({:.0} fps), median {:.2}, p99 {:.2}, max {:.2} ms, p99/median {:.2}",
        opts.seconds,
        opts.speed,
        times.len(),
        opts.size.0,
        opts.size.1,
        1e3 / mean.max(1e-3),
        percentile(&times, 0.5),
        percentile(&times, 0.99),
        times.last().copied().unwrap_or(0.0),
        percentile(&times, 0.99) / percentile(&times, 0.5).max(1e-3)
    );
    println!(
        "streaming during flight: peak {peak_pending} meshes waiting to upload, peak {peak_in_flight} jobs in flight, pool {} quads in {} slab(s)",
        last.pool_quads_used, last.pool_slabs
    );

    if let Some(path) = opts.screenshot {
        let o = camera.block_origin();
        streamer.update(BlockPos::new(o[0], o[1], o[2]).chunk(), &mut renderer);
        if opts.overlay {
            let info = DebugInfo {
                gpu: &renderer.gpu_description(),
                frame_ms_median: percentile(&times, 0.5),
                frame_ms_p99: percentile(&times, 0.99),
                position: camera.position,
                yaw: camera.yaw,
                pitch: camera.pitch,
                frame: last,
                stream: streamer.stats(),
                gpu_memory_bytes: renderer.gpu_memory_bytes(),
                radius: opts.radius,
            };
            // egui sizes a new panel on its first frame and shows it from the second, and the
            // font texture travels with the first. Every overlay must reach `render`, in order.
            let mut overlay = DebugOverlay::new(world.font.clone());
            for t in [0.0, 0.1] {
                let mut o = overlay.frame(&info, opts.size, 1.0, t);
                renderer.render(&camera, DimensionId::OVERWORLD, Some(&mut o))?;
            }
        } else {
            renderer.render(&camera, DimensionId::OVERWORLD, None)?;
        }
        renderer.wait_idle();
        let pixels = renderer.read_pixels()?;
        let (w, h) = opts.size;
        image::RgbaImage::from_raw(w, h, pixels)
            .context("screenshot size mismatch")?
            .save(&path)
            .with_context(|| format!("writing {}", path.display()))?;
        println!("screenshot: {}", path.display());
    }
    Ok(())
}
