//! The window: input, free camera and the frame loop.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use voxel_core::{BlockPos, DimensionId};
use voxel_render::{Camera, PresentPreference, RenderConfig, Renderer};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{
    DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::overlay::{DebugInfo, DebugOverlay};
use crate::stream::{StreamConfig, Streamer};
use crate::world::World;

pub struct Options {
    pub radius: i32,
    pub present: PresentPreference,
}

struct State {
    window: Arc<Window>,
    renderer: Renderer,
    streamer: Streamer,
    camera: Camera,
    keys: HashSet<KeyCode>,
    grabbed: bool,
    speed: f64,
    last_frame: Instant,
    frame_ms: VecDeque<f32>,
    overlay: DebugOverlay,
    overlay_on: bool,
    started: Instant,
    gpu_name: String,
    gpu_memory: Option<u64>,
    gpu_memory_at: Instant,
    radius: i32,
    last_stats: voxel_render::FrameStats,
}

struct App {
    world: World,
    options: Options,
    state: Option<State>,
    error: Option<anyhow::Error>,
}

pub fn run(world: World, options: Options) -> Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        world,
        options,
        state: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

impl App {
    fn start(&self, el: &ActiveEventLoop) -> Result<State> {
        let attrs = Window::default_attributes()
            .with_title("voxel  (F3: debug info)")
            .with_inner_size(LogicalSize::new(1280, 720));
        let window = Arc::new(el.create_window(attrs)?);
        let size = window.inner_size();
        let config = RenderConfig {
            present: self.options.present,
            ..RenderConfig::default()
        };
        let renderer = Renderer::new_window(
            Arc::clone(&window),
            (size.width, size.height),
            &self.world.images,
            config,
        )?;
        let (min_y, max_y) = self.world.vertical_chunks();
        let workers =
            std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(2).max(1));
        let streamer = Streamer::new(
            StreamConfig {
                radius: self.options.radius,
                min_y,
                max_y,
                workers,
            },
            DimensionId::OVERWORLD,
            self.world.generator,
            Arc::clone(&self.world.registry),
        );
        let ground = f64::from(self.world.generator.surface_height(0, 0));
        let overlay = DebugOverlay::new(self.world.font.clone());
        let gpu_name = renderer.gpu_description();
        Ok(State {
            window,
            renderer,
            streamer,
            camera: Camera {
                position: [0.5, ground + 12.0, 0.5],
                yaw: 0.0,
                pitch: -0.3,
                ..Camera::default()
            },
            keys: HashSet::new(),
            grabbed: false,
            speed: 14.0,
            last_frame: Instant::now(),
            frame_ms: VecDeque::new(),
            overlay,
            overlay_on: false,
            started: Instant::now(),
            gpu_name,
            gpu_memory: None,
            gpu_memory_at: Instant::now(),
            radius: self.options.radius,
            last_stats: Default::default(),
        })
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match self.start(el) {
            Ok(s) => {
                println!("graphics: {}", s.renderer.gpu_description());
                s.window.request_redraw();
                self.state = Some(s);
            }
            Err(e) => {
                self.error = Some(e);
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(s) = self.state.as_mut() else { return };
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => s.renderer.resize((size.width, size.height)),
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                match event.state {
                    ElementState::Pressed => {
                        if !event.repeat {
                            s.keys.insert(code);
                        }
                        if code == KeyCode::F3 && !event.repeat {
                            s.overlay_on = !s.overlay_on;
                        }
                        if code == KeyCode::Escape && !event.repeat {
                            if s.grabbed {
                                s.set_grab(false);
                            } else {
                                el.exit();
                            }
                        }
                    }
                    ElementState::Released => {
                        s.keys.remove(&code);
                    }
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                s.set_grab(true);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => f64::from(y),
                    MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                s.speed = (s.speed * 1.15f64.powf(lines)).clamp(1.0, 400.0);
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = s.frame() {
                    self.error = Some(e);
                    el.exit();
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        let Some(s) = self.state.as_mut() else { return };
        if let (DeviceEvent::MouseMotion { delta }, true) = (event, s.grabbed) {
            s.camera.yaw -= delta.0 as f32 * 0.0022;
            s.camera.pitch = (s.camera.pitch - delta.1 as f32 * 0.0022).clamp(-1.55, 1.55);
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(s) = &self.state {
            s.window.request_redraw();
        }
    }
}

impl State {
    fn set_grab(&mut self, on: bool) {
        let ok = if on {
            self.window.set_cursor_grab(CursorGrabMode::Locked).is_ok()
                || self
                    .window
                    .set_cursor_grab(CursorGrabMode::Confined)
                    .is_ok()
        } else {
            let _ = self.window.set_cursor_grab(CursorGrabMode::None);
            true
        };
        self.grabbed = on && ok;
        self.window.set_cursor_visible(!self.grabbed);
    }

    fn frame(&mut self) -> Result<()> {
        voxel_core::span!("game frame");
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f64().min(0.1);
        self.last_frame = now;
        self.frame_ms.push_back(dt as f32 * 1e3);
        if self.frame_ms.len() > 240 {
            self.frame_ms.pop_front();
        }

        // Fly: WASD along the horizontal view direction, Space/Shift up and down, Ctrl faster.
        let held = |k| f64::from(u8::from(self.keys.contains(&k)));
        let (f, r) = (self.camera.forward(), self.camera.right());
        let flat = glam_horizontal(f.x, f.z);
        let boost = if self.keys.contains(&KeyCode::ControlLeft) {
            4.0
        } else {
            1.0
        };
        let step = self.speed * boost * dt;
        let fwd = held(KeyCode::KeyW) - held(KeyCode::KeyS);
        let side = held(KeyCode::KeyD) - held(KeyCode::KeyA);
        let up = held(KeyCode::Space) - held(KeyCode::ShiftLeft);
        self.camera.position[0] += (flat.0 * fwd + f64::from(r.x) * side) * step;
        self.camera.position[1] += up * step;
        self.camera.position[2] += (flat.1 * fwd + f64::from(r.z) * side) * step;

        let o = self.camera.block_origin();
        self.streamer
            .update(BlockPos::new(o[0], o[1], o[2]).chunk(), &mut self.renderer);
        let mut sorted: Vec<f32> = self.frame_ms.iter().copied().collect();
        sorted.sort_by(f32::total_cmp);
        let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
        let p99 = sorted.get(sorted.len() * 99 / 100).copied().unwrap_or(0.0);

        let mut overlay = if self.overlay_on {
            // The allocator report walks every allocation, so it is refreshed twice a second.
            if self.gpu_memory_at.elapsed() > Duration::from_millis(500) {
                self.gpu_memory = self.renderer.gpu_memory_bytes();
                self.gpu_memory_at = Instant::now();
            }
            let size = self.window.inner_size();
            let info = DebugInfo {
                gpu: &self.gpu_name,
                frame_ms_median: median,
                frame_ms_p99: p99,
                position: self.camera.position,
                yaw: self.camera.yaw,
                pitch: self.camera.pitch,
                frame: self.last_stats,
                stream: self.streamer.stats(),
                gpu_memory_bytes: self.gpu_memory,
                radius: self.radius,
            };
            Some(self.overlay.frame(
                &info,
                (size.width, size.height),
                self.window.scale_factor() as f32,
                self.started.elapsed().as_secs_f64(),
            ))
        } else {
            None
        };
        self.last_stats =
            self.renderer
                .render(&self.camera, DimensionId::OVERWORLD, overlay.as_mut())?;
        crate::frame_mark();
        Ok(())
    }
}

/// Unit vector along the ground in the direction of (x, z), or zero if looking straight up/down.
fn glam_horizontal(x: f32, z: f32) -> (f64, f64) {
    let len = (x * x + z * z).sqrt();
    if len < 1e-4 {
        (0.0, 0.0)
    } else {
        (f64::from(x / len), f64::from(z / len))
    }
}
