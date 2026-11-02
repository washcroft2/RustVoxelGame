//! Shared vocabulary for every other crate: coordinates, IDs, faces, ticks.
//!
//! No heavy dependencies and no I/O, so anything can depend on it.

mod face;
mod id;
mod pos;
mod profile;

/// Used by `span!` in profile builds.
#[cfg(feature = "profile")]
#[doc(hidden)]
pub use tracing as __tracing;

pub use face::{Face, TANGENT_U, TANGENT_V};
pub use id::{
    BlockClass, BlockClasses, BlockId, DimensionId, FluidId, Identifier, IdentifierError, Tick,
};
pub use pos::{BlockPos, CHUNK_BITS, CHUNK_SIZE, CHUNK_VOLUME, ChunkKey, ChunkPos, LocalPos};
