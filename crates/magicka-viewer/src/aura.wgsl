// HaloAuraRenderData depth bias, converted to reverse-Z (outline.rs).
#import bevy_pbr::{mesh_functions, forward_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip}
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var<uniform> depth: vec4<f32>;
@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world = mesh_functions::get_world_from_local(v.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world, vec4(v.position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    out.position.z *= 1.0 - depth.x * out.position.w;
#ifdef VERTEX_NORMALS
    out.world_normal = mesh_functions::mesh_normal_local_to_world(v.normal, v.instance_index);
#endif
#ifdef VERTEX_UVS_A
    out.uv = v.uv;
#endif
#ifdef VERTEX_COLORS
    out.color = v.color;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = v.instance_index;
#endif
    return out;
}
