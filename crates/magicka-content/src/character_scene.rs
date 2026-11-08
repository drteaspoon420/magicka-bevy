//! Export a character (CharacterTemplate) as a skinned, animated glTF.
//!
//! Runtime semantics (decompiled `CharacterTemplate.Read`): the skeleton and
//! animation clips come from `skeleton_model`; each `models[i]` supplies
//! geometry whose blend indices address that skeleton. The game skins with
//! `skin[i] = invBind[i] · abs[i]`, `abs[i] = local[i] · abs[parent]`
//! (see `skinned.rs`). Model scale (`ModelProperties.scale`) is applied as a
//! uniform scale on the armature root.

use crate::assets::Asset;
use crate::character::CharacterTemplate;
use crate::gltf::{self, Builder};
use crate::mesh;
use crate::scene::{decl_stride, finite3, primitive, sanitize_q, Stats};
use crate::skinned::Resolved;
use crate::xna::XnaObject;
use std::collections::HashMap;
use std::fmt::Write as _;

#[derive(Default, Debug, Clone)]
pub struct CharStats {
    pub geometry: Stats,
    pub bones: usize,
    pub clips: usize,
    pub channels: usize,
    /// Clip channels naming a bone that is not in the skeleton.
    pub unmatched_channels: usize,
    /// Mesh-file skeleton differs from the animation skeleton (names/order).
    pub skeleton_name_mismatch: bool,
    /// Blend indices >= skeleton bone count (would read garbage in the game).
    pub bad_joint_indices: usize,
    /// Bind poses / keyframes with non-uniform scale. Exported exactly via the
    /// two-node (T·S, R) joint split; counted for information.
    pub anisotropic: usize,
    /// Channels resampled because of negative or non-increasing key times.
    pub dropped_keyframes: usize,
    /// Keys inserted so glTF slerp tracks the game's normalized lerp.
    pub densified_keys: usize,
    pub textured_materials: usize,
    pub missing_textures: Vec<String>,
    /// Game-math reference samples for the independent checker (JSON).
    pub reference_json: Option<String>,
}

impl CharStats {
    pub fn report(&self) -> String {
        let mut s = String::new();
        let g = &self.geometry;
        let _ = writeln!(
            s,
            "bones {} clips {} channels {} unmatched channels {} skeleton name mismatch {} bad joint indices {}",
            self.bones, self.clips, self.channels, self.unmatched_channels, self.skeleton_name_mismatch, self.bad_joint_indices
        );
        let _ = writeln!(
            s,
            "primitives {} triangles {} winding agree {} disagree {} renormalized weights {} zero weights {}",
            g.primitives, g.triangles, g.normal_agree, g.normal_disagree, g.renormalized_weights, g.zero_weights
        );
        let _ = writeln!(
            s,
            "anisotropic scales {} resampled channels {} densified keys {} non-finite {} textured materials {} missing textures {:?}",
            self.anisotropic, self.dropped_keyframes, self.densified_keys, g.non_finite_components, self.textured_materials, self.missing_textures
        );
        s
    }
}

fn anisotropic(s: [f32; 3]) -> bool {
    (s[0] - s[1]).abs() > 1e-4 * s[0].abs().max(1.0)
        || (s[0] - s[2]).abs() > 1e-4 * s[0].abs().max(1.0)
}

