//! Build a glTF scene for a level, following how the game places things
//! (see `output/research/level-scene.md`):
//! - static geometry: `BiTreeModel` roots with `visible == true`, world space;
//!   invisible roots that cast shadows become shadow-only primitives;
//! - animated parts: world = pose(t) * parent (row-vector order, as
//!   `AnimatedLevelPart.Update`); model meshes drawn with the part transform
//!   only (model bone transforms are ignored by the game); meshes whose
//!   `meshSettings.visible` is false are skipped unless they cast shadows
//!   (then shadow-only, as `DeferredRenderData.DrawShadow`);
//! - liquids: drawn with the owning part's world transform (identity at top);
//! - physics props: template model mesh 0 / part 0, placed at `StartTransform`.

use crate::gltf::{self, Builder, Primitive};
use crate::level::{AnimatedLevelPart, LevelModel, Liquid, LiquidMaterial};
use crate::mesh;
use crate::xna::{Model, VertexElement, XnaObject};
use std::collections::HashMap;
use std::fmt::Write as _;

/// Size in bytes of an XNA 3.1 `VertexElementFormat`.
pub fn format_size(f: u8) -> usize {
    match f {
        0 | 4 | 5 | 6 | 8 | 9 | 11 | 13 | 14 | 15 => 4,
        1 | 7 | 10 | 12 | 16 => 8,
        2 => 12,
        3 => 16,
        _ => 0,
    }
}

/// `VertexDeclaration.GetVertexStrideSize(0)`: end of the last stream-0 element.
pub fn decl_stride(decl: &[VertexElement]) -> usize {
    decl.iter()
        .filter(|e| e.stream == 0)
        .map(|e| e.offset as usize + format_size(e.format))
        .max()
        .unwrap_or(0)
}

/// Statistics used by the automated checks.
#[derive(Default, Debug, Clone)]
pub struct Stats {
    pub primitives: usize,
    pub triangles: usize,
    /// Faces whose (flipped, CCW) geometric normal agrees / disagrees with
    /// the mean of their stored vertex normals.
    pub normal_agree: usize,
    pub normal_disagree: usize,
    pub animated_parts: usize,
    pub animations: usize,
    pub anisotropic_scales: usize,
    /// Source keyframes with an all-zero quaternion (XNA's unnormalized
    /// `CreateFromQuaternion` yields identity for these; exported as identity).
    pub zero_quaternions: usize,
    /// Source quaternions whose length differs from 1 by > 1e-3 (normalized
    /// on export; the game would build a scaled/sheared matrix).
    pub non_unit_quaternions: usize,
    /// Non-finite (NaN/inf) pose components replaced by 0 (the game renders
    /// such a part degenerate/invisible; scale 0 collapses it likewise).
    pub non_finite_components: usize,
    pub dropped_keyframes: usize,
    pub props: usize,
    pub missing_props: Vec<String>,
    pub liquids: usize,
    pub textured_materials: usize,
    pub missing_textures: Vec<String>,
    /// Skinned vertices whose weights did not sum to 1 (renormalized) or
    /// summed to 0 (bound fully to the first listed joint).
    pub renormalized_weights: usize,
    pub zero_weights: usize,
}

/// A physics-prop template as needed for export: template asset name, its
/// XNA model, and the template file's shared resources (mesh effects).
pub type PropData = (String, Model, Vec<Option<XnaObject>>);

/// Inputs the builder needs from the environment.
pub struct Context<'a> {
    /// Asset name of the level (e.g. `Levels/BattleField/bf_s1`).
    pub asset: &'a str,
    /// Resolved texture asset name -> URI relative to the .glb, if exported.
    pub texture_uri: &'a dyn Fn(&str) -> Option<String>,
    /// Physics template name -> (template asset name, model, shared resources).
    pub prop: &'a dyn Fn(&str) -> Option<PropData>,
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-12 && l.is_finite() {
        [v[0] / l, v[1] / l, v[2] / l]
    } else {
        [0.0, 1.0, 0.0]
    }
}

