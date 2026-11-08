#import bevy_sprite::mesh2d_vertex_output::VertexOutput
struct CinemaUniforms {
    image_uv: vec4<f32>,
    image_bounds: vec4<f32>,
    mask_uv: vec4<f32>,
    lighting: vec4<f32>,
};
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> cinema: CinemaUniforms;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var page: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var page_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var picture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var picture_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var mask: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var mask_sampler: sampler;
@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    // Right-page quadrant of tome_pages_0. The native mask/image multipass
    // writes maskAlpha*pictureAlpha then uses destination-alpha RGB blending.
    let paper = textureSample(page, page_sampler, mesh.uv*0.5+vec2<f32>(0.5,0.0));
    let local = (mesh.uv-cinema.image_bounds.xy)/cinema.image_bounds.zw;
    let inside = all(local >= vec2<f32>(0.0)) && all(local <= vec2<f32>(1.0));
    let color = textureSample(picture, picture_sampler, local*cinema.image_uv.xy+cinema.image_uv.zw);
    let coverage = textureSample(mask, mask_sampler, local*cinema.mask_uv.xy).a*color.a;
    let alpha = select(0.0, coverage*cinema.lighting.y, inside);
    if cinema.lighting.z > 0.5 {
        // On the Tome: the sheet already holds the paper and the 3D light
        // replaces the flat page's dimming.
        return vec4<f32>(color.rgb, alpha);
    }
    return vec4<f32>(mix(paper.rgb,color.rgb,alpha)*cinema.lighting.x, paper.a);
}
