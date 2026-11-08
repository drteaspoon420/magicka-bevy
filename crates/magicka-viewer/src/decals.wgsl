// PolygonHead projection ps_3_0 shader 3. Native stencil receiver masking
// is not available here; Bevy's opaque depth prepass supplies receivers.
#import bevy_pbr::mesh_view_bindings::view
#import magicka_viewer::lighting::{MagickaSurface, magicka_shade, magicka_sun_shadow}
#import bevy_pbr::view_transformations::{frag_coord_to_ndc, position_ndc_to_world}
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils::prepass_depth
#endif
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var atlas: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var atlas_sampler: sampler;
struct Vertex {
    @location(0) position: vec3<f32>,
    @location(1) ix: vec4<f32>,
    @location(2) iy: vec4<f32>,
    @location(3) iz: vec4<f32>,
    @location(4) args: vec3<f32>,
    @location(5) color: vec4<f32>,
}
struct Varyings {
    @builtin(position) clip: vec4<f32>,
    @location(0) ix: vec4<f32>,
    @location(1) iy: vec4<f32>,
    @location(2) iz: vec4<f32>,
    @location(3) args: vec3<f32>,
    @location(4) color: vec4<f32>,
}
@vertex fn vertex(v: Vertex) -> Varyings {
    var out: Varyings;
    out.clip=view.clip_from_world*vec4(v.position,1.0);
    out.ix=v.ix;out.iy=v.iy;out.iz=v.iz;out.args=v.args;out.color=saturate(v.color);
    return out;
}
fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((max(c,vec3(0.0))+0.055)/1.055,vec3(2.4)),c/12.92,c<=vec3(0.04045));
}
struct Fragment {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}
@fragment fn fragment(v: Varyings, @builtin(sample_index) sample: u32) -> Fragment {
    var depth=0.0;
#ifdef DEPTH_PREPASS
    depth=prepass_depth(v.clip,sample);
#endif
    if depth==0.0 { discard; }
    let ndc=frag_coord_to_ndc(vec4(v.clip.xy,depth,1.0));
    let world=vec4(position_ndc_to_world(ndc),1.0);
    let local=vec3(dot(world,v.ix),dot(world,v.iy),dot(world,v.iz));
    let uv=vec2(local.x,-local.y)+0.5;
    if any(uv<vec2(0.0)) || any(uv>=vec2(1.0)) { discard; }
    let texel=textureSample(atlas,atlas_sampler,(uv+v.args.xy)*0.125);
    let alpha=texel.a*saturate(1.0-4.0*local.z*local.z)*saturate(v.args.z)*v.color.a;
    if alpha<0.0000001 { discard; }
    var out: Fragment;
    // Native projections enter the albedo buffer before scene lighting.
    // Forward host approximation: light atlas albedo with the projection
    // receiver normal, no emissive term, and the same scene/shadow model.
    let normal=normalize(v.iz.xyz);
    let shadow=magicka_sun_shadow(world,normal,v.clip.xy);
    let lit=magicka_shade(MagickaSurface(texel.rgb*v.color.rgb,0.0,1.0,0.0,normal,world.xyz,shadow));
    out.color=vec4(to_linear(lit),alpha);
    // Test the receiver against main-pass depth (including characters),
    // rather than the projection cube exit face. Bevy uses reverse Z.
    out.depth=depth;
    return out;
}
