// SkinnedShieldEffect shaders 1/2: native projection and tone arithmetic.
#import bevy_pbr::{mesh_functions, skinning, forward_io::Vertex,
    view_transformations::position_world_to_clip, mesh_view_bindings::view}
#import magicka_viewer::lighting::magicka_linear

struct ShieldParams {
    projection0: mat4x4<f32>, projection1: mat4x4<f32>, projection2: mat4x4<f32>,
    color: vec4<f32>, offset0: vec4<f32>, offset1: vec4<f32>, offset2: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> p: ShieldParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var projection_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var projection_sampler: sampler;
struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) projection0: vec2<f32>,
    @location(2) projection1: vec2<f32>,
    @location(3) projection2: vec2<f32>,
    @location(4) view_direction: vec3<f32>,
}
@vertex
fn vertex(v: Vertex) -> Varyings {
    var out: Varyings;
#ifdef SKINNED
    let world_from_local = skinning::skin_model(v.joint_indices, v.joint_weights, v.instance_index);
    let normal = skinning::skin_normals(world_from_local, v.normal);
#else
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    let normal = mesh_functions::mesh_normal_local_to_world(v.normal, v.instance_index);
#endif
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(v.position, 1.0));
    out.position = position_world_to_clip(world.xyz + normalize(normal) * 0.075);
    out.normal = normal;
    out.view_direction = normalize(view.world_position.xyz - world.xyz);
    // Native outputs projection xy and w=1, before Bloat.
    out.projection0 = (p.projection0 * world).xy;
    out.projection1 = (p.projection1 * world).xy;
    out.projection2 = (p.projection2 * world).xy;
    return out;
}
fn projection_uv(xy: vec2<f32>, offset: vec2<f32>) -> vec2<f32> {
    return (xy * vec2(0.5, -0.5) + vec2(0.5)) * 0.5 + offset;
}
@fragment
fn fragment(v: Varyings) -> @location(0) vec4<f32> {
    let a = textureSample(projection_map, projection_sampler, projection_uv(v.projection0, p.offset0.xy));
    let b = textureSample(projection_map, projection_sampler, projection_uv(v.projection1, p.offset1.xy));
    let c = textureSample(projection_map, projection_sampler, projection_uv(v.projection2, p.offset2.xy));
    let product = a * p.color * b * c;
    let facing = clamp(1.0 - 0.8 * dot(normalize(v.normal), normalize(v.view_direction)), 0.0, 1.0);
    let rgb = vec3(1.0) - exp2(-product.rgb * product.a * facing);
    return vec4(magicka_linear(rgb), 1.0);
}