/// Extract a triangle list from XNA buffers into a compact glTF primitive,
/// flipping winding to CCW. `base_vertex` and `stream_offset` follow
/// `DrawIndexedPrimitives` semantics.
#[allow(clippy::too_many_arguments)]
pub fn primitive(
    decl: &[VertexElement],
    vb: &[u8],
    stride: usize,
    stream_offset: usize,
    base_vertex: usize,
    ib: (bool, &[u8]),
    start_index: usize,
    primitive_count: usize,
    stats: &mut Stats,
) -> Option<Primitive> {
    primitive_with(
        decl,
        vb,
        stride,
        stream_offset,
        base_vertex,
        ib,
        start_index,
        primitive_count,
        false,
        stats,
    )
}

/// `liquid`: also export the liquid vertex extras — the single-float COLOR0
/// deep weight as COLOR_0 `(w, w, w, 1)` and TANGENT with the handedness of
/// the stored binormal (`sign(dot(cross(N, T), B))`).
#[allow(clippy::too_many_arguments)]
fn primitive_with(
    decl: &[VertexElement],
    vb: &[u8],
    stride: usize,
    stream_offset: usize,
    base_vertex: usize,
    ib: (bool, &[u8]),
    start_index: usize,
    primitive_count: usize,
    liquid: bool,
    stats: &mut Stats,
) -> Option<Primitive> {
    let vb = vb.get(stream_offset..)?;
    let pos = mesh::positions(decl, vb, stride)?;
    let nrm = mesh::normals(decl, vb, stride);
    let uv = mesh::texcoords(decl, vb, stride);
    let mut vc = mesh::colors(decl, vb, stride);
    let mut frames = None;
    if liquid {
        if vc.is_none() {
            vc = mesh::color_single(decl, vb, stride)
                .map(|w| w.into_iter().map(|w| [w, w, w, 1.0]).collect());
        }
        if let (Some(n), Some((t, b))) = (&nrm, mesh::tangent_frames(decl, vb, stride)) {
            frames = Some(
                (0..n.len().min(t.len()).min(b.len()))
                    .map(|i| {
                        let (n, t, b) = (n[i], t[i], b[i]);
                        let c = [
                            n[1] * t[2] - n[2] * t[1],
                            n[2] * t[0] - n[0] * t[2],
                            n[0] * t[1] - n[1] * t[0],
                        ];
                        let w = if c[0] * b[0] + c[1] * b[1] + c[2] * b[2] < 0.0 {
                            -1.0
                        } else {
                            1.0
                        };
                        let t = normalize(t);
                        [t[0], t[1], t[2], w]
                    })
                    .collect::<Vec<_>>(),
            );
        }
    }
    let uv1 = mesh::texcoords1(decl, vb, stride);
    let bj = mesh::blend_indices(decl, vb, stride);
    let bw = mesh::blend_weights(decl, vb, stride);
    let skinned = bj.is_some() && bw.is_some();
    let idx = mesh::indices(ib.0, ib.1);
    let tris = idx.get(start_index..start_index + primitive_count * 3)?;
    let mut remap: HashMap<u32, u32> = HashMap::new();
    let mut p = Primitive {
        positions: Vec::new(),
        normals: nrm.as_ref().map(|_| Vec::new()),
        uvs: uv.as_ref().map(|_| Vec::new()),
        indices: Vec::new(),
        material: None,
        joints: skinned.then(Vec::new),
        weights: skinned.then(Vec::new),
        colors: vc.as_ref().map(|_| Vec::new()),
        uvs1: uv1.as_ref().map(|_| Vec::new()),
        tangents: frames.as_ref().map(|_| Vec::new()),
    };
    for t in tris.as_chunks::<3>().0 {
        // Clockwise (XNA front) -> counter-clockwise (glTF front).
        for &i in &[t[0], t[2], t[1]] {
            let v = base_vertex as u32 + i;
            let next = remap.len() as u32;
            let n = *remap.entry(v).or_insert_with(|| next);
            if n == next {
                let vi = v as usize;
                p.positions.push(*pos.get(vi)?);
                if let (Some(out), Some(src)) = (&mut p.normals, &nrm) {
                    out.push(normalize(*src.get(vi)?));
                }
                if let (Some(out), Some(src)) = (&mut p.uvs, &uv) {
                    out.push(*src.get(vi)?);
                }
                if let (Some(out), Some(src)) = (&mut p.colors, &vc) {
                    out.push(*src.get(vi)?);
                }
                if let (Some(out), Some(src)) = (&mut p.uvs1, &uv1) {
                    out.push(*src.get(vi)?);
                }
                if let (Some(out), Some(src)) = (&mut p.tangents, &frames) {
                    out.push(*src.get(vi)?);
                }
                if let (Some(oj), Some(ow), Some(sj), Some(sw)) =
                    (&mut p.joints, &mut p.weights, &bj, &bw)
                {
                    let (j, w) = (*sj.get(vi)?, *sw.get(vi)?);
                    let w = w.map(|x| if x.is_finite() && x > 0.0 { x } else { 0.0 });
                    let sum: f32 = w.iter().sum();
                    let w = if sum <= 0.0 {
                        stats.zero_weights += 1;
                        [1.0, 0.0, 0.0, 0.0]
                    } else {
                        if (sum - 1.0).abs() > 1e-3 {
                            stats.renormalized_weights += 1;
                        }
                        w.map(|x| x / sum)
                    };
                    // Unused (zero-weight) slots point at joint 0, as glTF recommends.
                    oj.push([0, 1, 2, 3].map(|k| if w[k] > 0.0 { j[k] } else { 0 }));
                    ow.push(w);
                }
            }
            p.indices.push(n);
        }
    }
    if p.indices.is_empty() {
        return None;
    }
    if let Some(n) = &p.normals {
        for t in p.indices.as_chunks::<3>().0 {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| p.positions[i as usize]);
            let (u, w) = (
                [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
            );
            let g = [
                u[1] * w[2] - u[2] * w[1],
                u[2] * w[0] - u[0] * w[2],
                u[0] * w[1] - u[1] * w[0],
            ];
            let m =
                [0, 1, 2].map(|k| n[t[0] as usize][k] + n[t[1] as usize][k] + n[t[2] as usize][k]);
            let d = g[0] * m[0] + g[1] * m[1] + g[2] * m[2];
            if d > 0.0 {
                stats.normal_agree += 1;
            } else if d < 0.0 {
                stats.normal_disagree += 1;
            }
        }
    }
    stats.primitives += 1;
    stats.triangles += p.indices.len() / 3;
    Some(p)
}

