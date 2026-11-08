// AfterImageRenderData: SkinnedModelDeferredEffect additive technique with
// Colorize = (ColdColor, 1): diffuse luminance tinted by ColdColor, blended
// (SourceAlpha, One). The rewrite draws it unlit (INFERRED).
#import bevy_pbr::forward_io::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var diffuse: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var diffuse_sampler: sampler;

const LUMINANCE: vec3<f32> = vec3<f32>(0.3086, 0.6094, 0.082);
const COLD_COLOR: vec3<f32> = vec3<f32>(1.0, 1.6, 2.0);

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var c = vec3<f32>(1.0);
#ifdef VERTEX_UVS_A
    c = textureSample(diffuse, diffuse_sampler, in.uv).rgb;
#endif
    let lum = dot(c, LUMINANCE);
    // Premultiplied with zero alpha: AlphaMode::Add.
    return vec4<f32>(lum * COLD_COLOR * params.x, 0.0);
}