/// Preserve the original basic skinned material's separate fabric overlay.
fn skinned_overlay_extras(
    effect: Option<&XnaObject>,
    asset: &str,
    texture_uri: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    let Some(XnaObject::Asset(effect)) = effect else {
        return None;
    };
    let Asset::SkinnedModelBasicEffect(effect) = effect.as_ref() else {
        return None;
    };
    let map_uri = |index: usize| {
        effect
            .maps
            .get(index)
            .and_then(|map| map.as_ref())
            .and_then(|map| {
                let XnaObject::Asset(asset) = map else {
                    return None;
                };
                let Asset::External(path) = asset.as_ref() else {
                    return None;
                };
                path.as_deref()
            })
            .and_then(|path| texture_uri(&mesh::resolve_reference(asset, path)))
    };
    // SpecularMap (MaterialMapSampler): R emissive, G specular amount,
    // B specular power (SkinnedModelDeferredEffect shader 11).
    let material = map_uri(2).map_or("null".into(), |uri| format!("\"{}\"", gltf::escape(&uri)));
    let overlay = effect
        .maps
        .get(1)
        .and_then(|map| map.as_ref())
        .and_then(|map| {
            let XnaObject::Asset(asset) = map else {
                return None;
            };
            let Asset::External(path) = asset.as_ref() else {
                return None;
            };
            path.as_deref()
        })
        .and_then(|path| texture_uri(&mesh::resolve_reference(asset, path)));
    let overlay = overlay.map_or("null".into(), |uri| format!("\"{}\"", gltf::escape(&uri)));
    Some(format!(
        "{{\"magicka_skinned\":{{\"overlay\":{overlay},\"material\":{material},\"material_enabled\":{},\"overlay_alpha\":{},\"soft_light\":{},\"detail_enabled\":{},\"overlay_enabled\":{},\"diffuse\":[{},{},{}],\"spec\":[{},{},{}]}}}}",
        effect.map_enabled[2],
        effect.diffuse1_alpha, effect.use_soft_light_blend,
        effect.map_enabled[0], effect.map_enabled[1],
        effect.diffuse_color[0], effect.diffuse_color[1], effect.diffuse_color[2],
        effect.specular_amount, effect.specular_power, effect.emissive_amount,
    ))
}

/// Diffuse color + texture of a skinned mesh part's effect.
/// Alpha cutout (`texkill(alpha - 0.5)`, glTF MASK at 0.5). PROVEN from the
/// shader dumps (output/evidence/shaders/): RenderDeferredEffect kills on the
/// layer-0 diffuse alpha unless AlphaMask0Enabled or
/// DiffuseTexture0AlphaDisabled; SkinnedModelDeferred(NormalMapped)Effect
/// kills on the diffuse map alpha. With a second skinned diffuse map the
/// native test uses the combined alpha a1 + a0(1 - a1), which one texture
/// cannot express, so those materials stay opaque.
fn cutout(effect: Option<&XnaObject>) -> bool {
    match effect {
        Some(XnaObject::RenderDeferredEffect(e)) => {
            e.layer0.diffuse_texture.is_some()
                && !e.layer0.alpha_mask_enabled
                && !e.layer0.diffuse_alpha_disabled
        }
        Some(XnaObject::Asset(a)) => match a.as_ref() {
            Asset::SkinnedModelBasicEffect(e) => {
                e.map_enabled[0] && e.maps.first().is_some_and(|m| m.is_some()) && !e.map_enabled[1]
            }
            Asset::SkinnedModelDeferredNormalMappedEffect(e) => e.maps[0].is_some(),
            _ => false,
        },
        _ => false,
    }
}

fn skinned_material(effect: Option<&XnaObject>) -> ([f32; 3], Option<String>) {
    let ext = |o: &Option<XnaObject>| match o {
        Some(XnaObject::Asset(a)) => match a.as_ref() {
            Asset::External(Some(p)) => Some(p.clone()),
            _ => None,
        },
        _ => None,
    };
    match effect {
        Some(XnaObject::Asset(a)) => match a.as_ref() {
            Asset::SkinnedModelBasicEffect(e) => (e.diffuse_color, e.maps.first().and_then(ext)),
            Asset::SkinnedModelDeferredNormalMappedEffect(e) => {
                (e.diffuse_color, e.maps[0].clone())
            }
            Asset::BasicEffect(e) => (e.diffuse, e.texture.clone()),
            _ => ([0.8; 3], None),
        },
        Some(XnaObject::RenderDeferredEffect(e)) => {
            (e.layer0.diffuse_color, e.layer0.diffuse_texture.clone())
        }
        _ => ([0.8; 3], None),
    }
}

