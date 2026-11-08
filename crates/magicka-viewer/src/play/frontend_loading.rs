//! Native loading backdrop/tip and a readiness gate before simulation starts.
use super::*;
use bevy::{
    audio::{AudioPlayer, AudioSink, AudioSinkPlayback, PlaybackMode, PlaybackSettings, Volume},
    camera::visibility::RenderLayers,
};
use magicka_runtime::{
    audio::AudioLibrary,
    frontend_loading::{LoadingGate, Phase},
};

const LAYER: usize = 31;
#[derive(Component)]
struct LoadingCamera;
#[derive(Resource)]
pub(super) struct Loading {
    gate: LoadingGate,
    background: magicka_content::assets::Texture2D,
    /// ShowProgress: a random UI/Loading/images picture and the progress
    /// bar from UI/HUD/hud.
    image: Option<magicka_content::assets::Texture2D>,
    hud: Option<magicka_content::assets::Texture2D>,
    /// LoadingScreen.Progress, from the loading stages reached.
    progress: f32,
    font: magicka_content::assets::BitmapFont,
    tip: String,
    cue: Option<i32>,
    library: Arc<AudioLibrary>,
    silent: bool,
    started: bool,
    age: f32,
    voices: Vec<Voice>,
    root: Option<Entity>,
    output: Option<PathBuf>,
    records: Vec<serde_json::Value>,
    previous: Option<Phase>,
    capture_pending: bool,
    capture_done: bool,
    waiting_frames: u32,
    game_frames: u32,
}
struct Voice {
    entity: Option<Entity>,
    clip: Handle<crate::pcm_audio::PcmClip>,
    delay: f32,
    duration: f32,
    speed: f32,
    gain: f32,
    heard: bool,
    finished: bool,
}
#[derive(Resource)]
struct Draw {
    background: Entity,
    tip: Entity,
    shade: Entity,
    /// LoadingScreen.Draw's Clear(Black) under everything it draws.
    backdrop: Entity,
    /// The progress bar's fill, scaled by Progress.
    fill: Option<Entity>,
}

/// LoadingScreen.CreateVertices: the bar's frame (HUD rows 48-72), fill
/// (76-84 x 72-96, 608 wide) and overlay (rows 24-48), 24 pixels high,
/// centred 112 pixels below the middle of the screen.
const BAR_Y: f32 = -112.0;
const BAR_SLICES: [([f32; 2], [f32; 2]); 3] = [
    ([-320.0, -224.0], [0.0, 96.0]),
    ([-224.0, 224.0], [96.0, 160.0]),
    ([224.0, 320.0], [160.0, 256.0]),
];
const FILL_LEFT: f32 = -320.0 + 16.0;
const FILL_WIDTH: f32 = 608.0;

/// A unit quad showing `uv` (pixels) of a `size` texture.
fn uv_quad(uv: [f32; 4], size: [f32; 2]) -> Mesh {
    let mut mesh = Mesh::from(Rectangle::new(1.0, 1.0));
    let [u0, v0, u1, v1] = [
        uv[0] / size[0],
        uv[1] / size[1],
        uv[2] / size[0],
        uv[3] / size[1],
    ];
    // Rectangle's corners: top-right, top-left, bottom-left, bottom-right.
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[u1, v0], [u0, v0], [u0, v1], [u1, v1]],
    );
    mesh
}

pub(super) fn ready(loading: Option<Res<Loading>>) -> bool {
    loading.is_none_or(|s| !s.gate.blocks_play())
}

pub(super) fn plugin(app: &mut App) {
    crate::pcm_audio::register(app);
    // Not play-gated: a host presents loading before the session exists.
    app.add_systems(Update, drive.before(super::step_session));
}

