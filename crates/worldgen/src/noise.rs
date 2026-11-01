//! Gradient noise and fractal sums. Only exact IEEE operations (`+ - * /`, `floor`)
//! are used, so results are identical on every platform.

fn hash(seed: u64, x: i64, z: i64) -> u64 {
    let mut h = seed ^ 0x9E37_79B9_7F4A_7C15;
    for v in [x as u64, z as u64] {
        h = (h ^ v).wrapping_mul(0x0100_0000_01B3);
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 32;
    }
    h
}

/// Dot product of the offset with one of eight fixed gradient directions.
fn gradient(h: u64, dx: f64, dz: f64) -> f64 {
    match h & 7 {
        0 => dx,
        1 => -dx,
        2 => dz,
        3 => -dz,
        4 => dx + dz,
        5 => dx - dz,
        6 => -dx + dz,
        _ => -dx - dz,
    }
}

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Smooth noise in roughly -1..1, zero at integer lattice points.
pub fn gradient_noise(seed: u64, x: f64, z: f64) -> f64 {
    let (fx, fz) = (x.floor(), z.floor());
    let (ix, iz) = (fx as i64, fz as i64);
    let (dx, dz) = (x - fx, z - fz);
    let corner = |cx: i64, cz: i64| {
        gradient(
            hash(seed, ix.wrapping_add(cx), iz.wrapping_add(cz)),
            dx - cx as f64,
            dz - cz as f64,
        )
    };
    let (u, v) = (fade(dx), fade(dz));
    let bottom = lerp(corner(0, 0), corner(1, 0), u);
    let top = lerp(corner(0, 1), corner(1, 1), u);
    lerp(bottom, top, v)
}

/// Sum of `octaves` layers, each twice as fine and half as strong, scaled into about -1..1.
pub fn fbm(seed: u64, x: f64, z: f64, octaves: u32) -> f64 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves.min(16) {
        sum += amp * gradient_noise(seed.wrapping_add(u64::from(o)), x * freq, z * freq);
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    if norm == 0.0 {
        0.0
    } else {
        (sum / norm).clamp(-1.0, 1.0)
    }
}
