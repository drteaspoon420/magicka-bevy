//! Machine.RulerRenderData (ZIndex 100): the UI/Boss/MachineRuler strip
//! drawn in the GUI pass through the camera's view-projection (BasicEffect,
//! DiffuseColor 0.5), and the remaining drill distance "d.dddm" in
//! Maiandra14 at the projected anchor. INFERRED: the projected strip is
//! drawn as a screen-space mesh, so texture coordinates are interpolated
//! affinely rather than perspective-correctly.
use super::Play;
use super::PlayAppExt;
use crate::play_hud::HudMaterial;
use crate::text_box_visual::read_asset;
use bevy::{
    asset::{embedded_asset, RenderAssetUsages},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
    sprite_render::{AlphaMode2d, Material2d, Material2dPlugin},
};
use magicka_content::assets::Asset;
use magicka_runtime::{
    boss_machine::{ruler_geometry, ruler_text},
    hud::HudQuad,
    scene_world::boss_live::BossRef,
    text_geometry::{GeometryFont, TextAlign, TextGeometry, TextStyle},
};

/// GUI ZIndex 100 (below the HUD layers drawn at 200).
const LAYER: f32 = 100.;

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct RulerMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
}
impl Material2d for RulerMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/play/machine_ruler.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

#[derive(Component)]
struct RulerStrip;
#[derive(Component)]
struct RulerText;

struct RulerFont {
    font: GeometryFont,
    size: [f32; 2],
}
#[derive(Resource)]
struct RulerAssets(std::sync::Mutex<RulerFont>);

pub(super) fn plugin(app: &mut App) {
    embedded_asset!(app, "machine_ruler.wgsl");
    app.add_plugins(Material2dPlugin::<RulerMaterial>::default())
        .add_play_systems(Startup, setup)
        .add_play_systems(Update, present.after(super::step_session));
}

fn empty_mesh() -> Mesh {
    crate::play_hud::geometry(&[], [1.; 2], [1.; 2])
}

fn setup(
    mut commands: Commands,
    play: Res<Play>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut rulers: ResMut<Assets<RulerMaterial>>,
    mut hud: ResMut<Assets<HudMaterial>>,
) {
    let result = (|| -> Result<(), String> {
        let Asset::Texture2D(texture) = read_asset(&play.content.join("UI/Boss/MachineRuler.xnb"))?
        else {
            return Err("expected UI/Boss/MachineRuler".into());
        };
        let strip = rulers.add(RulerMaterial {
            texture: crate::play_hud::upload_texture(&texture, &mut images)?,
        });
        commands.spawn((
            RulerStrip,
            Mesh2d(meshes.add(empty_mesh())),
            MeshMaterial2d(strip),
            Transform::from_xyz(0., 0., LAYER),
            Visibility::Hidden,
        ));
        let Asset::BitmapFont(font) =
            read_asset(&play.content.join("Languages/eng/Font/Maiandra14.xnb"))?
        else {
            return Err("expected Maiandra14".into());
        };
        let (font, [w, h], rgba) = crate::native_text::padded_font_atlas(&font)?;
        let atlas = crate::play_hud::upload_rgba(w, h, rgba, &mut images);
        let size = font.atlas_size.map(|v| v as f32);
        commands.insert_resource(RulerAssets(std::sync::Mutex::new(RulerFont { font, size })));
        commands.spawn((
            RulerText,
            Mesh2d(meshes.add(empty_mesh())),
            // GUIBasicEffect with ScaleToHDR (the HUD shader's mapping).
            MeshMaterial2d(hud.add(HudMaterial {
                texture: atlas,
                saturation: Vec4::X,
            })),
            Transform::from_xyz(0., 0., LAYER + 0.5),
            Visibility::Hidden,
        ));
        Ok(())
    })();
    if let Err(e) = result {
        error!("machine ruler: {e}");
    }
}