/// `load(asset)` returns a resolved skinned model for an asset name;
/// `texture_uri(asset)` maps a texture asset to a URI relative to the .glb.
/// An AnimatedPhysicsEntity template (Data/AnimatedProps): its first model
/// skinned to the skeleton model's bones and clips (Initialize picks a
/// random model; the retail props have one).
pub fn export_animated_prop(
    id: &str,
    part: &crate::physics::AnimatedPart,
    asset: &str,
    load: &dyn Fn(&str) -> Option<Resolved>,
    texture_uri: &dyn Fn(&str) -> Option<String>,
    reference: bool,
) -> Result<(Vec<u8>, CharStats), String> {
    let skel_asset = part
        .skeleton_model
        .as_ref()
        .map(|r| mesh::resolve_reference(asset, r))
        .ok_or("no skeleton model")?;
    let skel = load(&skel_asset).ok_or_else(|| format!("cannot load skeleton {skel_asset}"))?;
    let props = part.models.first().ok_or("no models")?;
    let mesh_asset = props
        .model
        .as_ref()
        .map(|r| mesh::resolve_reference(asset, r))
        .ok_or("model 0 has no reference")?;
    let mres = load(&mesh_asset).ok_or_else(|| format!("cannot load mesh {mesh_asset}"))?;
    export_skinned(
        id,
        &skel,
        &mesh_asset,
        &mres,
        props.scale,
        texture_uri,
        reference,
    )
}

pub fn export_character(
    t: &CharacterTemplate,
    asset: &str,
    load: &dyn Fn(&str) -> Option<Resolved>,
    texture_uri: &dyn Fn(&str) -> Option<String>,
    reference: bool,
) -> Result<(Vec<u8>, CharStats), String> {
    let skel_asset = t
        .skeleton_model
        .as_ref()
        .map(|r| mesh::resolve_reference(asset, r))
        .ok_or("no skeleton model")?;
    let skel = load(&skel_asset).ok_or_else(|| format!("cannot load skeleton {skel_asset}"))?;
    let props = t.models.first().ok_or("no models")?;
    let mesh_asset = props
        .model
        .as_ref()
        .map(|r| mesh::resolve_reference(asset, r))
        .ok_or("model 0 has no reference")?;
    let mres = load(&mesh_asset).ok_or_else(|| format!("cannot load mesh {mesh_asset}"))?;
    export_skinned(
        &t.id,
        &skel,
        &mesh_asset,
        &mres,
        props.scale,
        texture_uri,
        reference,
    )
}

