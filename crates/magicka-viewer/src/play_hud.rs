//! Original HUD atlas, healthbar shader and settled queue in the play overlay.
use bevy::{
    asset::{embedded_asset, RenderAssetUsages},
    image::{ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat},
    shader::ShaderRef,
    sprite_render::{AlphaMode2d, Material2d, Material2dPlugin},
};
use magicka_content::{
    assets::{Asset, Texture2D},
    texture::decode_rgba,
};
use magicka_runtime::{
    hud::HudQuad,
    keyboard_hud::{self, KeyboardHud},
    text_geometry::{TextAlign, TextGeometry, TextStyle},
};

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct HudMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
    #[uniform(2)]
    pub saturation: Vec4,
}
impl Material2d for HudMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/play_hud.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}
#[derive(Component)]
pub struct HudGeometry {
    pub mesh: Handle<Mesh>,
    pub atlas_size: [f32; 2],
}
/// A keyboard HUD key letter (drawn with its icon).
#[derive(Component)]
pub struct KeyboardLabel;

#[derive(Component)]
pub struct KeyboardGeometry {
    pub index: usize,
    mesh: Handle<Mesh>,
    atlas_size: [f32; 2],
    key_mesh: Handle<Mesh>,
    key_material: Handle<HudMaterial>,
    glyphs: Vec<HudQuad>,
    font_size: [f32; 2],
}
/// IconRenderer's TomeMagick name text (Maiandra14, centred), one glyph
/// batch per MagickType value, relative to the text anchor.
#[derive(Component)]
pub struct TomeTextGeometry {
    pub mesh: Handle<Mesh>,
    pub font_size: [f32; 2],
    pub names: Vec<Vec<HudQuad>>,
    /// Rewrite extension: registered (mod) Magick names by registry index.
    pub registered_names: std::collections::BTreeMap<u16, Vec<HudQuad>>,
}
pub struct PlayHudPlugin;
impl Plugin for PlayHudPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "play_hud.wgsl");
        app.add_plugins(Material2dPlugin::<HudMaterial>::default());
    }
}
pub fn setup(
    commands: &mut Commands,
    content: &std::path::Path,
    definitions: &magicka_definitions::Definitions,
    images: &mut Assets<Image>,
    materials: &mut Assets<HudMaterial>,
    meshes: &mut Assets<Mesh>,
) -> Result<(), String> {
    let Asset::Texture2D(texture) =
        crate::text_box_visual::read_asset(&content.join("UI/HUD/hud.xnb"))?
    else {
        return Err("expected HUD texture".into());
    };
    let size = [texture.width as f32, texture.height as f32];
    let atlas = upload_texture(&texture, images)?;
    let material = materials.add(HudMaterial {
        texture: atlas.clone(),
        saturation: Vec4::ONE,
    });
    let mesh = meshes.add(geometry(&[], [1.; 2], size));
    commands.spawn((
        HudGeometry {
            mesh: mesh.clone(),
            atlas_size: size,
        },
        Mesh2d(mesh),
        MeshMaterial2d(material),
        Transform::from_xyz(0., 0., 98.),
        Visibility::Hidden,
    ));
    let Asset::BitmapFont(font) =
        crate::text_box_visual::read_asset(&content.join("Languages/eng/Font/Maiandra14.xnb"))?
    else {
        return Err("expected keyboard font".into());
    };
    let (geometry_font, [width, height], rgba) =
        crate::native_text::padded_font_atlas(&font).map_err(|e| format!("keyboard font: {e}"))?;
    let font_atlas = upload_rgba(width, height, rgba, images);
    let font_size = geometry_font.atlas_size.map(|v| v as f32);
    let glyph_quads = |text: &str| -> Result<Vec<HudQuad>, String> {
        let text = text.encode_utf16().collect::<Vec<_>>();
        let text = TextGeometry::build(
            &geometry_font,
            &text,
            TextStyle {
                align: TextAlign::Center,
                ..default()
            },
        )
        .map_err(|e| format!("HUD text geometry: {e:?}"))?;
        Ok(text
            .quads
            .iter()
            .map(|q| {
                let (a, b) = (&q.vertices[0], &q.vertices[2]);
                HudQuad {
                    bounds: [a.position[0], a.position[1], b.position[0], b.position[1]],
                    atlas: [
                        a.uv[0] * font_size[0],
                        a.uv[1] * font_size[1],
                        b.uv[0] * font_size[0],
                        b.uv[1] * font_size[1],
                    ],
                    color: [1.; 4],
                }
            })
            .collect())
    };
    // Magick.NAME_LOCALIZATION through the English string tables.
    let strings = magicka_runtime::play_content::english_strings(content, &["Wizard"]);
    let mut names = vec![];
    for i in 0..magicka_sim::spells::MAGICK_INDEX_COUNT {
        let magick = magicka_sim::spells::MagickType::try_from(i).unwrap_or_default();
        let text = magicka_runtime::tome_magick::name_key(magick)
            .and_then(|k| strings.get_string(magicka_runtime::scene_loading::custom_hash(&k)))
            .unwrap_or_else(|| {
                magicka_runtime::tome_magick::custom_name(magick)
                    .unwrap_or_default()
                    .to_owned()
            });
        names.push(glyph_quads(&text)?);
    }
    // Registered (mod) Magicks: their localized name, else their ID.
    let mut registered_names = std::collections::BTreeMap::new();
    for m in definitions.magicks.iter().filter(|m| m.legacy.is_none()) {
        let Some(index) = definitions.magicks.index_of(&m.common.id) else {
            continue;
        };
        let text = m
            .common
            .name_key
            .as_deref()
            .and_then(|k| definitions.localization.lookup("english", k))
            .map_or_else(|| m.common.id.to_string(), str::to_string);
        registered_names.insert(index.0, glyph_quads(&text)?);
    }
    let tome_mesh = meshes.add(geometry(&[], [1.; 2], font_size));
    let tome_material = materials.add(HudMaterial {
        texture: font_atlas.clone(),
        saturation: Vec4::ONE,
    });
    commands.spawn((
        TomeTextGeometry {
            mesh: tome_mesh.clone(),
            font_size,
            names,
            registered_names,
        },
        Mesh2d(tome_mesh),
        MeshMaterial2d(tome_material),
        Transform::from_xyz(0., 0., 98.5),
    ));
    for (order, index) in keyboard_hud::DRAW_ORDER.into_iter().enumerate() {
        let text = keyboard_hud::DEFAULT_KEY_LABELS[index]
            .encode_utf16()
            .collect::<Vec<_>>();
        let text = TextGeometry::build(
            &geometry_font,
            &text,
            TextStyle {
                align: TextAlign::Center,
                ..default()
            },
        )
        .map_err(|e| format!("keyboard key geometry: {e:?}"))?;
        let glyphs = text
            .quads
            .iter()
            .map(|q| {
                let (a, b) = (&q.vertices[0], &q.vertices[2]);
                HudQuad {
                    bounds: [a.position[0], a.position[1], b.position[0], b.position[1]],
                    atlas: [
                        a.uv[0] * font_size[0],
                        a.uv[1] * font_size[1],
                        b.uv[0] * font_size[0],
                        b.uv[1] * font_size[1],
                    ],
                    color: [1.; 4],
                }
            })
            .collect();
        let material = materials.add(HudMaterial {
            texture: atlas.clone(),
            saturation: Vec4::ONE,
        });
        let key_material = materials.add(HudMaterial {
            texture: font_atlas.clone(),
            saturation: Vec4::ONE,
        });
        let mesh = meshes.add(geometry(&[], [1.; 2], size));
        let key_mesh = meshes.add(geometry(&[], [1.; 2], font_size));
        let z = 600. + order as f32 * 0.01;
        commands.spawn((
            KeyboardLabel,
            Mesh2d(key_mesh.clone()),
            MeshMaterial2d(key_material.clone()),
            Transform::from_xyz(0., 0., z + 0.001),
        ));
        commands.spawn((
            KeyboardGeometry {
                index,
                mesh: mesh.clone(),
                atlas_size: size,
                key_mesh,
                key_material,
                glyphs,
                font_size,
            },
            Mesh2d(mesh),
            MeshMaterial2d(material),
            Transform::from_xyz(0., 0., z),
        ));
    }
    Ok(())
}
/// An RGBA image with the HUD's linear sampler.
pub(crate) fn upload_rgba(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    images: &mut Assets<Image>,
) -> Handle<Image> {
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    images.add(image)
}

