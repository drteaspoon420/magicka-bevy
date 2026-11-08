#import bevy_sprite::mesh2d_vertex_output::VertexOutput
struct GuiUniforms {
    color: vec4<f32>, overlay_tint: vec4<f32>, uv_transform: vec4<f32>,
    settings: vec4<f32>, flags: vec4<u32>,
};
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> gui: GuiUniforms;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var primary: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var primary_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var overlay: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var overlay_sampler: sampler;
// sRGB transfer, for flags.w == 2: the native GUI computes in gamma space
// and its encoded output must land in an sRGB target as the native bytes.
fn to_raw(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055, 12.92 * c, c <= vec3<f32>(0.0031308));
}
fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow(max((c + 0.055) / 1.055, vec3<f32>(0.0)), vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}
@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let raw = gui.flags.w == 2u;
    var color = gui.color;
    if gui.flags.y != 0u { color *= mesh.color; }
    let uv = mesh.uv * gui.uv_transform.xy + gui.uv_transform.zw;
    var rgb = color.rgb;
    var coverage = 1.0;
    if gui.flags.x != 0u {
        var value = textureSample(primary, primary_sampler, uv);
        // settings.z: a render target drawn with alpha blending over
        // transparent black holds premultiplied colour; restore it so the
        // coverage blend below does not darken the edges a second time.
        if gui.settings.z > 0.5 && value.a > 0.0 { value = vec4<f32>(value.rgb / value.a, value.a); }
        rgb *= select(value.rgb, to_raw(value.rgb), raw);
        coverage = value.a;
    }
    if gui.flags.z == 2u {
        // Mask mode: the overlay's alpha over the quad times the image's
        // coverage inside its own extent (DestinationAlpha compositing).
        let mask = textureSample(overlay, overlay_sampler, mesh.uv).a;
        let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
        coverage = select(0.0, coverage * mask, inside);
    } else if gui.flags.z != 0u {
        // settings.y: overlay V offset (avatar thumbnails keep their colour
        // mask in the lower half of the same texture).
        var value = textureSample(overlay, overlay_sampler, uv + vec2<f32>(0.0, gui.settings.y));
        if raw { value = vec4<f32>(to_raw(value.rgb), value.a); }
        let tint = gui.overlay_tint;
        let low = value.rgb * value.rgb * (vec3<f32>(1.0) - 2.0*tint.rgb) + 2.0*value.rgb*tint.rgb;
        let high = sqrt(value.rgb)*(2.0*tint.rgb-vec3<f32>(1.0)) + 2.0*value.rgb*(vec3<f32>(1.0)-tint.rgb);
        let soft = select(low, high, tint.rgb >= vec3<f32>(0.5));
        let alpha = value.a*tint.a;
        rgb = rgb*coverage + soft*alpha*(1.0-coverage);
        coverage = coverage*(1.0-alpha)+alpha;
    }
    let luminance = dot(rgb, vec3<f32>(0.3086,0.6094,0.082));
    rgb = rgb*gui.settings.x + vec3<f32>(luminance)*(1.0-gui.settings.x);
    if raw { rgb = to_linear(vec3<f32>(1.0) - exp2(-max(rgb, vec3<f32>(0.0)))); }
    else if gui.flags.w != 0u { rgb = vec3<f32>(1.0) - exp2(-rgb); }
    return vec4<f32>(rgb, coverage*color.a);
}
