// SkinnedModelDeferredEffect shader 11's detail/fabric albedo blend.
// Verified against the original bytecode and D3DX preshader; see robe-colour.md.
#import bevy_pbr::pbr_fragment::pbr_input_from_standard_material
#ifdef PREPASS_PIPELINE
#import bevy_pbr::{prepass_io::{VertexOutput, FragmentOutput}, pbr_deferred_functions::deferred_output}
#else
#import bevy_pbr::{forward_io::{VertexOutput, FragmentOutput}, pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT}
#import magicka_viewer::lighting::{MagickaSurface, magicka_shade, magicka_sun_shadow}
#endif
#import magicka_viewer::status_look

// spec: SpecularAmount, SpecularPower, EmissiveAmount, unused
struct RobeParams { diffuse: vec4<f32>, tint: vec4<f32>, control: vec4<f32>, spec: vec4<f32> }
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> p: RobeParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var detail_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var detail_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var overlay_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var overlay_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var material_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var material_sampler: sampler;

fn raw_rgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055, 12.92 * c, c <= vec3<f32>(0.0031308));
}
fn linear_rgb(c: vec3<f32>) -> vec3<f32> {
    return select(pow(max((c + 0.055) / 1.055, vec3<f32>(0.0)), vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}
fn robe_albedo(detail: vec4<f32>, overlay: vec4<f32>) -> vec3<f32> {
    let flags = u32(p.control.y);
    // DiffuseColor carries the Cold/Wet factors (status_look).
    let base = select(vec3<f32>(1.0), detail.rgb, (flags & 1u) != 0u) * p.diffuse.rgb
        * status_look::diffuse_factor();
    if (flags & 2u) == 0u { return base; }
    var fabric = overlay.rgb * p.tint.rgb;
    if (flags & 4u) != 0u {
        let lo = (1.0 - 2.0 * p.tint.rgb) * overlay.rgb * overlay.rgb + 2.0 * overlay.rgb * p.tint.rgb;
        let hi = (2.0 * p.tint.rgb - 1.0) * sqrt(overlay.rgb) + 2.0 * overlay.rgb * (1.0 - p.tint.rgb);
        fabric = select(lo, hi, p.tint.rgb >= vec3<f32>(0.5));
    }
    let weight = overlay.a * p.control.x;
    return base * (1.0 - weight) + fabric * weight;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var input = pbr_input_from_standard_material(in, is_front);
    var uv = vec2<f32>(0.0);
#ifdef VERTEX_UVS_A
    uv = in.uv;
#endif
    let a = textureSample(detail_tex, detail_sampler, uv);
    let b = textureSample(overlay_tex, overlay_sampler, uv);
    // Original D3D9 reads raw normalized bytes; Bevy image samples are linear.
    // control.z: LightningZap sets DiffuseColor and SpecularAmount to zero.
    let zap = p.control.z > 0.5;
    let normal = status_look::surface_normal(input.N);
    let albedo = select(
        status_look::albedo(robe_albedo(vec4<f32>(raw_rgb(a.rgb), a.a), vec4<f32>(raw_rgb(b.rgb), b.a)), input.N),
        vec3<f32>(0.0), zap);
    // SpecularMapEnabled: material map R scales EmissiveAmount, G
    // SpecularAmount, B SpecularPower (shader 11); otherwise 1.
    let m = select(vec3<f32>(1.0), raw_rgb(textureSample(material_tex, material_sampler, uv).rgb), p.control.w > 0.5);
    let spec_amount = select(p.spec.x * m.g + status_look::spec_bias(), 0.0, zap);
    input.material.base_color = vec4<f32>(linear_rgb(albedo), 1.0);
#ifdef PREPASS_PIPELINE
    return deferred_output(in, input);
#else
    var out: FragmentOutput;
    // Character.UpdateRenderData's AnimatedModelAdditive technique.
    if (u32(p.control.y) & 8u) != 0u {
        let flags = u32(p.control.y);
        let detail_alpha = select(1.0, a.a, (flags & 1u) != 0u);
        let weight = select(0.0, b.a * p.control.x, (flags & 2u) != 0u);
        let alpha = detail_alpha * (1.0 - weight) + weight;
        let intensity = dot(status_look::cubed_albedo(robe_albedo(vec4<f32>(raw_rgb(a.rgb), a.a), vec4<f32>(raw_rgb(b.rgb), b.a)), input.N), status_look::LUMINANCE) * status_look::COLD_COLOR * alpha * p.spec.w;
        out.color = vec4<f32>(linear_rgb(1.0 - exp2(-intensity)), 1.0);
        return out;
    }
    if (input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        // Original deferred lighting on the raw albedo (lighting.wgsl).
        let shadow = magicka_sun_shadow(in.world_position, input.N, in.position.xy);
        let shaded = magicka_shade(MagickaSurface(albedo, spec_amount,
            status_look::spec_power(p.spec.y) * m.b, status_look::emissive(p.spec.z) * m.r,
            normal, in.world_position.xyz, shadow));
        out.color = vec4<f32>(linear_rgb(shaded), 1.0);
    } else { out.color = input.material.base_color; }
    return out;
#endif
}
