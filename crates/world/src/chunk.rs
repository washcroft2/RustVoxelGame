use voxel_core::{BlockId, FluidId, LocalPos};

use crate::PalettedArray;

/// Counts kept up to date on every write so meshing and ticking can skip chunks in O(1).
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct ChunkSummary {
    /// Blocks that are not air.
    pub non_air: u32,
    /// Cells holding fluid.
    pub fluid_cells: u32,
}

/// One 32³ chunk. Share it as `Arc<ChunkData>` and write with
/// `Arc::make_mut`, so a write copies only while a snapshot is outstanding.
#[derive(Clone, Debug)]
pub struct ChunkData {
    blocks: PalettedArray,
    /// Separate layer so waterlogging needs no special block states.
    /// `None` while the chunk holds no fluid.
    fluid: Option<PalettedArray>,
    summary: ChunkSummary,
    /// Bumps on every change; meshes and replication compare against it.
    version: u64,
}

impl ChunkData {
    pub fn empty() -> Self {
        Self::filled(BlockId::AIR)
    }

    pub fn filled(block: BlockId) -> Self {
        let non_air = if block == BlockId::AIR {
            0
        } else {
            voxel_core::CHUNK_VOLUME as u32
        };
        Self {
            blocks: PalettedArray::uniform(block.0),
            fluid: None,
            summary: ChunkSummary {
                non_air,
                fluid_cells: 0,
            },
            version: 0,
        }
    }

    /// Wraps a ready-made block array, counting its non-air blocks.
    pub fn from_blocks(blocks: PalettedArray) -> Self {
        let air = blocks.count(BlockId::AIR.0);
        let non_air = (voxel_core::CHUNK_VOLUME - air) as u32;
        Self {
            blocks,
            fluid: None,
            summary: ChunkSummary {
                non_air,
                fluid_cells: 0,
            },
            version: 0,
        }
    }

    pub fn block(&self, pos: LocalPos) -> BlockId {
        BlockId(self.blocks.get(pos.index()))
    }

    pub fn fluid(&self, pos: LocalPos) -> FluidId {
        FluidId(self.fluid.as_ref().map_or(0, |f| f.get(pos.index())))
    }

    /// Sets a block and returns the old one. Bumps the version only on a real change.
    pub fn set_block(&mut self, pos: LocalPos, block: BlockId) -> BlockId {
        let old = BlockId(self.blocks.set(pos.index(), block.0));
        if old != block {
            self.version += 1;
            match (old == BlockId::AIR, block == BlockId::AIR) {
                (true, false) => self.summary.non_air += 1,
                (false, true) => self.summary.non_air -= 1,
                _ => {}
            }
        }
        old
    }

    /// Sets a fluid cell and returns the old one. Drops the layer when it empties.
    pub fn set_fluid(&mut self, pos: LocalPos, fluid: FluidId) -> FluidId {
        if self.fluid.is_none() && fluid == FluidId::NONE {
            return FluidId::NONE;
        }
        let layer = self.fluid.get_or_insert_with(|| PalettedArray::uniform(0));
        let old = FluidId(layer.set(pos.index(), fluid.0));
        if old != fluid {
            self.version += 1;
            match (old == FluidId::NONE, fluid == FluidId::NONE) {
                (true, false) => self.summary.fluid_cells += 1,
                (false, true) => self.summary.fluid_cells -= 1,
                _ => {}
            }
            if self.summary.fluid_cells == 0 {
                self.fluid = None;
            }
        }
        old
    }

    pub fn blocks(&self) -> &PalettedArray {
        &self.blocks
    }

    pub fn fluid_layer(&self) -> Option<&PalettedArray> {
        self.fluid.as_ref()
    }

    pub fn summary(&self) -> ChunkSummary {
        self.summary
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn is_empty(&self) -> bool {
        self.summary.non_air == 0
    }

    /// Re-packs both layers to their smallest form. Does not change the version, since contents are unchanged.
    pub fn compact(&mut self) {
        self.blocks.compact();
        if let Some(f) = &mut self.fluid {
            f.compact();
        }
    }
}
