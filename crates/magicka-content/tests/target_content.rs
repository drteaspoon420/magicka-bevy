//! Exact-consumption validation against the user's retail content in
//! `target/` (skipped when absent; override with `MAGICKA_CONTENT`).

use magicka_content::{parse_primary, Content};
use magicka_xnb::xnb;
use std::path::{Path, PathBuf};

fn content_dir() -> Option<PathBuf> {
    let p = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    p.is_dir().then_some(p)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xnb")) {
            out.push(p);
        }
    }
}

#[test]
fn retail_liquid_collision_geometry_decodes() {
    use magicka_content::level::{AnimatedLevelPart, Liquid};
    fn check(liquid: &Liquid) {
        let geometry = liquid.collision_geometry().unwrap();
        assert_eq!(geometry.vertices.len(), liquid.num_vertices as usize);
        assert_eq!(geometry.triangles.len(), liquid.primitive_count as usize);
        let (sixteen, bytes) = liquid.index_buffer.as_ref().unwrap();
        let width = if *sixteen { 2 } else { 4 };
        for (triangle, raw) in geometry.triangles.iter().zip(bytes.chunks_exact(width * 3)) {
            let index = |i: usize| {
                if *sixteen {
                    i16::from_le_bytes(raw[i * width..(i + 1) * width].try_into().unwrap()) as i32
                } else {
                    i32::from_le_bytes(raw[i * width..(i + 1) * width].try_into().unwrap())
                }
            };
            assert_eq!(*triangle, [index(0), index(2), index(1)]);
        }
    }
    fn part(part: &AnimatedLevelPart, nested: &mut usize) {
        for liquid in &part.liquids {
            check(liquid);
            *nested += 1;
        }
        for child in &part.children {
            part_visit(child, nested);
        }
    }
    fn part_visit(value: &AnimatedLevelPart, nested: &mut usize) {
        part(value, nested);
    }
    let Some(dir) = content_dir() else { return };
    let mut files = Vec::new();
    walk(&dir.join("Levels"), &mut files);
    let (mut decoded, mut nested) = (0, 0);
    for file in files {
        let parsed = xnb::parse(&std::fs::read(&file).unwrap()).unwrap();
        let Some(Ok(primary)) = parse_primary(&parsed) else {
            continue;
        };
        let Content::Level(level) = primary.content else {
            continue;
        };
        for liquid in &level.liquids {
            check(liquid);
            decoded += 1;
        }
        for animated in &level.animated_parts {
            part_visit(animated, &mut nested);
        }
    }
    eprintln!("decoded {decoded} top-level and {nested} animated-part liquids");
    assert!(decoded > 0);
}

/// Every XNB file in the game (Magicka types and all standard readers) (plus its
/// shared resources) must parse and be consumed to the
/// last byte. Any mis-transcribed field width or order desynchronizes the
/// stream and fails this.
#[test]
fn magicka_content_parses_exactly() {
    let Some(dir) = content_dir() else {
        eprintln!("SKIP: target/Magicka/Content not present");
        return;
    };
    let mut files = Vec::new();
    walk(&dir, &mut files);
    let (mut chars, mut items, mut physics, mut levels, mut other) = (0, 0, 0, 0, 0);
    let mut failures = Vec::new();
    for f in &files {
        let x = xnb::parse(&std::fs::read(f).unwrap()).unwrap();
        match parse_primary(&x) {
            None => failures.push(format!("{}: null primary object", f.display())),
            Some(Ok(p)) => {
                let trailing = p.trailing;
                match p.content {
                    Content::Character(_) => chars += 1,
                    Content::Item(_) => items += 1,
                    Content::PhysicsEntity(_) => physics += 1,
                    Content::Level(_) => levels += 1,
                    Content::Xna(_) => other += 1,
                }
                if trailing != 0 {
                    failures.push(format!("{}: {trailing} trailing bytes", f.display()));
                }
            }
            Some(Err(e)) => failures.push(format!("{}: {e}", f.display())),
        }
    }
    eprintln!("characters: {chars}, items: {items}, physics entities: {physics}, levels: {levels}, standard: {other}");
    assert!(
        failures.is_empty(),
        "{} failures, first: {:#?}",
        failures.len(),
        &failures[..failures.len().min(15)]
    );
    assert_eq!((chars, items, physics, levels), (338, 454, 69, 132));
    assert_eq!(files.len(), chars + items + physics + levels + other);
}

