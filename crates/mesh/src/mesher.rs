use voxel_content::{Registry, RenderLayer};
use voxel_core::{
    BlockClass, BlockClasses, BlockId, CHUNK_SIZE, CHUNK_VOLUME, Face, LocalPos, TANGENT_U,
    TANGENT_V,
};
use voxel_world::ChunkData;

use crate::{ChunkMesh, LayerMesh, Neighborhood, Quad, ao_corner, ao_flip};

const N: usize = CHUNK_SIZE;
/// Chunk edge plus one block of border each side.
const P: usize = N + 2;

/// Opaque-block bitmasks over the chunk plus its border: `cols[axis][v][u]`,
/// bit `d` = padded depth (bit 0 and bit 33 are the neighbours' blocks).
type PaddedCols = [[[u64; P]; P]; 3];
/// Interior-only masks for one non-opaque layer, `[axis][v * 32 + u]`.
type LayerCols = [[u32; N * N]; 3];

/// Reusable meshing scratch space. Make one per worker thread.
pub struct Mesher {
    opaque: Box<PaddedCols>,
    /// Cutout, then translucent.
    other: Box<[LayerCols; 2]>,
}

impl Default for Mesher {
    fn default() -> Self {
        Self::new()
    }
}

impl Mesher {
    pub fn new() -> Self {
        Self {
            opaque: Box::new([[[0; P]; P]; 3]),
            other: Box::new([[[0; N * N]; 3]; 2]),
        }
    }

    /// Meshes the centre chunk of `nb`. `out` is cleared first, so it can be reused.
    pub fn mesh(&mut self, nb: &Neighborhood, registry: &Registry, out: &mut ChunkMesh) {
        voxel_core::span!("mesh chunk");
        out.clear();
        let center = nb.center();
        let s = center.summary();
        if s.non_air == 0 {
            return;
        }
        self.build_padded_opaque(nb);
        if s.opaque > 0 {
            self.mesh_layer(nb, registry, RenderLayer::Opaque, &mut out.opaque);
        }
        if s.cutout > 0 || s.translucent > 0 {
            self.scan_other_layers(center, registry);
            if s.cutout > 0 {
                self.mesh_layer(nb, registry, RenderLayer::Cutout, &mut out.cutout);
            }
            if s.translucent > 0 {
                self.mesh_layer(nb, registry, RenderLayer::Translucent, &mut out.translucent);
            }
        }
    }

    /// Gathers the cached opaque columns of the centre chunk and its neighbours into padded columns.
    fn build_padded_opaque(&mut self, nb: &Neighborhood) {
        voxel_core::span!("gather opaque columns");
        let off = |p: usize| match p {
            0 => -1,
            p if p == P - 1 => 1,
            _ => 0,
        };
        let column = |chunk: Option<&ChunkData>, axis: usize, u: usize, v: usize| {
            chunk.map_or(0, |c| c.opaque_column(axis, u, v))
        };
        for axis in 0..3 {
            let (au, av) = (TANGENT_U[axis], TANGENT_V[axis]);
            for pv in 0..P {
                for pu in 0..P {
                    let mut o = [0i32; 3];
                    o[au] = off(pu);
                    o[av] = off(pv);
                    let (lu, lv) = ((pu + N - 1) % N, (pv + N - 1) % N);
                    let mid = column(nb.get(o[0], o[1], o[2]), axis, lu, lv);
                    o[axis] = -1;
                    let below = column(nb.get(o[0], o[1], o[2]), axis, lu, lv) >> 31;
                    o[axis] = 1;
                    let above = column(nb.get(o[0], o[1], o[2]), axis, lu, lv) & 1;
                    self.opaque[axis][pv][pu] =
                        u64::from(below) | u64::from(mid) << 1 | u64::from(above) << (N + 1);
                }
            }
        }
    }

    /// Builds interior masks for the cutout and translucent blocks of the centre chunk.
    fn scan_other_layers(&mut self, center: &ChunkData, registry: &Registry) {
        voxel_core::span!("scan cutout and translucent");
        for layer in self.other.iter_mut() {
            *layer = [[0; N * N]; 3];
        }
        let blocks = center.blocks();
        let mut cache: Vec<(u16, usize)> = Vec::new();
        for i in 0..CHUNK_VOLUME {
            let id = blocks.get(i);
            let slot = match cache.iter().find(|(v, _)| *v == id) {
                Some(&(_, s)) => s,
                None => {
                    let s = match registry.class(BlockId(id)) {
                        BlockClass::Cutout => 0,
                        BlockClass::Translucent => 1,
                        _ => 2,
                    };
                    cache.push((id, s));
                    s
                }
            };
            if slot == 2 {
                continue;
            }
            let Some(l) = LocalPos::from_index(i) else {
                continue;
            };
            let p = [l.x() as usize, l.y() as usize, l.z() as usize];
            for axis in 0..3 {
                let (u, v) = (p[TANGENT_U[axis]], p[TANGENT_V[axis]]);
                self.other[slot][axis][v * N + u] |= 1 << p[axis];
            }
        }
    }