/// Show the loading screen (with the opening's final narration when the
/// intro handed it over). Idempotent: a host presents it before the session
/// is prepared, and the session start then finds it already up.
pub(crate) fn present(
    world: &mut World,
    content: &std::path::Path,
    args: &[String],
) -> Result<(), String> {
    if world.contains_resource::<Loading>()
        || !args
            .iter()
            .any(|a| a == "--menu-loading" || a == "--loading-intro-tip")
    {
        return Ok(());
    }
    if !world.contains_resource::<super::save_slots::UserVolumes>() {
        let cache = arg(args, "--root").unwrap_or_else(|| "output/.cache".into());
        world.insert_resource(super::save_slots::volumes(std::path::Path::new(&cache))?);
    }
    let library =
        super::audio::library(world).unwrap_or_else(|| Arc::new(AudioLibrary::load(content)));
    let background_path = magicka_runtime::play_content::resolve(
        content,
        std::path::Path::new("UI/Loading/background.xnb"),
    )
    .ok_or("loading background missing")?;
    let Asset::Texture2D(background) = read_asset(&background_path)? else {
        return Err("expected loading background".into());
    };
    let font = magicka_runtime::play_content::bitmap_font(content, "Languages/eng/Font/Maiandra14")
        .map_err(|e| format!("loading font: {e:?}"))?;
    let index = arg(args, "--loading-intro-tip")
        .map(|s| s.parse::<usize>().map_err(|e| format!("intro tip: {e}")))
        .transpose()?;
    if index.is_some_and(|i| i != 7) {
        return Err("opening loading tip must be segment 7".into());
    }
    let strings = magicka_runtime::play_content::english_strings(content, &["Player"]);
    // PlayState's default: "#tip" over a random #tip01..#tip22, with the
    // ShowProgress picture. The opening hands over its own tip instead.
    let random = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos() as usize);
    let image = if index.is_none() {
        let dir = content.join("UI/Loading/images");
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xnb")))
            .collect();
        files.sort();
        match files
            .get(random % files.len().max(1))
            .map(|p| read_asset(p))
        {
            Some(Ok(Asset::Texture2D(texture))) => Some(texture),
            Some(Err(e)) => {
                warn!("loading image: {e}");
                None
            }
            _ => None,
        }
    } else {
        None
    };
    let hud = image.as_ref().and_then(|_| {
        let path = magicka_runtime::play_content::resolve(
            content,
            std::path::Path::new("UI/HUD/hud.xnb"),
        )?;
        match read_asset(&path) {
            Ok(Asset::Texture2D(texture)) => Some(texture),
            _ => None,
        }
    });
    let tip = if let Some(index) = index {
        let text = strings
            .get_string(magicka_runtime::scene_loading::custom_hash(
                &magicka_runtime::frontend_intro::IntroPlayback::text_key(index),
            ))
            .ok_or("loading intro text missing")?;
        magicka_runtime::bitmap_font::FontMetrics::from_content(&font)
            .map_err(|e| format!("loading font: {e:?}"))?
            .wrap(&text, 700, true)
            .map_err(|e| format!("loading tip: {e:?}"))?
            .text
    } else {
        let number = random / 7 % 22 + 1;
        let key = format!("#tip{number:02}");
        let mut text = strings
            .get_string(magicka_runtime::scene_loading::custom_hash(&key))
            .unwrap_or_default();
        if number == 6 {
            // The keyboard's boost glyph (a gamepad would show its button).
            text = text.replace("#KEY_BOOST;", "\u{032a}");
        }
        let heading = strings
            .get_string(magicka_runtime::scene_loading::custom_hash("#tip"))
            .unwrap_or_default();
        magicka_runtime::bitmap_font::FontMetrics::from_content(&font)
            .map_err(|e| format!("loading font: {e:?}"))?
            .wrap(&format!("{heading}\n{text}"), 700, true)
            .map_err(|e| format!("loading tip: {e:?}"))?
            .text
    };
    let cue = index.map(|i| {
        magicka_runtime::scene_loading::custom_hash(
            &magicka_runtime::frontend_intro::IntroPlayback::narration_name(i),
        )
    });
    let output = arg(args, "--loading-audit").map(PathBuf::from);
    if let Some(dir) = &output {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    world.insert_resource(Loading {
        gate: LoadingGate::default(),
        background,
        image,
        hud,
        progress: 0.0,
        font,
        tip,
        cue,
        library,
        silent: args.iter().any(|a| a == "--silent"),
        started: false,
        age: 0.0,
        voices: Vec::new(),
        root: None,
        output,
        records: Vec::new(),
        previous: None,
        capture_pending: false,
        capture_done: false,
        waiting_frames: 0,
        game_frames: 0,
    });
    world
        .run_system_cached(setup)
        .map_err(|e| format!("loading screen: {e}"))?;
    Ok(())
}

