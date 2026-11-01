//! Binary greedy mesher for 32³ chunks. Pure: no GPU types and no I/O.
//!
//! Reads a chunk and its 26 neighbours, and emits 12-byte quads grouped by
//! render layer (opaque, cutout, translucent) and then by the six face
//! directions, the order the renderer draws them in.
//!
//! Face rules: a block's face is emitted when the block next to it is not
//! opaque, and, for cutout and translucent blocks, is not the same block.
//! Ambient occlusion comes from opaque blocks only.

mod mesher;
mod neighborhood;
mod quad;

pub use mesher::Mesher;
pub use neighborhood::Neighborhood;
pub use quad::{Decoded, FLAG_FLIP, Quad, ao_corner, ao_flip};

use voxel_content::RenderLayer;
use voxel_core::Face;

/// Quads of one render layer, grouped by face direction in [`Face::ALL`] order:
/// direction `d` is `quads[face_ranges[d]..face_ranges[d + 1]]`.
#[derive(Default, Clone, Debug)]
pub struct LayerMesh {
    pub quads: Vec<Quad>,
    pub face_ranges: [u32; 7],
}

impl LayerMesh {
    pub fn clear(&mut self) {
        self.quads.clear();
        self.face_ranges = [0; 7];
    }

    pub fn face_quads(&self, face: Face) -> &[Quad] {
        let d = face.index();
        &self.quads[self.face_ranges[d] as usize..self.face_ranges[d + 1] as usize]
    }
}

/// Everything the mesher produces for one chunk.
#[derive(Default, Clone, Debug)]
pub struct ChunkMesh {
    pub opaque: LayerMesh,
    pub cutout: LayerMesh,
    pub translucent: LayerMesh,
}

impl ChunkMesh {
    pub fn clear(&mut self) {
        self.opaque.clear();
        self.cutout.clear();
        self.translucent.clear();
    }

    pub fn layer(&self, layer: RenderLayer) -> &LayerMesh {
        match layer {
            RenderLayer::Opaque => &self.opaque,
            RenderLayer::Cutout => &self.cutout,
            RenderLayer::Translucent => &self.translucent,
        }
    }

    pub fn quad_count(&self) -> usize {
        self.opaque.quads.len() + self.cutout.quads.len() + self.translucent.quads.len()
    }
}
