//! Bevy viewer for exported Magicka glTF scenes and characters.
//!
//! Interactive:
//!     magicka-viewer --root <dir> --scene <rel.glb> [--anim N]
//!       WASD/arrows pan, Q/E zoom, Z/C orbit, R/F tilt, Home re-frame,
//!       N next animation. The camera frames the scene automatically.
//!
//! Headless render check (offscreen, orthographic, exact framing):
//!     magicka-viewer --root <dir> --scene <rel.glb> --render out.png
//!         (--top | --side) --origin X Y --scale PX_PER_UNIT --size W H
//!         [--anim N --time T] [--opaque]
//!
//! `--top` looks down -Y (image right = +X, image down = +Z, top row = Z of
//! `--origin` Y). `--side` looks down -Z (image right = +X, image up = +Y,
//! top row = Y of `--origin` Y). `--origin X` is the world coordinate of the
//! image's left edge. This matches `tools/render_glb.py` / `check_skin.py`
//! framing so silhouettes can be compared pixel for pixel. The background is
//! pure magenta to make the silhouette mask unambiguous.
// Bevy systems and reconstructed native methods legitimately take many
// parameters and complex query types (Bevy's own recommended allows).
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

use bevy::animation::graph::{AnimationGraph, AnimationGraphHandle, AnimationNodeIndex};
use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::camera::{ImageRenderTarget, RenderTarget, ScalingMode};
use bevy::gltf::{Gltf, GltfAssetLabel};
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use bevy::world_serialization::WorldAssetRoot;
use std::time::Duration;

mod additive;
mod aura;
mod beam;
mod bindings;
mod blizzard;
mod box_devices;
mod controller;
mod cutaway;
mod decals;
mod effect_depth;
mod error_box_preview;
mod floating_text;
#[cfg(feature = "main-menu")]
mod frontend;
mod frontend_cinema;
#[cfg(feature = "main-menu")]
mod frontend_tome;
mod gui_basic;
mod lava;
mod level_animation;
mod lighting;
mod liquid;
mod live_dialog_preview;
mod magick_visuals;
mod menu_input;
pub mod native_dialog;
mod native_text;
mod outline;
mod outline_check;
mod particles;
mod pcm_audio;
mod play;
mod play_hud;
mod player_colors;
mod post;
mod robe;
mod self_shield;
mod shadow_only;
mod status_look;
mod terrain;
mod text_box_visual;
mod text_input_preview;
mod zap;
use terrain::{
    ScenePath, TerrainMaterial, TerrainNoDiscard, TerrainParams, TerrainPlugin, TerrainUnlit,
};

#[derive(Resource, Clone, Debug)]
struct Args {
    native_level_animation: bool,
    root: String,
    scene: String,
    render: Option<String>,
    side: bool,
    origin: Vec2,
    scale: f32,
    size: UVec2,
    anim: Option<usize>,
    time: f32,
    /// Force every material opaque (geometry-only silhouette checks).
    opaque: bool,
    /// Interactive mode: screenshot the window once framed, then exit
    /// (self-test of the interactive path).
    snapshot: Option<String>,
}

fn parse_args() -> Args {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| a.iter().position(|x| x == k).map(|i| a[i + 1].clone());
    let get2 = |k: &str| {
        a.iter().position(|x| x == k).map(|i| {
            (
                a[i + 1].parse::<f32>().unwrap(),
                a[i + 2].parse::<f32>().unwrap(),
            )
        })
    };
    let (ox, oy) = get2("--origin").unwrap_or((0.0, 0.0));
    let (w, h) = get2("--size").unwrap_or((1024.0, 1024.0));
    Args {
        native_level_animation: a.iter().any(|x| x == "--native-level-animation"),
        // Bevy resolves a relative asset root against CARGO_MANIFEST_DIR under
        // `cargo run`; make it absolute w.r.t. the caller's working directory.
        root: {
            let r = get("--root").expect("--root <dir>");
            match std::fs::canonicalize(&r) {
                Ok(p) => p.to_string_lossy().into_owned(),
                Err(e) => {
                    eprintln!("--root {r}: {e}");
                    std::process::exit(2);
                }
            }
        },
        scene: get("--scene").expect("--scene <rel.glb>"),
        render: get("--render"),
        side: a.iter().any(|x| x == "--side"),
        origin: Vec2::new(ox, oy),
        scale: get("--scale").map_or(20.0, |s| s.parse().unwrap()),
        size: UVec2::new(w as u32, h as u32),
        anim: get("--anim").map(|s| s.parse().unwrap()),
        time: get("--time").map_or(0.0, |s| s.parse().unwrap()),
        opaque: a.iter().any(|x| x == "--opaque"),
        snapshot: get("--snapshot"),
    }
}

