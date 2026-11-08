// Character status looks (status_look.rs, research/status-look.md),
// following SkinnedModelDeferredEffect shader 11.
#define_import_path magicka_viewer::status_look

// x Colorize.W, y Wet, z Frozen bloat, w Frozen
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var<uniform> status_look: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var ice_cube: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(109) var ice_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var ice_normal_cube: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(111) var ice_normal_sampler: sampler;

const COLD_COLOR: vec3<f32> = vec3<f32>(1.0, 1.6, 2.0);
const WET_COLOR: vec3<f32> = vec3<f32>(0.75, 0.8, 1.0);
const LUMINANCE: vec3<f32> = vec3<f32>(0.3086, 0.6094, 0.082);

fn raw(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055, 12.92 * c, c <= vec3<f32>(0.0031308));
}

fn frozen() -> bool {
    return status_look.w > 0.5;
}

// DiffuseColor factor: ×(1 + Colorize.W), and ×WetColor in the Wet branch.
fn diffuse_factor() -> vec3<f32> {
    return (1.0 + status_look.x) * select(vec3<f32>(1.0), WET_COLOR, status_look.y > 0.5);
}

// CubeNormalMapEnabled with CubeMapColor.W = 1: the ice normal replaces the
// surface normal (lookup by the surface normal; CubeMapRotation identity).
fn surface_normal(n: vec3<f32>) -> vec3<f32> {
    if !frozen() {
        return n;
    }
    return normalize(raw(textureSample(ice_normal_cube, ice_normal_sampler, n).rgb) * 2.0 - 1.0);
}

// CubeMapEnabled (Frozen), then Colorize.
fn cubed_albedo(a: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var c = a;
    if frozen() {
        let s = textureSample(ice_cube, ice_sampler, n);
        let ice = raw(s.rgb);
        c = c * (1.0 - s.a) + ice * s.a;
    }
    return c;
}
fn albedo(a: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let c = cubed_albedo(a, n);
    let lum = dot(c, LUMINANCE);
    return c * (1.0 - status_look.x) + lum * COLD_COLOR * status_look.x;
}

// (SpecularBias, SpecularPower factor, EmissiveAmount override or -1).
// Frozen: bias 1, power 10 (absolute), emissive 0.5; Wet: bias 0.5, power ×2.
fn spec_bias() -> f32 {
    if frozen() { return 1.0; }
    if status_look.y > 0.5 { return 0.5; }
    return 0.0;
}
fn spec_power(base: f32) -> f32 {
    if frozen() { return 10.0; }
    if status_look.y > 0.5 { return base * 2.0; }
    return base;
}
fn emissive(base: f32) -> f32 {
    return select(base, 0.5, frozen());
}
