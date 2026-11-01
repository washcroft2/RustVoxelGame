/// One merged quad, 12 bytes, as uploaded to the GPU.
///
/// ```text
/// word0: x:6 y:6 z:6 w:6 h:6 flags:2   (x,y,z = min-corner voxel; w,h in 1..=32)
/// word1: material:16 ao:8 extra:8      (ao = 4 corners x 2 bits; 3 = unoccluded)
/// word2: light, 4 corners x 8 bits     (zero until the light crate exists)
/// ```
///
/// Face direction is supplied by the draw record. For a positive-direction
/// face the shader places the quad on the voxel's far side (+1 along the
/// normal); for a negative one, on its near side. `w` runs along
/// `TANGENT_U[axis]` and `h` along `TANGENT_V[axis]`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quad {
    pub w0: u32,
    pub w1: u32,
    pub w2: u32,
}

/// Flag bit in `word0`: split the quad along the other diagonal.
pub const FLAG_FLIP: u32 = 1;

/// A quad unpacked into plain fields.
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
    pub(crate) fn pack(
        xyz: [usize; 3],
        w: usize,
        h: usize,
        flip: bool,
        material: u16,
        ao: u8,
    ) -> Self {
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

/// Picks the triangulation diagonal from the AO values: keep the diagonal
/// that joins the brighter pair, so occlusion doesn't interpolate across the
/// wrong corners. Corners are ordered (0,0), (1,0), (0,1), (1,1) in (u,v).
pub fn ao_flip(ao: u8) -> bool {
    let c = |i: u32| u32::from(ao >> (2 * i) & 3);
    c(0) + c(3) <= c(1) + c(2)
}

/// Combines one corner's two edge neighbours and diagonal neighbour.
/// 3 is unoccluded, 0 is fully occluded.
#[inline]
pub fn ao_corner(side1: bool, side2: bool, corner: bool) -> u8 {
    if side1 && side2 {
        0
    } else {
        3 - (u8::from(side1) + u8::from(side2) + u8::from(corner))
    }
}
