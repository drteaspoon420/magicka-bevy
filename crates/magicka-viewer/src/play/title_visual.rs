//! TitleRenderData.Draw (ZIndex 205) for the boss title (BossFight) and the
//! level title (Level.DisplayTitle): the title in Stonecross36 at
//! (w/2 + 0.5, 0.3h + 0.5), the subtitle in Stonecross28 one measured title
//! height lower, each first drawn black at (2, 2) with the title alpha, then
//! white at the title alpha. Strings are localized and upper-cased as the
//! actions' Initialize does (research/title-renderer.md).
use super::Play;
use super::PlayAppExt;
use crate::native_text::{geometry_mesh, upload_overlay_font, OverlayFontMaterial};
use crate::text_box_visual::read_asset;
use bevy::prelude::*;
use magicka_content::assets::Asset;
use magicka_runtime::localization::LocalizationTable;
use magicka_runtime::text_geometry::{GeometryFont, TextAlign, TextGeometry, TextStyle};
use magicka_runtime::title_renderer::{ShownTitle, TitleKind};
use std::sync::Arc;

const LAYER: f32 = 205.;
const SHADOW_OFFSET: f32 = 2.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Boss,
    Level,
}

/// One text of a title: (slot, subtitle, shadow).
#[derive(Component)]
struct TitleText {
    slot: Slot,
    subtitle: bool,
    shadow: bool,
    material: Handle<OverlayFontMaterial>,
}

#[derive(Resource)]
struct TitleFonts {
    title: Arc<GeometryFont>,
    subtitle: Arc<GeometryFont>,
    strings: LocalizationTable,
}

/// The resolved strings each slot's meshes were built for.
#[derive(Default)]
struct Built {
    boss: Option<(String, String, TextAlign)>,
    level: Option<(String, String, TextAlign)>,
}

pub(super) fn plugin(app: &mut App) {
    app.add_play_systems(Startup, setup)
        .add_play_systems(Update, present.after(super::step_session));
}

fn setup(
    mut commands: Commands,
    play: Res<Play>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<OverlayFontMaterial>>,
) {
    let result = (|| -> Result<(), String> {
        let font = |name: &str| -> Result<_, String> {
            let path = play.content.join(format!("Languages/eng/Font/{name}.xnb"));
            match read_asset(&path)? {
                Asset::BitmapFont(f) => Ok(f),
                _ => Err(format!("expected {name}")),
            }
        };
        let (title_font, subtitle_font) = (font("Stonecross36")?, font("Stonecross28")?);
        let mut title = None;
        let mut subtitle = None;
        for slot in [Slot::Boss, Slot::Level] {
            for is_subtitle in [false, true] {
                for shadow in [true, false] {
                    let source = if is_subtitle {
                        &subtitle_font
                    } else {
                        &title_font
                    };
                    let (geometry, material) =
                        upload_overlay_font(source, &mut images, &mut materials)?;
                    // FontManager's custom Spacing 1 for Stonecross28/36.
                    let mut geometry = (*geometry).clone();
                    geometry.metrics.spacing = 1;
                    let geometry = Arc::new(geometry);
                    *(if is_subtitle {
                        &mut subtitle
                    } else {
                        &mut title
                    }) = Some(geometry);
                    commands.spawn((
                        TitleText {
                            slot,
                            subtitle: is_subtitle,
                            shadow,
                            material: material.clone(),
                        },
                        Mesh2d(meshes.add(geometry_mesh(&TextGeometry::default(), 0)?)),
                        MeshMaterial2d(material),
                        Transform::from_xyz(0., 0., LAYER),
                        Visibility::Hidden,
                    ));
                }
            }
        }
        commands.insert_resource(TitleFonts {
            title: title.ok_or("no title font")?,
            subtitle: subtitle.ok_or("no subtitle font")?,
            strings: magicka_runtime::play_content::english_strings(&play.content, &["Wizard"]),
        });
        Ok(())
    })();
    if let Err(e) = result {
        error!("titles unavailable: {e}");
    }
}