/// Every texture mip (2D, cube faces, volume slices) must have exactly the
/// byte size implied by its XNA 3.1 SurfaceFormat and dimensions. This proves
/// the format interpretation used by the decoder.
#[test]
fn texture_mip_sizes_match_formats() {
    use magicka_content::assets::Asset;
    use magicka_content::texture::mip_size;
    use magicka_content::xna::XnaObject;
    let Some(dir) = content_dir() else {
        eprintln!("SKIP: target/Magicka/Content not present");
        return;
    };
    let mut files = Vec::new();
    walk(&dir, &mut files);
    let mut checked = 0;
    let mut failures = Vec::new();
    let mut check = |f: &Path, fmt: i32, w: i32, h: i32, d: i32, level: usize, len: usize| {
        let (w, h, d) = (
            (w as usize >> level).max(1),
            (h as usize >> level).max(1),
            (d as usize >> level).max(1),
        );
        match mip_size(fmt, w, h) {
            Some(n) if n * d == len => checked += 1,
            Some(n) => failures.push(format!(
                "{}: fmt {fmt} {w}x{h}x{d} mip {level}: {len} != {}",
                f.display(),
                n * d
            )),
            None => failures.push(format!("{}: unsupported format {fmt}", f.display())),
        }
    };
    for f in &files {
        let x = xnb::parse(&std::fs::read(f).unwrap()).unwrap();
        let Some(Ok(p)) = parse_primary(&x) else {
            continue;
        };
        let Content::Xna(o) = p.content else { continue };
        let XnaObject::Asset(a) = *o else { continue };
        match *a {
            Asset::Texture2D(t) => {
                for (i, m) in t.mips.iter().enumerate() {
                    check(f, t.format, t.width, t.height, 1, i, m.len());
                }
            }
            Asset::TextureCube {
                format,
                size,
                faces,
            } => {
                for face in &faces {
                    for (i, m) in face.iter().enumerate() {
                        check(f, format, size, size, 1, i, m.len());
                    }
                }
            }
            Asset::Texture3D {
                format,
                width,
                height,
                depth,
                mips,
            } => {
                for (i, m) in mips.iter().enumerate() {
                    check(f, format, width, height, depth, i, m.len());
                }
            }
            _ => {}
        }
    }
    eprintln!("texture mips checked: {checked}");
    assert!(
        failures.is_empty(),
        "{} failures, first: {:#?}",
        failures.len(),
        &failures[..failures.len().min(10)]
    );
    assert!(checked > 0);
}

