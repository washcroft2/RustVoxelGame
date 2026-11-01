//! The greedy mesher against a brute-force reference (plan section 11:
//! "mesher face coverage vs brute force"). The reference reads blocks from a
//! world-coordinate function and never touches the mesher's masks, so a bug
//! in the cached columns or the bit tricks shows up as a mismatch.

use std::collections::HashMap;

use proptest::prelude::*;
use voxel_content::{Pack, Registry, RenderLayer};
use voxel_core::{
    BlockClass, BlockClasses, BlockId, CHUNK_VOLUME, Face, LocalPos, TANGENT_U, TANGENT_V,
};
use voxel_mesh::{ChunkMesh, Mesher, Neighborhood, ao_corner, ao_flip};
use voxel_world::{ChunkData, PalettedArray};

const N: i32 = 32;

fn registry() -> Registry {
    let block =
        |tex: &str, layer: &str| format!("layer = \"{layer}\"\n[textures]\nall = \"{tex}\"\n");
    let blocks = vec![
        ("dirt".to_string(), block("t_dirt", "opaque")),
        ("glass".to_string(), block("t_glass", "translucent")),
        ("leaves".to_string(), block("t_leaves", "cutout")),
        ("stone".to_string(), block("t_stone", "opaque")),
    ];
    let textures = ["t_dirt", "t_glass", "t_leaves", "t_stone"]
        .map(String::from)
        .to_vec();
    let pack = Pack::from_parts("id = \"t\"\nversion = \"1\"\n", &blocks, textures).unwrap();
    Registry::build(&[pack]).unwrap()
}

/// Test block ids: 1 dirt, 2 glass, 3 leaves, 4 stone (see `registry`).
const PALETTE: [u16; 4] = [4, 1, 3, 2];

/// A 3×3×3 block of chunks, some possibly absent, with contents from a function of
/// world-local coordinates (the centre chunk spans 0..32).
struct Scene {
    chunks: Vec<Option<ChunkData>>,
    world: Box<dyn Fn([i32; 3]) -> u16>,
}

fn slot(o: [i32; 3]) -> usize {
    ((o[0] + 1) + 3 * ((o[2] + 1) + 3 * (o[1] + 1))) as usize
}

impl Scene {
    fn new(
        reg: &Registry,
        world: impl Fn([i32; 3]) -> u16 + 'static,
        absent: u32,
        by_edits: bool,
    ) -> Self {
        let mut chunks = Vec::new();
        for dy in -1..=1 {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let i = chunks.len();
                    let is_center = (dx, dy, dz) == (0, 0, 0);
                    if !is_center && absent >> i & 1 == 1 {
                        chunks.push(None);
                        continue;
                    }
                    let at = |l: usize| {
                        let l = LocalPos::from_index(l).unwrap();
                        world([
                            dx * N + l.x() as i32,
                            dy * N + l.y() as i32,
                            dz * N + l.z() as i32,
                        ])
                    };
                    let c = if by_edits {
                        // Start from uniform air and write block by block, so the
                        // incrementally maintained masks are what gets meshed.
                        let mut c = ChunkData::empty();
                        for l in 0..CHUNK_VOLUME {
                            let v = at(l);
                            if v != 0 {
                                c.set_block(LocalPos::from_index(l).unwrap(), BlockId(v), reg);
                            }
                        }
                        c
                    } else {
                        let vals: Vec<u16> = (0..CHUNK_VOLUME).map(at).collect();
                        ChunkData::from_blocks(PalettedArray::from_slice(&vals).unwrap(), reg)
                    };
                    chunks.push(Some(c));
                }
            }
        }
        Self {
            chunks,
            world: Box::new(world),
        }
    }

    fn neighborhood(&self) -> Neighborhood<'_> {
        let mut nb = Neighborhood::new(self.chunks[slot([0, 0, 0])].as_ref().unwrap());
        for dy in -1..=1 {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    nb.set([dx, dy, dz], self.chunks[slot([dx, dy, dz])].as_ref());
                }
            }
        }
        nb
    }

    /// The reference's view of the world: a missing chunk is air.
    fn at(&self, p: [i32; 3]) -> BlockId {
        let o = p.map(|c| c.div_euclid(N));
        if o.iter().any(|c| !(-1..=1).contains(c)) || self.chunks[slot(o)].is_none() {
            return BlockId::AIR;
        }
        BlockId((self.world)(p))
    }
}

