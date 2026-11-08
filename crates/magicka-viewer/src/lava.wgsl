// LavaEffect shader 5 and preshader, output/evidence/lava-render/.
#import bevy_pbr::forward_io::VertexOutput
#import magicka_viewer::lighting::{MagickaSurface, magicka_shade, magicka_sun_shadow}

struct LavaParams {
    speeds: vec4<f32>,
    motion: vec4<f32>,
    emissive: vec4<f32>,
    spec: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> p: LavaParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var temp_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var temp_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var mask_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var mask_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var tone_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var tone_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var rock_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var rock_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(9) var normal_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(10) var normal_samp: sampler;

fn raw(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(max(c, vec3(0.0)), vec3(1.0/2.4)) - 0.055, c * 12.92, c <= vec3(0.0031308));
}
fn linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((max(c, vec3(0.0)) + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}
fn temperature(uv: vec2<f32>) -> vec3<f32> {
    return textureSample(temp_tex, temp_samp, uv * p.motion.z).xyz * vec3(2.0, 2.0, 1.0) - vec3(1.0, 1.0, 0.0);
}
fn sharpen(t: f32) -> f32 {
    let h = t - 0.5;
    let u = 2.0 * h;
    let upper = u * (u * (h * -0.5 + 0.55) - 0.4) + 0.1;
    let v = 2.0 * t;
    let lower = v * (v * (t * 1.1 - 0.8) - 0.45) + 0.8;
    return saturate(select(lower, upper, h >= 0.0) * 12.5);
}
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var uv = vec2(0.0);
#ifdef VERTEX_UVS_A
    uv = in.uv;
#endif
    let a = uv + 0.6 * p.motion.x * p.speeds.xy;
    let b = uv + p.motion.x * p.speeds.zw;
    let temp = temperature(vec2(-a.y, a.x)) * 0.2
        + temperature(uv + p.motion.x * p.speeds.xy) * 0.3
        + temperature(vec2(b.y, -b.x)) * 0.3
        + temperature(-(uv + 0.6 * p.motion.x * p.speeds.zw)) * 0.2;
    var heat = 1.0;
#ifdef VERTEX_COLORS
    // Vertex shader 6: output COLOR0 = 1 - stored COLOR0.
    heat = 1.0 - in.color.r;
#endif
    let hot = temp * heat;
    let mask = textureSample(mask_tex, mask_samp, uv + hot.xy * p.motion.y).r;
    let tone_x = sharpen(saturate((hot.z * 0.05 + 0.25) * mask + 1.0 - hot.z));
    let lava = raw(textureSample(tone_tex, tone_samp, vec2(tone_x, 0.5)).rgb);
    let rock = textureSample(rock_tex, rock_samp, uv);
    var frozen = 0.0;
#ifdef VERTEX_UVS_B
    frozen = saturate(in.uv_b.x);
#endif
    let k = sharpen(saturate((frozen * 0.05 + 0.25) * rock.a + 1.0 - frozen)) * frozen;
    var rock_xy = vec2(0.0);
    if p.emissive.w > 0.5 {
        rock_xy = (textureSample(normal_tex, normal_samp, uv).xy * 2.0 - 1.0) * p.motion.w;
    }
    let local_normal = normalize(vec3(rock_xy * k, 1.0 + k * (sqrt(1.0 - dot(rock_xy, rock_xy)) - 1.0)));
    var tangent = vec3(1.0, 0.0, 0.0);
    var bitangent = vec3(0.0, 0.0, 1.0);
#ifdef VERTEX_TANGENTS
    tangent = in.world_tangent.xyz;
    bitangent = cross(in.world_normal, tangent) * in.world_tangent.w;
#endif
    let normal = local_normal.x * tangent + local_normal.y * bitangent + local_normal.z * in.world_normal;
    // RockColor is not referenced by native shader 5.
    let albedo = mix(lava, raw(rock.rgb), k);
    let emissive = mix(mix(p.emissive.x, p.emissive.y, hot.z), p.emissive.z, k);
    let shadow = magicka_sun_shadow(in.world_position, normal, in.position.xy);
    let colour = magicka_shade(MagickaSurface(albedo, mix(p.spec.x, p.spec.y, k), mix(p.spec.z, p.spec.w, k), emissive, normal, in.world_position.xyz, shadow));
    return vec4(linear(colour), 1.0);
}
