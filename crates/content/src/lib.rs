//! Content packs and the registries built from them.
//!
//! The base game is the pack at `packs/base`, loaded through the same path
//! as any mod. Packs are data only; every name is `namespace:path`.
//! Loading never panics on bad input: it returns a [`ContentError`] that
//! names the offending file.

mod error;
mod pack;
mod registry;
mod textures;

pub use error::ContentError;
pub use pack::{BlockSource, Pack, PackManifest};
pub use registry::{BlockDef, Registry, RenderLayer, TextureId};
pub use textures::{TextureImage, load_texture_images};
