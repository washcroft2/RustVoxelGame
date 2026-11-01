//! Chunks per second on one core: `cargo run --release -p voxel-worldgen --example gen_bench`.

use std::path::Path;
use std::time::Instant;

use voxel_content::{Pack, Registry};
use voxel_core::{ChunkKey, ChunkPos, DimensionId};
use voxel_worldgen::{Generator, NoiseParams, Terrain, TerrainBlocks};

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/base");
    let reg = Registry::build(&[Pack::read_dir(&dir).expect("base pack")]).expect("registry");
    let blocks = TerrainBlocks::from_registry(&reg).expect("terrain blocks");
    let g = Generator::new(1, Terrain::Noise(NoiseParams::default()), blocks);
    // One full column of the world height per (x, z), the way a server would request it.
    let t = Instant::now();
    let mut n = 0u32;
    for x in 0..16 {
        for z in 0..16 {
            for y in 0..g.height_chunks {
                let c = g.chunk(
                    ChunkKey::new(DimensionId::OVERWORLD, ChunkPos::new(x, y, z)),
                    &reg,
                );
                n += u32::from(!c.is_empty());
            }
        }
    }
    let s = t.elapsed().as_secs_f64();
    let total = 16 * 16 * g.height_chunks;
    println!(
        "{total} chunks ({n} non-empty) in {s:.2} s: {:.0} chunks/s on one core",
        f64::from(total) / s
    );
}
