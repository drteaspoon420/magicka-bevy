// SkinnedModelSkeletonEffect technique 1 pixel shader: the vertex colour
// (white). zap.rs explains the occlusion stand-in for the body stencil.
#import bevy_pbr::{
    forward_io::VertexOutput,
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
#ifdef DEPTH_PREPASS
    // View z is negative in front of the camera; nearer is larger.
    let line_z = depth_ndc_to_view_z(in.position.z);
    let occluder_z = depth_ndc_to_view_z(prepass_utils::prepass_depth(in.position, sample_index));
    if occluder_z > line_z + params.x {
        discard;
    }
#endif
    // Raw white written to the encoded buffer: the final decode saturates.
    return vec4<f32>(1.0, 1.0, 1.0, 1.0);
}
