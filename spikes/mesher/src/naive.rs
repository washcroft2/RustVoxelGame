//! Independent reference mesher: one unit face per exposed voxel side, with
//! ambient occlusion read straight from the 3D array. It shares no code with
//! the greedy mesher beyond the corner formula and the direction tables, so a
//! bug in the bit-mask path shows up as a mismatch.

use std::collections::HashMap;

use crate::{DIRS, N, TANGENT_U, TANGENT_V, ao_corner, idx};

/// One exposed face: direction plus chunk-local voxel `[x, y, z]` (0..32).
pub type Cell = (usize, [usize; 3]);

/// All exposed faces of the inner 32³ voxels, mapped to `(material, ao)`.
pub fn faces(voxels: &[u16]) -> HashMap<Cell, (u16, u8)> {
    let solid = |p: [isize; 3]| voxels[idx(p[0] as usize, p[1] as usize, p[2] as usize)] != 0;
    let mut out = HashMap::new();
    for z in 1..=N {
        for y in 1..=N {
            for x in 1..=N {
                let material = voxels[idx(x, y, z)];
                if material == 0 {
                    continue;
                }
                let p = [x as isize, y as isize, z as isize];
                for dir in 0..DIRS {
                    let axis = dir / 2;
                    let mut front = p;
                    front[axis] += if dir.is_multiple_of(2) { 1 } else { -1 };
                    if solid(front) {
                        continue;
                    }
                    let offset = |du: isize, dv: isize| {
                        let mut q = front;
                        q[TANGENT_U[axis]] += du;
                        q[TANGENT_V[axis]] += dv;
                        q
                    };
                    let mut ao = 0u8;
                    for cv in 0..2 {
                        for cu in 0..2 {
                            let su = if cu == 0 { -1 } else { 1 };
                            let sv = if cv == 0 { -1 } else { 1 };
                            let c = ao_corner(
                                solid(offset(su, 0)),
                                solid(offset(0, sv)),
                                solid(offset(su, sv)),
                            );
                            ao |= c << (2 * (cu + 2 * cv));
                        }
                    }
                    out.insert((dir, [x - 1, y - 1, z - 1]), (material, ao));
                }
            }
        }
    }
    out
}