/// A skinned model (`skel` supplies bones and clips, `mres` the mesh of
/// `mesh_asset`) as an animated glTF named `id`, uniformly scaled.
pub fn export_skinned(
    id: &str,
    skel: &Resolved,
    mesh_asset: &str,
    mres: &Resolved,
    scale: f32,
    texture_uri: &dyn Fn(&str) -> Option<String>,
    reference: bool,
) -> Result<(Vec<u8>, CharStats), String> {
    let mut st = CharStats::default();
    let bones = &skel.skeleton.bones;
    st.bones = bones.len();
    st.skeleton_name_mismatch = mres.skeleton.names() != skel.skeleton.names();

    let mut b = Builder::default();
    // Each bone is two nodes so glTF reproduces the game's order exactly:
    // the game's local matrix is rotate, then scale, then translate (column
    // form T·S·R; `Pose.GetMatrix`), while a single glTF node is T·R·S.
    // outer node = T and S, inner node (the skin joint) = R; child bones hang
    // under the parent's inner node.
    let outer: Vec<usize> = bones.iter().map(|_| b.placeholder()).collect();
    let joints: Vec<usize> = bones.iter().map(|_| b.placeholder()).collect();
    for (i, bone) in bones.iter().enumerate() {
        let children: Vec<usize> = bones
            .iter()
            .enumerate()
            .filter(|(_, c)| c.parent == Some(i))
            .map(|(k, _)| outer[k])
            .collect();
        if anisotropic(bone.scale) {
            st.anisotropic += 1;
        }
        let ts = format!(
            "\"translation\":{},\"scale\":{}",
            gltf_arr(&finite3(bone.translation, &mut st.geometry)),
            gltf_arr(&finite3(bone.scale, &mut st.geometry))
        );
        let r = format!(
            "\"rotation\":{}",
            gltf_arr(&sanitize_q(bone.orientation, &mut st.geometry))
        );
        b.set_node(
            outer[i],
            &format!("{}.ts", bone.name),
            &ts,
            None,
            &[joints[i]],
        );
        b.set_node(joints[i], &bone.name, &r, None, &children);
    }
    let roots: Vec<usize> = bones
        .iter()
        .enumerate()
        .filter(|(_, x)| x.parent.is_none())
        .map(|(i, _)| outer[i])
        .collect();
    let s = scale;
    let armature = b.node(
        "armature",
        &gltf::trs_json([0.0; 3], [0.0, 0.0, 0.0, 1.0], [s, s, s]),
        None,
        &roots,
    );
    let ibm = b.mat4(&bones.iter().map(|x| x.inverse_bind).collect::<Vec<_>>());
    let skin = b.skin(id, &joints, ibm, armature);

    // Geometry from model 0, materials from that file's shared effects.
    let model = mres.model.as_ref().ok_or("mesh file has no model")?;
    let mut mats: HashMap<String, usize> = HashMap::new();
    let mut prims = Vec::new();
    for m in &model.meshes {
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
            let Some(mut p) = primitive(
                decl,
                vb,
                decl_stride(decl),
                part.stream_offset.max(0) as usize,
                part.base_vertex.max(0) as usize,
                (*sixteen, ib),
                part.start_index.max(0) as usize,
                part.primitive_count.max(0) as usize,
                &mut st.geometry,
            ) else {
                continue;
            };
            if let Some(j) = &p.joints {
                st.bad_joint_indices += j
                    .iter()
                    .flatten()
                    .filter(|&&x| x as usize >= bones.len())
                    .count();
            }
            let effect = (part.effect as usize)
                .checked_sub(1)
                .and_then(|i| mres.shared.get(i))
                .and_then(|o| o.as_ref());
            let (color, tex) = skinned_material(effect);
            let tex = tex.map(|r| mesh::resolve_reference(mesh_asset, &r));
            let extras = skinned_overlay_extras(effect, mesh_asset, texture_uri);
            let mask = cutout(effect);
            let key = format!("{color:?}|{tex:?}|{extras:?}|{mask}");
            let mat = match mats.get(&key) {
                Some(m) => *m,
                None => {
                    let uri = tex.as_ref().and_then(|t| {
                        let u = texture_uri(t);
                        if u.is_none() {
                            st.missing_textures.push(t.clone());
                        }
                        u
                    });
                    st.textured_materials += uri.is_some() as usize;
                    let name = tex
                        .as_deref()
                        .and_then(|t| t.rsplit('/').next())
                        .unwrap_or("material");
                    let m = b.material(
                        name,
                        [color[0], color[1], color[2], 1.0],
                        uri.as_deref(),
                        mask && uri.is_some(),
                        extras.as_deref(),
                    );
                    mats.insert(key, m);
                    m
                }
            };
            p.material = Some(mat);
            prims.push(p);
        }
    }
    if prims.is_empty() {
        return Err("no skinned geometry".into());
    }
    let mesh_index = b.mesh(id, &prims);
    let skinned = b.skinned_node(id, mesh_index, skin);

    // Animation clips from the skeleton file.
    let by_name: HashMap<&str, usize> = bones
        .iter()
        .enumerate()
        .map(|(i, x)| (x.name.as_str(), i))
        .collect();
    for clip in &skel.clips {
        let mut channels = Vec::new();
        for (bone_name, keys) in &clip.channels {
            st.channels += 1;
            let Some(&bi) = by_name.get(bone_name.as_str()) else {
                st.unmatched_channels += 1;
                continue;
            };
            if keys.is_empty() {
                continue;
            }
            // Resample with the game's own sampler at the source key times
            // (>= 0, plus t = 0 when keys precede it) so negative or repeated
            // key times keep game parity; glTF needs strictly increasing,
            // non-negative inputs. Repeated times take the last key (the
            // game's binary search interpolates from it once time passes).
            let mut times: Vec<f32> = keys
                .iter()
                .map(|k| k.time)
                .filter(|t| *t >= 0.0 && t.is_finite())
                .collect();
            if keys.iter().any(|k| k.time < 0.0) {
                times.push(0.0);
            }
            times.sort_by(|a, b| a.partial_cmp(b).unwrap());
            times.dedup();
            let resampled = keys.len() != times.len()
                || keys.windows(2).any(|w| w[1].time <= w[0].time)
                || keys[0].time < 0.0;
            if resampled {
                st.dropped_keyframes += 1;
            }
            let kept: Vec<crate::assets::Keyframe> = times
                .iter()
                .map(|&t| {
                    let last = keys.iter().rposition(|k| k.time == t);
                    let (translation, orientation, scale) = match last {
                        Some(i) => (keys[i].translation, keys[i].orientation, keys[i].scale),
                        None => crate::skinned::sample_channel(keys, t),
                    };
                    crate::assets::Keyframe {
                        time: t,
                        translation,
                        orientation,
                        scale,
                    }
                })
                .collect();
            if kept.is_empty() {
                continue;
            }
            // Densify: the game blends rotations with normalized lerp, glTF
            // with slerp. Where consecutive keys differ by more than ~4 degrees,
            // insert keys sampled with the game's interpolation so both
            // agree closely everywhere.
            let kept = densify(&kept, &mut st);
            st.anisotropic += kept.iter().filter(|k| anisotropic(k.scale)).count();
            let input = b.scalars(&kept.iter().map(|k| k.time).collect::<Vec<_>>(), true);
            let tr = b.vec3(
                &kept
                    .iter()
                    .map(|k| finite3(k.translation, &mut st.geometry))
                    .collect::<Vec<_>>(),
                false,
            );
            let ro = b.vec4(
                &kept
                    .iter()
                    .map(|k| sanitize_q(k.orientation, &mut st.geometry))
                    .collect::<Vec<_>>(),
            );
            let sc = b.vec3(
                &kept
                    .iter()
                    .map(|k| finite3(k.scale, &mut st.geometry))
                    .collect::<Vec<_>>(),
                false,
            );
            channels.push((outer[bi], "translation", input, tr));
            channels.push((joints[bi], "rotation", input, ro));
            channels.push((outer[bi], "scale", input, sc));
        }
        if !channels.is_empty() {
            let c: Vec<(usize, &str, usize, usize)> = channels
                .iter()
                .map(|(n, p, i, o)| (*n, *p, *i, *o))
                .collect();
            b.animation(&clip.name, &c);
            st.clips += 1;
        }
    }
    if reference {
        st.reference_json = Some(reference_samples(skel, &prims[0], s));
    }
    // Skinned mesh node and armature are both scene roots (glTF recommends
    // the skinned mesh node not be parented).
    Ok((b.finish(&[armature, skinned]), st))
}

