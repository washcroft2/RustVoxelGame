// Terrain from 12-byte quads (vertex pulling). No vertex buffers.
//
// Each draw covers one face direction of one chunk layer:
//   first_vertex   = 6 * (quad index in the slab)
//   first_instance = (chunk record index << 3) | face direction
// Face directions: 0 +X, 1 -X, 2 +Y, 3 -Y, 4 +Z, 5 -Z.

struct Globals {
    view_proj: mat4x4<f32>,
    // Camera position inside its block, then the fog start distance.
    eye: vec4<f32>,
    // Fog colour (linear), then the fog end distance.
    fog: vec4<f32>,
    // x: half a pixel's angle (radians-ish: tan(fov/2) / height), used to enlarge quads.
    misc: vec4<f32>,
}

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> quads: array<u32>;
// xyz: chunk origin relative to the camera's block, in blocks.
@group(0) @binding(2) var<storage, read> records: array<vec4<f32>>;
@group(0) @binding(3) var tex: texture_2d_array<f32>;
@group(0) @binding(4) var samp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: i32,
    @location(2) shade: f32,
    @location(3) dist: f32,
}

// Corner order per vertex (2 bits each; corner = u + 2 * v).
// Default split joins corners 0 and 3; the flip flag joins 1 and 2 instead.
const TRIS_DEFAULT: u32 = 0xB34u; // 0,1,3, 0,3,2
const TRIS_FLIPPED: u32 = 0xDA4u; // 0,1,2, 2,1,3

fn face_shade(dir: u32) -> f32 {
    // +Y, then +/-Z, then +/-X, then -Y: fixed directional shading until lighting exists.
    if dir == 2u { return 1.0; }
    if dir == 3u { return 0.55; }
    if dir >= 4u { return 0.85; }
    return 0.7;
}

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) inst: u32) -> VOut {
    let dir = inst & 7u;
    let rec = records[inst >> 3u].xyz;
    let q = vi / 6u;
    var k = vi % 6u;
    let w0 = quads[3u * q];
    let w1 = quads[3u * q + 1u];
    let base = vec3<f32>(f32(w0 & 63u), f32((w0 >> 6u) & 63u), f32((w0 >> 12u) & 63u));
    let w = f32((w0 >> 18u) & 63u);
    let h = f32((w0 >> 24u) & 63u);
    let flip = ((w0 >> 30u) & 1u) != 0u;

    let positive = (dir & 1u) == 0u;
    let axis = dir / 2u;
    // Unit vectors built with comparisons: DX12's compiler rejects writes through a runtime index.
    // Tangents: X -> (Y, Z), Y -> (Z, X), Z -> (X, Y).
    let n = vec3<f32>(f32(axis == 0u), f32(axis == 1u), f32(axis == 2u));
    let tu = vec3<f32>(f32(axis == 2u), f32(axis == 0u), f32(axis == 1u));
    let tv = vec3<f32>(f32(axis == 1u), f32(axis == 2u), f32(axis == 0u));

    // Negative faces wind the other way so every face is counter-clockwise from outside.
    if !positive {
        let m = k % 3u;
        if m == 1u { k += 1u; } else if m == 2u { k -= 1u; }
    }
    let table = select(TRIS_DEFAULT, TRIS_FLIPPED, flip);
    let corner = (table >> (2u * k)) & 3u;
    let cu = f32(corner & 1u);
    let cv = f32(corner >> 1u);

    var p = base;
    if positive {
        p += n; // positive faces sit on the voxel's far side
    }
    var rel = rec + p + tu * (cu * w) + tv * (cv * h);

    // Enlarge each quad by about half a pixel on every side, scaled by distance, so
    // greedy-merge T-junctions can't leave one-pixel gaps.
    let d = length(rel - g.eye.xyz);
    let e = g.misc.x * d;
    let su = select(-e, e, cu == 1.0);
    let sv = select(-e, e, cv == 1.0);
    rel += tu * su + tv * sv;

    // Texture coordinates tile once per block. Side faces keep "up" up and are not mirrored
    // when seen from outside; t grows downward.
    let a = cu * w + su;
    let b = cv * h + sv;
    var uv: vec2<f32>;
    if axis == 0u {
        uv = vec2<f32>(select(b, -b, positive), -a);
    } else if axis == 1u {
        uv = vec2<f32>(b, a);
    } else {
        uv = vec2<f32>(select(-a, a, positive), -b);
    }

    var out: VOut;
    out.pos = g.view_proj * vec4<f32>(rel, 1.0);
    out.uv = uv;
    out.layer = i32(w1 & 0xffffu);
    // Ambient occlusion per corner (3 = open), interpolated; the flip flag picked the diagonal.
    let ao = f32((w1 >> (16u + 2u * corner)) & 3u) / 3.0;
    out.shade = face_shade(dir) * (0.45 + 0.55 * ao);
    out.dist = d;
    return out;
}

fn fogged(rgb: vec3<f32>, dist: f32) -> vec3<f32> {
    let f = clamp((dist - g.eye.w) / max(g.fog.w - g.eye.w, 1.0), 0.0, 1.0);
    return mix(rgb, g.fog.rgb, f);
}

// Opaque and cutout blocks: pixels that are mostly transparent are dropped.
@fragment
fn fs_opaque(in: VOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, in.uv, in.layer);
    if c.a < 0.5 {
        discard;
    }
    return vec4<f32>(fogged(c.rgb * in.shade, in.dist), 1.0);
}

@fragment
fn fs_translucent(in: VOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, in.uv, in.layer);
    return vec4<f32>(fogged(c.rgb * in.shade, in.dist), c.a);
}