#[derive(Resource)]
struct Loading {
    gltf: Handle<Gltf>,
    scene: Handle<bevy::world_serialization::WorldAsset>,
    graph: Option<(Handle<AnimationGraph>, Vec<AnimationNodeIndex>)>,
    current: usize,
    frames_after_load: u32,
    shot: bool,
}

/// End the process after the app loop, without process exit handlers.
///
/// Dropping the app (which finishes pending tasks such as saving screenshots)
/// leaves the render thread still releasing the GPU device. A normal exit would
/// run the NVIDIA driver's exit handlers at the same moment, and it crashes in
/// that race. Everything worth keeping is saved by now, and the operating system
/// reclaims the rest.
fn finish_process(app: App) -> ! {
    use std::io::Write;
    drop(app);
    std::io::stdout().flush().ok();
    std::io::stderr().flush().ok();
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn _exit(status: i32) -> !;
        }
        // SAFETY: `_exit` takes any status and never returns.
        unsafe { _exit(0) }
    }
    #[cfg(not(unix))]
    std::process::exit(0)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.iter().any(|a| a == "--controller-preview") {
        controller::preview(&a);
        return;
    }
    #[cfg(feature = "main-menu")]
    if a.len() == 1 || a.iter().any(|a| a == "--menu") {
        // Play sessions run inside the menu's window and return to it.
        if let Err(e) = frontend::run(&a) {
            eprintln!("menu: {e}");
            std::process::exit(2);
        }
        return;
    }
    if let Some(i) = a.iter().position(|x| x == "--intro-preview") {
        if let Err(error) = frontend_cinema::run(&a[i + 1], &a) {
            eprintln!("intro: {error}");
            std::process::exit(2);
        }
        return;
    }
    if let Some(i) = a.iter().position(|x| x == "--play") {
        let root = a.get(i + 1).expect("--play <original Content root>");
        if let Err(error) = play::run(root, &a) {
            eprintln!("play: {error}");
            std::process::exit(2);
        }
        return;
    }
    if let Some(i) = a.iter().position(|x| x == "--error-box-preview") {
        if let Err(error) = error_box_preview::preview(&a[i + 1], &a) {
            eprintln!("error box preview: {error}");
            std::process::exit(2);
        }
        return;
    }
    if let Some(i) = a.iter().position(|x| x == "--text-input-preview") {
        if let Err(error) = text_input_preview::preview(&a[i + 1], &a) {
            eprintln!("text input preview: {error}");
            std::process::exit(2);
        }
        return;
    }
    if let Some(i) = a.iter().position(|x| x == "--textbox-preview") {
        if let Err(error) = text_box_visual::preview(&a[i + 1], &a) {
            eprintln!("textbox preview: {error}");
            std::process::exit(2);
        }
        return;
    }
    if let Some(i) = a.iter().position(|x| x == "--dialog-preview") {
        let root = a
            .get(i + 1)
            .expect("--dialog-preview <original Content root>");
        if let Err(error) = live_dialog_preview::preview(root, &a) {
            eprintln!("dialog preview: {error}");
            std::process::exit(2);
        }
        return;
    }
    if let Some(i) = a.iter().position(|x| x == "--font-preview") {
        if let Err(error) = native_text::preview(&a[i + 1], &a) {
            eprintln!("font preview: {error}");
            std::process::exit(2);
        }
        return;
    }
    if let Some(i) = a.iter().position(|x| x == "--particle-test") {
        let scene = a
            .get(i + 1)
            .expect("--particle-test <scene.json> <out.png>");
        let out = a
            .get(i + 2)
            .expect("--particle-test <scene.json> <out.png>");
        if let Err(e) = particles::particle_test(scene, out.clone()) {
            eprintln!("particle test: {e}");
            std::process::exit(2);
        }
        return;
    }
    if let Some(i) = a.iter().position(|x| x == "--terrain-test") {
        let flags: u32 = a
            .iter()
            .position(|x| x == "--tflags")
            .map_or(0, |j| a[j + 1].parse().unwrap());
        let sharp: f32 = a
            .iter()
            .position(|x| x == "--sharp")
            .map_or(1.0, |j| a[j + 1].parse().unwrap());
        terrain_test(a[i + 1].clone(), flags, sharp);
        return;
    }
    if let Some(i) = a.iter().position(|v| v == "--outline-check") {
        outline_check::run(a[i + 1].clone(), a.iter().any(|v| v == "--msaa4"));
        return;
    }
    let args = parse_args();
    let headless = args.render.is_some();
    let mut app = App::new();
    let asset = AssetPlugin {
        file_path: args.root.clone(),
        ..default()
    };
    if headless {
        app.add_plugins(
            DefaultPlugins
                .set(asset)
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        )
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / 120.0,
        )));
    } else {
        app.add_plugins(DefaultPlugins.set(asset).set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("magicka-viewer — {}", args.scene),
                ..default()
            }),
            ..default()
        }));
    }
    // Magenta only for headless mask checks; neutral grey for people.
    let clear = if headless {
        Color::srgb(1.0, 0.0, 1.0)
    } else {
        Color::srgb(0.18, 0.18, 0.2)
    };
    app.insert_resource(ClearColor(clear))
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 1500.0,
            ..default()
        })
        .insert_resource(ScenePath(args.scene.clone()))
        .insert_resource(TerrainUnlit(std::env::args().any(|x| x == "--unlit")))
        .insert_resource(TerrainNoDiscard(args.opaque))
        .add_plugins(TerrainPlugin)
        .add_plugins(level_animation::LevelAnimationPlugin)
        .add_plugins(native_text::NativeTextPlugin)
        .add_plugins(text_box_visual::TextBoxBackgroundPlugin)
        .insert_resource(args)
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                wait_and_animate,
                controls,
                force_opaque,
                auto_frame,
                snapshot,
            ),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    args: Res<Args>,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
) {
    let gltf: Handle<Gltf> = assets.load(args.scene.clone());
    let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(args.scene.clone()));
    commands.spawn((lighting::OutlineOccluder, WorldAssetRoot(scene.clone())));
    commands.insert_resource(Loading {
        gltf,
        scene,
        graph: None,
        current: args.anim.unwrap_or(0),
        frames_after_load: 0,
        shot: false,
    });
    commands.spawn((
        DirectionalLight {
            illuminance: 4000.0,
            ..default()
        },
        Transform::from_xyz(3.0, 10.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    let w = args.size.x as f32 / args.scale;
    let h = args.size.y as f32 / args.scale;
    let projection = Projection::Orthographic(OrthographicProjection {
        scaling_mode: ScalingMode::Fixed {
            width: w,
            height: h,
        },
        near: -10_000.0,
        far: 10_000.0,
        ..OrthographicProjection::default_3d()
    });
    // Camera placed at the frame center; see module docs for axis mapping.
    let transform = if args.side {
        Transform::from_xyz(args.origin.x + w / 2.0, args.origin.y - h / 2.0, 1000.0)
            .looking_to(-Vec3::Z, Vec3::Y)
    } else {
        Transform::from_xyz(args.origin.x + w / 2.0, 1000.0, args.origin.y + h / 2.0)
            .looking_to(-Vec3::Y, -Vec3::Z)
    };
    if args.render.is_none() {
        // Interactive: perspective orbit camera, framed on the scene once loaded.
        commands.insert_resource(Orbit {
            target: Vec3::ZERO,
            distance: 10.0,
            yaw: 0.6,
            pitch: 0.75,
            framed: false,
        });
        commands.spawn((
            Camera3d::default(),
            Projection::Perspective(PerspectiveProjection {
                far: 5000.0,
                ..default()
            }),
            Transform::from_xyz(0.0, 5.0, 10.0),
        ));
        return;
    }
    let mut camera = commands.spawn((Camera3d::default(), projection, transform));
    if args.render.is_some() {
        let mut image = Image::new_target_texture(
            args.size.x,
            args.size.y,
            TextureFormat::Rgba8UnormSrgb,
            None,
        );
        image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        let handle = images.add(image);
        camera.insert(RenderTarget::Image(ImageRenderTarget {
            handle: handle.clone(),
            scale_factor: 1.0,
        }));
        commands.insert_resource(TargetImage(handle));
    }
}

#[derive(Resource)]
struct TargetImage(Handle<Image>);

fn force_opaque(args: Res<Args>, mut materials: ResMut<Assets<StandardMaterial>>) {
    if !args.opaque {
        return;
    }
    for (_, m) in materials.iter_mut() {
        if m.alpha_mode != AlphaMode::Opaque {
            m.alpha_mode = AlphaMode::Opaque;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn wait_and_animate(
    mut commands: Commands,
    args: Res<Args>,
    assets: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut state: ResMut<Loading>,
    mut players: Query<(Entity, &mut AnimationPlayer)>,
    target: Option<Res<TargetImage>>,
) {
    if !assets.is_loaded_with_dependencies(&state.scene) {
        return;
    }
    // Build the animation graph once the glTF (and its clips) are loaded.
    if state.graph.is_none() {
        if let Some(gltf) = gltfs.get(&state.gltf) {
            let (graph, nodes) = AnimationGraph::from_clips(gltf.animations.iter().cloned());
            state.graph = Some((graphs.add(graph), nodes));
        }
    }
    if let Some((graph, nodes)) = &state.graph {
        for (entity, mut player) in &mut players {
            if args.native_level_animation {
                player.stop_all();
                continue;
            }
            if nodes.is_empty() {
                continue;
            }
            let node = nodes[state.current % nodes.len()];
            if !player.is_playing_animation(node) {
                player.stop_all();
                commands
                    .entity(entity)
                    .insert(AnimationGraphHandle(graph.clone()));
                let active = player.start(node);
                if args.render.is_some() {
                    active.seek_to(args.time).pause();
                } else {
                    active.repeat();
                }
            }
        }
    }
    state.frames_after_load += 1;
    // Give pipelines time to compile before capturing.
    if let (Some(out), Some(target)) = (&args.render, &target) {
        if state.frames_after_load == 240 && !state.shot {
            state.shot = true;
            commands
                .spawn(Screenshot::image(target.0.clone()))
                .observe(save_to_disk(out.clone()))
                .observe(
                    |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                        exit.write(AppExit::Success);
                    },
                );
        }
        if state.frames_after_load > 2000 {
            eprintln!("timeout waiting for screenshot");
            std::process::exit(2);
        }
    }
}

fn snapshot(
    mut commands: Commands,
    args: Res<Args>,
    orbit: Option<Res<Orbit>>,
    mut wait: Local<u32>,
) {
    let (Some(path), Some(orbit)) = (&args.snapshot, orbit) else {
        return;
    };
    if !orbit.framed {
        return;
    }
    *wait += 1;
    if *wait == 180 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()))
            .observe(
                |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                    exit.write(AppExit::Success);
                },
            );
    }
}