type Cell = (RenderLayer, Face, [i32; 3]);

/// Every visible face of the centre chunk, as `(texture, ao)`.
fn reference(s: &Scene, reg: &Registry) -> HashMap<Cell, (u16, u8)> {
    let opaque = |p| reg.class(s.at(p)) == BlockClass::Opaque;
    let mut out = HashMap::new();
    for z in 0..N {
        for y in 0..N {
            for x in 0..N {
                let p = [x, y, z];
                let b = s.at(p);
                let layer = match reg.class(b) {
                    BlockClass::Air => continue,
                    BlockClass::Opaque => RenderLayer::Opaque,
                    BlockClass::Cutout => RenderLayer::Cutout,
                    BlockClass::Translucent => RenderLayer::Translucent,
                };
                for face in Face::ALL {
                    let n = face.normal();
                    let front = [p[0] + n[0], p[1] + n[1], p[2] + n[2]];
                    if opaque(front) || s.at(front) == b {
                        continue;
                    }
                    let (au, av) = (TANGENT_U[face.axis()], TANGENT_V[face.axis()]);
                    let off = |du: i32, dv: i32| {
                        let mut q = front;
                        q[au] += du;
                        q[av] += dv;
                        q
                    };
                    let mut ao = 0u8;
                    for cv in 0..2 {
                        for cu in 0..2 {
                            let su = if cu == 0 { -1 } else { 1 };
                            let sv = if cv == 0 { -1 } else { 1 };
                            let c = ao_corner(
                                opaque(off(su, 0)),
                                opaque(off(0, sv)),
                                opaque(off(su, sv)),
                            );
                            ao |= c << (2 * (cu + 2 * cv));
                        }
                    }
                    let tex = reg.face_texture(b, face).unwrap().0;
                    out.insert((layer, face, p), (tex, ao));
                }
            }
        }
    }
    out
}

/// Expands quads into unit cells, failing on any overlap or out-of-range quad.
fn expand(m: &ChunkMesh) -> Result<HashMap<Cell, (u16, u8)>, String> {
    let mut cells = HashMap::new();
    for layer in [
        RenderLayer::Opaque,
        RenderLayer::Cutout,
        RenderLayer::Translucent,
    ] {
        let lm = m.layer(layer);
        if lm.face_ranges[6] as usize != lm.quads.len() {
            return Err("face ranges do not cover the quads".into());
        }
        for face in Face::ALL {
            let axis = face.axis();
            for q in lm.face_quads(face) {
                let d = q.decode();
                if d.w == 0 || d.h == 0 {
                    return Err(format!("empty quad {d:?}"));
                }
                if d.flip != ao_flip(d.ao) {
                    return Err(format!("flip flag disagrees with ao: {d:?}"));
                }
                for dv in 0..d.h {
                    for du in 0..d.w {
                        let mut p = d.xyz.map(|c| c as i32);
                        p[TANGENT_U[axis]] += du as i32;
                        p[TANGENT_V[axis]] += dv as i32;
                        if p.iter().any(|&c| !(0..N).contains(&c)) {
                            return Err(format!("quad leaves the chunk: {d:?}"));
                        }
                        if cells.insert((layer, face, p), (d.material, d.ao)).is_some() {
                            return Err(format!("overlap at {layer:?} {face:?} {p:?}"));
                        }
                    }
                }
            }
        }
    }
    Ok(cells)
}

fn mesh_of(s: &Scene, reg: &Registry) -> ChunkMesh {
    let mut out = ChunkMesh::default();
    Mesher::new().mesh(&s.neighborhood(), reg, &mut out);
    out
}

