use crate::{DimensionId, Face};

/// log2 of the chunk edge.
pub const CHUNK_BITS: u32 = 5;
/// Chunk edge in blocks.
pub const CHUNK_SIZE: usize = 1 << CHUNK_BITS;
/// Blocks per chunk.
pub const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;

const MASK: i32 = CHUNK_SIZE as i32 - 1;

/// A block position in a dimension. All three axes are `i32`; the world's
/// height bound is a property of the dimension, not of this type.
///
/// Arithmetic wraps rather than panics, because positions can come from the
/// network and must never bring the process down.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// The chunk containing this block. Shifts are arithmetic, so negative
    /// coordinates round down (block -1 is in chunk -1).
    pub const fn chunk(self) -> ChunkPos {
        ChunkPos::new(
            self.x >> CHUNK_BITS,
            self.y >> CHUNK_BITS,
            self.z >> CHUNK_BITS,
        )
    }

    pub const fn local(self) -> LocalPos {
        LocalPos::new(
            (self.x & MASK) as u32,
            (self.y & MASK) as u32,
            (self.z & MASK) as u32,
        )
    }

    pub const fn offset(self, face: Face) -> Self {
        let n = face.normal();
        Self::new(
            self.x.wrapping_add(n[0]),
            self.y.wrapping_add(n[1]),
            self.z.wrapping_add(n[2]),
        )
    }
}

/// A chunk position, in chunks.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct ChunkPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl ChunkPos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// The block at this chunk's minimum corner.
    pub const fn origin(self) -> BlockPos {
        BlockPos::new(
            self.x.wrapping_shl(CHUNK_BITS),
            self.y.wrapping_shl(CHUNK_BITS),
            self.z.wrapping_shl(CHUNK_BITS),
        )
    }

    pub const fn block(self, local: LocalPos) -> BlockPos {
        let o = self.origin();
        BlockPos::new(
            o.x.wrapping_add(local.x() as i32),
            o.y.wrapping_add(local.y() as i32),
            o.z.wrapping_add(local.z() as i32),
        )
    }

    pub const fn neighbor(self, face: Face) -> Self {
        let n = face.normal();
        Self::new(
            self.x.wrapping_add(n[0]),
            self.y.wrapping_add(n[1]),
            self.z.wrapping_add(n[2]),
        )
    }
}

/// A block's position inside its chunk, packed as `x | z << 5 | y << 10`.
///
/// That order (x fastest, then z, then y) is the storage order of chunk
/// arrays and the padded voxel array the mesher reads.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct LocalPos(u16);

impl LocalPos {
    /// Coordinates are masked to 0..32, so no input can be out of range.
    pub const fn new(x: u32, y: u32, z: u32) -> Self {
        let m = MASK as u32;
        Self(((x & m) | ((z & m) << CHUNK_BITS) | ((y & m) << (2 * CHUNK_BITS))) as u16)
    }

    /// `None` when `i` is not below [`CHUNK_VOLUME`].
    pub const fn from_index(i: usize) -> Option<Self> {
        if i < CHUNK_VOLUME {
            Some(Self(i as u16))
        } else {
            None
        }
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }

    pub const fn x(self) -> u32 {
        self.0 as u32 & MASK as u32
    }

    pub const fn z(self) -> u32 {
        (self.0 as u32 >> CHUNK_BITS) & MASK as u32
    }

    pub const fn y(self) -> u32 {
        (self.0 as u32 >> (2 * CHUNK_BITS)) & MASK as u32
    }
}

/// Identifies one chunk in one dimension. Every storage table, network
/// message and cache is keyed by this, never by [`ChunkPos`] alone.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct ChunkKey {
    pub dim: DimensionId,
    pub pos: ChunkPos,
}

impl ChunkKey {
    pub const fn new(dim: DimensionId, pos: ChunkPos) -> Self {
        Self { dim, pos }
    }
}
