// Mesher output drawn in 3D, to look for pinholes at T-junctions.
// One draw per face direction; first_instance carries the direction.

struct Globals {
    view_proj: mat4x4<f32>,
    eps: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> quads: array<u32>;
@group(0) @binding(1) var<uniform> g: Globals;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
}

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) dir: u32) -> VOut {
    let q = vi / 6u;
    let k = vi % 6u;
    let w0 = quads[3u * q];
    let w1 = quads[3u * q + 1u];
    var p = vec3<f32>(f32(w0 & 63u), f32((w0 >> 6u) & 63u), f32((w0 >> 12u) & 63u));
    let w = f32((w0 >> 18u) & 63u);
    let h = f32((w0 >> 24u) & 63u);
    var tu = array<u32, 3>(1u, 2u, 0u);
    var tv = array<u32, 3>(2u, 0u, 1u);
    let axis = dir / 2u;
    if (dir % 2u) == 0u {
        p[axis] += 1.0; // positive faces sit on the voxel's far side
    }
    let cu = select(0.0, 1.0, k == 1u || k == 4u || k == 5u);
    let cv = select(0.0, 1.0, k == 2u || k == 3u || k == 5u);
    // Push each corner outward along both tangents by eps (0 disables).
    let e = g.eps.x;
    p[tu[axis]] += cu * w + select(-e, e, cu == 1.0);
    p[tv[axis]] += cv * h + select(-e, e, cv == 1.0);
    let m = w1 & 0xffffu;
    var out: VOut;
    out.pos = g.view_proj * vec4(p, 1.0);
    // Components stay at or above 0.25 so no material can match the magenta background.
    out.color = vec3(0.25 + 0.5 * fract(f32(m) * 0.37), 0.25 + 0.5 * fract(f32(m) * 0.61), 0.25 + 0.5 * fract(f32(m) * 0.17));
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    return vec4(in.color, 1.0);
}
