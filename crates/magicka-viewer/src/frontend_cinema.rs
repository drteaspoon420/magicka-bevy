//! Original-atlas frontend cinema host. Shared playback remains in runtime.
//! This flat right-page view is also a deterministic rendering fixture; tome
//! geometry, page-turn animation and level-loading handoff are separate hosts.
use crate::{
    native_text::{geometry_mesh, upload_font},
    pcm_audio::PcmClip,
    text_box_visual::read_asset,
};
use bevy::{
    app::{AppExit, ScheduleRunnerPlugin},
    asset::{embedded_asset, RenderAssetUsages},
    audio::{AudioPlayer, AudioSink, AudioSinkPlayback, PlaybackMode, PlaybackSettings, Volume},
    camera::{visibility::RenderLayers, ImageRenderTarget, RenderTarget},
    core_pipeline::tonemapping::Tonemapping,
    image::{ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::{
        render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat, TextureUsages},
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
    shader::ShaderRef,
    sprite_render::{AlphaMode2d, Material2d, Material2dPlugin},
    window::ExitCondition,
    winit::WinitPlugin,
};
use magicka_content::{
    assets::{Asset, BitmapFont, Texture2D},
    texture::{decode_rgba, encode_png},
};
use magicka_runtime::{
    audio::{AudioLibrary, Voice},
    frontend_intro::{illustration_uv, IntroEvent, IntroPlayback},
    localization::LocalizationTable,
    play_content::{bitmap_font, english_strings, resolve},
    scene_loading::custom_hash,
    scene_script::XmlElement,
    text_geometry::{GeometryFont, TextAlign, TextGeometry, TextStyle},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
struct CinemaMaterial {
    #[uniform(0)]
    image_uv: Vec4,
    #[uniform(0)]
    image_bounds: Vec4,
    #[uniform(0)]
    mask_uv: Vec4,
    #[uniform(0)]
    lighting: Vec4,
    #[texture(1)]
    #[sampler(2)]
    page: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    picture: Handle<Image>,
    #[texture(5)]
    #[sampler(6)]
    mask: Handle<Image>,
}
impl Material2d for CinemaMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/frontend_cinema.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

struct Loaded {
    page: Texture2D,
    mask: Texture2D,
    pictures: Vec<Texture2D>,
    title_font: BitmapFont,
    body_font: BitmapFont,
    subtitle_font: BitmapFont,
    title: String,
    description: String,
}

#[derive(Resource)]
struct Cinema {
    embedded: bool,
    loading_tip: Option<usize>,
    loaded: Loaded,
    strings: LocalizationTable,
    intro: IntroPlayback,
    light: f32,
    page_text: String,
    subtitles: String,
    fade: Option<(f32, f32)>,
    elapsed: f32,
    events: Vec<serde_json::Value>,
    output: Option<PathBuf>,
    captures: Vec<(String, f32)>,
    capture_index: usize,
    capture_pending: bool,
    frames: u32,
    target: Handle<Image>,
    finished: bool,
    audio: Option<AudioLibrary>,
    voices: Vec<PendingVoice>,
    music_volume: f32,
    sfx_volume: f32,
    screen_width: i32,
    skip_at: Option<f32>,
    skip_sent: bool,
    light_event: Option<[f32; 3]>,
    /// A page change asked the Tome to riffle one page forward.
    riffle: bool,
}
struct PendingVoice {
    clip: Handle<PcmClip>,
    voice: Voice,
    age: f32,
    entity: Option<Entity>,
    narration: bool,
}
#[derive(Resource, Default)]
pub(crate) struct IntroControl {
    pub requested: bool,
    pub active: bool,
    pub completed: Option<IntroCompletion>,
    pub volumes: [f32; 2],
    pub trace: Option<PathBuf>,
    /// The latest SubMenuIntro Tome light change (intensity, variation
    /// amount, variation speed) for the host's Tome.
    pub light: Option<[f32; 3]>,
    /// Tome.Riffle(Forward, 1) requested by a page change.
    pub riffle: bool,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct IntroCompletion {
    pub loading_tip: Option<usize>,
}
#[derive(Component)]
struct CinemaCamera;
/// Embedded: renders the intro's page for the Tome.
#[derive(Component)]
struct PageCamera;
/// Embedded: the two page targets and the new/old sheet materials.
#[derive(Resource)]
struct PageTargets {
    targets: [Handle<Image>; 2],
    current: usize,
    sheets: [Handle<crate::gui_basic::GuiBasicMaterial>; 2],
}
const CINEMA_LAYER: usize = 30;
/// Embedded: the intro's page content, rendered for the Tome's right sheet.
const CINEMA_PAGE_LAYER: usize = 29;
#[derive(SystemSet, Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct CinemaSet;

#[derive(Component)]
struct Narration;

struct FontView {
    font: Arc<GeometryFont>,
    material: Handle<ColorMaterial>,
}
struct TextView {
    entity: Entity,
    mesh: Handle<Mesh>,
    text: String,
}
#[derive(Resource)]
struct Screen {
    material: Handle<CinemaMaterial>,
    pictures: Vec<Handle<Image>>,
    page: Entity,
    title_font: FontView,
    body_font: FontView,
    subtitle_font: FontView,
    title: TextView,
    description: TextView,
    body: TextView,
    subtitle: TextView,
    shadow: TextView,
    fade: Entity,
}

fn texture(root: &Path, name: &str) -> Result<Texture2D, String> {
    let path = resolve(root, Path::new(name)).ok_or_else(|| format!("missing {name}"))?;
    match read_asset(&path)? {
        Asset::Texture2D(texture) => Ok(texture),
        _ => Err(format!("expected texture {name}")),
    }
}

pub fn run(root: &str, args: &[String]) -> Result<(), String> {
    let mut app = App::new();
    configure(&mut app, root, args, false)?;
    app.run();
    Ok(())
}
pub(crate) fn install(app: &mut App, root: &str, args: &[String]) -> Result<(), String> {
    configure(app, root, args, true)
}
fn configure(app: &mut App, root: &str, args: &[String], embedded: bool) -> Result<(), String> {
    let get = |key: &str| args.windows(2).find(|a| a[0] == key).map(|a| a[1].clone());
    let root = Path::new(root);
    let strings = english_strings(root, &["Player"]);
    let level_path = resolve(root, Path::new("Levels/Wizard_Castle.lvl"))
        .ok_or("missing first campaign level")?;
    let level = XmlElement::parse(&std::fs::read(level_path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("level XML: {e:?}"))?;
    let localize = |field: &str| -> Result<String, String> {
        let key = level
            .children
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(field))
            .ok_or_else(|| format!("missing chapter {field}"))?
            .inner_text
            .to_lowercase();
        let hash = custom_hash(&key);
        if !strings.try_get_string(hash).found {
            return Err(format!("missing chapter localization {key}"));
        }
        Ok(strings.get_string(hash).unwrap_or_default())
    };
    let loaded = Loaded {
        page: texture(root, "UI/ToM/tome_pages_0.xnb")?,
        mask: texture(root, "UI/Cutscenes/intro_mask.xnb")?,
        pictures: (1..=6)
            .map(|i| texture(root, &format!("UI/Cutscenes/intro_s{i}.xnb")))
            .collect::<Result<_, _>>()?,
        title_font: bitmap_font(root, "Languages/eng/Font/MenuTitle")
            .map_err(|e| format!("title font: {e:?}"))?,
        body_font: bitmap_font(root, "Languages/eng/Font/MenuDefault")
            .map_err(|e| format!("body font: {e:?}"))?,
        subtitle_font: bitmap_font(root, "Languages/eng/Font/Maiandra14")
            .map_err(|e| format!("subtitle font: {e:?}"))?,
        title: localize("name")?,
        description: localize("description")?,
    };
    let output = (!embedded)
        .then(|| get("--render").map(PathBuf::from))
        .flatten();
    if let Some(out) = &output {
        std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    }
    let silent = output.is_some() || args.iter().any(|a| a == "--silent");
    let audio = (!silent).then(|| AudioLibrary::load(root));
    let (intro, initial) = IntroPlayback::enter();
    let mut cinema = Cinema {
        embedded,
        loading_tip: None,
        loaded,
        strings,
        intro,
        light: 0.0,
        page_text: String::new(),
        subtitles: String::new(),
        fade: None,
        elapsed: 0.0,
        events: Vec::new(),
        output: output.clone(),
        captures: [
            ("dark", 0.0),
            ("opening", 4.0),
            ("chapter", 12.0),
            ("page1", 28.0),
            ("page2", 38.0),
            ("pan_start", 52.0),
            ("pan_end", 60.0),
            ("zoom1_start", 64.0),
            ("zoom1_end", 77.0),
            ("zoom2_start", 81.0),
            ("zoom2_end", 91.0),
            ("page6", 98.0),
            ("fade", 102.0),
            ("handoff", 104.0),
        ]
        .into_iter()
        .map(|(name, time)| (name.into(), time))
        .collect(),
        capture_index: 0,
        capture_pending: false,
        frames: 0,
        target: Handle::default(),
        finished: false,
        audio,
        voices: Vec::new(),
        music_volume: 1.0,
        sfx_volume: 1.0,
        screen_width: 1024,
        skip_at: get("--skip-at")
            .map(|v| v.parse::<f32>().map_err(|e| e.to_string()))
            .transpose()?,
        skip_sent: false,
        light_event: None,
        riffle: false,
    };
    // Initial effects are consumed by the same host as later playback events.
    apply_events(&mut cinema, initial, None);
    if !embedded {
        if output.is_some() {
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
            )));
        } else {
            app.add_plugins(DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Magicka — opening story".into(),
                    resolution: (1024, 768).into(),
                    ..default()
                }),
                ..default()
            }));
        }
    }
    if !app.is_plugin_added::<crate::menu_input::MenuInputPlugin>() {
        app.add_plugins(crate::menu_input::MenuInputPlugin);
    }
    embedded_asset!(app, "frontend_cinema.wgsl");
    crate::pcm_audio::register(app);
    app.add_plugins(Material2dPlugin::<CinemaMaterial>::default())
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(IntroControl {
            active: !embedded,
            volumes: [1.0; 2],
            trace: get("--opening-trace").map(PathBuf::from),
            ..default()
        })
        .insert_resource(cinema)
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (drive, turn_page, update_audio).chain().in_set(CinemaSet),
        );
    Ok(())
}

