//! Phase 0 mesher spike (plan sections 5 and 13). Throwaway.
//!
//! Binary greedy mesher for 32³ chunks with a 1-block border (a 34³ padded
//! array), emitting the plan's 12-byte quads grouped by face direction.
//! Cube-only and opaque; merging keys on material and ambient occlusion.
//! Light is not part of the key here, so real merge rates will be somewhat worse.

pub mod naive;
pub mod scenes;

/// Chunk edge in blocks.
pub const N: usize = 32;
/// Chunk edge plus one block of border on each side.
pub const P: usize = N + 2;
/// Voxels in the padded array.
pub const PADDED_LEN: usize = P * P * P;

/// Face directions in draw order: +X, -X, +Y, -Y, +Z, -Z.
pub const DIRS: usize = 6;

/// Tangent axes `(U, V)` for each normal axis. A quad's `w` runs along U and
/// `h` along V. X -> (Y, Z), Y -> (Z, X), Z -> (X, Y).
pub const TANGENT_U: [usize; 3] = [1, 2, 0];
pub const TANGENT_V: [usize; 3] = [2, 0, 1];

/// Index into the padded voxel array. Coordinates are padded (0..34).
#[inline]
pub fn idx(x: usize, y: usize, z: usize) -> usize {
    x + P * (z + P * y)
}

/// Maps `(depth along axis, u, v)` back to `[x, y, z]`.
#[inline]
pub fn to_xyz(axis: usize, d: usize, u: usize, v: usize) -> [usize; 3] {
    let mut p = [0; 3];
    p[axis] = d;
    p[TANGENT_U[axis]] = u;
    p[TANGENT_V[axis]] = v;
    p
}

/// Per-column opacity bitmasks along each axis, `cols[axis][v][u]`, bit = depth.
/// The plan keeps these up to date on every write, so the real cost of
/// building them is paid at edit time, not per remesh.
pub struct OpacityCols {
    pub cols: [[[u64; P]; P]; 3],
}

impl OpacityCols {
    /// Builds the masks from a padded voxel array where `0` is air.
    pub fn from_voxels(voxels: &[u16]) -> Self {
        let mut cols = [[[0u64; P]; P]; 3];
        for y in 0..P {
            for z in 0..P {
                let row = &voxels[idx(0, y, z)..idx(0, y, z) + P];
                for (x, &b) in row.iter().enumerate() {
                    if b != 0 {
                        let p = [x, y, z];
                        for axis in 0..3 {
                            cols[axis][p[TANGENT_V[axis]]][p[TANGENT_U[axis]]] |= 1 << p[axis];
                        }
                    }
                }
            }
        }
        Self { cols }
    }
}

/// One merged quad, 12 bytes, as uploaded to the GPU.
///
/// ```text
/// word0: x:6 y:6 z:6 w:6 h:6 flags:2   (x,y,z = min-corner voxel; w,h in 1..=32)
/// word1: material:16 ao:8 extra:8      (ao = 4 corners x 2 bits; 3 = unoccluded)
/// word2: light, 4 corners x 8 bits     (unused in this spike)
/// ```
///
/// Face direction is supplied by the draw record. For a positive-direction
/// face the shader places the quad on the voxel's far side (+1 along the
/// normal); for a negative one, on its near side.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quad {
    pub w0: u32,
    pub w1: u32,
    pub w2: u32,
}

/// Flag bit in `word0`: split the quad along the other diagonal.
pub const FLAG_FLIP: u32 = 1;

/// A quad decoded for a given direction into plane coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub xyz: [usize; 3],
    pub w: usize,
    pub h: usize,
    pub material: u16,
    pub ao: u8,
    pub flip: bool,
}

impl Quad {
    fn pack(xyz: [usize; 3], w: usize, h: usize, flip: bool, material: u16, ao: u8) -> Self {
        let flags = if flip { FLAG_FLIP } else { 0 };
        Self {
            w0: xyz[0] as u32
                | (xyz[1] as u32) << 6
                | (xyz[2] as u32) << 12
                | (w as u32) << 18
                | (h as u32) << 24
                | flags << 30,
            w1: u32::from(material) | u32::from(ao) << 16,
            w2: 0,
        }
    }

    /// Unpacks the quad. Fields are masked, so any bit pattern decodes.
    pub fn decode(&self) -> Decoded {
        Decoded {
            xyz: [
                (self.w0 & 63) as usize,
                (self.w0 >> 6 & 63) as usize,
                (self.w0 >> 12 & 63) as usize,
            ],
            w: (self.w0 >> 18 & 63) as usize,
            h: (self.w0 >> 24 & 63) as usize,
            flip: self.w0 >> 30 & FLAG_FLIP != 0,
            material: (self.w1 & 0xFFFF) as u16,
            ao: (self.w1 >> 16 & 0xFF) as u8,
        }
    }
}

/// Mesher output: quads grouped by direction, `face_ranges[d]..face_ranges[d+1]`.
#[derive(Default)]
pub struct Mesh {
    pub quads: Vec<Quad>,
    pub face_ranges: [u32; DIRS + 1],
}

impl Mesh {
    pub fn clear(&mut self) {
        self.quads.clear();
        self.face_ranges = [0; DIRS + 1];
    }

