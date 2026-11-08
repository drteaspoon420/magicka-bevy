// GUIBasicEffect texture/vertex-color path with ScaleToHDR enabled.
// The play GUI is decoded by BloomEffect after alpha compositing. Store
// encoded RGB through the sRGB target; leave coverage/fade alpha untouched.
#import bevy_sprite::mesh2d_vertex_output::VertexOutput
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> color: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var atlas: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var atlas_sampler: sampler;

fn linear_rgb(c: vec3<f32>) -> vec3<f32> {
    return select(pow(max((c + 0.055) / 1.055, vec3<f32>(0.0)), vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}
@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let value = textureSample(atlas, atlas_sampler, mesh.uv) * mesh.color * color;
    return vec4<f32>(linear_rgb(vec3<f32>(1.0) - exp2(-value.rgb)), value.a);
}
