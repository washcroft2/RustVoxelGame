//! Median remesh time on synthetic terrain: `cargo run --release -p voxel-mesh --example remesh_bench`.
//! Not a baseline: real terrain replaces this once worldgen exists (Phase 1 re-measure).

use std::time::Instant;

use voxel_content::{Pack, Registry};
use voxel_core::{BlockId, CHUNK_VOLUME, LocalPos};
use voxel_mesh::{ChunkMesh, Mesher, Neighborhood};
use voxel_world::{ChunkData, PalettedArray};

fn registry() -> Registry {
    let b = |t: &str| format!("[textures]\nall = \"{t}\"\n");
    let pack = Pack::from_parts(
        "id = \"t\"\nversion = \"1\"\n",
        &[("dirt".into(), b("d")), ("stone".into(), b("s"))],
        vec!["d".into(), "s".into()],
    )
    .expect("valid pack");
    Registry::build(&[pack]).expect("valid registry")
}

fn height(x: i32, z: i32, roughness: i32) -> i32 {
    let h = |a: i32, b: i32| {
        let mut v = (a as u32).wrapping_mul(73856093) ^ (b as u32).wrapping_mul(19349663);
        v ^= v >> 13;
        v = v.wrapping_mul(0x5bd1e995);
        (v >> 16) as i32
    };
    // Gentle hills plus per-column jitter scaled by `roughness`.
    let hills = ((x as f32 * 0.09).sin() * 5.0 + (z as f32 * 0.07).cos() * 4.0) as i32;
    16 + hills + h(x, z) % (roughness + 1)
}

fn chunk(reg: &Registry, c: [i32; 3], roughness: i32) -> ChunkData {
    let vals: Vec<u16> = (0..CHUNK_VOLUME)
        .map(|i| {
            let l = LocalPos::from_index(i).expect("in range");
            let (x, y, z) = (
                c[0] * 32 + l.x() as i32,
                c[1] * 32 + l.y() as i32,
                c[2] * 32 + l.z() as i32,
            );
            let h = height(x, z, roughness);
            if y >= h {
                0
            } else if y == h - 1 {
                1
            } else {
                2
            }
        })
        .collect();
    ChunkData::from_blocks(PalettedArray::from_slice(&vals).expect("right length"), reg)
}

fn main() {
    let reg = registry();
    assert_eq!(
        reg.block_id(&voxel_core::Identifier::new("t:dirt").expect("name")),
        Some(BlockId(1))
    );
    for (label, roughness) in [("smooth", 0), ("rough", 3)] {
        let mut chunks = Vec::new();
        for dy in -1..=1 {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    chunks.push(chunk(&reg, [dx, dy, dz], roughness));
                }
            }
        }
        let mut nb = Neighborhood::new(&chunks[13]);
        let mut i = 0;
        for dy in -1..=1 {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    nb.set([dx, dy, dz], Some(&chunks[i]));
                    i += 1;
                }
            }
        }
        let (mut mesher, mut out) = (Mesher::new(), ChunkMesh::default());
        for _ in 0..200 {
            mesher.mesh(&nb, &reg, &mut out);
        }
        let mut times: Vec<f64> = (0..3000)
            .map(|_| {
                let t = Instant::now();
                mesher.mesh(&nb, &reg, &mut out);
                t.elapsed().as_secs_f64() * 1e6
            })
            .collect();
        times.sort_by(f64::total_cmp);
        println!(
            "{label:7} quads {:5}  median {:6.1} us  p99 {:6.1} us",
            out.quad_count(),
            times[times.len() / 2],
            times[times.len() * 99 / 100]
        );
    }
}
