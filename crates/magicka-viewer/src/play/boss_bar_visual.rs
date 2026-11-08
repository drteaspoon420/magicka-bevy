//! BossHealthBar.RenderData.Draw over UI/HUD/hud: background strip, the
//! (Power, 0, 0) fill scaled by the displayed health from x + 16, then the
//! frame strip, all at the bar alpha; ZIndex 200. Geometry follows
//! CreateVertices: width = (int)(HealthBarWidth · screen width), 96-texel
//! end caps, rows 48..72 (background), 72..96 (fill), 24..48 (frame), y 8..32.
use super::Play;
use super::PlayAppExt;
use crate::play_hud::HudMaterial;
use crate::text_box_visual::read_asset;
use bevy::prelude::*;
use magicka_content::assets::Asset;
use magicka_runtime::hud::HudQuad;

const LAYER: f32 = 200.;

#[derive(Component)]
struct BossBar;

#[derive(Resource)]
struct BossBarAtlas([f32; 2]);

pub(super) fn plugin(app: &mut App) {
    app.add_play_systems(Startup, setup)
        .add_play_systems(Update, present.after(super::step_session));
}

fn setup(
    mut commands: Commands,
    play: Res<Play>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut hud_materials: ResMut<Assets<HudMaterial>>,
) {
    let result = (|| -> Result<(), String> {
        let Asset::Texture2D(hud) = read_asset(&play.content.join("UI/HUD/hud.xnb"))? else {
            return Err("expected UI/HUD/hud".into());
        };
        commands.insert_resource(BossBarAtlas([hud.width as f32, hud.height as f32]));
        let material = hud_materials.add(HudMaterial {
            texture: crate::play_hud::upload_texture(&hud, &mut images)?,
            saturation: Vec4::X,
        });
        commands.spawn((
            BossBar,
            Mesh2d(meshes.add(crate::play_hud::geometry(&[], [1.; 2], [1.; 2]))),
            MeshMaterial2d(material),
            Transform::from_xyz(0., 0., LAYER),
            Visibility::Hidden,
        ));
        Ok(())
    })();
    if let Err(e) = result {
        error!("boss health bar: {e}");
    }
}

/// The three strips and the fill for a bar `width` (native integer pixels).
pub fn quads(screen_width: f32, width: f32, health: f32, power: f32, alpha: f32) -> Vec<HudQuad> {
    let x = screen_width as i32;
    let half_screen = (x / 2) as f32;
    let w = (width * x as f32) as i32;
    let half = (w / 2) as f32;
    let (x0, x1) = (half_screen - half, half_screen + half);
    let white = [1., 1., 1., alpha];
    let strip = |v0: f32, v1: f32| {
        [
            ([x0, x0 + 96.], [0., 96.]),
            ([x0 + 96., x1 - 96.], [96., 160.]),
            ([x1 - 96., x1], [160., 256.]),
        ]
        .map(|(xs, us)| HudQuad {
            bounds: [xs[0], 8., xs[1], 32.],
            atlas: [us[0], v0, us[1], v1],
            color: white,
        })
    };
    let mut out = strip(48., 72.).to_vec();
    let fill_left = x0 + 16.;
    out.push(HudQuad {
        bounds: [fill_left, 8., fill_left + (w - 32) as f32 * health, 32.],
        atlas: [76., 72., 84., 96.],
        color: [power, 0., 0., alpha],
    });
    out.extend(strip(24., 48.));
    out
}

fn present(
    play: Res<Play>,
    atlas: Option<Res<BossBarAtlas>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut bar: Query<(&Mesh2d, &mut Visibility), With<BossBar>>,
) {
    let Some(atlas) = atlas else { return };
    let fight = &play.session.world.boss_fight;
    for (mesh, mut visibility) in &mut bar {
        let visible = fight.bar_visible && fight.bar.alpha > 0.0;
        *visibility = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if !visible {
            continue;
        }
        let q = quads(
            play.screen[0],
            fight.health_bar_width,
            fight.bar.display_health,
            fight.bar.power,
            fight.bar.alpha,
        );
        if let Some(mut m) = meshes.get_mut(&mesh.0) {
            *m = crate::play_hud::geometry(&q, play.screen, atlas.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::quads;

    #[test]
    fn strips_keep_native_integer_centering_and_fill_inset() {
        // Odd viewport and bar widths: each half truncates independently.
        let q = quads(1001., 0.8, 0.25, 1., 0.5);
        assert_eq!(q.len(), 7);
        assert_eq!(q[0].bounds, [100., 8., 196., 32.]);
        assert_eq!(q[1].bounds, [196., 8., 804., 32.]);
        assert_eq!(q[2].bounds, [804., 8., 900., 32.]);
        assert_eq!(q[3].bounds, [116., 8., 308., 32.]);
        assert_eq!(q[3].atlas, [76., 72., 84., 96.]);
        assert_eq!(q[3].color, [1., 0., 0., 0.5]);
        assert_eq!(q[0].atlas, [0., 48., 96., 72.]);
        assert_eq!(q[4].atlas, [0., 24., 96., 48.]);
        let odd = quads(1001., 0.801, 1., 1., 1.);
        assert_eq!(odd[2].bounds[2], 900.);
        assert_eq!(odd[3].bounds[2], 885.);
    }
}
