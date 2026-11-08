// SkinnedModelBasicEffect, NonDeffered technique (the menu Tome): one point
// light with ambient, diffuse and specular colours, inverse-square falloff,
// and the scene buffer's 1 - 2^-x encoding (post.rs decodes it).
// Disassembly: output/evidence/frontend-handoff/tome-shader.asm.txt, shader 1.
#import bevy_pbr::pbr_fragment::pbr_input_from_standard_material
#import bevy_pbr::{forward_io::{VertexOutput, FragmentOutput}, pbr_functions::alpha_discard}
#import magicka_viewer::lighting::{magicka_raw, magicka_linear}
#import bevy_pbr::{
    mesh_view_bindings as view_bindings,
    mesh_view_types,
    clustered_forward as clustering,
    shadows,
}

// RenderData.DrawShadows' map: the shadow-casting spot light in this
// fragment's cluster (the Tome's only one), sampled with Bevy's filtering in
// place of the original four-tap compare.
fn tome_shadow(in: VertexOutput) -> f32 {
    let view_z = dot(vec4<f32>(
        view_bindings::view.view_from_world[0].z,
        view_bindings::view.view_from_world[1].z,
        view_bindings::view.view_from_world[2].z,
        view_bindings::view.view_from_world[3].z
    ), in.world_position);
    let orthographic = view_bindings::view.clip_from_view[3].w == 1.0;
    let cluster = clustering::view_fragment_cluster_index(in.position.xy, view_z, orthographic);
    let ranges = clustering::unpack_clusterable_object_index_ranges(cluster);
    for (var i: u32 = ranges.first_spot_light_index_offset;
            i < ranges.first_reflection_probe_index_offset;
            i = i + 1u) {
        let id = clustering::get_clusterable_object_id(i);
        let light = view_bindings::clustered_lights.data[id];
        if (light.flags & mesh_view_types::POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
            return shadows::fetch_spot_shadow(
                id,
                in.world_position,
                normalize(in.world_normal),
                light.shadow_map_near_z,
                in.position.xy,
            );
        }
    }
    return 1.0;
}

struct TomeParams {
    // xyz LightPosition
    light_position: vec4<f32>,
    // rgb LightAmbientColor (c3)
    ambient: vec4<f32>,
    // rgb LightDiffuseColor (c5)
    diffuse: vec4<f32>,
    // rgb LightSpecularColor (c6)
    specular: vec4<f32>,
    // rgb DiffuseColor (c7), a diffuseIsHDR
    material: vec4<f32>,
    // SpecularAmount (c8), SpecularPower (c9), specularMapEnabled, unused
    spec: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> tome: TomeParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var specular_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var specular_sampler: sampler;

// D3D9 pow (see lighting.wgsl): |b|^e, 0^0 = 1.
fn d3d_pow(b: f32, e: f32) -> f32 {
    if e == 0.0 {
        return 1.0;
    }
    let a = abs(b);
    if a == 0.0 {
        return select(0.0, 1.0e30, e < 0.0);
    }
    return exp2(e * log2(a));
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var input = pbr_input_from_standard_material(in, is_front);
    let base = alpha_discard(input.material, input.material.base_color);
    var albedo = magicka_raw(base.rgb);
    if tome.material.a > 0.5 {
        // The page target is drawn with alpha blending over transparent
        // black, so its colour is premultiplied by the paper's alpha.
        albedo = magicka_raw(base.rgb / max(base.a, 1.0 / 255.0));
        albedo = -log2(1.0 - clamp(albedo, vec3<f32>(0.0), vec3<f32>(0.998047)));
    }
    var specular_mask = vec3<f32>(1.0);
#ifdef VERTEX_UVS_A
    if tome.spec.z > 0.5 {
        specular_mask = magicka_raw(textureSample(specular_map, specular_sampler, in.uv).rgb);
    }
#endif
    let shadow = tome_shadow(in);
    let to_light = tome.light_position.xyz - in.world_position.xyz;
    let distance_squared = dot(to_light, to_light);
    let l = to_light * inverseSqrt(distance_squared);
    let n = normalize(input.N);
    // The shader reflects L about N and then dots the reflection with N.
    let r = n * (2.0 * dot(l, n)) - l;
    let n_dot_l = saturate(dot(l, n));
    let spec = shadow * tome.spec.x * d3d_pow(dot(r, n), tome.spec.y) * d3d_pow(n_dot_l, 0.01);
    let diffuse = shadow * tome.material.rgb * n_dot_l * tome.diffuse.rgb + tome.ambient.rgb;
    let lit = (albedo * tome.material.rgb * diffuse + spec * tome.specular.rgb * specular_mask)
        / distance_squared;
    var out: FragmentOutput;
    out.color = vec4<f32>(magicka_linear(1.0 - exp2(-lit)), base.a);
    return out;
}
