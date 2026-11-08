// Magicka/PolygonHead RenderDeferredEffect layer blending, ported from the
// disassembled ps_3_0 shader 7 (output/research/shaders.md). Verified against
// the executed bytecode (check_terrain_formula.py) and pixel-checked on the
// GPU (tools/terrain_check.py).
//
// The game blends raw texture bytes (D3D9, no sRGB decode). Bevy samples sRGB
// textures as linear, so values are converted back to raw, blended exactly as
// the original, and re-encoded so the output byte equals the game's value.

#import bevy_pbr::mesh_view_bindings::view

#ifdef PREPASS_PIPELINE
#import bevy_pbr::prepass_io::VertexOutput
#else
#import bevy_pbr::forward_io::VertexOutput
#import magicka_viewer::lighting::{MagickaSurface, magicka_shade, magicka_sun_shadow}
#endif

struct TerrainParams {
    color0: vec4<f32>,
    color1: vec4<f32>,
    // x: flag bits (see terrain.rs), y/z/w unused
    flags: vec4<u32>,
    // x: Sharpness
    misc: vec4<f32>,
    // SpecAmount, SpecPower, EmissiveAmount per layer (w unused)
    spec0: vec4<f32>,
    spec1: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> p: TerrainParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var tex0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var samp0: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var tex1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var samp1: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var mat0_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var mat0_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var mat1_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var mat1_samp: sampler;

// Optional camera-dependent cutaway (cutaway.rs; not an original feature).
struct Cutaway {
    // x enabled (2: debug tint instead of cutting), y knee height, z depth
    // epsilon (world units), w edge px
    params: vec4<f32>,
    // x edge darkening, yz floor-depth target size (px), w grid cell
    target_size: vec4<f32>,
    // xy grid origin (world x, z), zw grid size (cells)
    grid: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(91) var<storage, read> cutaway: Cutaway;
// Navmesh-only depth from the gameplay camera (reverse-Z, 0 = no floor).
@group(#{MATERIAL_BIND_GROUP}) @binding(92) var cutaway_depth: texture_2d<f32>;
// Lowest navigable elevation per XZ cell (>= NO_FLOOR where none).
@group(#{MATERIAL_BIND_GROUP}) @binding(93) var cutaway_heights: texture_2d<f32>;

const TEX0: u32 = 1u;
const A0_OFF: u32 = 2u;
const MASK0: u32 = 4u;
const A1_OFF: u32 = 8u;
const MASK1: u32 = 16u;
const DUAL: u32 = 32u;
const VCOLOR: u32 = 64u;
const UNLIT: u32 = 128u;
const NO_DISCARD: u32 = 256u; // geometry-only checks (viewer --opaque)
const MAT0: u32 = 512u;
const MAT1: u32 = 1024u;
const CUTAWAY: u32 = 2048u;
const OUTLINE_OCCLUDER: u32 = 4096u;
const NO_FLOOR: f32 = 1.0e29;

fn to_raw(c: vec3<f32>) -> vec3<f32> {
    // linear -> sRGB transfer (exact piecewise curve)
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3(0.0031308));
}

fn to_lin(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3(0.0)) + 0.055) / 1.055, vec3(2.4));
    return select(hi, lo, c <= vec3(0.04045));
}

fn has(bit: u32) -> bool {
    return (p.flags.x & bit) != 0u;
}

struct Surface {
    albedo: vec3<f32>,
    coverage: f32,
    // layer-1 weight, which also blends the material properties
    w: f32,
    uv0: vec2<f32>,
    uv1: vec2<f32>,
}