struct Materials {
    cache: HashMap<String, usize>,
}

impl Materials {
    /// Invisible shadow casters (`magicka_shadow_only`): drawn by the viewer
    /// into shadow maps only.
    fn shadow_only(&mut self, b: &mut Builder) -> usize {
        if let Some(m) = self.cache.get("shadow_only") {
            return *m;
        }
        let m = b.material(
            "shadow_only",
            [1., 1., 1., 1.],
            None,
            false,
            Some("{\"magicka_shadow_only\":true}"),
        );
        self.cache.insert("shadow_only".into(), m);
        m
    }

    fn deferred(
        &mut self,
        b: &mut Builder,
        ctx: &Context,
        asset: &str,
        effect: Option<&XnaObject>,
        stats: &mut Stats,
    ) -> Option<usize> {
        if let Some(XnaObject::AdditiveEffect(e)) = effect {
            let uri = e.texture.as_ref().and_then(|t| {
                let path = mesh::resolve_reference(asset, t);
                let uri = (ctx.texture_uri)(&path);
                if uri.is_none() {
                    stats.missing_textures.push(path);
                }
                uri
            });
            let c = e.color_tint;
            let extras = format!(
                "{{\"magicka_additive\":{{\"color\":[{},{},{}],\"texture_enabled\":{},\"vertex_color\":{}}}}}",
                c[0], c[1], c[2], e.texture_enabled, e.vertex_color_enabled);
            let key = format!("additive|{uri:?}|{extras}");
            if let Some(m) = self.cache.get(&key) {
                return Some(*m);
            }
            if uri.is_some() {
                stats.textured_materials += 1;
            }
            let m = b.material(
                "additive",
                [c[0], c[1], c[2], 1.],
                uri.as_deref(),
                false,
                Some(&extras),
            );
            self.cache.insert(key, m);
            return Some(m);
        }
        let (color, tex, mask, alpha) = match effect {
            // Cutout rule PROVEN from PolygonHead's RenderDeferredEffect
            // pixel shaders (output/evidence/shaders/): texkill(alpha - 0.5)
            // uses the diffuse texture alpha only when neither
            // AlphaMask0Enabled (alpha is then a layer-blend mask) nor
            // DiffuseTexture0AlphaDisabled is set.
            Some(XnaObject::RenderDeferredEffect(e)) => (
                e.layer0.diffuse_color,
                e.layer0
                    .diffuse_texture
                    .as_ref()
                    .map(|t| mesh::resolve_reference(asset, t)),
                e.layer0.diffuse_texture.is_some()
                    && !e.layer0.alpha_mask_enabled
                    && !e.layer0.diffuse_alpha_disabled,
                e.alpha,
            ),
            Some(XnaObject::RenderDeferredLiquidEffect(e)) => (e.bottom_color, None, false, 1.0),
            Some(XnaObject::LavaEffect(e)) => (e.rock_color, None, false, 1.0),
            _ => return None,
        };
        // Everything the game's dual-layer shader needs (shaders.md), for a
        // custom renderer; standard glTF viewers use only layer 0.
        let extras = match effect {
            Some(XnaObject::RenderDeferredEffect(e)) => {
                Some(deferred_material_extras(e, asset, ctx.texture_uri))
            }
            Some(XnaObject::RenderDeferredLiquidEffect(e)) => Some(liquid_extras(e, asset, ctx)),
            Some(XnaObject::LavaEffect(e)) => Some(lava_extras(e, asset, ctx)),
            _ => None,
        };
        let key = format!("{color:?}|{tex:?}|{mask}|{alpha}|{extras:?}");
        if let Some(m) = self.cache.get(&key) {
            return Some(*m);
        }
        let uri = tex.as_ref().and_then(|t| {
            let u = (ctx.texture_uri)(t);
            if u.is_none() {
                stats.missing_textures.push(t.clone());
            }
            u
        });
        if uri.is_some() {
            stats.textured_materials += 1;
        }
        let name = tex
            .as_deref()
            .and_then(|t| t.rsplit('/').next())
            .unwrap_or("material")
            .to_string();
        let m = b.material(
            &name,
            [color[0], color[1], color[2], alpha.clamp(0.0, 1.0)],
            uri.as_deref(),
            mask,
            extras.as_deref(),
        );
        self.cache.insert(key, m);
        Some(m)
    }
}