fn check(s: &Scene, reg: &Registry) -> ChunkMesh {
    let m = mesh_of(s, reg);
    let got = expand(&m).unwrap_or_else(|e| panic!("{e}"));
    let want = reference(s, reg);
    assert_eq!(got.len(), want.len(), "covered cell count");
    for (cell, value) in &want {
        assert_eq!(got.get(cell), Some(value), "cell {cell:?}");
    }
    assert!(
        m.quad_count() <= want.len(),
        "greedy never emits more than one quad per face"
    );
    m
}

// ---- world functions --------------------------------------------------------

fn hash(seed: u64, a: i32, b: i32, c: i32) -> u64 {
    let mut h = seed ^ 0x9E37_79B9_7F4A_7C15;
    for v in [a, b, c] {
        h = (h ^ v as u32 as u64).wrapping_mul(0x0100_0000_01B3);
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 32;
    }
    h
}

fn noise(seed: u64, density: u8) -> impl Fn([i32; 3]) -> u16 {
    move |p| {
        let h = hash(seed, p[0], p[1], p[2]);
        if (h % 100) as u8 >= density {
            0
        } else {
            PALETTE[(h >> 20) as usize % 4]
        }
    }
}

/// Rolling terrain with a dirt skin, tree-like leaf blobs and a glass pane.
fn terrain(seed: u64) -> impl Fn([i32; 3]) -> u16 {
    move |[x, y, z]| {
        let h = 20 + ((x / 4 + z / 6) % 5) + (hash(seed, x / 8, 0, z / 8) % 4) as i32;
        let skin = if y == h - 1 { 1 } else { 4 };
        if y < h {
            skin
        } else if y < h + 3 && hash(seed, x / 3, y, z / 3).is_multiple_of(7) {
            3
        } else if (40..42).contains(&x) || (y == h + 4 && x < 6) {
            2
        } else {
            0
        }
    }
}

fn boxes(seed: u64, count: usize) -> impl Fn([i32; 3]) -> u16 {
    let list: Vec<([i32; 3], [i32; 3], u16)> = (0..count as i32)
        .map(|i| {
            let h = |k| hash(seed, i, k, 0);
            let lo = [0, 1, 2].map(|k| (h(k) % 70) as i32 - 20);
            let size = [3, 4, 5].map(|k| 1 + (h(k) % 14) as i32);
            (lo, size, PALETTE[(h(6) % 4) as usize])
        })
        .collect();
    move |p| {
        list.iter()
            .rev()
            .find(|(lo, size, _)| (0..3).all(|k| p[k] >= lo[k] && p[k] < lo[k] + size[k]))
            .map_or(0, |b| b.2)
    }
}

// ---- tests ------------------------------------------------------------------

#[test]
fn empty_and_buried_chunks_have_no_faces() {
    let reg = registry();
    assert_eq!(
        mesh_of(&Scene::new(&reg, |_| 0, 0, false), &reg).quad_count(),
        0
    );
    assert_eq!(
        mesh_of(&Scene::new(&reg, |_| 4, 0, false), &reg).quad_count(),
        0
    );
}

#[test]
fn flat_ground_is_one_quad() {
    let reg = registry();
    let s = Scene::new(&reg, |p| if p[1] < 16 { 4 } else { 0 }, 0, false);
    let m = check(&s, &reg);
    assert_eq!(m.quad_count(), 1);
    let d = m.opaque.face_quads(Face::PosY)[0].decode();
    assert_eq!((d.w, d.h, d.ao, d.xyz[1]), (32, 32, 0xFF, 15));
}

#[test]
fn lone_block_has_six_unit_quads() {
    let reg = registry();
    let s = Scene::new(&reg, |p| if p == [10, 10, 10] { 4 } else { 0 }, 0, false);
    let m = check(&s, &reg);
    assert_eq!(m.quad_count(), 6);
    for f in Face::ALL {
        assert_eq!(m.opaque.face_quads(f).len(), 1, "{f:?}");
    }
}