/// Insert intermediate keys (game interpolation) where the rotation between
/// consecutive keys exceeds `MAX_STEP` radians.
fn densify(keys: &[crate::assets::Keyframe], st: &mut CharStats) -> Vec<crate::assets::Keyframe> {
    const MAX_STEP: f32 = 0.07; // ~4 degrees
    let mut out = Vec::with_capacity(keys.len());
    for w in keys.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        out.push(a.clone());
        let dot = (0..4)
            .map(|k| a.orientation[k] * b.orientation[k])
            .sum::<f32>()
            .abs()
            .min(1.0);
        let angle = 2.0 * dot.acos();
        let n = (angle / MAX_STEP).ceil() as usize;
        for i in 1..n.max(1) {
            let u = i as f32 / n as f32;
            let lerp3 = |x: [f32; 3], y: [f32; 3]| [0, 1, 2].map(|k| x[k] + (y[k] - x[k]) * u);
            out.push(crate::assets::Keyframe {
                time: a.time + (b.time - a.time) * u,
                translation: lerp3(a.translation, b.translation),
                orientation: crate::skinned::xna_quat_lerp(a.orientation, b.orientation, u),
                scale: lerp3(a.scale, b.scale),
            });
            st.densified_keys += 1;
        }
    }
    out.push(keys[keys.len() - 1].clone());
    out
}