/// glTF material extras for a RenderDeferredEffect: `{"magicka_deferred": {...}}`.
pub fn deferred_material_extras(
    e: &crate::xna::RenderDeferredEffect,
    asset: &str,
    texture_uri: &dyn Fn(&str) -> Option<String>,
) -> String {
    let uri = |t: &Option<String>| -> String {
        t.as_ref()
            .and_then(|t| texture_uri(&mesh::resolve_reference(asset, t)))
            .map_or("null".into(), |u| format!("\"{}\"", gltf::escape(&u)))
    };
    let c = |v: [f32; 3]| format!("[{},{},{}]", v[0], v[1], v[2]);
    // SpecAmount, SpecPower, EmissiveAmount (G-buffer lighting inputs).
    let spec = |l: &crate::xna::DeferredLayer| {
        format!("[{},{},{}]", l.spec_amount, l.spec_power, l.emissive_amount)
    };
    let l1 = e.layer1.as_ref();
    format!(
        "{{\"magicka_deferred\":{{\"tex0\":{},\"tex1\":{},\"color0\":{},\"color1\":{},\"tex0_alpha_disabled\":{},\"alpha_mask0\":{},\"tex1_alpha_disabled\":{},\"alpha_mask1\":{},\"dual_layer\":{},\"sharpness\":{},\"vertex_color\":{},\"alpha\":{},\"spec0\":{},\"spec1\":{},\"mat0\":{},\"mat1\":{}}}}}",
        uri(&e.layer0.diffuse_texture),
        l1.map_or("null".into(), |l| uri(&l.diffuse_texture)),
        c(e.layer0.diffuse_color),
        c(l1.map_or([1.0; 3], |l| l.diffuse_color)),
        e.layer0.diffuse_alpha_disabled,
        e.layer0.alpha_mask_enabled,
        l1.is_some_and(|l| l.diffuse_alpha_disabled),
        l1.is_some_and(|l| l.alpha_mask_enabled),
        l1.is_some(),
        e.sharpness,
        e.vertex_color_enabled,
        e.alpha,
        spec(&e.layer0),
        l1.map_or("[0,0,0]".into(), spec),
        uri(&e.layer0.material_texture),
        l1.map_or("null".into(), |l| uri(&l.material_texture)),
    )
}