fn setup(
    mut commands: Commands,
    mut loading: ResMut<Loading>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut clips: ResMut<Assets<crate::pcm_audio::PcmClip>>,
    volumes: Res<super::save_slots::UserVolumes>,
) {
    use bevy::{
        asset::RenderAssetUsages,
        image::{ImageSampler, ImageSamplerDescriptor},
        render::render_resource::{Extent3d, TextureDimension},
    };
    let mut upload = |b: &magicka_content::assets::Texture2D| {
        let rgba = magicka_content::texture::decode_rgba(
            b.format,
            b.width as usize,
            b.height as usize,
            &b.mips[0],
        )
        .expect("loading texture decoding");
        let mut image = Image::new(
            Extent3d {
                width: b.width as u32,
                height: b.height as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            rgba,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
        images.add(image)
    };
    let texture = upload(&loading.background);
    let picture = loading
        .image
        .as_ref()
        .map(|i| (upload(i), Vec3::new(i.width as f32, i.height as f32, 1.0)));
    let background = commands
        .spawn((
            Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
            MeshMaterial2d(materials.add(ColorMaterial {
                texture: Some(texture),
                ..default()
            })),
            Transform::default(),
            RenderLayers::layer(LAYER),
        ))
        .id();
    let image = picture.map(|(texture, size)| {
        commands
            .spawn((
                Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
                MeshMaterial2d(materials.add(ColorMaterial {
                    texture: Some(texture),
                    ..default()
                })),
                // LoadingScreen.Draw: centred, 100 pixels above the middle.
                Transform::from_xyz(0.0, 100.0, 1.0).with_scale(size),
                RenderLayers::layer(LAYER),
            ))
            .id()
    });
    let mut fill = None;
    if let Some(hud) = loading.hud.as_ref() {
        let dims = [hud.width as f32, hud.height as f32];
        let texture = upload(hud);
        let mut quad = |uv: [f32; 4], x: [f32; 2], z: f32, color: Color| {
            commands
                .spawn((
                    Mesh2d(meshes.add(uv_quad(uv, dims))),
                    MeshMaterial2d(materials.add(ColorMaterial {
                        texture: Some(texture.clone()),
                        color,
                        ..default()
                    })),
                    Transform::from_xyz((x[0] + x[1]) * 0.5, BAR_Y, z).with_scale(Vec3::new(
                        x[1] - x[0],
                        24.0,
                        1.0,
                    )),
                    RenderLayers::layer(LAYER),
                ))
                .id()
        };
        for (x, u) in BAR_SLICES {
            quad([u[0], 48.0, u[1], 72.0], x, 1.0, Color::WHITE);
        }
        // mHealthColor.
        fill = Some(quad(
            [76.0, 72.0, 84.0, 96.0],
            [FILL_LEFT, FILL_LEFT],
            1.1,
            Color::srgb(1.0, 0.0, 0.0),
        ));
        for (x, u) in BAR_SLICES {
            quad([u[0], 24.0, u[1], 48.0], x, 1.2, Color::WHITE);
        }
    }
    let (font, material) =
        upload_font(&loading.font, &mut images, &mut materials).expect("loading font");
    let geometry = magicka_runtime::text_geometry::TextGeometry::build(
        &font,
        &loading.tip.encode_utf16().collect::<Vec<_>>(),
        TextStyle {
            align: magicka_runtime::text_geometry::TextAlign::Center,
            ..default()
        },
    )
    .expect("loading text");
    let height = font.metrics.measure(&loading.tip, true).unwrap()[1];
    let tip = commands
        .spawn((
            Mesh2d(meshes.add(
                crate::native_text::geometry_mesh(&geometry, geometry.quads.len() as i32).unwrap(),
            )),
            MeshMaterial2d(material),
            Transform::from_xyz(0.0, height + font.metrics.line_height as f32 * 2.0, 2.0),
            RenderLayers::layer(LAYER),
        ))
        .id();
    let shade = commands
        .spawn((
            Sprite::from_color(Color::BLACK, Vec2::ONE),
            Transform::from_xyz(0.0, 0.0, 3.0),
            RenderLayers::layer(LAYER),
        ))
        .id();
    let backdrop = commands
        .spawn((
            Sprite::from_color(Color::BLACK, Vec2::ONE),
            Transform::from_xyz(0.0, 0.0, -1.0),
            RenderLayers::layer(LAYER),
        ))
        .id();
    let _ = image;
    commands.insert_resource(Draw {
        background,
        tip,
        shade,
        backdrop,
        fill,
    });
    let camera = commands
        .spawn((
            Camera2d,
            LoadingCamera,
            RenderLayers::layer(LAYER),
            Camera {
                order: 200,
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
        ))
        .id();
    loading.root = Some(camera);
    if let Some(cue) = loading.cue {
        let library = loading.library.clone();
        let voices = library
            .resolve(1, cue, &mut || 0.5)
            .expect("intro final cue missing");
        for voice in voices {
            let pcm = library
                .try_decode(&voice.wave_bank, voice.track)
                .expect("intro final wave");
            assert_ne!(
                voice.loop_count, 255,
                "loading narration cannot loop forever"
            );
            let speed = 2.0_f32.powf((voice.pitch + library.rpc_pitch(&voice, &|_| None)) / 12.0);
            let duration = pcm.frames() as f32 / pcm.sample_rate as f32
                * (voice.loop_count as f32 + 1.0)
                / speed;
            let gain = 10.0_f32.powf(library.gain_db(&voice, &|_| None) / 20.0) * volumes.sfx;
            loading.voices.push(Voice {
                entity: None,
                clip: clips.add(crate::pcm_audio::PcmClip::from_pcm(pcm, voice.loop_count)),
                delay: voice.delay,
                duration,
                speed,
                gain,
                heard: false,
                finished: false,
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    loading: Option<ResMut<Loading>>,
    draw: Option<Res<Draw>>,
    play: Option<Res<Play>>,
    assets: Res<AssetServer>,
    avatar: Option<Res<AvatarAssets>>,
    animators: Query<(), With<AvatarAnimator>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    time: Res<Time>,
    mut cameras: Query<&mut Camera, With<LoadingCamera>>,
    mut transforms: Query<&mut Transform>,
    mut sprites: Query<&mut Sprite>,
    sinks: Query<&AudioSink>,
    entities: Query<Entity>,
) {
    let (Some(mut loading), Some(draw)) = (loading, draw) else {
        return;
    };
    // The session may still be preparing on a host's worker thread.
    let ready = play.is_some()
        && avatar.is_some_and(|avatar| {
            assets.is_loaded_with_dependencies(&avatar.scene)
                && avatar
                    .level
                    .as_ref()
                    .is_none_or(|l| assets.is_loaded_with_dependencies(l))
        })
        && !animators.is_empty();
    let size = windows
        .single()
        .map(|w| Vec2::new(w.width(), w.height()))
        .unwrap_or(AUTOPILOT_SIZE.as_vec2());
    // LoadingScreen.Draw clears the screen and presents on its own: the
    // window, above the game and its overlays, while it blocks play.
    for mut camera in &mut cameras {
        camera.is_active = loading.gate.blocks_play();
    }
    transforms.get_mut(draw.background).unwrap().scale = Vec3::new(size.y, size.y, 1.0);
    let mut tip = transforms.get_mut(draw.tip).unwrap();
    let height = magicka_runtime::bitmap_font::FontMetrics::from_content(&loading.font)
        .unwrap()
        .measure(&loading.tip, true)
        .unwrap()[1];
    tip.translation.y = -size.y * 0.5 + height + loading.font.line_height as f32 * 2.0;
    if loading.capture_pending {
        return;
    }
    if !loading.started {
        loading.started = true;
    }
    loading.age += time.delta_secs();
    let age = loading.age;
    let silent = loading.silent;
    for voice in &mut loading.voices {
        if voice.finished || age < voice.delay {
            continue;
        }
        if !silent && voice.entity.is_none() {
            voice.entity = Some(
                commands
                    .spawn((
                        AudioPlayer(voice.clip.clone()),
                        PlaybackSettings {
                            mode: PlaybackMode::Despawn,
                            volume: Volume::Linear(voice.gain),
                            speed: voice.speed,
                            ..default()
                        },
                    ))
                    .id(),
            );
        }
        if let Some(entity) = voice.entity {
            if let Ok(sink) = sinks.get(entity) {
                voice.heard = true;
                voice.finished = sink.empty();
            } else if voice.heard && entities.get(entity).is_err() {
                voice.finished = true;
            }
        }
        // With --silent or no audio device, use the actual decoded wave length;
        // an existing audio sink always owns completion once playback starts.
        if !voice.heard && age >= voice.delay + voice.duration {
            voice.finished = true;
        }
    }
    let cue_finished = loading.voices.iter().all(|v| v.finished);
    let capture_ready = loading.output.is_none() || loading.capture_done;
    loading
        .gate
        .update(time.delta_secs(), ready && capture_ready, cue_finished);
    let alpha = loading.gate.alpha;
    sprites.get_mut(draw.backdrop).unwrap().custom_size = Some(size);
    // Progress by stage: preparing the session, assets loading, ready (the
    // original sets fixed values through PlayState's load steps).
    let target = if ready {
        1.0
    } else if play.is_some() {
        0.75
    } else {
        (loading.age * 0.1).min(0.5)
    };
    loading.progress += (target - loading.progress) * (time.delta_secs() * 3.0).min(1.0);
    if let Some(fill) = draw.fill {
        let width = loading.progress * FILL_WIDTH;
        let mut t = transforms.get_mut(fill).unwrap();
        t.translation.x = FILL_LEFT + width * 0.5;
        t.scale.x = width.max(0.001);
    }
    let mut shade = sprites.get_mut(draw.shade).unwrap();
    shade.custom_size = Some(size);
    shade.color = Color::srgba(0.0, 0.0, 0.0, 1.0 - alpha);
    let phase = loading.gate.phase;
    if loading.previous != Some(phase) {
        loading.previous = Some(phase);
        let record = serde_json::json!({"time":loading.age,"phase":format!("{phase:?}"),"assets_ready":ready,"cue_finished":cue_finished,"intro_tip":loading.cue.is_some(),"world_scene":play.as_ref().map(|p|p.session.scene_name()),"voice_seconds":loading.voices.iter().map(|v|v.delay+v.duration).collect::<Vec<_>>()});
        loading.records.push(record);
        if let Some(dir) = &loading.output {
            std::fs::write(
                dir.join("loading.json"),
                serde_json::to_vec_pretty(&loading.records).unwrap(),
            )
            .expect("loading audit");
        }
    }
    if phase == Phase::Complete {
        for voice in &loading.voices {
            if let Some(entity) = voice.entity {
                if entities.get(entity).is_ok() {
                    commands.entity(entity).despawn();
                }
            }
        }
        loading.game_frames += 1;
        if loading.game_frames == 60 && loading.output.is_some() {
            let dir = loading.output.clone().unwrap();
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(dir.join("playing.png")))
                .observe(
                    |_: On<ScreenshotCaptured>, mut leave: MessageWriter<super::LeaveSession>| {
                        leave.write(super::LeaveSession::default());
                    },
                );
        }
    } else if phase == Phase::Waiting && loading.output.is_some() && !loading.capture_done {
        // The audit waits for loaded scene assets and settled rendered frames
        // before requesting its screenshot. Shader compilation is asynchronous.
        if !ready {
            return;
        }
        loading.waiting_frames += 1;
        if loading.waiting_frames < 30 {
            return;
        }
        // One retained screen capture; no polling timeout is treated as success.
        loading.capture_pending = true;
        let path = loading.output.as_ref().unwrap().join("loading.png");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path))
            .observe(|_: On<ScreenshotCaptured>, mut state: ResMut<Loading>| {
                state.capture_pending = false;
                state.capture_done = true;
                state.records.push(serde_json::json!({"captured":true}));
            });
    }
}
