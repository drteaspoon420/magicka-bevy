//! Headless regression: only level-owned meshes may produce an outline.
use bevy::{
    app::{AppExit, ScheduleRunnerPlugin},
    camera::{ImageRenderTarget, RenderTarget, ScalingMode},
    core_pipeline::{prepass::DepthPrepass, tonemapping::Tonemapping},
    prelude::*,
    render::{
        render_resource::{TextureFormat, TextureUsages},
        view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured},
    },
    window::ExitCondition,
    winit::WinitPlugin,
};
use std::time::Duration;

#[derive(Resource)]
struct Capture {
    path: String,
    target: Handle<Image>,
    msaa: Msaa,
}

pub fn run(path: String, multisampled: bool) {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .disable::<WinitPlugin>(),
    )
    .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
        1.0 / 120.0,
    )))
    .add_plugins((crate::terrain::TerrainPlugin, crate::outline::OutlinePlugin))
    .insert_resource(crate::robe::RobeColour([0., 1., 0.]))
    .insert_resource(crate::outline::AvatarOutline {
        submit: true,
        forced: false,
    })
    .insert_resource(ClearColor(Color::BLACK))
    .insert_resource(Capture {
        path,
        target: default(),
        msaa: if multisampled {
            Msaa::Sample4
        } else {
            Msaa::Off
        },
    })
    .add_systems(Startup, setup)
    .add_systems(Update, capture)
    .run();
}
fn setup(
    mut commands: Commands,
    mut capture: ResMut<Capture>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut terrain: ResMut<Assets<crate::terrain::TerrainMaterial>>,
    lights: Res<crate::lighting::MagickaLights>,
    cutaway: Res<crate::cutaway::CutawayBindings>,
) {
    let mut image = Image::new_target_texture(1536, 256, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    capture.target = images.add(image);
    commands.spawn((
        Camera3d::default(),
        DepthPrepass,
        crate::effect_depth::EffectDepthCamera,
        capture.msaa,
        Tonemapping::None,
        Camera { ..default() },
        RenderTarget::Image(ImageRenderTarget {
            handle: capture.target.clone(),
            scale_factor: 1.0,
        }),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed {
                width: 24.,
                height: 4.,
            },
            near: 0.1,
            far: 100.,
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(0., 0., 20.).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let body = meshes.add(Sphere::new(1.).mesh().uv(64, 32));
    let wall = meshes.add(Cuboid::new(1.5, 2.4, 0.3));
    let gray = standard.add(StandardMaterial {
        base_color: Color::srgb(0.25, 0.25, 0.25),
        ..default()
    });
    for i in 0..6 {
        let x = -10. + i as f32 * 4.;
        let root = commands
            .spawn((
                Transform::from_xyz(x, 0., 0.),
                Visibility::Visible,
                crate::outline::OutlinedEquipment,
            ))
            .id();
        commands.spawn((
            Mesh3d(body.clone()),
            MeshMaterial3d(gray.clone()),
            Transform::IDENTITY,
            ChildOf(root),
        ));
        let occluder = commands
            .spawn((Mesh3d(wall.clone()), Transform::from_xyz(x - 0.4, 0., 2.)))
            .id();
        if i == 0 || i == 2 || i == 5 {
            commands
                .entity(occluder)
                .insert(crate::lighting::OutlineOccluder);
        }
        if i == 5 {
            commands.spawn((
                Mesh3d(wall.clone()),
                MeshMaterial3d(gray.clone()),
                Transform::from_xyz(x - 0.4, 0., 3.),
                crate::lighting::KeepStandardMaterial,
            ));
        }
        if i == 2 || i == 3 {
            let material = terrain.add(crate::terrain::TerrainMaterial::new(
                crate::terrain::TerrainParams {
                    color0: Vec4::splat(0.25),
                    color1: Vec4::splat(0.25),
                    flags: UVec4::new(
                        if i == 2 {
                            crate::terrain::OUTLINE_OCCLUDER
                        } else {
                            0
                        },
                        0,
                        0,
                        0,
                    ),
                    ..default()
                },
                &lights,
                &cutaway,
            ));
            commands.entity(occluder).insert(MeshMaterial3d(material));
        } else {
            commands
                .entity(occluder)
                .insert(MeshMaterial3d(gray.clone()));
            if i == 4 {
                commands
                    .entity(occluder)
                    .insert(crate::lighting::KeepStandardMaterial);
            }
        }
    }
}
fn capture(mut commands: Commands, capture: Res<Capture>, mut frames: Local<u32>) {
    *frames += 1;
    if *frames == 120 {
        commands
            .spawn(Screenshot::image(capture.target.clone()))
            .observe(save_to_disk(capture.path.clone()))
            .observe(
                |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                    exit.write(AppExit::Success);
                },
            );
    }
}
