/// One of the six axis-aligned directions.
///
/// The order (+X, -X, +Y, -Y, +Z, -Z) is the order the mesher groups quads
/// in and the order draw records use, so `index()` is stable.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Face {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl Face {
    pub const ALL: [Face; 6] = [
        Face::PosX,
        Face::NegX,
        Face::PosY,
        Face::NegY,
        Face::PosZ,
        Face::NegZ,
    ];

    /// Position in [`Face::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn from_index(i: usize) -> Option<Face> {
        if i < 6 { Some(Self::ALL[i]) } else { None }
    }

    /// 0 for X, 1 for Y, 2 for Z.
    pub const fn axis(self) -> usize {
        self as usize / 2
    }

    /// +1 or -1 along [`Face::axis`].
    pub const fn sign(self) -> i32 {
        if (self as usize).is_multiple_of(2) {
            1
        } else {
            -1
        }
    }

    pub const fn normal(self) -> [i32; 3] {
        let mut n = [0; 3];
        n[self.axis()] = self.sign();
        n
    }

    pub const fn opposite(self) -> Face {
        Self::ALL[self as usize ^ 1]
    }
}
