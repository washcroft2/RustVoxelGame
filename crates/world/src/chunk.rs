use voxel_core::{
    BlockClass, BlockClasses, BlockId, CHUNK_SIZE, CHUNK_VOLUME, FluidId, LocalPos, TANGENT_U,
    TANGENT_V,
};

use crate::PalettedArray;

/// Counts kept up to date on every write so meshing and ticking can skip chunks in O(1).
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct ChunkSummary {
    /// Blocks that are not air (the sum of the three class counts).
    pub non_air: u32,
    pub opaque: u32,
    pub cutout: u32,
    pub translucent: u32,
    /// Cells holding fluid.
    pub fluid_cells: u32,
}

impl ChunkSummary {
    fn count_mut(&mut self, class: BlockClass) -> Option<&mut u32> {
        match class {
            BlockClass::Air => None,
            BlockClass::Opaque => Some(&mut self.opaque),
            BlockClass::Cutout => Some(&mut self.cutout),
            BlockClass::Translucent => Some(&mut self.translucent),
        }
    }
}

const COLUMNS: usize = CHUNK_SIZE * CHUNK_SIZE;

/// Per-column bitmasks of opaque blocks along each axis: `cols[axis][v * 32 + u]`,
/// bit = depth along `axis`, with `(u, v)` the tangent axes from `TANGENT_U`/`TANGENT_V`.
/// Kept current on every write, so meshing never rebuilds them.
#[derive(Clone, Debug, PartialEq, Eq)]
struct OpacityMasks {
    cols: [[u32; COLUMNS]; 3],
}

impl OpacityMasks {
    fn filled(bits: u32) -> Box<Self> {
        Box::new(Self {
            cols: [[bits; COLUMNS]; 3],
        })
    }

    fn toggle(&mut self, p: [u32; 3]) {
        for axis in 0..3 {
            let (u, v) = (p[TANGENT_U[axis]], p[TANGENT_V[axis]]);
            self.cols[axis][(v * 32 + u) as usize] ^= 1 << p[axis];
        }
    }
}

/// One 32³ chunk. Share it as `Arc<ChunkData>` and write with
/// `Arc::make_mut`, so a write copies only while a snapshot is outstanding.
#[derive(Clone, Debug)]
pub struct ChunkData {
    blocks: PalettedArray,
    /// Separate layer so waterlogging needs no special block states.
    /// `None` while the chunk holds no fluid.
    fluid: Option<PalettedArray>,
    /// Present only while the chunk is a mix of opaque and non-opaque blocks;
    /// an all-or-nothing chunk is described by the summary alone.
    opacity: Option<Box<OpacityMasks>>,
    summary: ChunkSummary,
    /// Bumps on every change; meshes and replication compare against it.
    version: u64,
}

impl ChunkData {
    pub fn empty() -> Self {
        Self {
            blocks: PalettedArray::uniform(BlockId::AIR.0),
            fluid: None,
            opacity: None,
            summary: ChunkSummary::default(),
            version: 0,
        }
    }

    /// A chunk of one block type.
    pub fn filled(block: BlockId, classes: &impl BlockClasses) -> Self {
        Self::from_blocks(PalettedArray::uniform(block.0), classes)
    }

    /// Wraps a ready-made block array, counting classes and building the opacity masks.
    pub fn from_blocks(blocks: PalettedArray, classes: &impl BlockClasses) -> Self {
        let mut summary = ChunkSummary::default();
        let mut masks = OpacityMasks::filled(0);
        // Classify each distinct value once; a uniform array has one.
        let mut cache: Vec<(u16, BlockClass)> = Vec::new();
        for i in 0..CHUNK_VOLUME {
            let id = blocks.get(i);
            let class = match cache.iter().find(|(v, _)| *v == id) {
                Some(&(_, c)) => c,
                None => {
                    let c = classes.class(BlockId(id));
                    cache.push((id, c));
                    c
                }
            };
            if let Some(n) = summary.count_mut(class) {
                *n += 1;
                summary.non_air += 1;
            }
            if class == BlockClass::Opaque {
                let p = LocalPos::from_index(i).map_or([0; 3], |l| [l.x(), l.y(), l.z()]);
                masks.toggle(p);
            }
        }
        let mixed = summary.opaque > 0 && (summary.opaque as usize) < CHUNK_VOLUME;
        Self {
            blocks,
            fluid: None,
            opacity: mixed.then_some(masks),
            summary,
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
    pub fn set_block(
        &mut self,
        pos: LocalPos,
        block: BlockId,
        classes: &impl BlockClasses,
    ) -> BlockId {
        let old = BlockId(self.blocks.set(pos.index(), block.0));
        if old == block {
            return old;
        }
        self.version += 1;
        let (old_class, new_class) = (classes.class(old), classes.class(block));
        if old_class == new_class {
            return old;
        }
        let was_opaque = self.summary.opaque;
        if let Some(n) = self.summary.count_mut(old_class) {
            *n -= 1;
            self.summary.non_air -= 1;
        }
        if let Some(n) = self.summary.count_mut(new_class) {
            *n += 1;
            self.summary.non_air += 1;
        }
        if (old_class == BlockClass::Opaque) != (new_class == BlockClass::Opaque) {
            let p = [pos.x(), pos.y(), pos.z()];
            let masks = self.opacity.get_or_insert_with(|| {
                // No masks means the chunk was entirely one way before this write.
                OpacityMasks::filled(if was_opaque == 0 { 0 } else { u32::MAX })
            });
            masks.toggle(p);
            if self.summary.opaque == 0 || self.summary.opaque as usize == CHUNK_VOLUME {
                self.opacity = None;
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

    /// Bitmask of opaque blocks along `axis` (0 = X, 1 = Y, 2 = Z) in the column at
    /// tangent coordinates `(u, v)`; bit `d` is the block at depth `d`.
    /// Out-of-range `u`/`v` are masked into the chunk.
    pub fn opaque_column(&self, axis: usize, u: usize, v: usize) -> u32 {
        if self.summary.opaque == 0 {
            return 0;
        }
        match &self.opacity {
            Some(m) => m
                .cols
                .get(axis)
                .map_or(0, |c| c[((v & 31) << 5) | (u & 31)]),
            None => u32::MAX,
        }
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