/// Native screen position (Y down) of each text's origin, before the shadow
/// offset: (w/2 + 0.5, 0.3h + 0.5), the subtitle `title_height` lower.
pub fn origin(screen: [f32; 2], subtitle: bool, title_height: f32) -> [f32; 2] {
    let y = screen[1] * 0.3 + 0.5;
    [
        screen[0] * 0.5 + 0.5,
        if subtitle { y + title_height } else { y },
    ]
}

fn build(font: &GeometryFont, text: &str, align: TextAlign) -> Result<Mesh, String> {
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let geometry = TextGeometry::build(font, &utf16, TextStyle { align, ..default() })
        .map_err(|e| format!("{e:?}"))?;
    geometry_mesh(&geometry, geometry.quads.len() as i32)
}

fn present(
    play: Res<Play>,
    fonts: Option<Res<TitleFonts>>,
    mut built: Local<Built>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<OverlayFontMaterial>>,
    mut texts: Query<(&TitleText, &Mesh2d, &mut Transform, &mut Visibility)>,
) {
    let Some(fonts) = fonts else { return };
    let world = &play.session.world;
    for (slot, shown) in [
        (Slot::Boss, world.boss_fight.shown_title.as_ref()),
        (Slot::Level, world.shown_level_title.as_ref()),
    ] {
        let cache = match slot {
            Slot::Boss => &mut built.boss,
            Slot::Level => &mut built.level,
        };
        let resolved = shown.map(|t: &ShownTitle| {
            let (title, subtitle) = t.resolve(&fonts.strings);
            let align = match t.kind {
                TitleKind::Boss => TextAlign::Center,
                TitleKind::Level { align, .. } => align,
            };
            (title, subtitle, align)
        });
        let rebuild = resolved.is_some() && *cache != resolved;
        let title_height = resolved
            .as_ref()
            .and_then(|(title, _, _)| fonts.title.metrics.measure(title, true).ok())
            .map_or(0., |m| m[1]);
        for (text, mesh, mut transform, mut visibility) in &mut texts {
            if text.slot != slot {
                continue;
            }
            let Some((title, subtitle, align)) = &resolved else {
                *visibility = Visibility::Hidden;
                continue;
            };
            let string = if text.subtitle { subtitle } else { title };
            if rebuild {
                let font = if text.subtitle {
                    &fonts.subtitle
                } else {
                    &fonts.title
                };
                match build(font, string, *align) {
                    Ok(m) => {
                        if let Some(mut target) = meshes.get_mut(&mesh.0) {
                            *target = m;
                        }
                    }
                    Err(e) => warn!("title text: {e}"),
                }
            }
            // TitleRenderData.Draw skips empty strings.
            *visibility = if string.is_empty() {
                Visibility::Hidden
            } else {
                Visibility::Visible
            };
            let alpha = shown.map_or(0., |t| t.alpha).clamp(0., 1.);
            let [x, y] = origin(play.screen, text.subtitle, title_height);
            let d = if text.shadow { SHADOW_OFFSET } else { 0. };
            // Camera2d: origin at the screen centre, Y up.
            transform.translation = Vec3::new(
                x + d - play.screen[0] * 0.5,
                play.screen[1] * 0.5 - (y + d),
                LAYER - if text.shadow { 0.01 } else { 0. },
            );
            if let Some(mut m) = materials.get_mut(&text.material) {
                let c = if text.shadow { 0. } else { 1. };
                m.color = Vec4::new(c, c, c, alpha);
            }
        }
        if rebuild {
            *cache = resolved;
        } else if resolved.is_none() {
            *cache = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::origin;

    #[test]
    fn titles_sit_at_three_tenths_with_the_subtitle_one_title_lower() {
        let near =
            |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3;
        assert!(near(origin([1280., 720.], false, 40.), [640.5, 216.5]));
        assert!(near(origin([1280., 720.], true, 40.), [640.5, 256.5]));
    }
}
