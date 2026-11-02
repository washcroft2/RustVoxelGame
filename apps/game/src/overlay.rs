//! The F3 debug overlay: text over the scene, drawn with egui in the base pack's pixel font.
//!
//! egui is used for display only here (no input), so the window never feeds it events.

use std::sync::Arc;

use egui::{Color32, FontData, FontDefinitions, FontFamily, RichText, TextStyle};
use voxel_render::{FrameStats, Overlay};

use crate::stream::StreamStats;

/// The pixel font is drawn at this multiple of its native 16-pixel height, so its pixels stay square.
const FONT_SCALE: f32 = 2.0;

/// Everything the panel shows.
pub struct DebugInfo<'a> {
    pub gpu: &'a str,
    pub frame_ms_median: f32,
    pub frame_ms_p99: f32,
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub frame: FrameStats,
    pub stream: StreamStats,
    pub gpu_memory_bytes: Option<u64>,
    pub radius: i32,
}

pub struct DebugOverlay {
    ctx: egui::Context,
}

impl DebugOverlay {
    pub fn new(font: Vec<u8>) -> Self {
        let ctx = egui::Context::default();
        let mut fonts = FontDefinitions::default();
        fonts
            .font_data
            .insert("pack".into(), Arc::new(FontData::from_owned(font)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, "pack".into());
        }
        ctx.set_fonts(fonts);
        ctx.all_styles_mut(|style| {
            for font in style.text_styles.values_mut() {
                font.size = 16.0 * FONT_SCALE;
            }
        });
        Self { ctx }
    }

    /// Lays out the panel for a window of `size` pixels and returns what the renderer needs to draw it.
    pub fn frame(&mut self, info: &DebugInfo, size: (u32, u32), scale: f32, time: f64) -> Overlay {
        self.ctx.set_pixels_per_point(scale);
        let points = egui::vec2(size.0 as f32 / scale, size.1 as f32 / scale);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, points)),
            time: Some(time),
            ..Default::default()
        };
        let lines = lines(info);
        let output = self.ctx.run_ui(raw, |ui| {
            // An area is only as big as its contents, unlike a frame in the root ui.
            egui::Area::new(egui::Id::unique("f3-overlay"))
                .fixed_pos(egui::Pos2::ZERO)
                .interactable(false)
                .show(ui.ctx(), |ui| {
                    egui::Frame::new()
                        .fill(Color32::from_black_alpha(150))
                        .inner_margin(egui::Margin::same(6))
                        .show(ui, |ui| {
                            for line in &lines {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(line)
                                            .text_style(TextStyle::Monospace)
                                            .color(Color32::WHITE),
                                    )
                                    .wrap_mode(egui::TextWrapMode::Extend),
                                );
                            }
                        });
                });
        });
        let pixels_per_point = output.pixels_per_point;
        Overlay {
            primitives: self.ctx.tessellate(output.shapes, pixels_per_point),
            textures_delta: output.textures_delta,
            pixels_per_point,
        }
    }
}

fn lines(i: &DebugInfo) -> Vec<String> {
    let f = &i.frame;
    let s = &i.stream;
    let [x, y, z] = i.position;
    let chunk =
        voxel_core::BlockPos::new(x.floor() as i32, y.floor() as i32, z.floor() as i32).chunk();
    let mb = |bytes: u64| bytes as f64 / (1024.0 * 1024.0);
    let mut out = vec![
        format!("voxel (Phase 1 test world)  {}", i.gpu),
        format!(
            "{:.0} fps  {:.1} ms  (p99 {:.1} ms)",
            1e3 / i.frame_ms_median.max(0.01),
            i.frame_ms_median,
            i.frame_ms_p99
        ),
        format!("XYZ {x:.2} / {y:.2} / {z:.2}"),
        format!(
            "chunk {} {} {}   yaw {:.0}  pitch {:.0}",
            chunk.x,
            chunk.y,
            chunk.z,
            i.yaw.to_degrees(),
            i.pitch.to_degrees()
        ),
        format!(
            "chunks: {} drawn, {} resident, {} loaded, {} waiting to upload",
            f.visible_chunks, f.resident_chunks, s.loaded, f.pending_uploads
        ),
        format!(
            "draw: {} {} calls, {:.2}M quads",
            f.draw_calls,
            if f.indirect { "indirect" } else { "direct" },
            f.quads_drawn as f64 / 1e6
        ),
        format!(
            "streaming: {} jobs, remesh {:.0} us",
            s.in_flight, s.remesh_median_us
        ),
        format!(
            "quad pool: {:.1} MB in {} slab(s), {} ranges waiting to free",
            mb(f.pool_quads_used * 12),
            f.pool_slabs,
            f.pool_frees_waiting
        ),
        format!("render distance: {} chunks", i.radius),
    ];
    if let Some(bytes) = i.gpu_memory_bytes {
        out.insert(8, format!("GPU memory: {:.0} MB", mb(bytes)));
    }
    out
}
