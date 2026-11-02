//! Terrain generation. Server only, pure: no GPU, no I/O.
//!
//! Phase 1 has two test terrains, flat and noise hills. The noise uses only
//! `+ - * /` and `floor` on `f64`, which are exactly specified, so the same
//! seed gives the same world on every OS and CPU.

mod noise;

use thiserror::Error;
use voxel_content::Registry;
use voxel_core::{BlockClasses, BlockId, CHUNK_SIZE, CHUNK_VOLUME, ChunkKey, Identifier};
use voxel_world::{ChunkData, PalettedArray};

#[derive(Debug, Error)]
pub enum WorldgenError {
    #[error("the terrain needs block {0}, which no loaded pack defines")]
    MissingBlock(&'static str),
}

/// The blocks terrain is made of. Resolved by name from the registry so no ID is hard-coded.
#[derive(Copy, Clone, Debug)]
pub struct TerrainBlocks {
    pub stone: BlockId,
    pub dirt: BlockId,
    pub grass: BlockId,
    pub sand: BlockId,
}

impl TerrainBlocks {
    pub fn from_registry(registry: &Registry) -> Result<Self, WorldgenError> {
        let find = |name: &'static str| {
            Identifier::new(name)
                .ok()
                .and_then(|id| registry.block_id(&id))
                .ok_or(WorldgenError::MissingBlock(name))
        };
        Ok(Self {
            stone: find("base:stone")?,
            dirt: find("base:dirt")?,
            grass: find("base:grass_block")?,
            sand: find("base:sand")?,
        })
    }
}

/// Hills made of a few layers of gradient noise.
#[derive(Copy, Clone, Debug)]
pub struct NoiseParams {
    /// Surface height with zero noise, in blocks.
    pub base: f64,
    /// Largest deviation from `base`, in blocks.
    pub amplitude: f64,
    /// Horizontal size of the biggest hills, in blocks.
    pub scale: f64,
    pub octaves: u32,
    /// Columns whose surface is at or below this are sand rather than grass.
    pub sea_level: i32,
}

impl Default for NoiseParams {
    fn default() -> Self {
        Self {
            base: 64.0,
            amplitude: 40.0,
            scale: 160.0,
            octaves: 4,
            sea_level: 58,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Terrain {
    /// A flat plain whose top block is at `surface`.
    Flat {
        surface: i32,
    },
    Noise(NoiseParams),
}

/// Generates chunks. Cheap to copy; holds no state beyond its settings.
#[derive(Copy, Clone, Debug)]
pub struct Generator {
    pub seed: u64,
    pub terrain: Terrain,
    pub blocks: TerrainBlocks,
    /// World height in chunks. Chunks outside `0..height_chunks` vertically are empty.
    pub height_chunks: i32,
}

/// Dirt (or sand) layer thickness under the surface block.
const SKIN: i32 = 3;
const SIZE: i32 = CHUNK_SIZE as i32;

impl Generator {
    /// The default world height: 4096 blocks.
    pub const DEFAULT_HEIGHT_CHUNKS: i32 = 128;

    pub fn new(seed: u64, terrain: Terrain, blocks: TerrainBlocks) -> Self {
        Self {
            seed,
            terrain,
            blocks,
            height_chunks: Self::DEFAULT_HEIGHT_CHUNKS,
        }
    }

    /// Y of the top solid block in the column at world `(x, z)`.
    pub fn surface_height(&self, x: i32, z: i32) -> i32 {
        match self.terrain {
            Terrain::Flat { surface } => surface,
            Terrain::Noise(p) => {
                let n = noise::fbm(
                    self.seed,
                    f64::from(x) / p.scale,
                    f64::from(z) / p.scale,
                    p.octaves,
                );
                (p.base + p.amplitude * n).floor() as i32
            }
        }
    }

    fn sea_level(&self) -> i32 {
        match self.terrain {
            Terrain::Flat { .. } => i32::MIN,
            Terrain::Noise(p) => p.sea_level,
        }
    }

    /// The block at height `y` in a column whose surface is `surface`.
    fn block_at(&self, y: i32, surface: i32) -> BlockId {
        let b = &self.blocks;
        let beach = surface <= self.sea_level();
        if y > surface {
            BlockId::AIR
        } else if y == surface {
            if beach { b.sand } else { b.grass }
        } else if y >= surface - SKIN {
            if beach { b.sand } else { b.dirt }
        } else {
            b.stone
        }
    }

    /// Generates the chunk at `key`. Chunks entirely above the terrain or outside the
    /// world's height are empty, and chunks entirely underground are a uniform stone
    /// chunk, which costs no block memory.
    pub fn chunk(&self, key: ChunkKey, classes: &impl BlockClasses) -> ChunkData {
        voxel_core::span!("generate chunk");
        let cy = key.pos.y;
        if !(0..self.height_chunks).contains(&cy) {
            return ChunkData::empty();
        }
        let origin = key.pos.origin();
        let mut heights = [0i32; CHUNK_SIZE * CHUNK_SIZE];
        let (mut lowest, mut highest) = (i32::MAX, i32::MIN);
        for lz in 0..SIZE {
            for lx in 0..SIZE {
                let h = self.surface_height(origin.x.wrapping_add(lx), origin.z.wrapping_add(lz));
                heights[(lz * SIZE + lx) as usize] = h;
                lowest = lowest.min(h);
                highest = highest.max(h);
            }
        }
        let (y0, y1) = (origin.y, origin.y.wrapping_add(SIZE - 1));
        if y0 > highest {
            return ChunkData::empty();
        }
        // Everything in the chunk is below every column's skin.
        if y1 < lowest.saturating_sub(SKIN) {
            return ChunkData::filled(self.blocks.stone, classes);
        }
        let mut values = vec![0u16; CHUNK_VOLUME];
        for ly in 0..SIZE {
            let y = y0.wrapping_add(ly);
            for lz in 0..SIZE {
                for lx in 0..SIZE {
                    let surface = heights[(lz * SIZE + lx) as usize];
                    // Index order matches `LocalPos`: x fastest, then z, then y.
                    values[(lx + SIZE * (lz + SIZE * ly)) as usize] = self.block_at(y, surface).0;
                }
            }
        }
        match PalettedArray::from_slice(&values) {
            Some(blocks) => ChunkData::from_blocks(blocks, classes),
            None => ChunkData::empty(),
        }
    }
}
