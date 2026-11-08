// PolygonHead deferred lighting, evaluated per fragment (see level-lighting.md).
//
// Original pipeline: each light writes 1 - 2^-x into a light buffer cleared
// to zero, blended with One/InvSrcColor (screen), so the buffer holds
// 1 - 2^-(sum x). DeferredShadingEffect then recovers the sum with log2 and
// outputs 1 - 2^-(albedo * sum ...). Evaluating the sums directly here is
// algebraically identical (8-bit buffer quantization aside).
#define_import_path magicka_viewer::lighting

#import bevy_pbr::{shadows::{fetch_directional_shadow, fetch_spot_shadow}, mesh_view_bindings::view}

struct MagickaLight {
    // xyz position, w radius/range
    position: vec4<f32>,
    // xyz direction, w host kind (0 point, 1 directional, 2 spot, 3 capsule)
    direction: vec4<f32>,
    // rgb diffuse * intensity, a specular amount * intensity
    diffuse: vec4<f32>,
    // rgb ambient * intensity, a 1 if spot attenuation is used
    ambient: vec4<f32>,
    // x cutoff angle, y sharpness, z 1 if this directional light uses the
    // view's first shadow map, w unused; capsule: xyz endpoint
    cone: vec4<f32>,
}

struct MagickaLights {
    // x light count, yzw unused
    header: vec4<f32>,
    // x: 1 if Bevy directional light 0 carries the sun's shadow map
    flags: vec4<f32>,
    fog_color: vec4<f32>,
    fog_range: vec4<f32>,
    lights: array<MagickaLight>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(90) var<storage, read> magicka_lights: MagickaLights;

// Gamma-encoded byte value <-> Bevy's linear sample/output space. The
// original shaders operate on raw texture bytes (no sRGB decode).
fn magicka_raw(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055, 12.92 * c, c <= vec3<f32>(0.0031308));
}
fn magicka_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow(max((c + 0.055) / 1.055, vec3<f32>(0.0)), vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

struct MagickaSurface {
    // raw (gamma-space) albedo, as written to the colour G-buffer
    albedo: vec3<f32>,
    // colour G-buffer alpha
    spec_amount: f32,
    // specular exponent (normal G-buffer alpha * 255)
    spec_power: f32,
    // EmissiveAmount; the G-buffer normal is scaled by 2^-emissive
    emissive: f32,
    normal: vec3<f32>,
    position: vec3<f32>,
    // visibility from the shadow-casting directional light (1 = lit)
    shadow: f32,
}

// DirectionalLightEffect WithShadows samples its own shadow map; Bevy's
// cascaded map for the matching directional light stands in for it.
fn magicka_sun_shadow(world_position: vec4<f32>, normal: vec3<f32>, frag_coord: vec2<f32>) -> f32 {
    if magicka_lights.flags.x < 0.5 {
        return 1.0;
    }
    let view_z = dot(vec4<f32>(view.view_from_world[0].z, view.view_from_world[1].z,
        view.view_from_world[2].z, view.view_from_world[3].z), world_position);
    return fetch_directional_shadow(0u, world_position, normalize(normal), view_z, frag_coord);
}

// MagickCamera.EyeOfTheBeholder: camera Position + Direction * 200.
fn magicka_eye() -> vec3<f32> {
    let forward = -normalize(view.world_from_view[2].xyz);
    return view.world_position.xyz + forward * 200.0;
}

// D3D9 pow: |base|^e evaluated as exp2(e * log2|base|) with 0 * inf = 0,
// so x^0 = 1 (including 0^0) and 0^e = 0 for e > 0. WGSL leaves pow(0, 0)
// undefined (NaN on common drivers), which would poison the light sum.
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

// Sum of light contributions: rgb diffuse+ambient, a specular.
fn magicka_light_sum(s: MagickaSurface) -> vec4<f32> {
    let n = normalize(s.normal);
    let v = normalize(magicka_eye() - s.position);
    var sum = vec4<f32>(0.0);
    let count = u32(magicka_lights.header.x);
    for (var i = 0u; i < count; i++) {
        let l = magicka_lights.lights[i];
        let kind = u32(l.direction.w);
        if kind == 1u {
            // DirectionalLightEffect: sqrt(sat(N.-L)) diffuse, unshadowed ambient.
            let to_light = -l.direction.xyz;
            let h = normalize(v + to_light);
            let spec = saturate(d3d_pow(dot(n, h), s.spec_power)) * l.diffuse.a;
            let diffuse = sqrt(saturate(dot(n, to_light)));
            let shadow = select(1.0, s.shadow, l.cone.z > 0.5);
            sum += vec4<f32>(diffuse * l.diffuse.rgb * shadow + l.ambient.rgb, spec * shadow);
            continue;
        }
        var source = l.position.xyz;
        if kind == 3u {
            // CapsuleLight ps_3_0 and preshader: clamp the projection onto
            // Start..End, then linear radial attenuation (not point-light
            // squared falloff). No specular contribution or shadows.
            let segment = l.cone.xyz - source;
            let length_squared = dot(segment, segment);
            // The native shader divides by zero for a collapsed segment.
            // ArcaneBlade is already dark when its current pair collapses.
            if length_squared <= 0.0 { continue; }
            let t = saturate(dot(s.position - source, segment) / length_squared);
            source += t * segment;
        }
        let offset = source - s.position;
        let d = length(offset);
        if d >= l.position.w || d <= 0.0 {
            continue;
        }
        let to_light = offset / d;
        let fall = saturate(1.0 - d / l.position.w);
        if kind == 3u {
            sum += vec4<f32>(fall * (saturate(dot(n, to_light)) * l.diffuse.rgb
                + l.ambient.rgb), 0.0);
            continue;
        }
        var att = fall * fall;
        if kind == 2u {
            // SpotLightEffect: the shader's polynomial acos of the cone angle.
            let c = dot(-to_light, l.direction.xyz);
            let a = abs(c);
            let r = (((-0.0187293 * a + 0.074261) * a - 0.212114) * a + 1.57073)
                * sqrt(max(1.0 - a, 0.0));
            // cmp/mad: (r * -2 + 3.14159) * (c < 0) + r
            let angle = select(r, (r * -2.0 + 3.14159) + r, c < 0.0);
            let cone = 1.0 - d3d_pow(angle / l.cone.x, l.cone.y);
            // SpotLight.Technique: the non-attenuation variants omit the
            // distance falloff.
            if l.ambient.a < 0.5 {
                att = 1.0;
            }
            // Each light renders to a UNORM target, so a negative result
            // outside the cone clamps to zero.
            att *= max(cone, 0.0);
            // ShadowMapSize/CastShadows spot lights: the PlaySpot map.
            if l.cone.z > 1.5 {
                att *= fetch_spot_shadow(0u, vec4<f32>(s.position, 1.0), n, 0.1, vec2<f32>(0.0));
            }
        }
        let h = normalize(v + to_light);
        let spec = d3d_pow(saturate(dot(n, h)), s.spec_power) * l.diffuse.a;
        let diffuse = saturate(dot(n, to_light));
        sum += att * vec4<f32>(diffuse * l.diffuse.rgb + l.ambient.rgb, spec);
    }
    return sum;
}

// DeferredShadingEffect composite (fog disabled). Returns the raw output.
fn magicka_shade(s: MagickaSurface) -> vec3<f32> {
    // The composite clamps the light buffer to 0.998047 before log2.
    let light = clamp(magicka_light_sum(s), vec4<f32>(0.0), vec4<f32>(-log2(1.0 - 0.998047)));
    let w = vec3<f32>(0.3086, 0.6094, 0.082);
    let lit = s.albedo * light.rgb;
    let lum = dot(lit, w);
    let saturation = saturate(2.0 - 0.75 * lum);
    let graded = mix(vec3<f32>(lum), lit, saturation);
    let normal_length = exp2(-max(s.emissive, 0.0));
    let e = clamp(1.0 - normal_length, 0.0, 0.998047);
    let emissive = -log2(1.0 - e);
    var r = graded * saturate(1.0 - emissive) + s.albedo * emissive
        + vec3<f32>(light.a * s.spec_amount);
    // DeferredShadingEffect blends FogColor into decoded intensity before
    // the final exponential encoding (shader 1, IL-equivalent instructions).
    if magicka_lights.fog_range.z > 0.5 {
        let depth = -(view.view_from_world * vec4<f32>(s.position, 1.0)).z;
        let alpha = saturate((depth - magicka_lights.fog_range.x)
            / (magicka_lights.fog_range.y - magicka_lights.fog_range.x)) * magicka_lights.fog_color.a;
        r = mix(r, magicka_lights.fog_color.rgb, alpha);
    }
    return 1.0 - exp2(-r);
}
