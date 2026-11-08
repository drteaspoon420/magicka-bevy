// Avatar.OutlineRenderData passes (outline.rs). SkinnedModelDeferredEffect
// vertex shader 10 bloats along the normalized skinned normal:
// world = skinned_position + normalize(skinned_normal) * Bloat.
#import bevy_pbr::{
    mesh_functions,
    skinning,
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    view_transformations::position_world_to_clip,
    prepass_utils,
}
#import magicka_viewer::lighting::{MagickaSurface, magicka_shade, magicka_sun_shadow, magicka_linear}

// color: TintColor (raw RGB);
// control: Bloat, DepthBias coefficient k, EmissiveAmount, unused
struct OutlineParams { color: vec4<f32>, control: vec4<f32> }
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> p: OutlineParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var opaque_depth: texture_2d<f32>;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
#ifdef SKINNED
    let world_from_local = skinning::skin_model(vertex.joint_indices, vertex.joint_weights, vertex.instance_index);
    let normal = normalize(skinning::skin_normals(world_from_local, vertex.normal));
#else
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let normal = normalize(mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index));
#endif
    let skinned = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    let world = skinned.xyz + normal * p.control.x;
    out.world_position = vec4<f32>(world, 1.0);
    out.world_normal = normal;
    var clip = position_world_to_clip(world);
    // D3D9 DepthBias on XNA's conventional depth, as a reverse-Z scale
    // (outline.rs depth_bias_coefficient).
    clip.z *= 1.0 - p.control.y * clip.w;
    out.position = clip;
#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
    return out;
}

@fragment
fn fragment(
    in: VertexOutput,
#ifdef MULTISAMPLED
    @builtin(sample_index) sample_index: u32,
#endif
) -> FragmentOutput {
#ifndef MULTISAMPLED
    let sample_index = 0u;
#endif
#ifdef DEPTH_PREPASS
    // Stencil stand-in: only level-owned prepass geometry hides the
    // player. Reverse-Z: hidden means a smaller depth than the occluder.
    if p.control.w > 0.5 {
        let level_depth = prepass_utils::prepass_depth(in.position, sample_index);
        // The full snapshot is sample 0, shared with particles. A nearer
        // entity replaces the level's stencil category in the original.
        let selected_depth = prepass_utils::prepass_depth(in.position, 0u);
        let nearest_depth = textureLoad(opaque_depth, vec2<i32>(in.position.xy), 0).r;
        if !(in.position.z < level_depth) || nearest_depth > selected_depth {
            discard;
        }
    }
#endif
    // Diffuse/specular maps off, SpecularAmount 0, EmissiveAmount 1: the
    // composite's lit term is weighted by saturate(1 - 1) = 0, so the result
    // is 1 - 2^-TintColor wherever the G-buffer was lit.
    let n = normalize(in.world_normal);
    let shadow = magicka_sun_shadow(in.world_position, n, in.position.xy);
    let shaded = magicka_shade(MagickaSurface(p.color.rgb, 0.0, 0.0, p.control.z,
        n, in.world_position.xyz, shadow));
    var out: FragmentOutput;
    out.color = vec4<f32>(magicka_linear(shaded), 1.0);
    return out;
}
