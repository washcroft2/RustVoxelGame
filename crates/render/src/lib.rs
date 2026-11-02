//! The wgpu renderer. The only crate that imports `wgpu`.
//!
//! Terrain is drawn from 12-byte quads in storage-buffer slabs (vertex
//! pulling): there are no vertex buffers. Each frame the CPU culls resident
//! chunks against the view frustum and writes one compact indirect draw list
//! per slab. Rendering is camera-relative: chunk origins are subtracted as
//! integers before they become `f32`.

mod camera;
mod error;
mod gpu;
mod pool;
mod renderer;
mod texture;

pub use camera::{Camera, Frustum};
pub use error::RenderError;
pub use renderer::{FrameStats, Overlay, PresentPreference, RenderConfig, Renderer};
pub use texture::mip_count;
