//! Deterministic test chunks, without a `rand` dependency. All return padded arrays.

use crate::{N, P, PADDED_LEN, idx};

/// Small xorshift generator; quality doesn't matter, repeatability does.
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn hash3(x: usize, y: usize, z: usize) -> u64 {
    let mut r = Rng((x as u64) << 40 ^ (y as u64) << 20 ^ z as u64 ^ 0x9E37_79B9_7F4A_7C15);
    r.next_u64();
    r.next_u64()
}

/// Everything air.
pub fn empty() -> Vec<u16> {
    vec![0; PADDED_LEN]
}

/// Everything solid, border included: no visible faces at all.
pub fn solid() -> Vec<u16> {
    vec![1; PADDED_LEN]
}

/// Solid up to `level` (padded y), one material.
pub fn flat(level: usize) -> Vec<u16> {
    let mut v = empty();
    for y in 0..=level {
        for z in 0..P {
            for x in 0..P {
                v[idx(x, y, z)] = 1;
            }
        }
    }
    v
}

/// Terrain-like surface chunk: rolling heightfield, three layered materials,
/// and scattered one-block pockets underground. The nearest thing to a real
/// chunk that this spike has.
pub fn terrain() -> Vec<u16> {
    terrain_with(true)
}

/// Gentle low-frequency hills with no underground pockets: closer to the
/// plan's estimate of about 1.5k quads per surface chunk.
pub fn terrain_smooth() -> Vec<u16> {
    terrain_with(false)
}

fn terrain_with(rough: bool) -> Vec<u16> {
    let mut v = empty();
    for z in 0..P {
        for x in 0..P {
            let (fx, fz) = (x as f32, z as f32);
            let h = 16.0
                + 5.0 * (fx * 0.31).sin()
                + 4.0 * (fz * 0.23 + fx * 0.11).sin()
                + if rough {
                    2.0 * (fx * 0.9 + fz * 0.7).sin()
                } else {
                    0.0
                };
            let h = (h as usize).min(P - 1);
            for y in 0..=h {
                let m = if y == h {
                    2
                } else if y + 3 >= h {
                    3
                } else {
                    1
                };
                let pocket = rough && y + 2 < h && hash3(x, y, z).is_multiple_of(23);
                v[idx(x, y, z)] = if pocket { 0 } else { m };
            }
        }
    }
    v
}

/// 3D checkerboard of one material: every face is visible and nothing merges.
pub fn checker() -> Vec<u16> {
    let mut v = empty();
    for y in 0..P {
        for z in 0..P {
            for x in 0..P {
                v[idx(x, y, z)] = u16::from((x + y + z) % 2 == 0);
            }
        }
    }
    v
}

/// Random voxels: `density` percent solid, materials 1..=`mats`.
pub fn noise(seed: u64, density: u8, mats: u16) -> Vec<u16> {
    let mut rng = Rng(seed | 1);
    (0..PADDED_LEN)
        .map(|_| {
            if (rng.next_u64() % 100) < u64::from(density) {
                1 + (rng.next_u64() % u64::from(mats.max(1))) as u16
            } else {
                0
            }
        })
        .collect()
}

/// Blocky noise: random solid boxes of a few materials. Merges much better
/// than white noise, so it exercises large quads.
pub fn boxes(seed: u64, count: usize) -> Vec<u16> {
    let mut rng = Rng(seed | 1);
    let mut v = empty();
    for _ in 0..count {
        let mut pick = |max: usize| (rng.next_u64() % max as u64) as usize;
        let (x0, y0, z0) = (pick(P), pick(P), pick(P));
        let (dx, dy, dz) = (1 + pick(N / 2), 1 + pick(N / 2), 1 + pick(N / 2));
        let m = 1 + pick(4) as u16;
        for y in y0..(y0 + dy).min(P) {
            for z in z0..(z0 + dz).min(P) {
                for x in x0..(x0 + dx).min(P) {
                    v[idx(x, y, z)] = m;
                }
            }
        }
    }
    v
}
