#import bevy_sprite::mesh2d_vertex_output::VertexOutput
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var ruler: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var ruler_sampler: sampler;
// BasicEffect (lighting off): texture · DiffuseColor, alpha · Alpha (both
// carried by the vertex colour); no GUI HDR mapping.
@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let value = textureSample(ruler, ruler_sampler, mesh.uv) * mesh.color;
    return vec4<f32>(to_lin(value.rgb), value.a);
}
fn to_lin(c: vec3<f32>) -> vec3<f32> {
    return select(pow(max((c + 0.055) / 1.055, vec3<f32>(0.0)), vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}