/// Structural invariants of all level geometry: positions decode via the
/// vertex declaration, decoded vertex counts equal the declared counts, the
/// BiTree leaf ranges exactly tile each index buffer, every index is in
/// range, and nav-mesh triangles reference valid vertices.
#[test]
fn level_geometry_is_consistent() {
    use magicka_content::mesh;
    use magicka_content::xna::BiTreeNode;
    fn leaves(n: &BiTreeNode, out: &mut Vec<(i32, i32)>) {
        if n.child_a.is_none() && n.child_b.is_none() {
            out.push((n.start_index, n.primitive_count));
        }
        for c in [&n.child_a, &n.child_b].into_iter().flatten() {
            leaves(c, out);
        }
    }
    let Some(dir) = content_dir() else {
        eprintln!("SKIP: target/Magicka/Content not present");
        return;
    };
    let mut files = Vec::new();
    walk(&dir, &mut files);
    let (mut levels, mut roots, mut tris) = (0, 0, 0usize);
    let mut failures = Vec::new();
    for f in &files {
        let x = xnb::parse(&std::fs::read(f).unwrap()).unwrap();
        let Some(Ok(p)) = parse_primary(&x) else {
            continue;
        };
        let Content::Level(l) = p.content else {
            continue;
        };
        levels += 1;
        let fail = |m: String| format!("{}: {m}", f.display());
        for (ri, root) in l.model.iter().flat_map(|m| m.roots.iter()).enumerate() {
            roots += 1;
            let (Some(decl), Some(vb), Some((sixteen, ib))) = (
                &root.vertex_declaration,
                &root.vertex_buffer,
                &root.index_buffer,
            ) else {
                failures.push(fail(format!("root {ri}: missing buffers")));
                continue;
            };
            let Some(pos) = mesh::positions(decl, vb, root.vertex_stride as usize) else {
                failures.push(fail(format!("root {ri}: no position element")));
                continue;
            };
            if pos.len() != root.num_vertices as usize
                || vb.len() % root.vertex_stride as usize != 0
            {
                failures.push(fail(format!(
                    "root {ri}: {} vertices vs declared {}",
                    pos.len(),
                    root.num_vertices
                )));
            }
            let idx = mesh::indices(*sixteen, ib);
            let mut ranges = Vec::new();
            leaves(&root.node, &mut ranges);
            ranges.sort();
            let mut next = 0i32;
            for (start, prims) in &ranges {
                if *start != next {
                    failures.push(fail(format!(
                        "root {ri}: leaf range gap/overlap at {start} (expected {next})"
                    )));
                }
                next = start + prims * 3;
            }
            if next as usize != idx.len() {
                failures.push(fail(format!(
                    "root {ri}: leaves cover {next} of {} indices",
                    idx.len()
                )));
            }
            if idx.iter().any(|&i| i as usize >= pos.len()) {
                failures.push(fail(format!("root {ri}: index out of range")));
            }
            tris += idx.len() / 3;
        }
        let nv = l.nav_mesh.vertices.len();
        if l.nav_mesh
            .triangles
            .iter()
            .any(|t| t.vertices.iter().any(|&v| v as usize >= nv))
        {
            failures.push(fail("nav mesh vertex index out of range".into()));
        }
    }
    eprintln!("levels: {levels}, bitree roots: {roots}, triangles: {tris}");
    assert!(
        failures.is_empty(),
        "{} failures, first: {:#?}",
        failures.len(),
        &failures[..failures.len().min(10)]
    );
    assert_eq!(levels, 132);
}

/// Export every level scene to glTF in memory and check the automated
/// invariants: well-formed GLB container, winding agrees with stored vertex
/// normals (CCW after the XNA->glTF flip) for >= 97% of faces, every
/// physics-prop template resolves, and only the known, counted source-data
/// defects occur (non-finite pose components are replaced, never emitted).
#[test]
fn level_scenes_export_to_valid_glb() {
    use magicka_content::scene::{export_level, Context};
    let Some(dir) = content_dir() else {
        eprintln!("SKIP: target/Magicka/Content not present");
        return;
    };
    let mut files = Vec::new();
    walk(&dir, &mut files);
    let index: std::collections::HashMap<String, PathBuf> = files
        .iter()
        .map(|p| {
            (
                p.strip_prefix(&dir)
                    .unwrap()
                    .with_extension("")
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_lowercase(),
                p.clone(),
            )
        })
        .collect();
    let prop = |name: &str| {
        let path = index.get(&format!("data/physicsentities/{}", name.to_lowercase()))?;
        let x = xnb::parse(&std::fs::read(path).ok()?).ok()?;
        let p = parse_primary(&x)?.ok()?;
        let Content::PhysicsEntity(t) = p.content else {
            return None;
        };
        Some((format!("Data/PhysicsEntities/{name}"), t.model?, p.shared))
    };
    let no_textures = |_: &str| None;
    let (mut scenes, mut agree, mut disagree) = (0, 0usize, 0usize);
    let mut failures = Vec::new();
    for f in &files {
        let x = xnb::parse(&std::fs::read(f).unwrap()).unwrap();
        let Some(Ok(p)) = parse_primary(&x) else {
            continue;
        };
        let Content::Level(l) = &p.content else {
            continue;
        };
        scenes += 1;
        let asset = f
            .strip_prefix(&dir)
            .unwrap()
            .with_extension("")
            .to_string_lossy()
            .replace('\\', "/");
        let ctx = Context {
            asset: &asset,
            texture_uri: &no_textures,
            prop: &prop,
        };
        let (glb, st) = export_level(l, &p.shared, &ctx);
        if &glb[..4] != b"glTF"
            || u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize != glb.len()
        {
            failures.push(format!("{asset}: malformed GLB header"));
        }
        if !st.missing_props.is_empty() {
            failures.push(format!("{asset}: missing props {:?}", st.missing_props));
        }
        agree += st.normal_agree;
        disagree += st.normal_disagree;
    }
    let ratio = agree as f64 / (agree + disagree).max(1) as f64;
    eprintln!(
        "scenes: {scenes}, winding agreement {:.4} ({agree}/{})",
        ratio,
        agree + disagree
    );
    assert!(
        failures.is_empty(),
        "{:#?}",
        &failures[..failures.len().min(10)]
    );
    assert_eq!(scenes, 132);
    assert!(ratio > 0.97, "winding agreement {ratio}");
}

