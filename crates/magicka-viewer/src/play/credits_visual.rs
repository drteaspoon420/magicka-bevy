//! Credits.RenderData.Draw (ZIndex 200): a black band over the top half of
//! the screen fading to transparent by 0.55 h, at the credits' fade-in alpha,
//! then each line in the drawn range centred at (w/2, Position) with its
//! credit colour at the line alpha (research/credits.md).
use super::Play;
use super::PlayAppExt;
use crate::native_text::{geometry_mesh, upload_font};
use crate::text_box_visual::read_asset;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use magicka_content::assets::Asset;
use magicka_runtime::credits::FONTS;
use magicka_runtime::text_geometry::{GeometryFont, TextAlign, TextGeometry, TextStyle};
use std::collections::HashMap;
use std::sync::Arc;

const LAYER: f32 = 200.;

#[derive(Component)]
struct CreditsBand(Handle<ColorMaterial>);

#[derive(Component)]
struct CreditsLine {
    index: usize,
    material: Handle<ColorMaterial>,
}

pub(super) fn plugin(app: &mut App) {
    app.add_play_systems(Startup, setup)
        .add_play_systems(Update, present.after(super::step_session));
}

/// The band's six strip vertices as triangles, in native screen units
/// (Y down): opaque black to h/2, then alpha 1 → 0 down to 0.55 h.
pub fn band_vertices(screen: [f32; 2]) -> Vec<([f32; 2], f32)> {
    let [w, h] = screen;
    let (y1, y2) = (h * 0.5, h * 0.55);
    let v = [
        ([0., 0.], 1.),
        ([w, 0.], 1.),
        ([0., y1], 1.),
        ([w, y1], 1.),
        ([0., y2], 0.),
        ([w, y2], 0.),
    ];
    // TriangleStrip (0,1,2) (1,3,2) (2,3,4) (3,5,4).
    [0, 1, 2, 1, 3, 2, 2, 3, 4, 3, 5, 4]
        .into_iter()
        .map(|i| v[i])
        .collect()
}

fn band_mesh(screen: [f32; 2]) -> Mesh {
    let vertices = band_vertices(screen);
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vertices
            .iter()
            .map(|(p, _)| [p[0] - screen[0] * 0.5, screen[1] * 0.5 - p[1], 0.])
            .collect::<Vec<_>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        vertices
            .iter()
            .map(|(_, a)| [0., 0., 0., *a])
            .collect::<Vec<_>>(),
    );
    mesh.insert_indices(Indices::U16((0..vertices.len() as u16).collect()));
    mesh
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let material = materials.add(ColorMaterial::from(Color::srgba(1., 1., 1., 0.)));
    commands.spawn((
        CreditsBand(material.clone()),
        Mesh2d(meshes.add(band_mesh([1280., 720.]))),
        MeshMaterial2d(material),
        Transform::from_xyz(0., 0., LAYER),
        Visibility::Hidden,
    ));
}

/// Fonts by MagickaFont index with FontManager spacing, uploaded on demand.
#[derive(Default)]
struct Fonts(HashMap<usize, (Arc<GeometryFont>, Handle<Image>)>);

#[allow(clippy::too_many_arguments)]
fn present(
    mut commands: Commands,
    play: Res<Play>,
    mut built: Local<Option<(usize, f32)>>,
    mut fonts: Local<Fonts>,
    mut screen_built: Local<[f32; 2]>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut band: Query<(&CreditsBand, &Mesh2d, &mut Visibility), Without<CreditsLine>>,
    mut lines: Query<(Entity, &CreditsLine, &mut Transform, &mut Visibility)>,
) {
    let screen = play.screen;
    let credits = play.session.credits.as_ref().filter(|c| c.started);
    for (b, mesh, mut visibility) in &mut band {
        *visibility = match credits {
            Some(c) if c.fade_in_alpha > 0. => Visibility::Visible,
            _ => Visibility::Hidden,
        };
        if *screen_built != screen {
            if let Some(mut m) = meshes.get_mut(&mesh.0) {
                *m = band_mesh(screen);
            }
        }
        if let (Some(c), Some(mut m)) = (credits, materials.get_mut(&b.0)) {
            m.color = Color::srgba(1., 1., 1., c.fade_in_alpha);
        }
    }
    *screen_built = screen;
    let Some(c) = credits else {
        for (_, _, _, mut v) in &mut lines {
            *v = Visibility::Hidden;
        }
        return;
    };
    // Build the line entities once per credits layout.
    let key = (c.lines.len(), c.total_length);
    if *built != Some(key) {
        for (e, ..) in &lines {
            commands.entity(e).despawn();
        }
        for (index, line) in c.lines.iter().enumerate() {
            let font = match fonts.0.get(&line.font) {
                Some(f) => f.clone(),
                None => {
                    let path = play.content.join(format!("{}.xnb", FONTS[line.font].1));
                    let Ok(Asset::BitmapFont(bitmap)) = read_asset(&path) else {
                        warn!("credits font {} unavailable", FONTS[line.font].0);
                        continue;
                    };
                    let Ok((geometry, material)) =
                        upload_font(&bitmap, &mut images, &mut materials)
                    else {
                        continue;
                    };
                    let mut geometry = (*geometry).clone();
                    geometry.metrics.spacing = FONTS[line.font].2;
                    let texture = materials
                        .get(&material)
                        .and_then(|m| m.texture.clone())
                        .unwrap_or_default();
                    let entry = (Arc::new(geometry), texture);
                    fonts.0.insert(line.font, entry.clone());
                    entry
                }
            };
            let utf16: Vec<u16> = line.text.encode_utf16().collect();
            let [r, g, b] = line.color;
            let Ok(geometry) = TextGeometry::build(
                &font.0,
                &utf16,
                TextStyle {
                    default_color: [r, g, b, 1.],
                    align: TextAlign::Center,
                    ..default()
                },
            ) else {
                continue;
            };
            let Ok(mesh) = geometry_mesh(&geometry, geometry.quads.len() as i32) else {
                continue;
            };
            let material = materials.add(ColorMaterial {
                color: Color::srgba(1., 1., 1., 0.),
                texture: Some(font.1.clone()),
                ..default()
            });
            commands.spawn((
                CreditsLine {
                    index,
                    material: material.clone(),
                },
                Mesh2d(meshes.add(mesh)),
                MeshMaterial2d(material),
                Transform::from_xyz(0., 0., LAYER + 0.1),
                Visibility::Hidden,
            ));
        }
        *built = Some(key);
        return;
    }
    let (start, end) = c.render_range;
    for (_, line, mut transform, mut visibility) in &mut lines {
        let Some(l) = c.lines.get(line.index) else {
            continue;
        };
        let shown = (start..end).contains(&line.index);
        *visibility = if shown {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if !shown {
            continue;
        }
        // Text.Draw at (w/2, Position); Camera2d has its origin centred.
        transform.translation = Vec3::new(0., screen[1] * 0.5 - l.position, LAYER + 0.1);
        if let Some(mut m) = materials.get_mut(&line.material) {
            m.color = Color::srgba(1., 1., 1., l.alpha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::band_vertices;

    #[test]
    fn band_is_opaque_to_half_height_then_fades_by_055() {
        let v = band_vertices([100., 200.]);
        assert_eq!(v.len(), 12);
        assert!(v.iter().any(|(p, a)| p == &[0., 100.] && *a == 1.));
        assert!(v.iter().any(|(p, a)| p == &[100., 110.] && *a == 0.));
    }
}
