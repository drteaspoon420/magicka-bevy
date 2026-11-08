// NormalDistortionEffect (output/evidence/wave/shader, retail vs_2_0/ps_2_0;
// research/wave.md). The vertex shader passes TexCoord / TextureScale and
// vertex alpha · Distortion. The pixel shader averages two NormalSampler
// reads at (u ∓ Time, v), each (2n − 1), and samples the scene at
//   uv + halfPixel + n · 0.02 · Distortion · saturate((sceneDepth − depth)/2)
//        · (1, PixelSize.y / PixelSize.x) · alpha · Distortion,
// then writes it opaque.
#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings as view_bindings,
    prepass_utils,
    view_transformations::depth_ndc_to_view_z,
}

// x: Distortion, y: Time, z: 1 / TextureScale.
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var normal_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var normal_sampler: sampler;

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
    let pixel = 1.0 / size;
    let uv = in.position.xy / size;
    let own_depth = -depth_ndc_to_view_z(in.position.z);
    // Without a depth prepass nothing limits the offset.
    var scene_depth = own_depth + 2.0;
#ifdef DEPTH_PREPASS
    scene_depth = -depth_ndc_to_view_z(prepass_utils::prepass_depth(in.position, sample_index));
#endif
    var t = vec2<f32>(0.0);
#ifdef VERTEX_UVS_A
    t = in.uv * params.z;
#endif
    var alpha = 0.0;
#ifdef VERTEX_COLORS
    alpha = in.color.a;
#endif
    let a = textureSample(normal_texture, normal_sampler, vec2<f32>(t.x - params.y, t.y)).xy * 2.0 - 1.0;
    let b = textureSample(normal_texture, normal_sampler, vec2<f32>(t.x + params.y, t.y)).xy * 2.0 - 1.0;
    let n = (a + b) * 0.5;
    let w = clamp((scene_depth - own_depth) * 0.5, 0.0, 1.0) * params.x * 0.02;
    let offset = n * vec2<f32>(w, w * pixel.y / pixel.x) * (alpha * params.x) + pixel * 0.5;
    let c = textureSampleLevel(
        view_bindings::view_transmission_texture,
        view_bindings::view_transmission_sampler,
        uv + offset,
        0.0,
    ).rgb;
    return vec4<f32>(c, 1.0);
}