#[test]
fn a_missing_neighbour_reads_as_air_and_a_present_one_hides_the_face() {
    let reg = registry();
    let world = |p: [i32; 3]| {
        if p[0] < 40 && (0..32).contains(&p[1]) {
            4
        } else {
            0
        }
    };
    // Absent +X neighbour (slot of offset [1,0,0]): the +X wall of the centre is exposed.
    let absent = 1u32 << slot([1, 0, 0]);
    let with = mesh_of(&Scene::new(&reg, world, 0, false), &reg);
    let without = mesh_of(&Scene::new(&reg, world, absent, false), &reg);
    assert_eq!(with.opaque.face_quads(Face::PosX).len(), 0);
    assert!(!without.opaque.face_quads(Face::PosX).is_empty());
}

#[test]
fn like_translucent_and_cutout_blocks_share_no_inner_faces() {
    let reg = registry();
    // A 2x1x1 slab of glass and of leaves, side by side.
    for id in [2u16, 3] {
        let s = Scene::new(
            &reg,
            move |p| {
                if p[1] == 5 && p[2] == 5 && (5..7).contains(&p[0]) {
                    id
                } else {
                    0
                }
            },
            0,
            false,
        );
        let m = check(&s, &reg);
        // Four long sides plus two ends; the joined faces are gone.
        assert_eq!(m.quad_count(), 6);
    }
    // Different non-opaque blocks do show their shared faces.
    let s = Scene::new(
        &reg,
        |p| match (p[0], p[1], p[2]) {
            (5, 5, 5) => 2,
            (6, 5, 5) => 3,
            _ => 0,
        },
        0,
        false,
    );
    assert_eq!(check(&s, &reg).quad_count(), 12);
}

#[test]
fn fixed_scenes_match_the_reference() {
    let reg = registry();
    for by_edits in [false, true] {
        check(&Scene::new(&reg, terrain(1), 0, by_edits), &reg);
        check(&Scene::new(&reg, terrain(2), 0x0529_424B, by_edits), &reg);
        check(&Scene::new(&reg, boxes(7, 40), 0, by_edits), &reg);
        check(&Scene::new(&reg, noise(1, 50), 0, by_edits), &reg);
    }
}

#[test]
fn reusing_a_mesher_leaves_no_trace_of_earlier_chunks() {
    let reg = registry();
    let (a, b) = (
        Scene::new(&reg, terrain(3), 0, false),
        Scene::new(&reg, boxes(5, 30), 0, false),
    );
    let mut mesher = Mesher::new();
    let mut run = |s: &Scene| {
        let mut out = ChunkMesh::default();
        mesher.mesh(&s.neighborhood(), &reg, &mut out);
        out
    };
    let first = run(&a);
    let _ = run(&b);
    let again = run(&a);
    for layer in [
        RenderLayer::Opaque,
        RenderLayer::Cutout,
        RenderLayer::Translucent,
    ] {
        assert_eq!(first.layer(layer).quads, again.layer(layer).quads);
        assert_eq!(
            first.layer(layer).face_ranges,
            again.layer(layer).face_ranges
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(60))]

    #[test]
    fn random_noise_matches(seed in any::<u64>(), density in 0u8..=100, absent in any::<u32>(), edits in any::<bool>()) {
        let reg = registry();
        check(&Scene::new(&reg, noise(seed, density), absent, edits), &reg);
    }

    #[test]
    fn random_boxes_match(seed in any::<u64>(), count in 1usize..60, absent in any::<u32>(), edits in any::<bool>()) {
        let reg = registry();
        check(&Scene::new(&reg, boxes(seed, count), absent, edits), &reg);
    }

    #[test]
    fn random_terrain_matches(seed in any::<u64>(), absent in any::<u32>()) {
        let reg = registry();
        check(&Scene::new(&reg, terrain(seed), absent, false), &reg);
    }
}