// Layer colors and coverage, shared by the color pass and the depth prepass
// (which must discard the same fragments).
fn surface(in: VertexOutput) -> Surface {
    var uv0 = vec2(0.0);
    var uv1 = vec2(0.0);
#ifdef VERTEX_UVS_A
    uv0 = in.uv;
    uv1 = in.uv;
#endif
#ifdef VERTEX_UVS_B
    uv1 = in.uv_b;
#endif
    var vcol = vec4(1.0);
#ifdef VERTEX_COLORS
    if has(VCOLOR) {
        vcol = in.color;
    }
#endif

    // Layer 0 (DiffuseSampler0); without a texture the color is the constant.
    var c0 = p.color0.rgb;
    var a0 = 1.0;
    if has(TEX0) {
        let t = textureSample(tex0, samp0, uv0);
        c0 = to_raw(t.rgb) * p.color0.rgb;
        a0 = select(t.a, 1.0, has(A0_OFF));
    }

    var albedo = c0;
    var coverage = select(a0, 1.0, has(MASK0));
    var weight = 0.0;
    if has(DUAL) {
        let t1 = textureSample(tex1, samp1, uv1);
        let c1 = to_raw(t1.rgb) * p.color1.rgb;
        let a1 = select(t1.a, 1.0, has(A1_OFF));
        let v = vcol.a;
        var m: f32;
        var k: f32;
        var cov0: f32;
        if has(MASK0) {
            m = select(a0, max(a0, 1.0 - a1), has(MASK1));
            k = select(a1, 1.0, has(MASK1));
            cov0 = 1.0;
        } else {
            m = select(v, 1.0 - a1, has(MASK1));
            k = select(a1, 1.0, has(MASK1));
            cov0 = a0;
        }
        let t = saturate((0.05 * v + 0.25) * m + (1.0 - v));
        var f: f32;
        if t - 0.5 >= 0.0 {
            let u = 2.0 * (t - 0.5);
            f = u * (u * ((t - 0.5) * -0.5 + 0.55) - 0.4) + 0.1;
        } else {
            let z = 2.0 * t;
            f = z * (z * (t * 1.1 - 0.8) - 0.45) + 0.8;
        }
        let s = p.misc.x;
        let w = saturate((s + 1.0) * saturate(f * 12.5) - s * 0.5) * k;
        albedo = c0 * (1.0 - w) + c1 * w;
        coverage = cov0 * (1.0 - w) + w;
        weight = w;
    }
    return Surface(albedo * vcol.rgb, coverage, weight, uv0, uv1);
}


// Floor elevation under a world XZ position: bilinear over the valid cells
// of the navmesh height grid, NO_FLOOR when none is near.
fn cutaway_floor_y(xz: vec2<f32>) -> f32 {
    let size = vec2<i32>(cutaway.grid.zw);
    let g = (xz - cutaway.grid.xy) / cutaway.target_size.w - 0.5;
    let base = vec2<i32>(floor(g));
    let f = g - floor(g);
    var sum = 0.0;
    var weight = 0.0;
    for (var i = 0; i < 4; i++) {
        let o = vec2<i32>(i & 1, i >> 1u);
        let c = base + o;
        if any(c < vec2(0)) || any(c >= size) {
            continue;
        }
        let h = textureLoad(cutaway_heights, c, 0).r;
        if h >= NO_FLOOR {
            continue;
        }
        let w = max(select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1), 1e-4);
        sum += h * w;
        weight += w;
    }
    return select(NO_FLOOR, sum / max(weight, 1e-6), weight > 0.0);
}

// Is a point (pixel, reverse-Z depth) nearer than the navigable floor seen
// through that pixel? Both depths are linearised to view distance before
// the world-unit epsilon is applied.
fn cutaway_occludes(pixel: vec2<f32>, depth: f32) -> bool {
    if any(pixel < vec2(0.0)) || any(pixel >= cutaway.target_size.yz) || depth <= 0.0 {
        return false;
    }
    let floor_depth = textureLoad(cutaway_depth, vec2<i32>(pixel), 0).r;
    if floor_depth <= 0.0 {
        return false;
    }
    // Infinite reverse-Z perspective: distance = near / depth.
    let near = view.clip_from_view[3][2];
    let wall_in_front_of_floor = near / floor_depth - near / depth > cutaway.params.z;
    return wall_in_front_of_floor;
}

// Debug view: navmesh floor depth present at this pixel.
fn cutaway_floor_seen(pixel: vec2<f32>) -> bool {
    if any(pixel >= cutaway.target_size.yz) {
        return false;
    }
    return textureLoad(cutaway_depth, vec2<i32>(pixel), 0).r > 0.0;
}

