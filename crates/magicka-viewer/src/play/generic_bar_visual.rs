//! GenericHealthBar.RenderData.Draw (PlayState.GenericHealthBar: scripted
//! timer/counter bars such as Havindr's dynamite fuse) over UI/HUD/hud, and
//! its animated UI/HUD/CounterAnimation sprite. See
//! research/generic-health-bar.md.
//!
//! CreateVertices (HealthBarWidth 0.8, integer halves): three strips of
//! native rows 0..24 at y 8..32 with 96-texel caps, drawn with the graphics
//! type's TextureOffset (UV · TextureScale + TextureOffset, GUIBasicEffect).
//! The fill is a 256-texel strip over `w - 32` pixels, transformed by
//! M11 = displayed health and M41 = x0 + 16 (+40 per dynamite Activate);
//! IsScaled crops its texture by the same health. The sprite is a 64-pixel
//! quad at the fill end, additive (SourceAlpha, One), while ticking.
use super::Play;
use super::PlayAppExt;
use crate::play_hud::HudMaterial;
use crate::text_box_visual::read_asset;
use bevy::{
    asset::RenderAssetUsages,
    image::{ImageSampler, ImageSamplerDescriptor},
    mesh::MeshVertexBufferLayoutRef,
    prelude::*,
    render::render_resource::{
        AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, Extent3d,
        RenderPipelineDescriptor, SpecializedMeshPipelineError, TextureDimension, TextureFormat,
    },
    shader::ShaderRef,
    sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPlugin},
};
use magicka_content::assets::Asset;
use magicka_runtime::{
    hud::HudQuad,
    scene_world::boss_live::{GenericHealthBar, COUNTER_ANIMATION_SLICES},
};

const LAYER: f32 = 200.;
/// GenericHealthBar.mHealthBarWidth (SetWidth has no caller).
const WIDTH: f32 = 0.8;
/// CounterAnimation slice size and the drawn quad half extent.
const SPRITE_TEXELS: f32 = 32.;
const SPRITE_HALF: f32 = 32.;

#[derive(Component)]
struct GenericBar;
#[derive(Component)]
struct GenericBarSprite;
#[derive(Resource)]
struct GenericBarAtlas([f32; 2]);

/// The additive animated sprite (GUIBasicEffect with SourceAlpha/One).
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct AdditiveHudMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
    #[uniform(2)]
    pub saturation: Vec4,
}
impl Material2d for AdditiveHudMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/play_hud.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let blend = BlendComponent {
            src_factor: BlendFactor::SrcAlpha,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        };
        if let Some(fragment) = &mut descriptor.fragment {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState {
                    color: blend,
                    alpha: blend,
                });
            }
        }
        Ok(())
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_plugins(Material2dPlugin::<AdditiveHudMaterial>::default())
        .add_play_systems(Startup, setup)
        .add_play_systems(Update, present.after(super::step_session));
}

/// GenericHealthBarGraphics texture offsets (background, fill, overlay), in
/// texels (the GraphicsType setter divides them by the atlas size).
fn offsets(dynamite: bool) -> [[f32; 2]; 3] {
    if dynamite {
        [[72., 392.], [128., 360.], [0., 450.]]
    } else {
        [[0., 48.], [128., 424.], [0., 24.]]
    }
}

/// The bar quads for the screen width and the runtime bar state.
pub fn quads(screen_width: f32, bar: &GenericHealthBar) -> Vec<HudQuad> {
    let x = screen_width as i32;
    let half_screen = (x / 2) as f32;
    let w = (WIDTH * x as f32) as i32;
    let half = (w / 2) as f32;
    let (x0, x1) = (half_screen - half, half_screen + half);
    let [background, fill, overlay] = offsets(bar.config.dynamite);
    let alpha = bar.alpha;
    let white = [1., 1., 1., alpha];
    let strip = |offset: [f32; 2]| {
        [
            ([x0, x0 + 96.], [0., 96.]),
            ([x0 + 96., x1 - 96.], [96., 160.]),
            ([x1 - 96., x1], [160., 256.]),
        ]
        .map(|(xs, us)| HudQuad {
            bounds: [xs[0], 8., xs[1], 32.],
            atlas: [
                us[0] + offset[0],
                offset[1],
                us[1] + offset[0],
                24. + offset[1],
            ],
            color: white,
        })
    };
    let mut out = strip(background).to_vec();
    let health = bar.display_health;
    let left = x0 + 16. + bar.dynamite_shift;
    let scale = if bar.config.is_scaled { health } else { 1. };
    let color = if bar.config.is_colored_red {
        [bar.config.power, 0., 0., alpha]
    } else {
        white
    };
    out.push(HudQuad {
        bounds: [left, 8., left + (w - 32) as f32 * health, 32.],
        atlas: [fill[0], fill[1], 256. * scale + fill[0], 24. + fill[1]],
        color,
    });
    out.extend(strip(overlay));
    out
}