fn upload(texture: &Texture2D, images: &mut Assets<Image>) -> Handle<Image> {
    let data = decode_rgba(
        texture.format,
        texture.width as usize,
        texture.height as usize,
        &texture.mips[0],
    )
    .expect("supported cinema texture");
    let mut image = Image::new(
        Extent3d {
            width: texture.width as u32,
            height: texture.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    images.add(image)
}
fn font_view(
    font: &BitmapFont,
    images: &mut Assets<Image>,
    materials: &mut Assets<ColorMaterial>,
    spacing: i32,
) -> FontView {
    let (mut font, material) = upload_font(font, images, materials).expect("cinema font");
    Arc::make_mut(&mut font).metrics.spacing = spacing;
    FontView { font, material }
}
fn text_view(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    font: &FontView,
    layer: usize,
) -> TextView {
    let geometry = TextGeometry::build(&font.font, &[], TextStyle::default()).unwrap();
    let mesh = meshes.add(geometry_mesh(&geometry, 0).unwrap());
    let entity = commands
        .spawn((
            RenderLayers::layer(layer),
            Mesh2d(mesh.clone()),
            MeshMaterial2d(font.material.clone()),
            Transform::default(),
        ))
        .id();
    TextView {
        entity,
        mesh,
        text: String::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn setup(
    mut commands: Commands,
    #[cfg(feature = "main-menu")] gui: Option<ResMut<Assets<crate::gui_basic::GuiBasicMaterial>>>,
    mut cinema: ResMut<Cinema>,
    mut images: ResMut<Assets<Image>>,
    mut colors: ResMut<Assets<ColorMaterial>>,
    mut materials: ResMut<Assets<CinemaMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut clips: ResMut<Assets<PcmClip>>,
) {
    let title_font = font_view(&cinema.loaded.title_font, &mut images, &mut colors, 0);
    let body_font = font_view(&cinema.loaded.body_font, &mut images, &mut colors, 1);
    let subtitle_font = font_view(&cinema.loaded.subtitle_font, &mut images, &mut colors, 0);
    let pictures: Vec<_> = cinema
        .loaded
        .pictures
        .iter()
        .map(|p| upload(p, &mut images))
        .collect();
    let mask_size = [
        cinema.loaded.pictures[0].width as f32,
        cinema.loaded.pictures[0].height as f32,
    ];
    let material = materials.add(CinemaMaterial {
        image_uv: Vec4::new(1.0, 1.0, 0.0, 0.0),
        image_bounds: Vec4::new(86.0 / 1024.0, 90.0 / 1024.0, 852.0 / 1024.0, 652.0 / 1024.0),
        mask_uv: Vec4::new(852.0 / mask_size[0], 652.0 / mask_size[1], 0.0, 0.0),
        lighting: Vec4::ZERO,
        page: upload(&cinema.loaded.page, &mut images),
        picture: pictures[0].clone(),
        mask: upload(&cinema.loaded.mask, &mut images),
    });
    // On the Tome the page content renders for the right sheet; subtitles
    // and the fade stay on the screen.
    let page_layer = if cinema.embedded {
        CINEMA_PAGE_LAYER
    } else {
        CINEMA_LAYER
    };
    let page = commands
        .spawn((
            RenderLayers::layer(page_layer),
            Mesh2d(meshes.add(Rectangle::new(1024.0, 1024.0))),
            MeshMaterial2d(material.clone()),
            Transform::default(),
        ))
        .id();
    let title = text_view(&mut commands, &mut meshes, &title_font, page_layer);
    let description = text_view(&mut commands, &mut meshes, &title_font, page_layer);
    let body = text_view(&mut commands, &mut meshes, &body_font, page_layer);
    let subtitle = text_view(&mut commands, &mut meshes, &subtitle_font, CINEMA_LAYER);
    let shadow = text_view(&mut commands, &mut meshes, &subtitle_font, CINEMA_LAYER);
    let fade = commands
        .spawn((
            RenderLayers::layer(CINEMA_LAYER),
            Sprite::from_color(Color::BLACK, Vec2::new(1024.0, 768.0)),
            Transform::from_xyz(0.0, 0.0, 20.0),
        ))
        .id();
    commands.insert_resource(Screen {
        material,
        pictures,
        page,
        title_font,
        body_font,
        subtitle_font,
        title,
        description,
        body,
        subtitle,
        shadow,
        fade,
    });
    if cinema.output.is_some() {
        let mut image = Image::new_target_texture(1024, 768, TextureFormat::Rgba8UnormSrgb, None);
        image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        cinema.target = images.add(image);
        commands.spawn((
            CinemaCamera,
            RenderLayers::layer(CINEMA_LAYER),
            Camera2d,
            Tonemapping::None,
            RenderTarget::Image(ImageRenderTarget::from(cinema.target.clone())),
        ));
    } else {
        commands.spawn((
            CinemaCamera,
            RenderLayers::layer(CINEMA_LAYER),
            Camera2d,
            Tonemapping::None,
            Camera {
                order: 100,
                is_active: !cinema.embedded,
                // Embedded, the Tome is behind the subtitles and fade.
                clear_color: if cinema.embedded {
                    ClearColorConfig::None
                } else {
                    ClearColorConfig::Custom(Color::BLACK)
                },
                ..default()
            },
        ));
    }
    // The standalone preview has no GUI material (no Tome to draw on).
    #[cfg(feature = "main-menu")]
    if let (true, Some(mut gui)) = (cinema.embedded, gui) {
        // Two page targets: a riffle freezes the page being turned away in
        // one while the next page renders into the other.
        let targets = [(); 2].map(|_| {
            let mut image =
                Image::new_target_texture(1024, 1024, TextureFormat::Rgba8UnormSrgb, None);
            image.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;
            images.add(image)
        });
        commands.spawn((
            CinemaCamera,
            PageCamera,
            RenderLayers::layer(CINEMA_PAGE_LAYER),
            Camera2d,
            Tonemapping::None,
            Camera {
                // Before the Tome's page target composes the sheets.
                order: -20,
                is_active: false,
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            RenderTarget::Image(ImageRenderTarget::from(targets[0].clone())),
        ));
        // SubMenuIntro.Draw on the right page (and DrawOld on the turning
        // one): through the GUI's encoding, like every page draw.
        let sheets = [false, true].map(|old| {
            let root = commands
                .spawn((
                    crate::frontend_tome::SheetRoot {
                        menu: crate::frontend_tome::MenuId::Intro,
                        right: true,
                        old,
                    },
                    Transform::default(),
                    Visibility::Hidden,
                ))
                .id();
            let mut material = crate::frontend::book::page_material(
                targets[usize::from(old)].clone(),
                [1.0; 4],
                false,
            );
            // The page target is premultiplied by its blended draws.
            material.settings.z = 1.0;
            let material = gui.add(material);
            commands.spawn((
                Mesh2d(meshes.add(crate::frontend::book::quad(
                    [1024.0; 2],
                    [0.0, 0.0, 1.0, 1.0],
                ))),
                MeshMaterial2d(material.clone()),
                Transform::from_xyz(-512.0, 512.0, 0.5),
                RenderLayers::layer(crate::frontend_tome::PAGE_LAYER),
                ChildOf(root),
            ));
            material
        });
        commands.insert_resource(PageTargets {
            targets,
            current: 0,
            sheets,
        });
    }
    if cinema.audio.is_some() && !cinema.embedded {
        start_cue(&mut cinema, 2, "music_intro", false, &mut clips);
    }
}

fn start_cue(
    cinema: &mut Cinema,
    bank: u16,
    cue: &str,
    narration: bool,
    clips: &mut Assets<PcmClip>,
) {
    let Some(library) = &cinema.audio else {
        return;
    };
    let Some(voices) = library.resolve(bank, custom_hash(cue), &mut || 0.5) else {
        warn!("cinema cue missing: {cue}");
        return;
    };
    for voice in voices {
        match library.try_decode(&voice.wave_bank, voice.track) {
            Ok(pcm) => {
                cinema.voices.push(PendingVoice {
                    clip: clips.add(PcmClip::from_pcm(pcm, voice.loop_count)),
                    voice,
                    age: 0.0,
                    entity: None,
                    narration,
                });
            }
            Err(error) => warn!("cinema {cue}: {error}"),
        }
    }
}

fn apply_events(
    cinema: &mut Cinema,
    events: Vec<IntroEvent>,
    mut audio: Option<(
        &mut Commands,
        &mut Assets<PcmClip>,
        &Query<Entity, With<Narration>>,
    )>,
) {
    for event in events {
        cinema
            .events
            .push(serde_json::json!({"time":cinema.elapsed,"event":format!("{event:?}")}));
        match event {
            IntroEvent::Light(light) => {
                cinema.light = light[0];
                cinema.light_event = Some(light);
            }
            IntroEvent::ShowOpeningSubtitles => {
                let text = cinema
                    .strings
                    .get_string(custom_hash(&IntroPlayback::text_key(0)))
                    .unwrap_or_default();
                let metrics = magicka_runtime::bitmap_font::FontMetrics::from_content(
                    &cinema.loaded.subtitle_font,
                )
                .unwrap();
                cinema.subtitles = metrics
                    .wrap(&text, cinema.screen_width * 8 / 10, true)
                    .unwrap()
                    .text;
            }
            IntroEvent::HideSubtitles => cinema.subtitles.clear(),
            IntroEvent::PreparePage(index) => {
                let text = cinema
                    .strings
                    .get_string(custom_hash(&IntroPlayback::text_key(index)))
                    .unwrap_or_default();
                let mut metrics = magicka_runtime::bitmap_font::FontMetrics::from_content(
                    &cinema.loaded.body_font,
                )
                .unwrap();
                metrics.spacing = 1;
                cinema.page_text = metrics.wrap(&text, 896, true).unwrap().text;
            }
            IntroEvent::PlayNarration(index) => {
                if let Some((_, clips, _)) = audio.as_mut() {
                    start_cue(
                        cinema,
                        1,
                        &IntroPlayback::narration_name(index),
                        true,
                        clips,
                    );
                }
            }
            IntroEvent::StopNarration => {
                cinema.voices.retain(|v| !v.narration);
                if let Some((commands, _, voices)) = audio.as_mut() {
                    for entity in *voices {
                        commands.entity(entity).despawn();
                    }
                }
            }
            IntroEvent::FadeToBlack { seconds } => cinema.fade = Some((0.0, seconds)),
            IntroEvent::LoadFirstLevel { loading_tip } => {
                cinema.loading_tip = loading_tip;
                cinema.finished = true;
                apply_events(cinema, IntroPlayback::exit_events().into(), None);
            }
            IntroEvent::Riffle => cinema.riffle = true,
            IntroEvent::StartMusic => {}
        }
    }
}

fn tick(
    cinema: &mut Cinema,
    dt: f32,
    mut audio: Option<(
        &mut Commands,
        &mut Assets<PcmClip>,
        &Query<Entity, With<Narration>>,
    )>,
) {
    cinema.elapsed += dt;
    if let Some((age, seconds)) = &mut cinema.fade {
        *age += dt;
        if *age >= *seconds {
            if let Some(event) = cinema.intro.transition_end() {
                apply_events(cinema, vec![event], None);
            }
        }
    }
    if !cinema.skip_sent && cinema.skip_at.is_some_and(|at| cinema.elapsed >= at) {
        cinema.skip_sent = true;
        let events = cinema.intro.skip(true);
        apply_events(
            cinema,
            events,
            audio.as_mut().map(|(a, b, c)| (&mut **a, &mut **b, *c)),
        );
    }
    let events = cinema.intro.update(dt, true);
    apply_events(cinema, events, audio);
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    mut cinema: ResMut<Cinema>,
    mut screen: ResMut<Screen>,
    mut materials: ResMut<Assets<CinemaMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut clips: ResMut<Assets<PcmClip>>,
    mut sprites: Query<&mut Sprite>,
    narration: Query<Entity, With<Narration>>,
    input: Res<crate::menu_input::MenuInput>,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    time: Res<Time>,
    mut exit: MessageWriter<AppExit>,
    mut control: ResMut<IntroControl>,
    mut cameras: Query<&mut Camera, With<CinemaCamera>>,
) {
    if cinema.embedded && control.requested {
        control.requested = false;
        control.active = true;
        control.completed = None;
        for voice in std::mem::take(&mut cinema.voices) {
            if let Some(entity) = voice.entity {
                commands.entity(entity).despawn();
            }
        }
        let (intro, initial) = IntroPlayback::enter();
        cinema.intro = intro;
        cinema.elapsed = 0.0;
        cinema.frames = 0;
        cinema.finished = false;
        cinema.loading_tip = None;
        cinema.page_text.clear();
        cinema.subtitles.clear();
        cinema.events.clear();
        cinema.fade = None;
        cinema.skip_sent = false;
        cinema.music_volume = control.volumes[0];
        cinema.sfx_volume = control.volumes[1];
        apply_events(&mut cinema, initial, None);
        start_cue(&mut cinema, 2, "music_intro", false, &mut clips);
    }
    if cinema.embedded {
        for mut camera in &mut cameras {
            camera.is_active = control.active;
        }
        if !control.active {
            return;
        }
    }
    if cinema.capture_pending
        || (cinema.output.is_some() && cinema.capture_index >= cinema.captures.len())
    {
        return;
    }
    cinema.screen_width = windows.single().map(|w| w.width() as i32).unwrap_or(1024);
    cinema.frames += 1;
    if cinema.frames < 15 {
        return;
    }
    if cinema.output.is_some() {
        let target = cinema.captures[cinema.capture_index].1;
        while cinema.elapsed + 0.0001 < target {
            let dt = (target - cinema.elapsed).min(1.0 / 60.0);
            tick(&mut cinema, dt, None);
            cinema.intro.draw_page();
        }
    } else {
        if input.confirm || mouse.just_released(MouseButton::Left) {
            let events = cinema.intro.skip(true);
            apply_events(
                &mut cinema,
                events,
                Some((&mut commands, &mut clips, &narration)),
            );
        }
        tick(
            &mut cinema,
            time.delta_secs(),
            Some((&mut commands, &mut clips, &narration)),
        );
        if cinema.finished {
            if cinema.embedded {
                control.completed = Some(IntroCompletion {
                    loading_tip: cinema.loading_tip,
                });
                control.active = false;
                for voice in std::mem::take(&mut cinema.voices) {
                    if let Some(entity) = voice.entity {
                        commands.entity(entity).despawn();
                    }
                }
                if let Some(path) = &control.trace {
                    std::fs::write(path, serde_json::to_vec_pretty(&cinema.events).unwrap())
                        .expect("intro handoff trace");
                }
            } else {
                exit.write(AppExit::Success);
            }
        }
    }
    if let Some(light) = cinema.light_event.take() {
        control.light = Some(light);
    }
    let page = cinema.intro.draw_page();
    let size = windows
        .single()
        .map(|w| Vec2::new(w.width(), w.height()))
        .unwrap_or(Vec2::new(1024.0, 768.0));
    // On the Tome the page is drawn at its native 1024 pixels.
    let scale = if cinema.embedded {
        1.0
    } else {
        size.min_element() / 1024.0
    };
    let mut mat = materials.get_mut(&screen.material).unwrap();
    mat.lighting = Vec4::new(
        cinema.light,
        if page.index > 0 { 1.0 } else { 0.0 },
        if cinema.embedded { 1.0 } else { 0.0 },
        0.0,
    );
    if page.index > 0 && page.index <= 6 {
        let image = &cinema.loaded.pictures[page.index - 1];
        let (offset, uv_scale) = illustration_uv(
            page.index,
            page.motion,
            [image.width as f32, image.height as f32],
        );
        mat.picture = screen.pictures[page.index - 1].clone();
        mat.image_uv = Vec4::new(
            852.0 / image.width as f32 * uv_scale[0],
            652.0 / image.height as f32 * uv_scale[1],
            offset[0],
            offset[1],
        );
    }
    commands
        .entity(screen.page)
        .insert(Transform::from_scale(Vec3::splat(scale)));
    let page_color = [0.1, 0.1, 0.1, 0.9];
    let chapter_color = [0.1, 0.05, 0.0, 0.7];
    let s = &mut *screen;
    for entity in [s.title.entity, s.description.entity, s.body.entity] {
        commands.entity(entity).insert(if cinema.light > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    update_text(
        &mut commands,
        &mut meshes,
        &s.title_font,
        &mut s.title,
        if page.index == 0 {
            &cinema.loaded.title
        } else {
            ""
        },
        Vec3::new(0.0, (512.0 - 360.0) * scale, 2.0),
        scale,
        chapter_color,
    );
    update_text(
        &mut commands,
        &mut meshes,
        &s.title_font,
        &mut s.description,
        if page.index == 0 {
            &cinema.loaded.description
        } else {
            ""
        },
        Vec3::new(0.0, (512.0 - 480.0) * scale, 2.0),
        scale,
        chapter_color,
    );
    update_text(
        &mut commands,
        &mut meshes,
        &s.body_font,
        &mut s.body,
        &cinema.page_text,
        Vec3::new(0.0, (512.0 - 768.0) * scale, 2.0),
        scale,
        page_color,
    );
    let height = s
        .subtitle_font
        .font
        .metrics
        .measure(&cinema.subtitles, true)
        .unwrap()[1];
    let position = Vec3::new(0.0, -size.y / 2.0 + height + 32.0, 5.0);
    update_text(
        &mut commands,
        &mut meshes,
        &s.subtitle_font,
        &mut s.subtitle,
        &cinema.subtitles,
        position,
        1.0,
        [1.0; 4],
    );
    update_text(
        &mut commands,
        &mut meshes,
        &s.subtitle_font,
        &mut s.shadow,
        &cinema.subtitles,
        position + Vec3::new(1.0, -1.0, -0.1),
        1.0,
        [0.0, 0.0, 0.0, 0.8],
    );
    let alpha = cinema
        .fade
        .map_or(0.0, |(age, seconds)| (age / seconds).min(1.0));
    if let Ok(mut sprite) = sprites.get_mut(s.fade) {
        sprite.color = Color::srgba(0.0, 0.0, 0.0, alpha);
        sprite.custom_size = Some(size);
    }
    if cinema.output.is_some() && cinema.frames >= 30 {
        capture(&mut commands, &mut cinema, &page);
    }
}

#[allow(clippy::too_many_arguments)]
fn update_text(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    font: &FontView,
    view: &mut TextView,
    text: &str,
    position: Vec3,
    scale: f32,
    color: [f32; 4],
) {
    if view.text != text {
        let geometry = TextGeometry::build(
            &font.font,
            &text.encode_utf16().collect::<Vec<_>>(),
            TextStyle {
                align: TextAlign::Center,
                default_color: color,
                ..default()
            },
        )
        .expect("cinema text geometry");
        *meshes.get_mut(&view.mesh).unwrap() =
            geometry_mesh(&geometry, geometry.quads.len() as i32).unwrap();
        view.text = text.into();
    }
    commands
        .entity(view.entity)
        .insert(Transform::from_translation(position).with_scale(Vec3::splat(scale)));
}

fn capture(
    commands: &mut Commands,
    cinema: &mut Cinema,
    page: &magicka_runtime::frontend_intro::IntroPage,
) {
    let output = cinema.output.as_ref().unwrap();
    let name = &cinema.captures[cinema.capture_index].0;
    let path = output.join(format!("{name}.png"));
    let state = serde_json::json!({"time":cinema.elapsed,"index":cinema.intro.index,"texture":page.index,"motion":page.motion,"subtitles":!cinema.subtitles.is_empty(),"finished":cinema.finished});
    std::fs::write(
        output.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&state).unwrap(),
    )
    .expect("private state record");
    let event_path = output.join("events.json");
    std::fs::write(
        event_path,
        serde_json::to_vec_pretty(&cinema.events).unwrap(),
    )
    .expect("private event record");
    cinema.frames = 0;
    cinema.capture_pending = true;
    // Do not advance to another state until the GPU readback for this frame
    // finishes. A frame timeout never masquerades as a completed capture.
    commands
        .spawn(Screenshot::image(cinema.target.clone()))
        .observe(
            move |shot: On<ScreenshotCaptured>,
                  mut cinema: ResMut<Cinema>,
                  mut exit: MessageWriter<AppExit>| {
                let image = &shot.image;
                let bytes = image.data.as_ref().expect("captured pixels");
                std::fs::write(
                    &path,
                    encode_png(image.width() as usize, image.height() as usize, bytes),
                )
                .expect("private cinema capture");
                cinema.capture_index += 1;
                cinema.capture_pending = false;
                if cinema.capture_index == cinema.captures.len() {
                    exit.write(AppExit::Success);
                }
            },
        );
}

/// SubMenuIntro's page change: Tome.Riffle(Forward, 1). The page just shown
/// stays frozen in its target for the turning sheet (DrawOld) and the next
/// page renders into the other target.
fn turn_page(
    mut cinema: ResMut<Cinema>,
    mut control: ResMut<IntroControl>,
    targets: Option<ResMut<PageTargets>>,
    #[cfg(feature = "main-menu")] gui: Option<ResMut<Assets<crate::gui_basic::GuiBasicMaterial>>>,
    mut cameras: Query<&mut RenderTarget, With<PageCamera>>,
) {
    if !std::mem::take(&mut cinema.riffle) || !cinema.embedded {
        return;
    }
    control.riffle = true;
    let Some(mut targets) = targets else {
        return;
    };
    let old = targets.current;
    targets.current = 1 - old;
    let next = targets.targets[targets.current].clone();
    for mut target in &mut cameras {
        *target = RenderTarget::Image(ImageRenderTarget::from(next.clone()));
    }
    #[cfg(feature = "main-menu")]
    if let Some(mut gui) = gui {
        for (sheet, image) in [(0, next), (1, targets.targets[old].clone())] {
            if let Some(mut material) = gui.get_mut(&targets.sheets[sheet]) {
                material.texture = image.clone();
                material.overlay = image;
            }
        }
    }
}

// Category/RPC gain and pitch follow cue age, as in the gameplay audio host.
fn update_audio(
    mut commands: Commands,
    mut cinema: ResMut<Cinema>,
    mut sinks: Query<&mut AudioSink>,
    entities: Query<Entity>,
    time: Res<Time>,
) {
    let mut voices = std::mem::take(&mut cinema.voices);
    let Some(library) = &cinema.audio else {
        return;
    };
    voices.retain_mut(|live| {
        if live
            .entity
            .is_some_and(|entity| entities.get(entity).is_err())
        {
            return false;
        }
        live.age += time.delta_secs();
        let since = live.age - live.voice.delay;
        if since < 0.0 {
            return true;
        }
        let variables = |name: &str| match name {
            "AttackTime" => Some((since * 1000.0).min(15_000.0)),
            "ReleaseTime" | "Volume" | "Distance" | "NormalizedDistance" => Some(0.0),
            _ => None,
        };
        let gain = 10.0_f32.powf(library.gain_db(&live.voice, &variables) / 20.0)
            * if live.narration {
                cinema.sfx_volume
            } else {
                cinema.music_volume
            };
        let speed =
            2.0_f32.powf((live.voice.pitch + library.rpc_pitch(&live.voice, &variables)) / 12.0);
        match live.entity {
            None => {
                let mut entity = commands.spawn((
                    AudioPlayer(live.clip.clone()),
                    PlaybackSettings {
                        mode: PlaybackMode::Despawn,
                        volume: Volume::Linear(gain),
                        speed,
                        ..default()
                    },
                ));
                if live.narration {
                    entity.insert(Narration);
                }
                live.entity = Some(entity.id());
            }
            Some(entity) => {
                if let Ok(mut sink) = sinks.get_mut(entity) {
                    sink.set_volume(Volume::Linear(gain));
                    sink.set_speed(speed);
                }
            }
        }
        true
    });
    cinema.voices = voices;
}