pub(crate) fn upload_texture(
    texture: &Texture2D,
    images: &mut Assets<Image>,
) -> Result<Handle<Image>, String> {
    let rgba = decode_rgba(
        texture.format,
        texture.width as usize,
        texture.height as usize,
        texture.mips.first().ok_or("missing HUD mip")?,
    )
    .ok_or("unsupported HUD encoding")?;
    let mut image = Image::new(
        Extent3d {
            width: texture.width as u32,
            height: texture.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    Ok(images.add(image))
}

pub fn update_keyboard(
    hud: &KeyboardHud,
    screen: [f32; 2],
    controller: Option<crate::controller::ElementStyle>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<HudMaterial>,
    draws: &Query<(&KeyboardGeometry, &MeshMaterial2d<HudMaterial>)>,
) {
    for (draw, material) in draws {
        let icon = hud.icons[draw.index];
        if let Some(mut mesh) = meshes.get_mut(&draw.mesh) {
            let mut quads = keyboard_hud::quads(draw.index, screen[1], icon).to_vec();
            if let Some(style) = controller {
                // One key background per control (see element_layout).
                let glyphs = crate::controller::native::element_glyphs(draw.index, style);
                let anchor = keyboard_hud::key_anchor(draw.index, screen[1]);
                let (_, badges) = crate::controller::native::element_layout(&glyphs, anchor);
                let key = quads.pop().unwrap();
                quads.extend(badges.into_iter().map(|bounds| HudQuad { bounds, ..key }));
            }
            *mesh = geometry(&quads, screen, draw.atlas_size);
        }
        let anchor = keyboard_hud::key_anchor(draw.index, screen[1]);
        let keys: Vec<_> = draw
            .glyphs
            .iter()
            .map(|q| HudQuad {
                bounds: [
                    q.bounds[0] + anchor[0],
                    q.bounds[1] + anchor[1],
                    q.bounds[2] + anchor[0],
                    q.bounds[3] + anchor[1],
                ],
                atlas: q.atlas,
                color: [icon.intensity, icon.intensity, icon.intensity, 1.],
            })
            .collect();
        if let Some(mut mesh) = meshes.get_mut(&draw.key_mesh) {
            *mesh = geometry(&keys, screen, draw.font_size);
        }
        for handle in [&material.0, &draw.key_material] {
            if let Some(mut material) = materials.get_mut(handle) {
                material.saturation = Vec4::splat(icon.saturation);
            }
        }
    }
}
pub fn geometry(quads: &[HudQuad], screen: [f32; 2], atlas: [f32; 2]) -> Mesh {
    let mut positions = vec![];
    let mut uvs = vec![];
    let mut colors = vec![];
    let mut indices = vec![];
    // Degenerate placeholder avoids the Bevy slab allocator's empty mesh issue.
    let placeholder = HudQuad {
        bounds: [0.; 4],
        atlas: [0.; 4],
        color: [0.; 4],
    };
    let quads = if quads.is_empty() {
        std::slice::from_ref(&placeholder)
    } else {
        quads
    };
    for q in quads {
        let i = positions.len() as u32;
        for (x, y, u, v) in [
            (q.bounds[0], q.bounds[1], q.atlas[0], q.atlas[1]),
            (q.bounds[2], q.bounds[1], q.atlas[2], q.atlas[1]),
            (q.bounds[2], q.bounds[3], q.atlas[2], q.atlas[3]),
            (q.bounds[0], q.bounds[3], q.atlas[0], q.atlas[3]),
        ] {
            positions.push([x - screen[0] * 0.5, screen[1] * 0.5 - y, 0.]);
            uvs.push([u / atlas[0], v / atlas[1]]);
            colors.push(q.color);
        }
        indices.extend([i, i + 2, i + 1, i, i + 3, i + 2]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}
