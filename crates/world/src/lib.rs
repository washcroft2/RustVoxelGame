//! Chunk storage: palette-compressed block and fluid layers, plus the
//! per-chunk summary and version that meshing and replication key on.

mod chunk;
mod palette;

pub use chunk::{ChunkData, ChunkSummary};
pub use palette::PalettedArray;
