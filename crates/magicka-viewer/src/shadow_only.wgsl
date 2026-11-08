#import bevy_pbr::forward_io::VertexOutput
// Shadow-map-only level meshes: never visible in the main pass.
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    discard;
}
