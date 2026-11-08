// PolygonHead RenderDeferredLiquidEffect, Default technique (no reflection
// map: Water.cs selects technique 1 only when ReflectionMap != null), ported
// from the disassembled ps_3_0 shader 7 and its preshader
// (output/evidence/shaders/RenderDeferredLiquidEffect.{asm,preshader}.txt,
// research/water-render.md), including the ice blend driven by the
// per-vertex freeze stream (UV_1.x here). Lighting is the original deferred
// lighting (lighting.wgsl).

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::view
#import magicka_viewer::lighting::{MagickaSurface, magicka_shade, magicka_sun_shadow}

struct LiquidParams {
    // xyz BottomColor, w BottomTextureEnabled
    bottom: vec4<f32>,
    // xyz DeepBottomColor, w WaveHeight
    deep: vec4<f32>,
    // xyz ReflectColor (fx default 0.6, 0.9, 1.0), w WaterReflectiveness
    reflect: vec4<f32>,
    // WaveSpeed0.xy, WaveSpeed1.xy
    speeds: vec4<f32>,
    // x Time, y WaterEmissiveAmount, z WaterSpecAmount, w WaterSpecPower
    misc: vec4<f32>,
    // xyz IceColor, w IceDiffuseMap present
    ice: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> p: LiquidParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var normal_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var normal_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var bottom_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var bottom_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var ice_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var ice_samp: sampler;

// Shader 7's freeze curve: f is the saturated vertex freeze value, ice_a the
// IceDiffuse alpha; returns the ice weight k.
fn ice_weight(f: f32, ice_a: f32) -> f32 {
    let s = sqrt(f);
    let t = saturate((s * 0.05 + 0.25) * (1.0 - ice_a) + (1.0 - s));
    let h = t - 0.5;
    let u = 2.0 * h;
    let upper = u * (u * (h * -0.5 + 0.55) - 0.4) + 0.1;
    let t2 = 2.0 * t;
    let lower = t2 * (t2 * (t * 1.1 - 0.8) - 0.45) + 0.8;
    let poly = select(lower, upper, h >= 0.0);
    return saturate(saturate(poly * 12.5) * 3.0 - 1.0);
}

fn to_raw(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3(0.0031308));
}

fn to_lin(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3(0.0)) + 0.055) / 1.055, vec3(2.4));
    return select(hi, lo, c <= vec3(0.04045));
}

fn wave(uv: vec2<f32>) -> vec2<f32> {
    return textureSample(normal_tex, normal_samp, uv).xy;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var uv = vec2(0.0);
#ifdef VERTEX_UVS_A
    uv = in.uv;
#endif
    // Preshader: c0 = Time·WaveSpeed0, c1 = 0.6·Time·WaveSpeed0,
    // c2 = Time·WaveSpeed1, c3 = 0.6·Time·WaveSpeed1.
    let t = p.misc.x;
    let c0 = t * p.speeds.xy;
    let c1 = 0.6 * t * p.speeds.xy;
    let c2 = t * p.speeds.zw;
    let c3 = 0.6 * t * p.speeds.zw;
    let a = uv + c1;
    var w = wave(vec2(-a.y, a.x)) * 0.2;
    w = wave(uv + c0) * 0.3 + w;
    let b = uv + c2;
    w = wave(vec2(b.y, -b.x)) * 0.3 + w;
    w = wave(-(uv + c3)) * 0.2 + w;
    let r1 = (2.0 * w - 1.0) * p.deep.w;
    let up = sqrt(1.0 - dot(r1, r1));

    let nrm = in.world_normal;
    var tangent = vec3(1.0, 0.0, 0.0);
    var binormal = vec3(0.0, 0.0, 1.0);
#ifdef VERTEX_TANGENTS
    tangent = in.world_tangent.xyz;
    binormal = cross(nrm, tangent) * in.world_tangent.w;
#endif
    let n = r1.x * tangent + r1.y * binormal + up * nrm;

    let eye = normalize(view.world_position.xyz - in.world_position.xyz);
    let facing = dot(eye, n);

    var bottom = p.bottom.rgb;
    if p.bottom.w > 0.5 {
        let s = textureSample(bottom_tex, bottom_samp, n.xy * 0.1 + uv);
        bottom = to_raw(s.rgb) * p.bottom.rgb;
    }
    var deep_w = 0.0;
#ifdef VERTEX_COLORS
    deep_w = in.color.r;
#endif
    let under = mix(bottom, p.deep.rgb, deep_w);
    let k = 1.0 - p.reflect.w * (1.0 - facing);
    let water = mix(p.reflect.rgb, under, k);

    // Ice: albedo → IceColor·ice, normal → mix of water, surface and the
    // ice texture's (1, 1, 0.1)-scaled colour, emissive → 0.25, spec → 1/20.
    var freeze = 0.0;
#ifdef VERTEX_UVS_B
    freeze = saturate(in.uv_b.x);
#endif
    // No IceDiffuseMap: D3D9 samples an unset texture as (0, 0, 0, 1)
    // (INFERRED; 9 of 62 retail liquids).
    var ice = vec4(0.0, 0.0, 0.0, 1.0);
    if p.ice.w > 0.5 {
        let s = textureSample(ice_tex, ice_samp, uv);
        ice = vec4(to_raw(s.rgb), s.a);
    }
    let iw = ice_weight(freeze, ice.a);
    let normal = normalize(n * saturate(1.0 - 5.0 * iw) + nrm * saturate(5.0 * iw - 4.0)
        + normalize(ice.rgb * vec3(1.0, 1.0, 0.1)) * saturate(min(5.0 - 5.0 * iw, 5.0 * iw)));
    let albedo = mix(water, p.ice.rgb * ice.rgb, iw);
    let emissive = mix(p.misc.y, 0.25, iw);
    let spec_amount = mix(p.misc.z, 1.0, iw);
    let spec_power = mix(p.misc.w, 20.0, iw);

    // The liquid G-buffer uses the same encoding as RenderDeferredEffect
    // (normal scaled by 2^-emissive, spec amount/power), INFERRED here.
    let shadow = magicka_sun_shadow(in.world_position, normal, in.position.xy);
    let out = magicka_shade(MagickaSurface(albedo, spec_amount, spec_power, emissive,
        normal, in.world_position.xyz, shadow));
    return vec4(to_lin(out), 1.0);
}
