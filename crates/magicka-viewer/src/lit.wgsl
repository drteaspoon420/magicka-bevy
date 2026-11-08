// StandardMaterial albedo (texture * base colour * vertex colour) shaded with
// the original PolygonHead deferred lighting (lighting.wgsl).
#import bevy_pbr::pbr_fragment::pbr_input_from_standard_material
#import bevy_pbr::{forward_io::{VertexOutput, FragmentOutput}, pbr_functions::alpha_discard}
#import magicka_viewer::lighting::{MagickaSurface, magicka_shade, magicka_sun_shadow, magicka_raw, magicka_linear}
#import magicka_viewer::status_look

struct LitParams {
    // SpecAmount, SpecPower, EmissiveAmount, zapped (DiffuseColor and
    // SpecularAmount zero while Character.mZapTimer > 0)
    material: vec4<f32>,
    // x: SpecularMap enabled, y: ethereal alpha, z: additive body enabled
    maps: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> lit: LitParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var material_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var material_sampler: sampler;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var input = pbr_input_from_standard_material(in, is_front);
    let base = alpha_discard(input.material, input.material.base_color);
    if lit.maps.z > 0.5 {
        let raw = magicka_raw(base.rgb) * status_look::diffuse_factor();
        let intensity = dot(status_look::cubed_albedo(raw, input.N), status_look::LUMINANCE)
            * status_look::COLD_COLOR * base.a * lit.maps.y;
        var out: FragmentOutput;
        out.color = vec4<f32>(magicka_linear(1.0 - exp2(-intensity)), 1.0);
        return out;
    }
    let shadow = magicka_sun_shadow(in.world_position, input.N, in.position.xy);
    let zap = lit.material.w > 0.5;
    // Skinned SpecularMap: R emissive, G spec amount, B spec power.
    var m = vec3<f32>(1.0);
#ifdef VERTEX_UVS_A
    if lit.maps.x > 0.5 {
        m = magicka_raw(textureSample(material_map, material_sampler, in.uv).rgb);
    }
#endif
    // Status looks: DiffuseColor factor, ice cube, Colorize (status_look).
    let albedo = status_look::albedo(magicka_raw(base.rgb) * status_look::diffuse_factor(), input.N);
    let shaded = magicka_shade(MagickaSurface(
        select(albedo, vec3<f32>(0.0), zap),
        select(lit.material.x * m.g + status_look::spec_bias(), 0.0, zap),
        status_look::spec_power(lit.material.y) * m.b,
        status_look::emissive(lit.material.z) * m.r,
        status_look::surface_normal(input.N),
        in.world_position.xyz,
        shadow,
    ));
    var out: FragmentOutput;
    out.color = vec4<f32>(magicka_linear(shaded), base.a);
    return out;
}