/// Authored LavaEffect parameters and private texture references for shader 5.
fn lava_extras(e: &crate::xna::LavaEffect, asset: &str, ctx: &Context) -> String {
    let uri = |t: &Option<String>| -> String {
        t.as_ref()
            .and_then(|t| (ctx.texture_uri)(&mesh::resolve_reference(asset, t)))
            .map_or("null".into(), |u| format!("\"{}\"", gltf::escape(&u)))
    };
    format!(
        "{{\"magicka_lava\":{{\"mask_distortion\":{},\"speed0\":{:?},\"speed1\":{:?},\"hot_emissive\":{},\"cold_emissive\":{},\"spec_amount\":{},\"spec_power\":{},\"temp_frequency\":{},\"tone_map\":{},\"temp_map\":{},\"mask_map\":{},\"rock_color\":{:?},\"rock_emissive\":{},\"rock_spec_amount\":{},\"rock_spec_power\":{},\"rock_normal_power\":{},\"rock_texture\":{},\"rock_normal_map\":{}}}}}",
        e.mask_distortion, e.speed0, e.speed1, e.lava_hot_emissive,
        e.lava_cold_emissive, e.lava_spec_amount, e.lava_spec_power,
        e.temp_frequency, uri(&e.tone_map), uri(&e.temp_map), uri(&e.mask_map),
        e.rock_color, e.rock_emissive, e.rock_spec_amount, e.rock_spec_power,
        e.rock_normal_power, uri(&e.rock_texture), uri(&e.rock_normal_map),
    )
}

/// RenderDeferredLiquidEffect (Water) parameters for the liquid shader.
fn liquid_extras(e: &crate::xna::LiquidEffect, asset: &str, ctx: &Context) -> String {
    let uri = |t: &Option<String>| -> String {
        t.as_ref()
            .and_then(|t| (ctx.texture_uri)(&mesh::resolve_reference(asset, t)))
            .map_or("null".into(), |u| format!("\"{}\"", gltf::escape(&u)))
    };
    let c = |v: [f32; 3]| format!("[{},{},{}]", v[0], v[1], v[2]);
    format!(
        "{{\"magicka_liquid\":{{\"reflection_map\":{},\"wave_height\":{},\"wave_speed0\":[{},{}],\"wave_speed1\":[{},{}],\"reflectiveness\":{},\"bottom_color\":{},\"deep_bottom_color\":{},\"emissive\":{},\"spec_amount\":{},\"spec_power\":{},\"bottom_texture\":{},\"normal_map\":{},\"ice_reflectiveness\":{},\"ice_color\":{},\"ice_emissive\":{},\"ice_spec_amount\":{},\"ice_spec_power\":{},\"ice_diffuse_map\":{},\"ice_normal_map\":{}}}}}",
        e.reflection_map.is_some(),
        e.wave_height,
        e.wave_speed0[0],
        e.wave_speed0[1],
        e.wave_speed1[0],
        e.wave_speed1[1],
        e.water_reflectiveness,
        c(e.bottom_color),
        c(e.deep_bottom_color),
        e.water_emissive,
        e.water_spec_amount,
        e.water_spec_power,
        uri(&e.bottom_texture),
        uri(&e.water_normal_map),
        e.ice_reflectiveness,
        c(e.ice_color),
        e.ice_emissive,
        e.ice_spec_amount,
        e.ice_spec_power,
        uri(&e.ice_diffuse_map),
        uri(&e.ice_normal_map),
    )
}

/// MeshSettings (visible, castShadows) as the exporter needs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MeshVisibility {
    Visible,
    ShadowOnly,
    Hidden,
}