// -1 cuts the fragment; otherwise the cut-edge rim weight in [0, 1].
// `dy` is fwidth(world y) (0 skips the rim).
fn cutaway_state(frag: vec4<f32>, world: vec3<f32>, dy: f32) -> f32 {
    if !has(CUTAWAY) || has(NO_DISCARD) || cutaway.params.x < 0.5 {
        return 0.0;
    }
    // Only the perspective gameplay view the floor depth was rendered for:
    // shadow maps and other targets keep the full walls.
    if view.clip_from_view[3][3] != 0.0 || any(view.viewport.xy != vec2(0.0))
        || any(abs(view.viewport.zw - cutaway.target_size.yz) > vec2(0.5)) {
        return 0.0;
    }
    let floor_y = cutaway_floor_y(world.xz);
    if floor_y >= NO_FLOOR {
        return 0.0;
    }
    let cut_y = floor_y + cutaway.params.y;
    let below = cut_y - world.y;
    // The floor itself, and anything lower than the cut edge, never changes.
    let rim_width = cutaway.params.w * dy;
    if world.y <= floor_y + cutaway.params.z || (below > rim_width && cutaway.params.x < 1.5) {
        return 0.0;
    }
    // The pixel's own floor, else the floor behind the wall column's point at
    // the cut height: upper wall pixels with no floor directly behind them
    // follow their column instead of floating.
    var occludes = cutaway_occludes(frag.xy, frag.z);
    let knee = view.clip_from_world * vec4(world.x, cut_y, world.z, 1.0);
    if !occludes && knee.w > 0.0 {
        let ndc = knee.xyz / knee.w;
        let pixel = view.viewport.xy + (vec2(ndc.x, -ndc.y) * 0.5 + 0.5) * view.viewport.zw;
        occludes = cutaway_occludes(pixel, ndc.z);
    }
    if !occludes {
        return 0.0;
    }
    if cutaway.params.x > 1.5 {
        // Debug view: tint (forward pass) instead of cutting.
        return select(2.0, 3.0, world.y > cut_y);
    }
    if below < 0.0 {
        return -1.0;
    }
    if rim_width <= 0.0 {
        return 0.0;
    }
    return 1.0 - smoothstep(0.0, rim_width, below);
}

#ifdef PREPASS_PIPELINE
@fragment
fn fragment(in: VertexOutput) {
    if !has(OUTLINE_OCCLUDER) {
        discard;
    }
    if surface(in).coverage - 0.5 < 0.0 && !has(NO_DISCARD) {
        discard;
    }
    if cutaway_state(in.position, in.world_position.xyz, 0.0) < 0.0 {
        discard;
    }
}
#else
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Derivatives before any discard (uniform control flow).
    let dy = fwidth(in.world_position.y);
    let surf = surface(in);
    if surf.coverage - 0.5 < 0.0 && !has(NO_DISCARD) {
        discard;
    }
    let cut = cutaway_state(in.position, in.world_position.xyz, dy);
    if cut < 0.0 {
        discard;
    }
    let albedo = surf.albedo;

    var out = albedo;
    if !has(UNLIT) {
        // Original deferred lighting (lighting.wgsl). Material and normal
        // maps are not exported yet; vertex normals are used.
        // Shader 7: (emissive, spec amount, spec power) × the material
        // texture (layer 1's map only blends in when layer 0 has one).
        var material = vec3(1.0);
        if has(MAT0) {
            material = textureSample(mat0_tex, mat0_samp, surf.uv0).rgb;
            if has(MAT1) {
                let m1 = textureSample(mat1_tex, mat1_samp, surf.uv1).rgb;
                material = material * (1.0 - surf.w) + m1 * surf.w;
            }
        }
        // spec0/1 are (amount, power, emissive); the map is (R emissive,
        // G spec amount, B spec power).
        let m = (p.spec0.xyz * (1.0 - surf.w) + p.spec1.xyz * surf.w) * material.gbr;
        let shadow = magicka_sun_shadow(in.world_position, in.world_normal, in.position.xy);
        out = magicka_shade(MagickaSurface(albedo, m.x, m.y, m.z, in.world_normal,
            in.world_position.xyz, shadow));
    }
    if cut >= 2.0 {
        // Debug view: red above the cut height, orange below it.
        out = mix(out, select(vec3(1.0, 0.5, 0.0), vec3(1.0, 0.0, 0.0), cut > 2.5), 0.6);
    } else {
        // Faint dark edge just below the cut (cutaway only; 0 otherwise).
        out *= 1.0 - cut * cutaway.target_size.x;
    }
    if has(CUTAWAY) && cutaway.params.x > 1.5 && cutaway_floor_seen(in.position.xy) {
        out = mix(out, vec3(0.0, 0.3, 1.0), 0.35);
    }
    return vec4(to_lin(out), 1.0);
}
#endif