/// The animated sprite's screen bounds and slice, when it draws
/// (HasAnimatedSprite, ticking and not done).
pub fn sprite(screen_width: f32, bar: &GenericHealthBar) -> Option<([f32; 4], u32)> {
    if !(bar.config.has_animated_sprite && bar.ticking && !bar.done) {
        return None;
    }
    let s = screen_width;
    let half_screen = ((s as i32) / 2) as f32;
    let x = half_screen - WIDTH * s / 2.
        + bar.display_health * (WIDTH * s - 40. * WIDTH)
        + 40. * WIDTH
        + 32.;
    let y = bar.config.animation_offset_y;
    // W = (0.5 + frames) / slices samples a slice centre; the 3D texture
    // wraps, so the frame is the advance modulo the slice count.
    let slice = bar.sprite_frames % COUNTER_ANIMATION_SLICES as u32;
    Some((
        [
            x - SPRITE_HALF,
            y - SPRITE_HALF,
            x + SPRITE_HALF,
            y + SPRITE_HALF,
        ],
        slice,
    ))
}

fn setup(
    mut commands: Commands,
    play: Res<Play>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut hud_materials: ResMut<Assets<HudMaterial>>,
    mut additive: ResMut<Assets<AdditiveHudMaterial>>,
) {
    let result = (|| -> Result<(), String> {
        let Asset::Texture2D(hud) = read_asset(&play.content.join("UI/HUD/hud.xnb"))? else {
            return Err("expected UI/HUD/hud".into());
        };
        commands.insert_resource(GenericBarAtlas([hud.width as f32, hud.height as f32]));
        let material = hud_materials.add(HudMaterial {
            texture: crate::play_hud::upload_texture(&hud, &mut images)?,
            saturation: Vec4::X,
        });
        commands.spawn((
            GenericBar,
            Mesh2d(meshes.add(crate::play_hud::geometry(&[], [1.; 2], [1.; 2]))),
            MeshMaterial2d(material),
            Transform::from_xyz(0., 0., LAYER),
            Visibility::Hidden,
        ));
        let strip = counter_strip(&play.content, &mut images)?;
        commands.spawn((
            GenericBarSprite,
            Mesh2d(meshes.add(crate::play_hud::geometry(&[], [1.; 2], [1.; 2]))),
            MeshMaterial2d(additive.add(AdditiveHudMaterial {
                texture: strip,
                saturation: Vec4::X,
            })),
            Transform::from_xyz(0., 0., LAYER + 1.),
            Visibility::Hidden,
        ));
        Ok(())
    })();
    if let Err(e) = result {
        error!("generic health bar: {e}");
    }
}

/// UI/HUD/CounterAnimation as a slice strip.
fn counter_strip(
    content: &std::path::Path,
    images: &mut Assets<Image>,
) -> Result<Handle<Image>, String> {
    texture3d_strip(
        content,
        "UI/HUD/CounterAnimation",
        COUNTER_ANIMATION_SLICES as usize,
        SPRITE_TEXELS as usize,
        images,
    )
}

