// FogEffect.Linear (research/blizzard.md). Blend in the original encoded
// color space; the sRGB target itself receives linearized values.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
struct Blizzard {
    offsets: vec4<f32>,
    control: vec4<f32>,
    view_from_clip: mat4x4<f32>,
}
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var noise: texture_2d<f32>;
@group(0) @binding(2) var depth: texture_2d<f32>;
@group(0) @binding(3) var wrapped_linear: sampler;
@group(0) @binding(4) var<uniform> blizzard: Blizzard;
fn raw_rgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055, 12.92 * c, c <= vec3<f32>(0.0031308));
}
fn linear_rgb(c: vec3<f32>) -> vec3<f32> {
    return select(pow(max((c + 0.055) / 1.055, vec3<f32>(0.0)), vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}
@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.position.xy);
    let samples = textureSample(noise, wrapped_linear, in.uv + blizzard.offsets.xy)
        * textureSample(noise, wrapped_linear, in.uv + blizzard.offsets.zw);
    let ndc = textureLoad(depth, p, 0).x;
    let view = blizzard.view_from_clip * vec4<f32>(0.0, 0.0, ndc, 1.0);
    // Native ClearEffect writes 1e6 into the positive scene-depth target.
    let distance = select(-view.z / view.w, 1e6, ndc <= 0.0);
    let alpha = clamp(samples.a * (distance - blizzard.control.y)
        / (blizzard.control.z - blizzard.control.y), 0.0, 1.0) * blizzard.control.x;
    let fog = 1.0 - exp2(-samples.rgb * vec3<f32>(1.4, 1.8, 2.0));
    let base = raw_rgb(textureLoad(scene, p, 0).rgb);
    return vec4<f32>(linear_rgb(mix(base, fog, alpha)), 1.0);
}
