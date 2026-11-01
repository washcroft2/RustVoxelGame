// Vertex pulling with per-chunk records addressed through first_instance.
// Each quad is three u32 words (see spike-mesher's Quad). A draw covers one
// chunk's quads: first_vertex = 6 * first quad, and first_instance = chunk index.

struct Rec { origin: vec4<f32> }

@group(0) @binding(0) var<storage, read> quads: array<u32>;
@group(0) @binding(1) var<storage, read> recs: array<Rec>;
@group(0) @binding(2) var<uniform> screen: vec4<f32>;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
}

fn palette(m: u32) -> vec3<f32> {
    // 16 flat colours whose components survive Rgba8Unorm exactly.
    var c = array<vec3<f32>, 16>(
        vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0), vec3(1.0, 1.0, 0.0),
        vec3(0.0, 1.0, 1.0), vec3(0.6, 0.2, 0.0), vec3(0.2, 0.6, 0.0), vec3(0.0, 0.2, 0.6),
        vec3(0.8, 0.8, 0.8), vec3(0.4, 0.4, 0.4), vec3(1.0, 0.6, 0.0), vec3(0.6, 0.0, 0.6),
        vec3(0.0, 0.6, 0.6), vec3(1.0, 0.4, 0.4), vec3(0.4, 1.0, 0.4), vec3(0.4, 0.4, 1.0),
    );
    return c[m & 15u];
}

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    let q = vi / 6u;
    let k = vi % 6u;
    let w0 = quads[3u * q];
    let w1 = quads[3u * q + 1u];
    let x = f32(w0 & 63u);
    let y = f32((w0 >> 6u) & 63u);
    let w = f32((w0 >> 18u) & 63u);
    let h = f32((w0 >> 24u) & 63u);
    let cx = select(0.0, 1.0, k == 1u || k == 4u || k == 5u);
    let cy = select(0.0, 1.0, k == 2u || k == 3u || k == 5u);
    let px = recs[ii].origin.x + x + cx * w;
    let py = recs[ii].origin.y + y + cy * h;
    var out: VOut;
    out.pos = vec4(px / screen.x * 2.0 - 1.0, 1.0 - py / screen.y * 2.0, 0.0, 1.0);
    out.color = palette(w1 & 0xffffu);
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    return vec4(in.color, 1.0);
}
