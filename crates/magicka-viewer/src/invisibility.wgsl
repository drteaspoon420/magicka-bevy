// InvisibilityEffect ps_3_0 (output/evidence/invisibility): screen uv of the
// fragment, scene depth behind it, offset = normal.xy · Distortion·0.02 ·
// saturate((sceneDepth - ownDepth)·0.5), y scaled by width/height, then the
// scene colour at the offset uv, opaque.
#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings as view_bindings,
    prepass_utils,
    view_transformations::depth_ndc_to_view_z,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: vec4<f32>;

@fragment
fn fragment(
    in: VertexOutput,
#ifdef MULTISAMPLED
    @builtin(sample_index) sample_index: u32,
#endif
) -> @location(0) vec4<f32> {
#ifndef MULTISAMPLED
    let sample_index = 0u;
#endif
    let size = vec2<f32>(textureDimensions(view_bindings::view_transmission_texture));
    let uv = in.position.xy / size;
    let own_depth = -depth_ndc_to_view_z(in.position.z);
    // Without a depth prepass nothing limits the offset.
    var scene_depth = own_depth + 2.0;
#ifdef DEPTH_PREPASS
    scene_depth = -depth_ndc_to_view_z(prepass_utils::prepass_depth(in.position, sample_index));
#endif
    let w = clamp((scene_depth - own_depth) * 0.5, 0.0, 1.0) * params.x;
    let offset = vec2<f32>(w * 0.02, w * 0.02 * size.x / size.y);
    // o1: normalize(ViewProjection 3x3 · normal).
    let n = normalize((view_bindings::view.clip_from_world * vec4<f32>(in.world_normal, 0.0)).xyz);
    let c = textureSampleLevel(
        view_bindings::view_transmission_texture,
        view_bindings::view_transmission_sampler,
        uv + n.xy * offset,
        0.0,
    ).rgb;
    return vec4<f32>(c, 1.0);
}
