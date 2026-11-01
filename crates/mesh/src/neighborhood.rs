use voxel_core::{BlockId, CHUNK_SIZE, LocalPos};
use voxel_world::ChunkData;

/// A chunk and the 26 around it, which is all the mesher reads.
///
/// A missing neighbour (not loaded yet) reads as air, so the faces on that
/// side are emitted and hidden again by a remesh when the neighbour arrives.
pub struct Neighborhood<'a> {
    chunks: [Option<&'a ChunkData>; 27],
}

const fn slot(dx: i32, dy: i32, dz: i32) -> usize {
    ((dx + 1) + 3 * ((dz + 1) + 3 * (dy + 1))) as usize
}

impl<'a> Neighborhood<'a> {
    pub fn new(center: &'a ChunkData) -> Self {
        let mut chunks = [None; 27];
        chunks[slot(0, 0, 0)] = Some(center);
        Self { chunks }
    }

    /// Sets the neighbour at `offset` (each component -1, 0 or 1; the centre cannot be replaced).
    pub fn set(&mut self, offset: [i32; 3], chunk: Option<&'a ChunkData>) {
        if offset.iter().all(|c| (-1..=1).contains(c)) && offset != [0, 0, 0] {
            self.chunks[slot(offset[0], offset[1], offset[2])] = chunk;
        }
    }

    pub fn get(&self, dx: i32, dy: i32, dz: i32) -> Option<&'a ChunkData> {
        self.chunks[slot(dx, dy, dz)]
    }

    pub fn center(&self) -> &'a ChunkData {
        // `new` always fills the centre slot and `set` never replaces it.
        self.chunks[slot(0, 0, 0)].expect("centre chunk is always present")
    }

    /// The block at `p`, in chunk-local coordinates from -1 to 32 (one block of border).
    /// Anything farther out, or in a missing chunk, is air.
    pub fn block_at(&self, p: [i32; 3]) -> BlockId {
        let n = CHUNK_SIZE as i32;
        let off = |c: i32| {
            if c < 0 {
                -1
            } else if c >= n {
                1
            } else {
                0
            }
        };
        if p.iter().any(|&c| !(-1..=n).contains(&c)) {
            return BlockId::AIR;
        }
        match self.get(off(p[0]), off(p[1]), off(p[2])) {
            Some(c) => c.block(LocalPos::new(
                (p[0] & 31) as u32,
                (p[1] & 31) as u32,
                (p[2] & 31) as u32,
            )),
            None => BlockId::AIR,
        }
    }
}
