// VortexEffect (output/evidence/vortex/shader/VortexEffect.asm.txt, the
// retail ps_3_0/vs_3_0 pair). The vertex shader passes the view-space
// direction of the unit-sphere vertex (World·View 3x3). The pixel shader
// normalises it, takes its screen-plane part n.xy (length l, direction
// d = n.xy / l) and samples the scene at
//   uv + halfPixel + Distortion·(1 − l^DistortionPower)·(d.x, d.y·c1)
// with c1 = −PixelSize.y / PixelSize.x, then writes it opaque.
#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings as view_bindings,
}

// x: Distortion (0.025), y: DistortionPower (2).
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: vec4<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(view_bindings::view_transmission_texture));
    let pixel = 1.0 / size;
    let uv = in.position.xy / size;
    // The sphere vertex direction is its normal; World·View 3x3 of it.
    let n = normalize((view_bindings::view.view_from_world * vec4<f32>(in.world_normal, 0.0)).xyz);
    let l = length(n.xy);
    var d = vec2<f32>(0.0);
    if l > 1e-6 {
        d = n.xy / l;
    }
    let w = (1.0 - pow(l, params.y)) * params.x;
    let c1 = -pixel.y / pixel.x;
    let offset = vec2<f32>(d.x, d.y * c1) * w + pixel * 0.5;
    let c = textureSampleLevel(
        view_bindings::view_transmission_texture,
        view_bindings::view_transmission_sampler,
        uv + offset,
        0.0,
    ).rgb;
    return vec4<f32>(c, 1.0);
}