/// A GUI Texture3D (SurfaceFormat.Color BGRA) as a horizontal strip of its
/// slices, slice-major as stored. Sampling a slice centre with W is then
/// one strip cell; the expected depth and width guard the conversion.
pub(super) fn texture3d_strip(
    content: &std::path::Path,
    name: &str,
    slices: usize,
    texels: usize,
    images: &mut Assets<Image>,
) -> Result<Handle<Image>, String> {
    let Asset::Texture3D {
        format,
        width,
        height,
        depth,
        mips,
    } = read_asset(&content.join(format!("{name}.xnb")))?
    else {
        return Err(format!("{name}: not a Texture3D"));
    };
    if format != 1 || depth as usize != slices || width as usize != texels {
        return Err(format!("{name}: format {format}, {width}x{height}x{depth}"));
    }
    let (w, h, d) = (width as usize, height as usize, depth as usize);
    let level = &mips[0];
    if level.len() != w * h * d * 4 {
        return Err(format!("{name}: short top mip"));
    }
    let mut data = vec![0u8; w * d * h * 4];
    for slice in 0..d {
        for row in 0..h {
            for col in 0..w {
                let src = ((slice * h + row) * w + col) * 4;
                let dst = (row * w * d + slice * w + col) * 4;
                // BGRA to RGBA.
                data[dst] = level[src + 2];
                data[dst + 1] = level[src + 1];
                data[dst + 2] = level[src];
                data[dst + 3] = level[src + 3];
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: (w * d) as u32,
            height: h as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    Ok(images.add(image))
}

fn present(
    play: Res<Play>,
    atlas: Option<Res<GenericBarAtlas>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut bar: Query<(&Mesh2d, &mut Visibility), (With<GenericBar>, Without<GenericBarSprite>)>,
    mut sprite_q: Query<(&Mesh2d, &mut Visibility), (With<GenericBarSprite>, Without<GenericBar>)>,
) {
    let Some(atlas) = atlas else { return };
    let state = &play.session.world.generic_health_bar;
    let visible = state.active && state.alpha > 0.0 && play.session.world.ui_enabled;
    for (mesh, mut visibility) in &mut bar {
        *visibility = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if visible {
            if let Some(mut m) = meshes.get_mut(&mesh.0) {
                *m = crate::play_hud::geometry(&quads(play.screen[0], state), play.screen, atlas.0);
            }
        }
    }
    let shown = sprite(play.screen[0], state).filter(|_| visible);
    for (mesh, mut visibility) in &mut sprite_q {
        let Some((bounds, slice)) = shown else {
            *visibility = Visibility::Hidden;
            continue;
        };
        *visibility = Visibility::Visible;
        let u = slice as f32 * SPRITE_TEXELS;
        let quad = HudQuad {
            bounds,
            atlas: [u, 0., u + SPRITE_TEXELS, SPRITE_TEXELS],
            color: [1.; 4],
        };
        let size = [SPRITE_TEXELS * COUNTER_ANIMATION_SLICES, SPRITE_TEXELS];
        if let Some(mut m) = meshes.get_mut(&mesh.0) {
            *m = crate::play_hud::geometry(&[quad], play.screen, size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use magicka_runtime::boss_action::HealthBarConfig;

    fn dynamite(health: f32) -> GenericHealthBar {
        let mut bar = GenericHealthBar::default();
        bar.config = HealthBarConfig {
            dynamite: true,
            is_scaled: true,
            has_animated_sprite: true,
            ..Default::default()
        };
        bar.display_health = health;
        bar.alpha = 1.;
        bar.dynamite_shift = 40.;
        bar.ticking = true;
        bar
    }

    #[test]
    fn dynamite_strips_use_its_atlas_offsets_and_shifted_cropped_fill() {
        let q = quads(1280., &dynamite(0.5));
        assert_eq!(q.len(), 7);
        // x0 = 640 - 512 = 128.
        assert_eq!(q[0].bounds, [128., 8., 224., 32.]);
        assert_eq!(q[0].atlas, [72., 392., 168., 416.]);
        assert_eq!(q[2].atlas, [232., 392., 328., 416.]);
        // Fill: x0 + 16 + 40, (1024 - 32) * 0.5 wide; texture cropped to half.
        assert_eq!(q[3].bounds, [184., 8., 184. + 496., 32.]);
        assert_eq!(q[3].atlas, [128., 360., 256., 384.]);
        assert_eq!(q[3].color, [1., 1., 1., 1.]);
        assert_eq!(q[4].atlas, [0., 450., 96., 474.]);
    }

    #[test]
    fn default_graphics_and_red_fill() {
        let mut bar = dynamite(1.);
        bar.config.dynamite = false;
        bar.config.is_scaled = false;
        bar.config.is_colored_red = true;
        bar.config.power = 1.1;
        bar.dynamite_shift = 0.;
        let q = quads(1280., &bar);
        assert_eq!(q[0].atlas, [0., 48., 96., 72.]);
        assert_eq!(q[3].atlas, [128., 424., 384., 448.]);
        assert_eq!(q[3].color, [1.1, 0., 0., 1.]);
        assert_eq!(q[4].atlas, [0., 24., 96., 48.]);
    }

    #[test]
    fn sprite_follows_the_fill_end_and_hides_when_done() {
        let mut bar = dynamite(1.);
        bar.config.animation_offset_y = 16.;
        let (b, slice) = sprite(1280., &bar).unwrap();
        // 640 - 512 + (1024 - 32) + 32 + 32 = 1184.
        assert_eq!(b, [1152., -16., 1216., 48.]);
        assert_eq!(slice, 0);
        bar.sprite_frames = 9;
        assert_eq!(sprite(1280., &bar).unwrap().1, 1);
        bar.done = true;
        assert!(sprite(1280., &bar).is_none());
    }
}