impl MeshVisibility {
    fn new(visible: bool, cast_shadows: bool) -> Self {
        match (visible, cast_shadows) {
            (true, _) => Self::Visible,
            (false, true) => Self::ShadowOnly,
            (false, false) => Self::Hidden,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn model_primitives(
    b: &mut Builder,
    mats: &mut Materials,
    ctx: &Context,
    asset: &str,
    model: &Model,
    shared: &[Option<XnaObject>],
    visible: &dyn Fn(Option<&str>) -> MeshVisibility,
    only_first: bool,
    stats: &mut Stats,
) -> Vec<Primitive> {
    let mut out = Vec::new();
    for m in &model.meshes {
        let mode = visible(m.name.as_deref());
        if mode == MeshVisibility::Hidden {
            continue;
        }
        let (Some(vb), Some((sixteen, ib))) = (&m.vertex_buffer, &m.index_buffer) else {
            continue;
        };
        for part in &m.parts {
            let Some(decl) = model
                .vertex_declarations
                .get(part.vertex_declaration as usize)
            else {
                continue;
            };
            let effect = (part.effect as usize)
                .checked_sub(1)
                .and_then(|i| shared.get(i))
                .and_then(|o| o.as_ref());
            // Native deferred and additive passes share geometry/transforms;
            // material extras let the viewer select the matching pass.
            if !matches!(
                effect,
                Some(XnaObject::RenderDeferredEffect(_) | XnaObject::AdditiveEffect(_)) | None
            ) {
                continue;
            }
            if let Some(mut p) = primitive(
                decl,
                vb,
                decl_stride(decl),
                part.stream_offset.max(0) as usize,
                part.base_vertex.max(0) as usize,
                (*sixteen, ib),
                part.start_index.max(0) as usize,
                part.primitive_count.max(0) as usize,
                stats,
            ) {
                p.material = if mode == MeshVisibility::ShadowOnly {
                    // Additive geometry has no shadow technique.
                    if matches!(effect, Some(XnaObject::AdditiveEffect(_))) {
                        continue;
                    }
                    Some(mats.shadow_only(b))
                } else {
                    mats.deferred(b, ctx, asset, effect, stats)
                };
                out.push(p);
            }
            if only_first {
                return out;
            }
        }
    }
    out
}

fn liquid_primitive(
    b: &mut Builder,
    mats: &mut Materials,
    ctx: &Context,
    l: &Liquid,
    stats: &mut Stats,
) -> Option<Primitive> {
    let (Some(decl), Some(vb), Some((sixteen, ib))) =
        (&l.vertex_declaration, &l.vertex_buffer, &l.index_buffer)
    else {
        return None;
    };
    let mut p = primitive_with(
        decl,
        vb,
        l.vertex_stride.max(0) as usize,
        0,
        0,
        (*sixteen, ib),
        0,
        l.primitive_count.max(0) as usize,
        true,
        stats,
    )?;
    let effect = match &l.material {
        LiquidMaterial::Water(e) => XnaObject::RenderDeferredLiquidEffect(e.clone()),
        LiquidMaterial::Lava(e) => XnaObject::LavaEffect(e.clone()),
    };
    // Native Water/Lava initialize stream 1 to one when AutoFreeze is set.
    p.uvs1 = Some(vec![
        [if l.auto_freeze { 1.0 } else { 0.0 }, 0.0];
        p.positions.len()
    ]);
    p.material = mats.deferred(b, ctx, ctx.asset, Some(&effect), stats);
    stats.liquids += 1;
    Some(p)
}

/// Decompose a pose for glTF TRS. The game's `Pose.GetMatrix` builds R·S
/// (rotation, then scale, in row-vector order); glTF applies S then R. They
/// agree only for uniform scale, so non-uniform poses are counted.
fn check_scale(s: [f32; 3], stats: &mut Stats) {
    let tol = 1e-4 * s[0].abs().max(1.0);
    if (s[0] - s[1]).abs() > tol || (s[0] - s[2]).abs() > tol {
        stats.anisotropic_scales += 1;
    }
}

fn part_node(
    b: &mut Builder,
    mats: &mut Materials,
    ctx: &Context,
    shared: &[Option<XnaObject>],
    part: &AnimatedLevelPart,
    stats: &mut Stats,
) -> usize {
    stats.animated_parts += 1;
    let index = b.placeholder();
    let mut children = Vec::new();
    for c in &part.children {
        children.push(part_node(b, mats, ctx, shared, c, stats));
    }
    let settings: HashMap<&str, MeshVisibility> = part
        .mesh_settings
        .iter()
        .map(|(n, vis, cast)| (n.as_str(), MeshVisibility::new(*vis, *cast)))
        .collect();
    let visible = |name: Option<&str>| {
        name.and_then(|n| settings.get(n))
            .copied()
            .unwrap_or(MeshVisibility::Visible)
    };
    let mut prims = Vec::new();
    if let Some(model) = &part.model {
        // Effects of an animated part's model are shared resources of the level file.
        prims = model_primitives(
            b, mats, ctx, ctx.asset, model, shared, &visible, false, stats,
        );
    }
    for l in &part.liquids {
        if let Some(p) = liquid_primitive(b, mats, ctx, l, stats) {
            prims.push(p);
        }
    }
    let mesh = (!prims.is_empty()).then(|| b.mesh(&part.name, &prims));
    // Rest pose: the keyframe the game samples at time 0.
    let transform = part.animation.first().map_or(String::new(), |k| {
        let (t, r, sc) = (
            finite3(k.translation, stats),
            sanitize_q(k.orientation, stats),
            finite3(k.scale, stats),
        );
        gltf::trs_json(t, r, sc)
    });
    b.set_node(index, &part.name, &transform, mesh, &children);
    // Preserve exact stored key bits independently of glTF's sorted/filtered
    // animation channels. Native runtime sampling must not consume those edits.
    let raw_keys: Vec<String> = part
        .animation
        .iter()
        .map(|key| {
            let mut bits = vec![key.time.to_bits()];
            bits.extend(key.translation.map(f32::to_bits));
            bits.extend(key.orientation.map(f32::to_bits));
            bits.extend(key.scale.map(f32::to_bits));
            format!("{bits:?}")
        })
        .collect();
    b.node_extras(
        index,
        &format!(
            "{{\"magicka_level_animation\":{{\"version\":1,\"duration_bits\":{},\"keys\":[{}]}}}}",
            part.animation_duration.to_bits(),
            raw_keys.join(",")
        ),
    );
    // Keyframes -> LINEAR TRS animation (strictly increasing times only).
    let mut keys = Vec::new();
    let mut last = f32::MIN;
    for k in &part.animation {
        check_scale(k.scale, stats);
        if k.time > last && k.time >= 0.0 {
            keys.push(k);
            last = k.time;
        } else {
            stats.dropped_keyframes += 1;
        }
    }
    let moving = keys.windows(2).any(|w| {
        w[0].translation != w[1].translation
            || w[0].orientation != w[1].orientation
            || w[0].scale != w[1].scale
    });
    if keys.len() >= 2 && moving {
        let times: Vec<f32> = keys.iter().map(|k| k.time).collect();
        let input = b.scalars(&times, true);
        let t = b.vec3(
            &keys
                .iter()
                .map(|k| finite3(k.translation, stats))
                .collect::<Vec<_>>(),
            false,
        );
        let r = b.vec4(
            &keys
                .iter()
                .map(|k| sanitize_q(k.orientation, stats))
                .collect::<Vec<_>>(),
        );
        let s = b.vec3(
            &keys
                .iter()
                .map(|k| finite3(k.scale, stats))
                .collect::<Vec<_>>(),
            false,
        );
        b.animation(
            &part.name,
            &[
                (index, "translation", input, t),
                (index, "rotation", input, r),
                (index, "scale", input, s),
            ],
        );
        stats.animations += 1;
    }
    index
}

pub(crate) fn finite3(v: [f32; 3], stats: &mut Stats) -> [f32; 3] {
    v.map(|x| {
        if x.is_finite() {
            x
        } else {
            stats.non_finite_components += 1;
            0.0
        }
    })
}

pub(crate) fn sanitize_q(q: [f32; 4], stats: &mut Stats) -> [f32; 4] {
    if q.iter().any(|x| !x.is_finite()) {
        stats.non_finite_components += 1;
        return [0.0, 0.0, 0.0, 1.0];
    }
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if l == 0.0 {
        stats.zero_quaternions += 1;
        return [0.0, 0.0, 0.0, 1.0];
    }
    if (l - 1.0).abs() > 1e-3 {
        stats.non_unit_quaternions += 1;
    }
    q.map(|x| x / l)
}

/// Build a .glb for a parsed level. `shared` are the level file's shared
/// resources (effects referenced by animated-part models).
pub fn export_level(
    level: &LevelModel,
    shared: &[Option<XnaObject>],
    ctx: &Context,
) -> (Vec<u8>, Stats) {
    let mut b = Builder::default();
    let mut stats = Stats::default();
    let mut mats = Materials {
        cache: HashMap::new(),
    };
    let mut top = Vec::new();

    // Static geometry (shadow-only roots in their own node).
    let mut prims = Vec::new();
    let mut shadow_prims = Vec::new();
    for root in level
        .model
        .iter()
        .flat_map(|m| m.roots.iter())
        .filter(|r| r.visible || r.cast_shadows)
    {
        let (Some(decl), Some(vb), Some((sixteen, ib))) = (
            &root.vertex_declaration,
            &root.vertex_buffer,
            &root.index_buffer,
        ) else {
            continue;
        };
        let n = mesh::indices(*sixteen, ib).len() / 3;
        if let Some(mut p) = primitive(
            decl,
            vb,
            root.vertex_stride.max(0) as usize,
            0,
            0,
            (*sixteen, ib),
            0,
            n,
            &mut stats,
        ) {
            if root.visible {
                p.material =
                    mats.deferred(&mut b, ctx, ctx.asset, root.effect.as_ref(), &mut stats);
                prims.push(p);
            } else if !matches!(root.effect, Some(XnaObject::AdditiveEffect(_))) {
                // BiTreeModel render data: invisible roots keep only their
                // shadow technique (additive roots have none).
                p.material = Some(mats.shadow_only(&mut b));
                shadow_prims.push(p);
            }
        }
    }
    if !prims.is_empty() {
        let m = b.mesh("static", &prims);
        top.push(b.node("static", "", Some(m), &[]));
    }
    if !shadow_prims.is_empty() {
        let m = b.mesh("static_shadow", &shadow_prims);
        top.push(b.node("static_shadow", "", Some(m), &[]));
    }

    // Top-level liquids.
    let liquids: Vec<Primitive> = level
        .liquids
        .iter()
        .filter_map(|l| liquid_primitive(&mut b, &mut mats, ctx, l, &mut stats))
        .collect();
    if !liquids.is_empty() {
        let m = b.mesh("liquids", &liquids);
        top.push(b.node("liquids", "", Some(m), &[]));
    }

    // Animated parts.
    let mut parts = Vec::new();
    for p in &level.animated_parts {
        parts.push(part_node(&mut b, &mut mats, ctx, shared, p, &mut stats));
    }
    if !parts.is_empty() {
        top.push(b.node("animated_parts", "", None, &parts));
    }

    // Physics props.
    let mut props = Vec::new();
    let mut prop_meshes: HashMap<String, Option<usize>> = HashMap::new();
    for (i, (m, name)) in level.physics_entities.iter().enumerate() {
        let mesh = match prop_meshes.get(name) {
            Some(x) => *x,
            None => {
                let mesh = (ctx.prop)(name).and_then(|(asset, model, shared)| {
                    let p = model_primitives(
                        &mut b,
                        &mut mats,
                        ctx,
                        &asset,
                        &model,
                        &shared,
                        &|_| MeshVisibility::Visible,
                        true,
                        &mut stats,
                    );
                    (!p.is_empty()).then(|| b.mesh(name, &p))
                });
                if mesh.is_none() {
                    stats.missing_props.push(name.clone());
                }
                prop_meshes.insert(name.clone(), mesh);
                mesh
            }
        };
        // The physics body keeps only rotation + position of StartTransform.
        let mut t = *m;
        let rows: [[f32; 3]; 3] = [[t[0], t[1], t[2]], [t[4], t[5], t[6]], [t[8], t[9], t[10]]];
        for (r, row) in rows.iter().enumerate() {
            let n = normalize(*row);
            t[r * 4..r * 4 + 3].copy_from_slice(&n);
        }
        props.push(b.node(&format!("{name}#{i}"), &gltf::matrix_json(&t), mesh, &[]));
        stats.props += 1;
    }
    if !props.is_empty() {
        top.push(b.node("physics_props", "", None, &props));
    }

    let root = b.node(ctx.asset, "", None, &top);
    (b.finish(&[root]), stats)
}

impl Stats {
    pub fn report(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "primitives {} triangles {} liquids {} animated parts {} animations {} props {}",
            self.primitives,
            self.triangles,
            self.liquids,
            self.animated_parts,
            self.animations,
            self.props
        );
        let _ = writeln!(
            s,
            "winding: {} faces agree with vertex normals, {} disagree",
            self.normal_agree, self.normal_disagree
        );
        let _ = writeln!(
            s,
            "anisotropic pose scales {} dropped keyframes {} zero quaternions {} non-unit quaternions {} non-finite components {}",
            self.anisotropic_scales, self.dropped_keyframes, self.zero_quaternions, self.non_unit_quaternions, self.non_finite_components
        );
        let _ = writeln!(
            s,
            "textured materials {} missing textures {:?} missing props {:?}",
            self.textured_materials, self.missing_textures, self.missing_props
        );
        s
    }
}
