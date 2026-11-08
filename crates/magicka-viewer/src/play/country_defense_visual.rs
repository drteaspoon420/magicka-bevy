//! CountryDefense.RenderData (ZIndex 100): five UI/Boss/CountryDefense house
//! icons (Texture3D 64x64x4, the slice picked by each house's health) from
//! x = 0.1 · screen width at y = 32, each 64 · min(GUIScale, 1) wide and
//! 8 pixels apart, at alpha min(timer, 1). INFERRED: GUIScale is taken as 1.
use super::Play;
use super::PlayAppExt;
use crate::play_hud::HudMaterial;
use bevy::prelude::*;
use magicka_runtime::{
    hud::HudQuad,
    scene_world::{
        boss_live::BossRef,
        country_defense_live::{HOUSES, HOUSE_SLICES},
    },
};

const LAYER: f32 = 100.;
const TEXELS: f32 = 64.;
const PADDING: f32 = 8.;

#[derive(Component)]
struct HouseIcons;

pub(super) fn plugin(app: &mut App) {
    app.add_play_systems(Startup, setup)
        .add_play_systems(Update, present.after(super::step_session));
}

fn setup(
    mut commands: Commands,
    play: Res<Play>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut hud: ResMut<Assets<HudMaterial>>,
) {
    match super::generic_bar_visual::texture3d_strip(
        &play.content,
        "UI/Boss/CountryDefense",
        HOUSE_SLICES as usize,
        TEXELS as usize,
        &mut images,
    ) {
        Ok(strip) => {
            commands.spawn((
                HouseIcons,
                Mesh2d(meshes.add(crate::play_hud::geometry(&[], [1.; 2], [1.; 2]))),
                MeshMaterial2d(hud.add(HudMaterial {
                    texture: strip,
                    saturation: Vec4::X,
                })),
                Transform::from_xyz(0., 0., LAYER),
                Visibility::Hidden,
            ));
        }
        Err(e) => error!("country defense icons: {e}"),
    }
}

/// The icon quads (strip texels: slice · 64 across).
pub fn quads(screen_width: f32, slices: &[u32; HOUSES], alpha: f32) -> Vec<HudQuad> {
    let mut x = screen_width * 0.1;
    slices
        .iter()
        .map(|&slice| {
            let u = slice as f32 * TEXELS;
            let q = HudQuad {
                bounds: [x, 32., x + TEXELS, 32. + TEXELS],
                atlas: [u, 0., u + TEXELS, TEXELS],
                color: [1., 1., 1., alpha],
            };
            x += PADDING + TEXELS;
            q
        })
        .collect()
}

fn present(
    play: Res<Play>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut icons: Query<(&Mesh2d, &mut Visibility), With<HouseIcons>>,
) {
    let world = &play.session.world;
    let host = world
        .boss_fight
        .active
        .then(|| {
            world.boss_fight.bosses.iter().find_map(|b| match b {
                BossRef::CountryDefense(c) => Some(c),
                _ => None,
            })
        })
        .flatten()
        .filter(|_| world.ui_enabled);
    for (mesh, mut visibility) in &mut icons {
        let Some(host) = host else {
            *visibility = Visibility::Hidden;
            continue;
        };
        *visibility = Visibility::Visible;
        let size = [TEXELS * HOUSE_SLICES as f32, TEXELS];
        if let Some(mut m) = meshes.get_mut(&mesh.0) {
            *m = crate::play_hud::geometry(
                &quads(play.screen[0], &host.slices, host.alpha()),
                play.screen,
                size,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::quads;

    #[test]
    fn icons_start_at_a_tenth_of_the_width_and_step_by_72() {
        let q = quads(1280., &[0, 1, 2, 3, 3], 0.5);
        assert_eq!(q[0].bounds, [128., 32., 192., 96.]);
        assert_eq!(q[1].bounds, [200., 32., 264., 96.]);
        assert_eq!(q[1].atlas, [64., 0., 128., 64.]);
        assert_eq!(q[4].atlas, [192., 0., 256., 64.]);
        assert_eq!(q[4].color[3], 0.5);
    }
}
