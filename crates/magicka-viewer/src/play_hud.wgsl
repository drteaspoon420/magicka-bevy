#import bevy_sprite::mesh2d_vertex_output::VertexOutput
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var atlas: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var atlas_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> saturation: vec4<f32>;
// GUIHardwareInstancing Healthbars PS; settled Sprites use saturation = 0.
@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let value = textureSample(atlas, atlas_sampler, mesh.uv) * mesh.color;
    let luminance = dot(value.rgb, vec3<f32>(0.3086, 0.6094, 0.082));
    let rgb = value.rgb*saturation.x + vec3<f32>(luminance)*(1.0-saturation.x);
    // Raw atlas bytes, encoded as the GUI buffer stores them; the sRGB
    // target needs the linear value (post.rs decodes after the GUI).
    return vec4<f32>(to_lin(vec3<f32>(1.0)-exp2(-rgb)), value.a);
}
fn to_lin(c: vec3<f32>) -> vec3<f32> {
    return select(pow(max((c + 0.055) / 1.055, vec3<f32>(0.0)), vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}