/// Every skinned model's shared bone/clip references resolve, bone indices
/// equal skeleton positions, parents precede children, and for every bone
/// not under a non-uniform scale the stored inverse-bind matrix is the inverse
/// of the bind pose composed through the hierarchy (`Pose.GetMatrix`,
/// `local · abs[parent]`). This proves the skeleton semantics used by the
/// glTF export and confines the known source-asset mismatch.
#[test]
fn skinned_skeletons_are_consistent() {
    use magicka_content::assets::Asset;
    use magicka_content::skinned::resolve;
    use magicka_content::xna::XnaObject;
    let Some(dir) = content_dir() else {
        eprintln!("SKIP: target/Magicka/Content not present");
        return;
    };
    let mut files = Vec::new();
    walk(&dir, &mut files);
    let (mut models, mut with_mesh, mut with_clips, mut clips, mut worst, mut inconsistent_files) =
        (0, 0, 0, 0, 0f32, 0);
    let mut failures = Vec::new();
    for f in &files {
        let x = xnb::parse(&std::fs::read(f).unwrap()).unwrap();
        let Some(Ok(p)) = parse_primary(&x) else {
            continue;
        };
        let Content::Xna(o) = p.content else { continue };
        let XnaObject::Asset(a) = *o else { continue };
        let Asset::SkinnedModel(sm) = *a else {
            continue;
        };
        models += 1;
        match resolve(&sm, p.shared) {
            Ok(r) => {
                if r.model.as_ref().is_some_and(|m| !m.meshes.is_empty()) {
                    with_mesh += 1;
                }
                if !r.clips.is_empty() {
                    with_clips += 1;
                    clips += r.clips.len();
                }
                // Every bone whose stored inverse bind does not invert its
                // composed bind pose must have non-uniform scale on itself or
                // an ancestor (a source-asset property: the mesh was bound in
                // a pose that differs from the stored rest pose only there).
                let errs = r.skeleton.bind_errors();
                let mut file_bad = false;
                for (i, e) in errs.iter().enumerate() {
                    if *e > 1e-2 {
                        file_bad = true;
                        if !r.skeleton.under_nonuniform_scale(i) {
                            failures.push(format!(
                                "{}: bone {} bind error {e} without non-uniform scale",
                                f.display(),
                                r.skeleton.bones[i].name
                            ));
                        }
                    } else {
                        worst = worst.max(*e);
                    }
                }
                if file_bad {
                    inconsistent_files += 1;
                }
            }
            Err(e) => failures.push(format!("{}: {e}", f.display())),
        }
    }
    eprintln!("skinned models: {models} ({with_mesh} with meshes, {with_clips} with clips, {clips} clips), worst consistent-bone bind error {worst:e}, files with non-uniform-scale bind mismatch: {inconsistent_files}");
    assert!(
        failures.is_empty(),
        "{} failures: {:#?}",
        failures.len(),
        &failures[..failures.len().min(10)]
    );
    assert_eq!(models, 327);
}