    fn mesh_layer(
        &self,
        nb: &Neighborhood,
        registry: &Registry,
        layer: RenderLayer,
        out: &mut LayerMesh,
    ) {
        voxel_core::span!("mesh layer");
        out.clear();
        for face in Face::ALL {
            self.mesh_face(nb, registry, layer, face, &mut out.quads);
            out.face_ranges[face.index() + 1] = out.quads.len() as u32;
        }
    }

    fn mesh_face(
        &self,
        nb: &Neighborhood,
        registry: &Registry,
        layer: RenderLayer,
        face: Face,
        quads: &mut Vec<Quad>,
    ) {
        let axis = face.axis();
        let positive = face.sign() > 0;
        let padded = &self.opaque[axis];
        let own: Option<&[u32; N * N]> = match layer {
            RenderLayer::Opaque => None,
            RenderLayer::Cutout => Some(&self.other[0][axis]),
            RenderLayer::Translucent => Some(&self.other[1][axis]),
        };
        let center = nb.center();

        // planes[d][v] has bit u set where a face of this direction sits at depth d.
        let mut planes = [[0u32; N]; N];
        for v in 0..N {
            for u in 0..N {
                let col = padded[v + 1][u + 1];
                let mine = own.map_or((col >> 1) as u32, |o| o[v * N + u]);
                // A face exists where the block in front of it is not opaque.
                let front = if positive {
                    (col >> 2) as u32
                } else {
                    col as u32
                };
                let mut bits = mine & !front;
                while bits != 0 {
                    planes[bits.trailing_zeros() as usize][v] |= 1 << u;
                    bits &= bits - 1;
                }
            }
        }

        let normal = face.normal();
        let mut keys = [[0u32; N]; N];
        for (d, plane) in planes.iter_mut().enumerate() {
            if plane.iter().all(|&row| row == 0) {
                continue;
            }
            for v in 0..N {
                let mut bits = plane[v];
                while bits != 0 {
                    let u = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let p = to_xyz(axis, d, u, v);
                    let block = center.block(LocalPos::new(p[0] as u32, p[1] as u32, p[2] as u32));
                    if layer != RenderLayer::Opaque {
                        let q = [
                            p[0] as i32 + normal[0],
                            p[1] as i32 + normal[1],
                            p[2] as i32 + normal[2],
                        ];
                        if nb.block_at(q) == block {
                            plane[v] &= !(1 << u);
                            continue;
                        }
                    }
                    let texture = registry.face_texture(block, face).map_or(0, |t| t.0);
                    let ao = cell_ao(padded, positive, d, u, v);
                    keys[v][u] = u32::from(texture) | u32::from(ao) << 16;
                }
            }
            greedy_plane(plane, &keys, axis, d, quads);
        }
    }
}

/// Maps `(depth along axis, u, v)` back to `[x, y, z]`.
#[inline]
fn to_xyz(axis: usize, d: usize, u: usize, v: usize) -> [usize; 3] {
    let mut p = [0; 3];
    p[axis] = d;
    p[TANGENT_U[axis]] = u;
    p[TANGENT_V[axis]] = v;
    p
}

/// Ambient occlusion for one face cell, read from the opaque columns.
#[inline]
fn cell_ao(cols: &[[u64; P]; P], positive: bool, d: usize, u: usize, v: usize) -> u8 {
    // The occluders sit in the layer in front of the face (padded depth d + 1 +- 1).
    let front = if positive { d + 2 } else { d };
    let occ = |du: usize, dv: usize| cols[v + dv][u + du] >> front & 1 != 0;
    // Padded neighbour offsets: 0 = -1, 1 = centre, 2 = +1.
    let mut ao = 0u8;
    for cv in 0..2 {
        for cu in 0..2 {
            let su = 2 * cu;
            let sv = 2 * cv;
            let c = ao_corner(occ(su, 1), occ(1, sv), occ(su, sv));
            ao |= c << (2 * (cu + 2 * cv));
        }
    }
    ao
}

/// Merges runs of equal keys into rectangles: width along u (bit runs), height along v.
fn greedy_plane(
    plane: &mut [u32; N],
    keys: &[[u32; N]; N],
    axis: usize,
    d: usize,
    out: &mut Vec<Quad>,
) {
    for v in 0..N {
        while plane[v] != 0 {
            let u = plane[v].trailing_zeros() as usize;
            let key = keys[v][u];
            let mut w = 1;
            while u + w < N && plane[v] >> (u + w) & 1 != 0 && keys[v][u + w] == key {
                w += 1;
            }
            let mask = (((1u64 << w) - 1) << u) as u32;
            let mut h = 1;
            while v + h < N
                && plane[v + h] & mask == mask
                && keys[v + h][u..u + w].iter().all(|&k| k == key)
            {
                h += 1;
            }
            for row in &mut plane[v..v + h] {
                *row &= !mask;
            }
            let ao = (key >> 16) as u8;
            out.push(Quad::pack(
                to_xyz(axis, d, u, v),
                w,
                h,
                ao_flip(ao),
                (key & 0xFFFF) as u16,
                ao,
            ));
        }
    }
}