/// MagickaMath.WorldToScreenPosition (pixels from the top-left).
fn to_screen(p: [f32; 3], vp: &[f32; 16], screen: [f32; 2]) -> [f32; 2] {
    let c: [f32; 4] =
        std::array::from_fn(|j| p[0] * vp[j] + p[1] * vp[4 + j] + p[2] * vp[8 + j] + vp[12 + j]);
    let w = if c[3].abs() > 1e-6 { c[3] } else { 1e-6 };
    [
        (c[0] / w + 1.0) * 0.5 * screen[0],
        (1.0 - c[1] / w) * 0.5 * screen[1],
    ]
}

#[allow(clippy::type_complexity)]
fn present(
    play: Res<Play>,
    assets: Option<Res<RulerAssets>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut strip: Query<(&Mesh2d, &mut Visibility), (With<RulerStrip>, Without<RulerText>)>,
    mut text: Query<(&Mesh2d, &mut Visibility), (With<RulerText>, Without<RulerStrip>)>,
) {
    let ruler = play
        .session
        .world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| match b {
            BossRef::Machine(m) => m.machine.ruler,
            _ => None,
        })
        .filter(|r| r.alpha > 0.0 && play.session.world.ui_enabled);
    let hide = |v: &mut Visibility| *v = Visibility::Hidden;
    let Some(ruler) = ruler else {
        strip.iter_mut().for_each(|(_, mut v)| hide(&mut v));
        text.iter_mut().for_each(|(_, mut v)| hide(&mut v));
        return;
    };
    let screen = play.screen;
    let camera = play.session.camera();
    let vp = magicka_runtime::camera::multiply(
        &camera.view(),
        &camera.projection(screen[0] / screen[1].max(1.0)),
    );
    let (positions, uvs, anchor) = ruler_geometry(&ruler);
    for (mesh, mut visibility) in &mut strip {
        *visibility = Visibility::Visible;
        let mut m = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
        );
        let points: Vec<[f32; 3]> = positions
            .iter()
            .map(|p| {
                let s = to_screen(*p, &vp, screen);
                [s[0] - screen[0] * 0.5, screen[1] * 0.5 - s[1], 0.0]
            })
            .collect();
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, points);
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.to_vec());
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.5, 0.5, 0.5, ruler.alpha]; 8]);
        // TriangleStrip of six primitives.
        let mut indices = Vec::new();
        for i in 0..6u16 {
            if i % 2 == 0 {
                indices.extend([i, i + 1, i + 2]);
            } else {
                indices.extend([i + 1, i, i + 2]);
            }
        }
        m.insert_indices(Indices::U16(indices));
        if let Some(mut target) = meshes.get_mut(&mesh.0) {
            *target = m;
        }
    }
    let Some(assets) = assets else { return };
    let Ok(font) = assets.0.lock() else { return };
    let label: Vec<u16> = ruler_text(ruler.time).encode_utf16().collect();
    let Ok(geometry) = TextGeometry::build(
        &font.font,
        &label,
        TextStyle {
            align: TextAlign::Center,
            ..default()
        },
    ) else {
        return;
    };
    let at = to_screen(anchor, &vp, screen).map(f32::floor);
    let quads: Vec<HudQuad> = geometry
        .quads
        .iter()
        .map(|q| {
            let (a, b) = (&q.vertices[0], &q.vertices[2]);
            HudQuad {
                bounds: [
                    a.position[0] + at[0],
                    a.position[1] + at[1],
                    b.position[0] + at[0],
                    b.position[1] + at[1],
                ],
                atlas: [
                    a.uv[0] * font.size[0],
                    a.uv[1] * font.size[1],
                    b.uv[0] * font.size[0],
                    b.uv[1] * font.size[1],
                ],
                color: [1., 1., 1., ruler.alpha],
            }
        })
        .collect();
    for (mesh, mut visibility) in &mut text {
        *visibility = Visibility::Visible;
        if let Some(mut m) = meshes.get_mut(&mesh.0) {
            *m = crate::play_hud::geometry(&quads, screen, font.size);
        }
    }
}