/// Export every character (CharacterTemplate) as a skinned glTF in memory and
/// check: skeleton/clip resolution succeeds, every clip channel names a
/// skeleton bone, all blend indices address the animation skeleton, skin
/// weights are non-degenerate, and CCW winding agrees with stored normals.
/// Game-math parity of the animated vertices is checked separately by
/// `tools/check_skin.py` against `character-gltf --reference` samples.
#[test]
fn characters_export_to_skinned_glb() {
    use magicka_content::assets::Asset;
    use magicka_content::character_scene::export_character;
    use magicka_content::skinned::{resolve, Resolved};
    use magicka_content::xna::XnaObject;
    let Some(dir) = content_dir() else {
        eprintln!("SKIP: target/Magicka/Content not present");
        return;
    };
    let mut files = Vec::new();
    walk(&dir, &mut files);
    let index: std::collections::HashMap<String, PathBuf> = files
        .iter()
        .map(|p| {
            (
                p.strip_prefix(&dir)
                    .unwrap()
                    .with_extension("")
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_lowercase(),
                p.clone(),
            )
        })
        .collect();
    let cache =
        std::cell::RefCell::new(std::collections::HashMap::<String, Option<Resolved>>::new());
    let load = |asset: &str| -> Option<Resolved> {
        let key = asset.to_lowercase();
        if let Some(r) = cache.borrow().get(&key) {
            return r.clone();
        }
        let r = (|| {
            let x = xnb::parse(&std::fs::read(index.get(&key)?).ok()?).ok()?;
            let p = parse_primary(&x)?.ok()?;
            let Content::Xna(o) = p.content else {
                return None;
            };
            let XnaObject::Asset(a) = *o else { return None };
            let Asset::SkinnedModel(sm) = *a else {
                return None;
            };
            resolve(&sm, p.shared).ok()
        })();
        cache.borrow_mut().insert(key, r.clone());
        r
    };
    let no_textures = |_: &str| None;
    let (mut n, mut agree, mut disagree) = (0, 0usize, 0usize);
    let mut failures = Vec::new();
    for f in &files {
        let x = xnb::parse(&std::fs::read(f).unwrap()).unwrap();
        let Some(Ok(p)) = parse_primary(&x) else {
            continue;
        };
        let Content::Character(t) = &p.content else {
            continue;
        };
        let asset = f
            .strip_prefix(&dir)
            .unwrap()
            .with_extension("")
            .to_string_lossy()
            .replace('\\', "/");
        match export_character(t, &asset, &load, &no_textures, false) {
            Ok((glb, st)) => {
                n += 1;
                if &glb[..4] != b"glTF" {
                    failures.push(format!("{asset}: bad GLB"));
                }
                if st.unmatched_channels > 0
                    || st.bad_joint_indices > 0
                    || st.geometry.zero_weights > 0
                {
                    failures.push(format!(
                        "{asset}: unmatched {} bad joints {} zero weights {}",
                        st.unmatched_channels, st.bad_joint_indices, st.geometry.zero_weights
                    ));
                }
                agree += st.geometry.normal_agree;
                disagree += st.geometry.normal_disagree;
            }
            Err(e) => failures.push(format!("{asset}: {e}")),
        }
    }
    let ratio = agree as f64 / (agree + disagree).max(1) as f64;
    eprintln!("characters exported: {n}, winding agreement {ratio:.4}");
    assert!(
        failures.is_empty(),
        "{:#?}",
        &failures[..failures.len().min(10)]
    );
    assert_eq!(n, 338);
    assert!(ratio > 0.97);
}