fn gltf_arr(v: &[f32]) -> String {
    format!(
        "[{}]",
        v.iter()
            .map(|x| format!("{x}"))
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// Game-math skinned positions (first <= 300 vertices of primitive 0) at
/// interior times of every exported clip, in glTF animation order. Times stay
/// inside each clip's non-negative keyframe range (non-looping semantics).
fn reference_samples(skel: &Resolved, p: &gltf::Primitive, scale: f32) -> String {
    use crate::skinned::{skin_matrices, skin_vertex, IDENTITY};
    let (Some(j), Some(w)) = (&p.joints, &p.weights) else {
        return "{}".into();
    };
    let mut parent = IDENTITY;
    parent[0] = scale;
    parent[5] = scale;
    parent[10] = scale;
    let n = p.positions.len().min(300);
    let mut out = String::from("{\"vertices\":");
    let _ = write!(out, "{n},\"samples\":[");
    let mut first = true;
    let mut anim = 0;
    for clip in &skel.clips {
        // Mirrors the exporter: a clip becomes a glTF animation only if some
        // channel matches a bone and keeps keyframes.
        let names: std::collections::HashSet<&str> = skel
            .skeleton
            .bones
            .iter()
            .map(|b| b.name.as_str())
            .collect();
        if !clip
            .channels
            .iter()
            .any(|(nm, k)| names.contains(nm.as_str()) && !k.is_empty())
        {
            continue;
        }
        let (lo, hi) = clip
            .channels
            .iter()
            .flat_map(|(_, k)| k.iter().map(|x| x.time))
            .fold((f32::MAX, f32::MIN), |(a, b), t| (a.min(t), b.max(t)));
        // Playback time starts at 0; keys before it only shape t >= 0.
        let lo = lo.max(0.0);
        for frac in [0.13f32, 0.37, 0.61, 0.89] {
            let time = lo + (hi - lo) * frac;
            let skins = skin_matrices(&skel.skeleton, clip, time, &parent);
            if !first {
                out.push(',');
            }
            first = false;
            let _ = write!(
                out,
                "{{\"animation\":{anim},\"time\":{time},\"positions\":["
            );
            for v in 0..n {
                let q = skin_vertex(p.positions[v], j[v], w[v], &skins);
                let _ = write!(
                    out,
                    "{}[{},{},{}]",
                    if v > 0 { "," } else { "" },
                    q[0],
                    q[1],
                    q[2]
                );
            }
            out.push_str("]}");
        }
        anim += 1;
    }
    out.push_str("]}");
    out
}

/// A static XNA Model (e.g. Models/Missiles) as glTF: one node per mesh at
/// its parent bone's absolute transform (Model.CopyAbsoluteBoneTransformsTo:
/// local · parent, row vectors), materials from the file's shared effects.
pub fn export_static_model(
    model: &crate::xna::Model,
    shared: &[Option<XnaObject>],
    asset: &str,
    texture_uri: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<u8>, String> {
    let absolute = |mut index: usize| {
        let mut m = model.bones[index].transform;
        while let Some(parent) = model.bones[index].parent {
            m = crate::skinned::mul(&m, &model.bones[parent].transform);
            index = parent;
        }
        m
    };
    let mut stats = Default::default();
    let mut b = Builder::default();
    let mut mats: HashMap<String, usize> = HashMap::new();
    let mut nodes = Vec::new();
    for (mi, m) in model.meshes.iter().enumerate() {
        let (Some(vb), Some((sixteen, ib))) = (&m.vertex_buffer, &m.index_buffer) else {
            continue;
        };
        let mut prims = Vec::new();
        for part in &m.parts {
            let Some(decl) = model
                .vertex_declarations
                .get(part.vertex_declaration as usize)
            else {
                continue;
            };
            let Some(mut p) = primitive(
                decl,
                vb,
                decl_stride(decl),
                part.stream_offset.max(0) as usize,
                part.base_vertex.max(0) as usize,
                (*sixteen, ib),
                part.start_index.max(0) as usize,
                part.primitive_count.max(0) as usize,
                &mut stats,
            ) else {
                continue;
            };
            let effect = (part.effect as usize)
                .checked_sub(1)
                .and_then(|i| shared.get(i))
                .and_then(|o| o.as_ref());
            // AdditiveEffect parts (e.g. barrier runes) keep their additive
            // parameters, as the level exporter writes them.
            let additive = match effect {
                Some(XnaObject::AdditiveEffect(e)) => Some(e),
                _ => None,
            };
            let (color, tex) = match additive {
                Some(e) => (e.color_tint, e.texture.clone()),
                None => skinned_material(effect),
            };
            let extras = additive.map(|e| {
                format!(
                    "{{\"magicka_additive\":{{\"color\":[{},{},{}],\"texture_enabled\":{},\"vertex_color\":{}}}}}",
                    e.color_tint[0], e.color_tint[1], e.color_tint[2], e.texture_enabled, e.vertex_color_enabled
                )
            }).or_else(|| match effect {
                Some(XnaObject::RenderDeferredEffect(e)) => Some(crate::scene::deferred_material_extras(e, asset, texture_uri)),
                _ => None,
            });
            let tex = tex.map(|r| mesh::resolve_reference(asset, &r));
            let mask = additive.is_none() && cutout(effect);
            let key = format!("{color:?}|{tex:?}|{extras:?}|{mask}");
            let mat = match mats.get(&key) {
                Some(m) => *m,
                None => {
                    let uri = tex.as_ref().and_then(|t| texture_uri(t));
                    let m = b.material(
                        if additive.is_some() {
                            "additive"
                        } else {
                            "material"
                        },
                        [color[0], color[1], color[2], 1.0],
                        uri.as_deref(),
                        mask && uri.is_some(),
                        extras.as_deref(),
                    );
                    mats.insert(key, m);
                    m
                }
            };
            p.material = Some(mat);
            prims.push(p);
        }
        if prims.is_empty() {
            continue;
        }
        let name = m.name.clone().unwrap_or_else(|| format!("mesh{mi}"));
        let mesh_index = b.mesh(&name, &prims);
        let transform = m.parent_bone.map_or(crate::skinned::IDENTITY, absolute);
        nodes.push(b.node(&name, &gltf::matrix_json(&transform), Some(mesh_index), &[]));
    }
    if nodes.is_empty() {
        return Err("no geometry".into());
    }
    Ok(b.finish(&nodes))
}

#[cfg(test)]
mod overlay_tests {
    use super::*;
    #[test]
    fn fabric_overlay_is_distinct_from_details_and_part_of_material_identity() {
        let material = |name: &str, soft| {
            XnaObject::Asset(Box::new(Asset::SkinnedModelBasicEffect(
                crate::assets::SkinnedModelBasicEffect {
                    technique: 0,
                    emissive_amount: 0.0,
                    diffuse_color: [0.7, 0.8, 0.9],
                    specular_amount: 1.0,
                    specular_power: 20.0,
                    diffuse1_alpha: 0.75,
                    use_soft_light_blend: soft,
                    map_enabled: [true, true, false, false, false, false],
                    maps: vec![
                        None,
                        Some(XnaObject::Asset(Box::new(Asset::External(Some(
                            name.into(),
                        ))))),
                    ],
                },
            )))
        };
        let resolve = |asset: &str| Some(format!("{asset}.png"));
        let a = skinned_overlay_extras(
            Some(&material("fabric_a", true)),
            "Models/Robe/mesh",
            &resolve,
        )
        .unwrap();
        let b = skinned_overlay_extras(
            Some(&material("fabric_b", false)),
            "Models/Robe/mesh",
            &resolve,
        )
        .unwrap();
        assert!(a.contains("Models/Robe/fabric_a.png"));
        assert!(a.contains("\"overlay_alpha\":0.75"));
        assert!(a.contains("\"soft_light\":true"));
        assert_ne!(
            a, b,
            "parts with identical details must not merge differing overlays"
        );
        let missing = skinned_overlay_extras(
            Some(&material("missing", true)),
            "Models/Robe/mesh",
            &|_| None,
        )
        .unwrap();
        assert!(missing.contains("\"overlay\":null"));
        assert!(
            missing.contains("\"overlay_enabled\":true"),
            "retain the original flag for missing-asset detection"
        );
    }
}
