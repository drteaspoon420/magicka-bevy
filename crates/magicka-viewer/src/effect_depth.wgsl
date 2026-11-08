// Snapshot opaque scene depth, including meshes excluded from the outline
// prepass. Keep sample 0, matching ParticleEffect's existing depth fetch.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

#ifdef MULTISAMPLED
@group(0) @binding(0) var scene_depth: texture_depth_multisampled_2d;
#else
@group(0) @binding(0) var scene_depth: texture_depth_2d;
#endif

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) f32 {
    return textureLoad(scene_depth, vec2<i32>(in.position.xy), 0);
}
