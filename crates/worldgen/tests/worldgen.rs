use std::path::Path;

use proptest::prelude::*;
use voxel_content::{Pack, Registry};
use voxel_core::{BlockId, CHUNK_VOLUME, ChunkKey, ChunkPos, DimensionId, LocalPos};
use voxel_worldgen::{Generator, NoiseParams, Terrain, TerrainBlocks, WorldgenError};

fn registry() -> Registry {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/base");
    Registry::build(&[Pack::read_dir(&dir).unwrap()]).unwrap()
}

fn generator(reg: &Registry, seed: u64, terrain: Terrain) -> Generator {
    Generator::new(seed, terrain, TerrainBlocks::from_registry(reg).unwrap())
}

fn key(x: i32, y: i32, z: i32) -> ChunkKey {
    ChunkKey::new(DimensionId::OVERWORLD, ChunkPos::new(x, y, z))
}

/// Order-sensitive hash of a chunk's blocks, for the golden checks.
fn digest(blocks: &[u16]) -> u64 {
    blocks.iter().fold(0xCBF2_9CE4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3)
    })
}

#[test]
fn flat_world_has_the_expected_layers() {
    let reg = registry();
    let g = generator(&reg, 1, Terrain::Flat { surface: 40 });
    // Surface 40 is in chunk y = 1 (blocks 32..64), local y = 8.
    let c = g.chunk(key(3, 1, -2), &reg);
    let at = |y| c.block(LocalPos::new(5, y, 9));
    assert_eq!(at(8), g.blocks.grass);
    for y in 5..8 {
        assert_eq!(at(y), g.blocks.dirt);
    }
    assert_eq!(at(4), g.blocks.stone);
    assert_eq!(at(9), BlockId::AIR);
    // Below is uniform stone, above is empty.
    assert_eq!(
        g.chunk(key(3, 0, -2), &reg).blocks().uniform_value(),
        Some(g.blocks.stone.0)
    );
    assert!(g.chunk(key(3, 2, -2), &reg).is_empty());
}

#[test]
fn outside_the_world_height_is_empty() {
    let reg = registry();
    let g = generator(&reg, 1, Terrain::Flat { surface: 40 });
    assert!(g.chunk(key(0, -1, 0), &reg).is_empty());
    assert!(g.chunk(key(0, g.height_chunks, 0), &reg).is_empty());
    assert!(g.chunk(key(0, i32::MIN, 0), &reg).is_empty());
}

#[test]
fn generation_is_deterministic_and_seed_dependent() {
    let reg = registry();
    let params = Terrain::Noise(NoiseParams::default());
    let (a, b) = (generator(&reg, 7, params), generator(&reg, 7, params));
    let other = generator(&reg, 8, params);
    let k = key(5, 2, -3);
    assert_eq!(
        a.chunk(k, &reg).blocks().to_vec(),
        b.chunk(k, &reg).blocks().to_vec()
    );
    assert_ne!(
        a.chunk(k, &reg).blocks().to_vec(),
        other.chunk(k, &reg).blocks().to_vec()
    );
}

/// Fixed seed, fixed output: catches accidental changes to the noise or layering.
/// If you change terrain on purpose, update these digests in the same commit.
#[test]
fn golden_chunks() {
    let reg = registry();
    let g = generator(&reg, 12345, Terrain::Noise(NoiseParams::default()));
    let got: Vec<u64> = [key(0, 2, 0), key(-7, 2, 4), key(40, 2, -40)]
        .iter()
        .map(|&k| digest(&g.chunk(k, &reg).blocks().to_vec()))
        .collect();
    assert_eq!(got, GOLDEN, "golden digests changed: {got:#x?}");
}
const GOLDEN: [u64; 3] = [
    0x412f_062f_825f_b191,
    0xbdde_ae01_2259_160f,
    0x7631_bd94_28d5_57c3,
];

#[test]
fn hills_are_continuous_across_chunk_edges() {
    let reg = registry();
    let g = generator(&reg, 99, Terrain::Noise(NoiseParams::default()));
    // Surface height is a function of world x/z, so the two chunks either side of
    // an edge must agree with it, and neighbouring columns differ by a bounded step.
    for x in [-33, -1, 31, 32, 63] {
        let (a, b) = (g.surface_height(x, 10), g.surface_height(x + 1, 10));
        assert!((a - b).abs() <= 4, "cliff of {} at x={x}", (a - b).abs());
    }
    let cy = g.surface_height(32, 10).div_euclid(32);
    let left = g.chunk(key(0, cy, 0), &reg);
    let right = g.chunk(key(1, cy, 0), &reg);
    let top = |c: &voxel_world::ChunkData, lx: u32| {
        (0..32)
            .rev()
            .find(|&y| c.block(LocalPos::new(lx, y, 10)) != BlockId::AIR)
    };
    let (l, r) = (top(&left, 31), top(&right, 0));
    if let (Some(l), Some(r)) = (l, r) {
        assert_eq!(l as i32 + cy * 32, g.surface_height(31, 10));
        assert_eq!(r as i32 + cy * 32, g.surface_height(32, 10));
    }
}

#[test]
fn the_default_hills_stay_inside_the_world() {
    let reg = registry();
    let g = generator(&reg, 5, Terrain::Noise(NoiseParams::default()));
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    for i in 0..20_000 {
        let h = g.surface_height(i * 37 % 9001 - 4500, i * 91 % 7001 - 3500);
        lo = lo.min(h);
        hi = hi.max(h);
    }
    assert!(lo >= 20 && hi <= 110, "heights {lo}..{hi}");
    assert!(hi - lo > 30, "terrain should not be flat: {lo}..{hi}");
}

#[test]
fn beaches_are_sand() {
    let reg = registry();
    let g = generator(
        &reg,
        5,
        Terrain::Noise(NoiseParams {
            sea_level: 200,
            ..NoiseParams::default()
        }),
    );
    let h = g.surface_height(0, 0);
    let c = g.chunk(key(0, h.div_euclid(32), 0), &reg);
    assert_eq!(
        c.block(LocalPos::new(0, h.rem_euclid(32) as u32, 0)),
        g.blocks.sand
    );
}

#[test]
fn missing_base_blocks_are_an_error() {
    let pack = Pack::from_parts("id = \"t\"\nversion = \"1\"\n", &[], vec![]).unwrap();
    let reg = Registry::build(&[pack]).unwrap();
    assert!(matches!(
        TerrainBlocks::from_registry(&reg),
        Err(WorldgenError::MissingBlock(_))
    ));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn any_chunk_key_generates_consistent_data(x: i32, y in -5i32..140, z: i32, seed: u64, flat: bool) {
        let reg = registry();
        let terrain = if flat { Terrain::Flat { surface: 70 } } else { Terrain::Noise(NoiseParams::default()) };
        let c = generator(&reg, seed, terrain).chunk(key(x, y, z), &reg);
        let blocks = c.blocks().to_vec();
        prop_assert_eq!(blocks.len(), CHUNK_VOLUME);
        prop_assert_eq!(c.summary().non_air as usize, blocks.iter().filter(|&&b| b != 0).count());
    }
}

#[test]
fn golden_chunks_are_not_trivial() {
    let reg = registry();
    let g = generator(&reg, 12345, Terrain::Noise(NoiseParams::default()));
    for k in [key(0, 2, 0), key(-7, 2, 4), key(40, 2, -40)] {
        let s = g.chunk(k, &reg).summary();
        let n = s.non_air as usize;
        assert!(n > 0 && n < CHUNK_VOLUME, "{k:?} is empty or solid: {n}");
    }
}
