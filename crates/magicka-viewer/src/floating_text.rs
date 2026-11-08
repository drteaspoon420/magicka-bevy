//! FloatingTextManager RenderData: `UI/HUD/Digits` glyph quads drawn with the
//! GUIHardwareInstancing pixel stage (research/floating-text.md).
use crate::play_hud::{geometry, HudMaterial};
use bevy::prelude::*;
use magicka_content::assets::Asset;
use magicka_runtime::floating_text::{quads, NumberInstance};

#[derive(Component)]
pub struct FloatingTextGeometry {
    mesh: Handle<Mesh>,
    atlas_size: [f32; 2],
}

pub fn setup(
    commands: &mut Commands,
    content: &std::path::Path,
    images: &mut Assets<Image>,
    materials: &mut Assets<HudMaterial>,
    meshes: &mut Assets<Mesh>,
) -> Result<(), String> {
    let Asset::Texture2D(texture) =
        crate::text_box_visual::read_asset(&content.join("UI/HUD/Digits.xnb"))?
    else {
        return Err("expected Digits texture".into());
    };
    let atlas_size = [texture.width as f32, texture.height as f32];
    let image = crate::play_hud::upload_texture(&texture, images)?;
    let material = materials.add(HudMaterial {
        texture: image,
        saturation: Vec4::ONE,
    });
    let mesh = meshes.add(geometry(&[], [1.; 2], atlas_size));
    // RenderData.ZIndex 20: below Healthbars (98).
    commands.spawn((
        FloatingTextGeometry {
            mesh: mesh.clone(),
            atlas_size,
        },
        Mesh2d(mesh),
        MeshMaterial2d(material),
        Transform::from_xyz(0., 0., 20.),
        Visibility::Hidden,
    ));
    Ok(())
}

/// RenderData.PreRenderUpdate (MagickaMath.WorldToScreenPosition) and the
/// Numbers shader cells. Instances behind the camera (w <= 0) are skipped
/// instead of mirrored.
pub fn screen_quads(
    numbers: &[NumberInstance],
    vp: &[f32; 16],
    screen: [f32; 2],
) -> Vec<magicka_runtime::hud::HudQuad> {
    let mut out = vec![];
    for n in numbers.iter().filter(|n| n.color[3] > 0.0) {
        let p = n.world_position;
        let clip: [f32; 4] = std::array::from_fn(|i| {
            p[0] * vp[i] + p[1] * vp[4 + i] + p[2] * vp[8 + i] + vp[12 + i]
        });
        if clip[3] <= 0.0 {
            continue;
        }
        let at = [
            (clip[0] / clip[3] * 0.5 + 0.5) * screen[0],
            (clip[1] / clip[3] * -0.5 + 0.5) * screen[1],
        ];
        out.extend(quads(n, at));
    }
    out
}

/// Autopilot audit line for the paired damage-number capture.
pub fn audit(numbers: &[NumberInstance], vp: &[f32; 16], screen: [f32; 2]) {
    let quads = screen_quads(numbers, vp, screen);
    let glyphs: Vec<u32> = quads.iter().map(|q| (q.atlas[0] / 22.0) as u32).collect();
    let bounds: Vec<[f32; 4]> = quads.iter().map(|q| q.bounds).collect();
    let colors: Vec<[f32; 4]> = quads.iter().map(|q| q.color).collect();
    eprintln!("damage numbers audit: glyphs={glyphs:?} bounds={bounds:?} colors={colors:?}");
}

/// RenderData.Draw into the GUI layer.
pub fn draw(
    numbers: &[NumberInstance],
    view_projection: &[f32; 16],
    screen: [f32; 2],
    meshes: &mut Assets<Mesh>,
    draws: &mut Query<(&FloatingTextGeometry, &mut Visibility)>,
) {
    let out = screen_quads(numbers, view_projection, screen);
    for (draw, mut visibility) in draws {
        *visibility = if out.is_empty() {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        if let Some(mut mesh) = meshes.get_mut(&draw.mesh) {
            *mesh = geometry(&out, screen, draw.atlas_size);
        }
    }
}