    pub fn dir_quads(&self, dir: usize) -> &[Quad] {
        &self.quads[self.face_ranges[dir] as usize..self.face_ranges[dir + 1] as usize]
    }
}

/// Picks the triangulation diagonal from the AO values: keep the diagonal
/// that joins the brighter pair, so occlusion doesn't interpolate across the
/// wrong corners. Corners are ordered (0,0), (1,0), (0,1), (1,1) in (u,v).
pub fn ao_flip(ao: u8) -> bool {
    let c = |i: u32| u32::from(ao >> (2 * i) & 3);
    c(0) + c(3) <= c(1) + c(2)
}

/// Combines one corner's two edge neighbours and diagonal neighbour.
#[inline]
pub fn ao_corner(side1: bool, side2: bool, corner: bool) -> u8 {
    if side1 && side2 {
        0
    } else {
        3 - (u8::from(side1) + u8::from(side2) + u8::from(corner))
    }
}

/// Greedy-meshes one padded chunk. `out` is cleared first, so it can be reused.
pub fn mesh(voxels: &[u16], opacity: &OpacityCols, out: &mut Mesh) {
    out.clear();
    for dir in 0..DIRS {
        mesh_dir(voxels, opacity, dir, out);
        out.face_ranges[dir + 1] = out.quads.len() as u32;
    }
}

fn mesh_dir(voxels: &[u16], opacity: &OpacityCols, dir: usize, out: &mut Mesh) {
    let axis = dir / 2;
    let positive = dir.is_multiple_of(2);
    let cols = &opacity.cols[axis];

    // planes[d][v] has bit u set where a face of this direction sits at depth d.
    let mut planes = [[0u32; N]; N];
    for v in 0..N {
        for u in 0..N {
            let col = cols[v + 1][u + 1];
            // A face exists where a solid voxel's neighbour in the facing direction is air.
            let faces = if positive {
                col & !(col >> 1)
            } else {
                col & !(col << 1)
            };
            // Drop the border bits; bit 1 of the padded column is depth 0.
            let mut bits = (faces >> 1) as u32;
            while bits != 0 {
                planes[bits.trailing_zeros() as usize][v] |= 1 << u;
                bits &= bits - 1;
            }
        }
    }

    let mut keys = [[0u32; N]; N];
    for (d, plane) in planes.iter_mut().enumerate() {
        if plane.iter().all(|&row| row == 0) {
            continue;
        }
        for (v, &row) in plane.iter().enumerate() {
            let mut bits = row;
            while bits != 0 {
                let u = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let [x, y, z] = to_xyz(axis, d + 1, u + 1, v + 1);
                let ao = cell_ao(cols, positive, d, u, v);
                keys[v][u] = u32::from(voxels[idx(x, y, z)]) | u32::from(ao) << 16;
            }
        }
        greedy_plane(plane, &keys, axis, d, out);
    }
}

/// Ambient occlusion for one face cell, read from the opacity columns.
#[inline]
fn cell_ao(cols: &[[u64; P]; P], positive: bool, d: usize, u: usize, v: usize) -> u8 {
    // The occluders sit in the layer in front of the face.
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
fn greedy_plane(plane: &mut [u32; N], keys: &[[u32; N]; N], axis: usize, d: usize, out: &mut Mesh) {
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
            out.quads.push(Quad::pack(
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

/// Counts quad corners that land strictly inside another coplanar quad's edge.
///
/// Greedy merging can't avoid these. Each one is a place where two quads
/// disagree about where an edge has a vertex, which can leave a one-pixel gap
/// when rasterised. Only same-plane pairs are counted; edges shared across
/// perpendicular planes need the GPU spike to judge.
pub fn t_junctions(mesh: &Mesh) -> usize {
    let mut total = 0;
    for dir in 0..DIRS {
        let axis = dir / 2;
        // Group by plane depth, then compare pairs within each group.
        let mut by_depth: Vec<Vec<[usize; 4]>> = vec![Vec::new(); N];
        for q in mesh.dir_quads(dir) {
            let dq = q.decode();
            let (u0, v0) = (dq.xyz[TANGENT_U[axis]], dq.xyz[TANGENT_V[axis]]);
            by_depth[dq.xyz[axis]].push([u0, v0, u0 + dq.w, v0 + dq.h]);
        }
        for rects in &by_depth {
            for (i, a) in rects.iter().enumerate() {
                for (j, b) in rects.iter().enumerate() {
                    if i == j {
                        continue;
                    }
                    for (cu, cv) in [(a[0], a[1]), (a[2], a[1]), (a[0], a[3]), (a[2], a[3])] {
                        total += usize::from(strictly_on_edge(b, cu, cv));
                    }
                }
            }
        }
    }
    total
}

/// True when `(u, v)` lies on `r`'s boundary but is not one of its corners.
fn strictly_on_edge(r: &[usize; 4], u: usize, v: usize) -> bool {
    let on_vertical = (u == r[0] || u == r[2]) && v > r[1] && v < r[3];
    let on_horizontal = (v == r[1] || v == r[3]) && u > r[0] && u < r[2];
    on_vertical || on_horizontal
}
