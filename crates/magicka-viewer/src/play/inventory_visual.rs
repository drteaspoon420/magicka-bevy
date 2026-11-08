//! InventoryBox.RenderData.Draw: the Dialog_Say TextBoxEffect at the screen
//! centre (mBox size, BorderSize 32, animated scale), the 250x230
//! UI/Menu/Magicks picture at the box's top-left, and Maiandra14 text
//! (dialogue default colour) from x + 250 at the box top.
use super::PlayAppExt;
use super::{Play, TextBoxMaterial};
use crate::native_text::{upload_font, NativeBitmapText};
use crate::play_hud::HudMaterial;
use crate::text_box_visual::{read_asset, upload_background};
use bevy::prelude::*;
use magicka_content::assets::Asset;
use magicka_runtime::{hud::HudQuad, inventory_box, text_geometry::TextStyle};

/// GUI ZIndex 150, below dialogs (layer 201).
const LAYER: f32 = 150.;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Part {
    Background,
    Image,
    Text,
}

#[derive(Resource)]
struct InventoryAtlas([f32; 2]);

pub(super) fn plugin(app: &mut App) {
    app.add_play_systems(Startup, setup)
        .add_play_systems(Update, present.after(super::step_session));
}

fn setup(
    mut commands: Commands,
    play: Res<Play>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut colors: ResMut<Assets<ColorMaterial>>,
    mut hud_materials: ResMut<Assets<HudMaterial>>,
    mut boxes: ResMut<Assets<TextBoxMaterial>>,
) {
    let result = (|| -> Result<(), String> {
        let Asset::BitmapFont(font) =
            read_asset(&play.content.join("Languages/eng/Font/Maiandra14.xnb"))?
        else {
            return Err("expected Maiandra14".into());
        };
        let (font, color_material) = upload_font(&font, &mut images, &mut colors)?;
        // As the pause menu: this overlay precedes the final HDR decode, so
        // bitmap text uses the HUD encoding rather than a plain ColorMaterial.
        let font_material = hud_materials.add(HudMaterial {
            texture: colors
                .get(&color_material)
                .and_then(|m| m.texture.clone())
                .ok_or("font atlas texture")?,
            saturation: Vec4::X,
        });
        let Asset::Texture2D(say) = read_asset(&play.content.join("UI/HUD/Dialog_Say.xnb"))? else {
            return Err("expected Dialog_Say".into());
        };
        let background = upload_background(&say, &mut images, &mut boxes)?;
        let Asset::Texture2D(magicks) = read_asset(&play.content.join("UI/Menu/Magicks.xnb"))?
        else {
            return Err("expected UI/Menu/Magicks".into());
        };
        commands.insert_resource(InventoryAtlas([
            magicks.width as f32,
            magicks.height as f32,
        ]));
        let image = hud_materials.add(HudMaterial {
            texture: crate::play_hud::upload_texture(&magicks, &mut images)?,
            saturation: Vec4::X,
        });
        let empty = || crate::text_box_visual::background_mesh([0.; 2], [0.; 2], 0., [1.; 2]);
        commands.spawn((
            Part::Background,
            Mesh2d(meshes.add(empty())),
            MeshMaterial2d(background),
            Transform::from_xyz(0., 0., LAYER),
            Visibility::Hidden,
        ));
        commands.spawn((
            Part::Image,
            Mesh2d(meshes.add(crate::play_hud::geometry(&[], [1.; 2], [1.; 2]))),
            MeshMaterial2d(image),
            Transform::from_xyz(0., 0., LAYER + 0.01),
            Visibility::Hidden,
        ));
        let mut text = NativeBitmapText::new(font, "", TextStyle::dialogue());
        text.reveal_enabled = false;
        commands.spawn((
            Part::Text,
            text,
            Mesh2d(meshes.add(empty())),
            MeshMaterial2d(font_material),
            Transform::from_xyz(0., 0., LAYER + 0.02),
            Visibility::Hidden,
        ));
        Ok(())
    })();
    if let Err(e) = result {
        error!("inventory box: {e}");
    }
}

#[allow(clippy::type_complexity)]
fn present(
    play: Res<Play>,
    atlas: Option<Res<InventoryAtlas>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut parts: Query<(
        &Part,
        &Mesh2d,
        &mut Transform,
        &mut Visibility,
        Option<&mut NativeBitmapText>,
    )>,
    mut shown_text: Local<String>,
) {
    let Some(atlas) = atlas else { return };
    let inventory = &play.session.inventory;
    let screen = play.screen;
    let scale = inventory.scale;
    let visible = scale > 0.0;
    let center = [screen[0] * 0.5, screen[1] * 0.5];
    let size = inventory.size;
    let corner = [
        center[0] - size[0] * 0.5 * scale,
        center[1] - size[1] * 0.5 * scale,
    ];
    for (part, mesh, mut transform, mut visibility, text) in &mut parts {
        *visibility = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if !visible {
            continue;
        }
        match part {
            Part::Background => {
                if let Some(mut m) = meshes.get_mut(&mesh.0) {
                    *m = crate::text_box_visual::background_mesh(center, size, scale, screen);
                }
            }
            Part::Image => {
                let [u, v, w, h] = inventory_box::IMAGE;
                let quad = HudQuad {
                    bounds: [
                        corner[0],
                        corner[1],
                        corner[0] + w * scale,
                        corner[1] + h * scale,
                    ],
                    atlas: [u, v, u + w, v + h],
                    color: [1.; 4],
                };
                if let Some(mut m) = meshes.get_mut(&mesh.0) {
                    *m = crate::play_hud::geometry(&[quad], screen, atlas.0);
                }
            }
            Part::Text => {
                let x = center[0] - (size[0] * 0.5 - inventory_box::TEXT_OFFSET) * scale;
                let y = corner[1];
                transform.translation.x = x - screen[0] * 0.5;
                transform.translation.y = screen[1] * 0.5 - y;
                transform.scale = Vec3::new(scale, scale, 1.);
                if let Some(mut text) = text {
                    if *shown_text != inventory.text {
                        text.typing.set_text(Some(&inventory.text));
                        *shown_text = inventory.text.clone();
                    }
                    if !text.typing.dirty {
                        text.typing.finish();
                    }
                }
            }
        }
    }
}