/// User report: the wc_s3 opening's repaired planks cast no shadow. Their
/// AnimatedLevelPart MeshSettings are (visible false, castShadows true), and
/// wc_s3 has five such static BiTree roots; both native render datas keep
/// only the shadow technique. The exporter must emit them with the
/// `magicka_shadow_only` material (static ones in `static_shadow`).
#[test]
fn castle_shadow_only_meshes_export_as_shadow_casters() {
    use magicka_content::scene::{export_level, Context};
    let Some(dir) = content_dir() else {
        eprintln!("SKIP: target/Magicka/Content not present");
        return;
    };
    let path = dir.join("Levels/WizardCastle/wc_s3.xnb");
    let x = xnb::parse(&std::fs::read(path).unwrap()).unwrap();
    let p = parse_primary(&x).unwrap().unwrap();
    let Content::Level(l) = &p.content else {
        panic!("wc_s3 is a level");
    };
    let ctx = Context {
        asset: "Levels/WizardCastle/wc_s3",
        texture_uri: &|_: &str| None,
        prop: &|_: &str| None,
    };
    let (glb, _) = export_level(l, &p.shared, &ctx);
    let n = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let json = std::str::from_utf8(&glb[20..20 + n]).unwrap();
    assert!(json.contains(r#""extras":{"magicka_shadow_only":true}"#));
    assert!(json.contains(r#"{"name":"static_shadow","mesh":"#));
    for i in 1..=7 {
        let key = format!(r#"{{"name":"plank{i}""#);
        let start = json.find(&key).unwrap_or_else(|| panic!("plank{i} node"));
        let rest = &json[start + key.len()..];
        let node = &rest[..rest.find(r#"{"name":"#).unwrap_or(rest.len())];
        assert!(
            node.contains(r#""mesh":"#),
            "plank{i} must keep its shadow mesh"
        );
    }
}

/// User report: the Staff of Life's leaves were opaque quads. Its
/// RenderDeferredEffect has a layer-0 diffuse texture with alpha enabled and
/// no alpha mask, so the native shader kills texels below 0.5 alpha; the
/// static model export must emit glTF alphaMode MASK.
#[test]
fn staff_of_life_leaves_export_alpha_cutout() {
    use magicka_content::character_scene::export_static_model;
    use magicka_content::xna::XnaObject;
    let Some(dir) = content_dir() else {
        eprintln!("SKIP: target/Magicka/Content not present");
        return;
    };
    let path = dir.join("Models/Items_Wizard/staff_life_0.xnb");
    let x = xnb::parse(&std::fs::read(path).unwrap()).unwrap();
    let p = parse_primary(&x).unwrap().unwrap();
    let Content::Xna(object) = p.content else {
        panic!("staff_life_0 is a model");
    };
    let XnaObject::Model(model) = *object else {
        panic!("staff_life_0 is a model");
    };
    let glb = export_static_model(
        &model,
        &p.shared,
        "Models/Items_Wizard/staff_life_0",
        &|_: &str| Some("staff.png".into()),
    )
    .unwrap();
    let n = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let json = std::str::from_utf8(&glb[20..20 + n]).unwrap();
    assert!(json.contains(r#""alphaMode":"MASK""#), "{json:.400}");
}

#[test]
fn retail_bomb_export_preserves_its_authored_deferred_surface() {
    use magicka_content::{character_scene::export_static_model, xna::XnaObject};
    let Some(root) = content_dir() else {
        return;
    };
    let bytes = std::fs::read(root.join("Models/Items_NPC/goblin_bomb_0.xnb")).unwrap();
    let parsed = parse_primary(&xnb::parse(&bytes).unwrap())
        .unwrap()
        .unwrap();
    let Content::Xna(object) = parsed.content else {
        panic!("bomb model");
    };
    let XnaObject::Model(model) = *object else {
        panic!("static bomb");
    };
    let Some(XnaObject::RenderDeferredEffect(effect)) = &parsed.shared[0] else {
        panic!("bomb deferred material");
    };
    assert!(effect.layer0.diffuse_texture.is_none());
    assert!(effect.layer0.spec_amount > 0. && effect.layer0.spec_power > 0.);
    let glb = export_static_model(
        &model,
        &parsed.shared,
        "Models/Items_NPC/goblin_bomb_0",
        &|_| None,
    )
    .unwrap();
    let n = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let json = std::str::from_utf8(&glb[20..20 + n]).unwrap();
    assert!(
        json.contains("magicka_deferred"),
        "native surface lost from static model export"
    );
    assert!(json.contains(&format!(
        "\"spec0\":[{},{},{}]",
        effect.layer0.spec_amount, effect.layer0.spec_power, effect.layer0.emissive_amount
    )));
    eprintln!(
        "bomb surface: diffuse={:?}, spec_amount={}, spec_power={}, no authored diffuse texture",
        effect.layer0.diffuse_color, effect.layer0.spec_amount, effect.layer0.spec_power
    );
}

/// CharacterTemplate reads equipment into mEquipment[slot], so a repeated
/// slot keeps the last entry: the Firedrake breathes firedrake_puke, not the
/// spider fangs listed first in the same slot (fire lizard attack report).
#[test]
fn repeated_equipment_slot_keeps_the_last_attachment() {
    let Some(dir) = content_dir() else { return };
    let path = dir.join("Data/Characters/Firedrake.xnb");
    let parsed = xnb::parse(&std::fs::read(path).unwrap()).unwrap();
    let Some(Ok(primary)) = parse_primary(&parsed) else {
        panic!("Firedrake did not decode");
    };
    let Content::Character(template) = primary.content else {
        panic!("not a character");
    };
    let items: Vec<_> = template
        .equipment
        .iter()
        .map(|(slot, a)| (*slot, a.item.clone().unwrap_or_default()))
        .collect();
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0].0, 0);
    assert!(items[0].1.ends_with("firedrake_puke_0"), "{items:?}");
}
