// PolygonHead RenderManager post-processing (post.rs, research/post-processing.md).
//
// Every scene, effect and GUI shader writes the screen-blend encoding
// c = 1 - 2^-x. FilterEffect works on x and re-encodes; BloomEffect's Combine
// decodes the GUI buffer and adds the blurred bloom, writing x itself (the
// back buffer clamps it to [0, 1]).
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Post {
    // Brightness, Contrast, Saturation, Flash (additive white, encoded space)
    filter_params: vec4<f32>,
    // Threshold, Multiplier, Sigma (BlurSigma), unused
    bloom: vec4<f32>,
}
@group(0) @binding(0) var t0: texture_2d<f32>;
@group(0) @binding(1) var t1: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;
@group(0) @binding(3) var<uniform> post: Post;

// The main texture is sRGB: shaders store raw (gamma-space) values.
fn raw_rgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055, 12.92 * c, c <= vec3<f32>(0.0031308));
}
fn linear_rgb(c: vec3<f32>) -> vec3<f32> {
    return select(pow(max((c + 0.055) / 1.055, vec3<f32>(0.0)), vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}
// -log2(1 - clamp(c, 0, 0.998047)), as in every PolygonHead decode.
fn decode(c: vec3<f32>) -> vec3<f32> {
    return -log2(1.0 - clamp(c, vec3<f32>(0.0), vec3<f32>(0.998047)));
}
fn encode(x: vec3<f32>) -> vec3<f32> {
    return 1.0 - exp2(-x);
}
fn main_raw(p: vec2<i32>) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(t0));
    return raw_rgb(textureLoad(t0, clamp(p, vec2<i32>(0), size - 1), 0).rgb);
}

// FilterEffect: x·Brightness, contrast about 0.5, then the saturation
// matrix S·I + (1 − S)·luminance.
@fragment
fn filter_pass(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.position.xy);
    let b = post.filter_params.x;
    let c = post.filter_params.y;
    let s = post.filter_params.z;
    // Flash draws as an additive object into the encoded scene buffer
    // before FilterEffect (Flash.Draw: Color = intensity, alpha 1).
    let scene = min(main_raw(p) + vec3<f32>(post.filter_params.w), vec3<f32>(1.0));
    let y = decode(scene) * b * c + 0.5 - 0.5 * c;
    let lum = dot(y, vec3<f32>(0.3086, 0.6094, 0.082));
    let out = s * y + (1.0 - s) * vec3<f32>(lum);
    return vec4<f32>(linear_rgb(encode(out)), 1.0);
}

// BloomEffect DownSample: the mean of max(x − Threshold, 0) over the
// 16×16 source block of each sixteenth-size texel.
@fragment
fn downsample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let origin = vec2<i32>(in.position.xy) * 16;
    var sum = vec3<f32>(0.0);
    for (var j = 0; j < 16; j++) {
        for (var i = 0; i < 16; i++) {
            let x = decode(main_raw(origin + vec2<i32>(i, j)));
            sum += max(x - post.bloom.x, vec3<f32>(0.0));
        }
    }
    return vec4<f32>(encode(sum / 256.0), 1.0);
}

// 13 taps at −6..6 texels with weight exp(−t²/(2σ²)) / (σ·√(2π)).
fn blur(p: vec2<i32>, axis: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(t1));
    let sigma = post.bloom.z;
    var sum = vec3<f32>(0.0);
    for (var t = -6; t <= 6; t++) {
        let w = exp(-f32(t * t) / (2.0 * sigma * sigma)) / (sigma * 2.50663);
        let q = clamp(p + axis * t, vec2<i32>(0), size - 1);
        sum += decode(textureLoad(t1, q, 0).rgb) * w;
    }
    return vec4<f32>(encode(sum), 1.0);
}
@fragment
fn blur_h(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return blur(vec2<i32>(in.position.xy), vec2<i32>(1, 0));
}
@fragment
fn blur_v(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return blur(vec2<i32>(in.position.xy), vec2<i32>(0, 1));
}

// BloomEffect Combine: x(GUI buffer) + Multiplier · x(bilinear bloom).
@fragment
fn combine(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let base = decode(main_raw(vec2<i32>(in.position.xy)));
    let bloom = decode(textureSample(t1, linear_sampler, in.uv).rgb);
    let out = clamp(base + post.bloom.y * bloom, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(linear_rgb(out), 1.0);
}
