// RenderDeferredEffectWOVT AdditiveFresnel PS at 0x1ed0:
// white * pow(1-dot(normalize(camera-position),normalize(normal)),power),
// encoded as 1-exp2(-rgb), screen blended by Scene.DrawAdditive.
#import bevy_pbr::{forward_io::VertexOutput, mesh_view_bindings::view}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> power: f32;
fn linear(c: f32) -> f32 {
    return select(pow((c+0.055)/1.055, 2.4), c/12.92, c <= 0.04045);
}
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let direction = normalize(view.world_position.xyz-in.world_position.xyz);
    let fresnel = pow(max(1.0-dot(direction, normalize(in.world_normal)), 0.0), power);
    let value = linear(1.0-exp2(-fresnel));
    return vec4<f32>(value, value, value, 1.0);
}
