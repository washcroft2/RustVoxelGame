//! The greedy mesher against the naive reference (plan section 11:
//! "mesher face coverage vs brute force").

use std::collections::HashMap;

use proptest::prelude::*;
use spike_mesher::naive::{self, Cell};
use spike_mesher::{
    DIRS, Mesh, N, OpacityCols, TANGENT_U, TANGENT_V, ao_flip, mesh, scenes, t_junctions,
};

fn greedy(voxels: &[u16]) -> Mesh {
    let mut out = Mesh::default();
    mesh(voxels, &OpacityCols::from_voxels(voxels), &mut out);
    out
}

/// Expands quads into unit cells, failing on any overlap or out-of-range quad.
fn expand(m: &Mesh) -> Result<HashMap<Cell, (u16, u8)>, String> {
    let mut cells = HashMap::new();
    for dir in 0..DIRS {
        let axis = dir / 2;
        for q in m.dir_quads(dir) {
            let d = q.decode();
            if d.w == 0 || d.h == 0 {
                return Err(format!("empty quad {d:?}"));
            }
            if d.flip != ao_flip(d.ao) {
                return Err(format!("flip flag disagrees with ao: {d:?}"));
            }
            for dv in 0..d.h {
                for du in 0..d.w {
                    let mut p = d.xyz;
                    p[TANGENT_U[axis]] += du;
                    p[TANGENT_V[axis]] += dv;
                    if p.iter().any(|&c| c >= N) {
                        return Err(format!("quad leaves the chunk: {d:?}"));
                    }
                    if cells.insert((dir, p), (d.material, d.ao)).is_some() {
                        return Err(format!("overlap at dir {dir} {p:?}"));
                    }
                }
            }
        }
    }
    Ok(cells)
}

fn check(voxels: &[u16]) {
    let m = greedy(voxels);
    let got = expand(&m).unwrap_or_else(|e| panic!("{e}"));
    let want = naive::faces(voxels);
    assert_eq!(got.len(), want.len(), "covered cell count");
    for (cell, value) in &want {
        assert_eq!(got.get(cell), Some(value), "cell {cell:?}");
    }
    assert!(
        m.quads.len() <= want.len(),
        "greedy never emits more than naive"
    );
}

#[test]
fn empty_and_solid_have_no_faces() {
    assert!(greedy(&scenes::empty()).quads.is_empty());
    assert!(greedy(&scenes::solid()).quads.is_empty());
}

#[test]
fn flat_plane_is_one_quad() {
    let m = greedy(&scenes::flat(16));
    assert_eq!(m.quads.len(), 1);
    let d = m.dir_quads(2)[0].decode(); // +Y
    assert_eq!((d.w, d.h, d.ao), (32, 32, 0xFF));
    check(&scenes::flat(16));
}

#[test]
fn lone_voxel_has_six_unit_quads() {
    let mut v = scenes::empty();
    v[spike_mesher::idx(10, 10, 10)] = 5;
    let m = greedy(&v);
    assert_eq!(m.quads.len(), 6);
    for dir in 0..DIRS {
        assert_eq!(m.dir_quads(dir).len(), 1, "dir {dir}");
    }
    check(&v);
}

#[test]
fn fixed_scenes_match_reference() {
    for v in [
        scenes::terrain(),
        scenes::terrain_smooth(),
        scenes::checker(),
        scenes::boxes(7, 40),
        scenes::noise(1, 50, 4),
    ] {
        check(&v);
    }
}

#[test]
fn border_voxels_emit_no_faces() {
    // Solid chunk surrounded by a border full of other material: border never meshes.
    let mut v = scenes::noise(3, 100, 1);
    for z in 0..N + 2 {
        for x in 0..N + 2 {
            v[spike_mesher::idx(x, 0, z)] = 9;
        }
    }
    let m = greedy(&v);
    assert!(m.quads.iter().all(|q| q.decode().material != 9));
}

#[test]
fn t_junctions_exist_and_are_counted() {
    // An L-shaped top face needs two quads that meet with mismatched edges.
    let mut v = scenes::empty();
    for z in 1..=4 {
        for x in 1..=4 {
            v[spike_mesher::idx(x, 1, z)] = 1;
        }
    }
    for x in 5..=8 {
        v[spike_mesher::idx(x, 1, 1)] = 1;
    }
    check(&v);
    assert!(t_junctions(&greedy(&v)) > 0);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn random_noise_matches_reference(seed in any::<u64>(), density in 0u8..=100, mats in 1u16..=6) {
        check(&scenes::noise(seed, density, mats));
    }

    #[test]
    fn random_boxes_match_reference(seed in any::<u64>(), count in 1usize..60) {
        check(&scenes::boxes(seed, count));
    }
}