/// Interactive orbit camera state.
#[derive(Resource)]
struct Orbit {
    target: Vec3,
    distance: f32,
    yaw: f32,
    pitch: f32,
    framed: bool,
}

/// Aim the interactive camera at the world bounds of all meshes once the
/// scene has loaded (and again when `Home` is pressed).
fn auto_frame(
    orbit: Option<ResMut<Orbit>>,
    state: Res<Loading>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    meshes: Query<(&GlobalTransform, &bevy::camera::primitives::Aabb), With<Mesh3d>>,
) {
    let Some(mut orbit) = orbit else { return };
    let refit = keys.is_some_and(|k| k.just_pressed(KeyCode::Home));
    if (orbit.framed && !refit) || state.frames_after_load < 10 {
        return;
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for (gt, aabb) in &meshes {
        let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
        for i in 0..8 {
            let corner = c + h * Vec3::new(
                if i & 1 == 0 { -1.0 } else { 1.0 },
                if i & 2 == 0 { -1.0 } else { 1.0 },
                if i & 4 == 0 { -1.0 } else { 1.0 },
            );
            let w = gt.transform_point(corner);
            lo = lo.min(w);
            hi = hi.max(w);
        }
    }
    if lo.x > hi.x {
        return; // nothing spawned yet
    }
    orbit.target = (lo + hi) / 2.0;
    orbit.distance = ((hi - lo).length() * 0.9).max(1.0);
    orbit.framed = true;
    info!("framed scene: bounds {lo} .. {hi}");
}

fn controls(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    time: Res<Time>,
    mut state: ResMut<Loading>,
    orbit: Option<ResMut<Orbit>>,
    mut cams: Query<&mut Transform, With<Camera3d>>,
    gltfs: Res<Assets<Gltf>>,
) {
    let (Some(keys), Some(mut orbit)) = (keys, orbit) else {
        return;
    };
    let dt = time.delta_secs();
    let fwd = Vec3::new(-orbit.yaw.sin(), 0.0, -orbit.yaw.cos());
    let right = Vec3::new(orbit.yaw.cos(), 0.0, -orbit.yaw.sin());
    let pan = orbit.distance * 0.8 * dt;
    if keys.any_pressed([KeyCode::KeyW, KeyCode::ArrowUp]) {
        orbit.target += fwd * pan;
    }
    if keys.any_pressed([KeyCode::KeyS, KeyCode::ArrowDown]) {
        orbit.target -= fwd * pan;
    }
    if keys.any_pressed([KeyCode::KeyD, KeyCode::ArrowRight]) {
        orbit.target += right * pan;
    }
    if keys.any_pressed([KeyCode::KeyA, KeyCode::ArrowLeft]) {
        orbit.target -= right * pan;
    }
    if keys.pressed(KeyCode::KeyQ) {
        orbit.distance *= 1.0 + 1.5 * dt;
    }
    if keys.pressed(KeyCode::KeyE) {
        orbit.distance /= 1.0 + 1.5 * dt;
    }
    if keys.pressed(KeyCode::KeyZ) {
        orbit.yaw += 1.5 * dt;
    }
    if keys.pressed(KeyCode::KeyC) {
        orbit.yaw -= 1.5 * dt;
    }
    if keys.pressed(KeyCode::KeyR) {
        orbit.pitch = (orbit.pitch + dt).min(1.55);
    }
    if keys.pressed(KeyCode::KeyF) {
        orbit.pitch = (orbit.pitch - dt).max(0.05);
    }
    let offset = Vec3::new(
        orbit.yaw.sin() * orbit.pitch.cos(),
        orbit.pitch.sin(),
        orbit.yaw.cos() * orbit.pitch.cos(),
    ) * orbit.distance;
    for mut t in &mut cams {
        *t = Transform::from_translation(orbit.target + offset).looking_at(orbit.target, Vec3::Y);
    }
    if keys.just_pressed(KeyCode::KeyN) {
        state.current += 1;
        if let Some(g) = gltfs.get(&state.gltf) {
            let n = g.animations.len().max(1);
            let name = g
                .named_animations
                .iter()
                .find(|(_, h)| g.animations.get(state.current % n) == Some(*h))
                .map(|(k, _)| k.to_string());
            info!("animation {} / {}: {:?}", state.current % n, n, name);
        }
    }
}

/// GPU parity test of `terrain.wgsl`: a 256x256 orthographic view of one quad
/// whose attributes are affine in screen position (uv0 = 2*p, uv1 = 3*p.yx,
/// color = (p.x, p.y, 0.5, 0.6 p.x + 0.4 p.y), p = world/256) and two 64x64
/// deterministic textures sampled nearest/repeat. `tools/terrain_check.py`
/// computes the expected image from the same definitions.
fn terrain_test(out: String, flags: u32, sharp: f32) {
    use bevy::asset::RenderAssetUsages;
    use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
    use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
    use bevy::mesh::{Indices, PrimitiveTopology};
    use bevy::render::render_resource::{Extent3d, TextureDimension};

    #[derive(Resource)]
    struct T(String, u32, f32, Handle<Image>);
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
    .add_plugins(TerrainPlugin)
    .insert_resource(ClearColor(Color::srgb(1.0, 0.0, 1.0)))
    .insert_resource(T(out, flags, sharp, Handle::default()))
    .add_systems(
        Startup,
        |mut commands: Commands,
         mut t: ResMut<T>,
         mut images: ResMut<Assets<Image>>,
         mut meshes: ResMut<Assets<Mesh>>,
         mut mats: ResMut<Assets<TerrainMaterial>>,
         lights: Res<lighting::MagickaLights>,
         cutaway: Res<cutaway::CutawayBindings>| {
            let tex = |seed: u32, images: &mut Assets<Image>| {
                let mut img = Image::new(
                    Extent3d {
                        width: 64,
                        height: 64,
                        depth_or_array_layers: 1,
                    },
                    TextureDimension::D2,
                    terrain::test_texture_bytes(seed, 64),
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::RENDER_WORLD,
                );
                img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                    address_mode_u: ImageAddressMode::Repeat,
                    address_mode_v: ImageAddressMode::Repeat,
                    ..ImageSamplerDescriptor::nearest()
                });
                images.add(img)
            };
            let (t0, t1) = (tex(1, &mut images), tex(2, &mut images));
            // Quad in world XY covering [0,256]^2; attributes affine in position.
            let corners = [[0.0f32, 0.0], [256.0, 0.0], [256.0, 256.0], [0.0, 256.0]];
            let p = |c: [f32; 2]| [c[0] / 256.0, c[1] / 256.0];
            let mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD,
            )
            .with_inserted_attribute(
                Mesh::ATTRIBUTE_POSITION,
                corners
                    .iter()
                    .map(|c| [c[0], c[1], 0.0])
                    .collect::<Vec<_>>(),
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 0.0, 1.0]; 4])
            .with_inserted_attribute(
                Mesh::ATTRIBUTE_UV_0,
                corners
                    .iter()
                    .map(|c| {
                        let q = p(*c);
                        [2.0 * q[0], 2.0 * q[1]]
                    })
                    .collect::<Vec<_>>(),
            )
            .with_inserted_attribute(
                Mesh::ATTRIBUTE_UV_1,
                corners
                    .iter()
                    .map(|c| {
                        let q = p(*c);
                        [3.0 * q[1], 3.0 * q[0]]
                    })
                    .collect::<Vec<_>>(),
            )
            .with_inserted_attribute(
                Mesh::ATTRIBUTE_COLOR,
                corners
                    .iter()
                    .map(|c| {
                        let q = p(*c);
                        [q[0], q[1], 0.5, 0.6 * q[0] + 0.4 * q[1]]
                    })
                    .collect::<Vec<_>>(),
            )
            .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
            let mat = mats.add(TerrainMaterial {
                tex0: Some(t0),
                tex1: Some(t1),
                ..TerrainMaterial::new(
                    TerrainParams {
                        color0: Vec4::new(1.0, 0.9, 0.8, 1.0),
                        color1: Vec4::new(0.7, 1.0, 1.2, 1.0),
                        flags: UVec4::new(t.1, 0, 0, 0),
                        misc: Vec4::new(t.2, 0.0, 0.0, 0.0),
                        spec0: Vec4::ZERO,
                        spec1: Vec4::ZERO,
                    },
                    &lights,
                    &cutaway,
                )
            });
            commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(mat)));
            let mut image =
                Image::new_target_texture(256, 256, TextureFormat::Rgba8UnormSrgb, None);
            image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
            let target = images.add(image);
            t.3 = target.clone();
            commands.spawn((
                Camera3d::default(),
                Tonemapping::None,
                DebandDither::Disabled,
                Msaa::Off,
                Projection::Orthographic(OrthographicProjection {
                    scaling_mode: ScalingMode::Fixed {
                        width: 256.0,
                        height: 256.0,
                    },
                    near: -100.0,
                    far: 100.0,
                    ..OrthographicProjection::default_3d()
                }),
                Transform::from_xyz(128.0, 128.0, 10.0).looking_to(-Vec3::Z, Vec3::Y),
                RenderTarget::Image(ImageRenderTarget {
                    handle: target,
                    scale_factor: 1.0,
                }),
            ));
        },
    )
    .add_systems(
        Update,
        |mut commands: Commands, t: Res<T>, mut n: Local<u32>| {
            *n += 1;
            if *n == 120 {
                commands
                    .spawn(Screenshot::image(t.3.clone()))
                    .observe(save_to_disk(t.0.clone()))
                    .observe(
                        |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                            exit.write(AppExit::Success);
                        },
                    );
            }
        },
    )
    .run();
}
