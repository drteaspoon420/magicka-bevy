//! Private original-atlas TextInputMessageBox GPU fixture: the real editor, CPU
//! Draw and retained presentation snapshots drive GUIBasicEffect meshes each frame.
use crate::{
    error_box_preview::{language_argument, load_menu_font, PreviewLanguageHost},
    gui_basic::{GuiBasicMaterial, GuiBasicPlugin},
    native_text::{geometry_mesh, upload_font},
    text_box_visual::read_asset,
};
use bevy::{
    app::{AppExit, ScheduleRunnerPlugin},
    asset::RenderAssetUsages,
    camera::{ImageRenderTarget, RenderTarget},
    core_pipeline::tonemapping::Tonemapping,
    image::{ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, PrimitiveTopology, VertexAttributeValues},
    prelude::*,
    render::{
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
    window::ExitCondition,
    winit::WinitPlugin,
};
use magicka_content::{
    assets::{Asset, Texture2D},
    texture::{decode_rgba, encode_png},
};
use magicka_runtime::{
    language_state::{Language, LanguageState},
    localization::LocalizationTable,
    message_box::background_vertices,
    scene_loading::custom_hash,
    text_geometry::GeometryFont,
    text_input_draw::{InputButton, TextInputDrawContext},
    text_input_message_box::{TextInputHost, TextInputMessageBoxState},
    text_input_presentation::{
        PresentationDrawHost, PresentationEditHost, TextDrawSnapshot, TextInputPresentation,
        TextTarget,
    },
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const SCREEN: [f32; 2] = [800., 600.];
const DT: f32 = 1. / 60.;
const POOL: usize = 8;
/// SubMenuAccountLogin: `new TextInputMessageBox(SubMenu.LOC_EMAIL, 128)`.
const CAPTION: &str = "#acc_login_email";
const MAX_LENGTH: i32 = 128;
/// Synthetic input typed one UTF-16 unit per frame; long enough to wrap.
const FIRST: &str = "Wizard";
const REST: &str = ".apprentice.of.the.order@magicka.example";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Empty,
    TypeFirst,
    Single,
    TypeRest,
    Multi,
    Fade,
    Hidden,
    Done,
}
struct Editing;
impl TextInputHost for Editing {
    type Error = ();
    fn register(&mut self, _: &mut TextInputMessageBoxState) -> Result<(), ()> {
        Ok(())
    }
    fn set_text(&mut self, _: &mut TextInputMessageBoxState, _: Option<&[u16]>) -> Result<(), ()> {
        Ok(())
    }
    fn complete(&mut self, _: &mut TextInputMessageBoxState, _: u64, _: &[u16]) -> Result<(), ()> {
        Ok(())
    }
}

#[derive(Resource)]
struct Session {
    output: PathBuf,
    target: Handle<Image>,
    state: TextInputMessageBoxState,
    presentation: TextInputPresentation,
    strings: LocalizationTable,
    font: Arc<GeometryFont>,
    page_base: Mesh,
    page: (Entity, Handle<Mesh>, Handle<GuiBasicMaterial>),
    pool: Vec<(Entity, Handle<Mesh>, Handle<GuiBasicMaterial>)>,
    stage: Stage,
    typed: usize,
    captured: Vec<String>,
    frame: u32,
    states: Vec<serde_json::Value>,
    last: Option<serde_json::Value>,
}
struct Loaded {
    output: PathBuf,
    font: crate::error_box_preview::MenuFont,
    languages: LanguageState,
    page: Texture2D,
}
#[derive(Resource)]
struct Pending(Option<Loaded>);

pub fn preview(root: &str, args: &[String]) -> Result<(), String> {
    let out = args
        .iter()
        .position(|x| x == "--render")
        .and_then(|i| args.get(i + 1))
        .ok_or("--render <private capture directory> required")?;
    let root = Path::new(root);
    let mut available = vec![];
    for entry in std::fs::read_dir(root.join("Languages")).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if let Some(language) = Language::from_code(&entry.file_name().to_string_lossy()) {
            if entry.path().is_dir() {
                available.push(language);
            }
        }
    }
    let mut languages = LanguageState::new(available);
    let mut events = 0;
    languages
        .set_language(
            language_argument(args, "--language")?.unwrap_or(Language::ENGLISH),
            &mut PreviewLanguageHost::unbound(root, &mut events),
        )
        .map_err(|e| format!("{e:?}"))?;
    let font = load_menu_font(root, languages.current())?;
    let Asset::Texture2D(page) = read_asset(&root.join("UI/Menu/pages.xnb"))? else {
        return Err("expected menu pages".into());
    };
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        )
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1. / 120.,
        )))
        .add_plugins(GuiBasicPlugin)
        .insert_resource(ClearColor(Color::linear_rgb(0.2, 0.2, 0.2)))
        .insert_resource(Pending(Some(Loaded {
            output: out.into(),
            font,
            languages,
            page,
        })))
        .add_systems(Startup, setup)
        .add_systems(Update, drive)
        .add_systems(Last, capture)
        .run();
    Ok(())
}

fn texture_image(texture: &Texture2D) -> Result<(Image, Vec<u8>), String> {
    let bytes = decode_rgba(
        texture.format,
        texture.width as usize,
        texture.height as usize,
        &texture.mips[0],
    )
    .ok_or("page decode")?;
    let mut image = Image::new(
        Extent3d {
            width: texture.width as u32,
            height: texture.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        bytes.clone(),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    Ok((image, bytes))
}

fn setup(
    mut commands: Commands,
    mut pending: ResMut<Pending>,
    mut images: ResMut<Assets<Image>>,
    mut colors: ResMut<Assets<ColorMaterial>>,
    mut materials: ResMut<Assets<GuiBasicMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let mut loaded = pending.0.take().expect("preview inputs");
    let (mut font, color) =
        upload_font(&loaded.font.content, &mut images, &mut colors).expect("font upload");
    Arc::make_mut(&mut font)
        .metrics
        .clone_from(&loaded.font.metrics);
    let atlas = colors.get(&color).unwrap().texture.clone().unwrap();
    let (page_image, page_bytes) = texture_image(&loaded.page).expect("page");
    let (w, h) = (loaded.page.width as usize, loaded.page.height as usize);
    std::fs::write(
        loaded.output.join("pages-atlas.png"),
        encode_png(w, h, &page_bytes),
    )
    .expect("private page cache");
    let page_texture = images.add(page_image);

    let strings = std::mem::take(&mut loaded.languages.strings);
    let mut state = TextInputMessageBoxState::new(SCREEN, MAX_LENGTH).unwrap();
    let mut presentation =
        TextInputPresentation::new_localized(&state, custom_hash(CAPTION), &strings, &font.metrics)
            .expect("presentation");
    state
        .show(
            Some(1),
            &mut PresentationEditHost {
                inner: &mut Editing,
                input: &mut presentation.input,
            },
        )
        .unwrap();

    let vertices = background_vertices([w as f32, h as f32]);
    let mut page_base = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    page_base.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vertices
            .iter()
            .map(|v| [v[0], -v[1], 0.])
            .collect::<Vec<_>>(),
    );
    page_base.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vertices.iter().map(|v| [v[2], v[3]]).collect::<Vec<_>>(),
    );
    page_base.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1f32; 4]; 4]);
    page_base.insert_indices(Indices::U16(vec![0, 1, 2, 0, 2, 3]));
    let mut spawn = |mesh: Mesh, material: GuiBasicMaterial, z: f32| {
        let mesh = meshes.add(mesh);
        let material = materials.add(material);
        let entity = commands
            .spawn((
                Mesh2d(mesh.clone()),
                MeshMaterial2d(material.clone()),
                Transform::from_xyz(0., 0., z),
                Visibility::Hidden,
            ))
            .id();
        (entity, mesh, material)
    };
    let page = spawn(
        page_base.clone(),
        GuiBasicMaterial::textured(page_texture, Vec4::ONE),
        0.,
    );
    let pool = (0..POOL)
        .map(|i| {
            spawn(
                page_base.clone(),
                GuiBasicMaterial::textured(atlas.clone(), Vec4::ONE),
                1. + i as f32 * 0.01,
            )
        })
        .collect();
    let mut target = Image::new_target_texture(800, 600, TextureFormat::Rgba8Unorm, None);
    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = images.add(target);
    commands.spawn((
        Camera2d,
        Msaa::Off,
        Tonemapping::None,
        RenderTarget::Image(ImageRenderTarget {
            handle: target.clone(),
            scale_factor: 1.,
        }),
    ));
    commands.insert_resource(Session {
        output: loaded.output,
        target,
        state,
        presentation,
        strings,
        font,
        page_base,
        page,
        pool,
        stage: Stage::Empty,
        typed: 0,
        captured: vec![],
        frame: 0,
        states: vec![],
        last: None,
    });
}

/// Native row-vector transform, then the existing GUIBasicEffect half-pixel
/// screen placement used by the other message-box fixtures.
fn placed(mesh: &Mesh, m: [[f32; 4]; 4]) -> Mesh {
    let mut result = mesh.clone();
    let Some(VertexAttributeValues::Float32x3(points)) =
        result.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("positions")
    };
    for p in points {
        let (x, y) = (p[0], -p[1]);
        let sx = x * m[0][0] + y * m[1][0] + m[3][0];
        let sy = x * m[0][1] + y * m[1][1] + m[3][1];
        p[0] = sx - 0.5 - SCREEN[0] * 0.5;
        p[1] = SCREEN[1] * 0.5 - (sy - 0.5);
    }
    result
}
/// Image-space bounds (pixel edges) of a snapshot's glyph quads.
fn bounds(draw: &TextDrawSnapshot) -> [f32; 4] {
    let m = draw.transform;
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for v in draw.geometry.quads.iter().flat_map(|q| q.vertices.iter()) {
        let [x, y] = v.position;
        let sx = x * m[0][0] + y * m[1][0] + m[3][0] - 0.5;
        let sy = x * m[0][1] + y * m[1][1] + m[3][1] - 0.5;
        b = [b[0].min(sx), b[1].min(sy), b[2].max(sx), b[3].max(sy)];
    }
    b
}
fn target_name(target: TextTarget) -> &'static str {
    match target {
        TextTarget::Caption => "caption",
        TextTarget::Input => "input",
        TextTarget::Blinker => "blinker",
        TextTarget::Button(InputButton::Okay) => "ok",
        TextTarget::Button(InputButton::Cancel) => "cancel",
    }
}

fn type_next(session: &mut Session, text: &str) -> bool {
    let units: Vec<u16> = text.encode_utf16().collect();
    let Some(&unit) = units.get(session.typed) else {
        session.typed = 0;
        return true;
    };
    let s = &mut *session;
    s.state
        .on_text_input(
            unit,
            (48..=57).contains(&unit),
            false,
            &mut PresentationEditHost {
                inner: &mut Editing,
                input: &mut s.presentation.input,
            },
        )
        .unwrap();
    s.typed += 1;
    false
}

fn drive(
    mut session: ResMut<Session>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GuiBasicMaterial>>,
    mut visibility: Query<&mut Visibility>,
) {
    let s = &mut *session;
    s.frame += 1;
    match s.stage {
        Stage::TypeFirst => {
            if type_next(s, FIRST) {
                s.stage = Stage::Single;
            }
        }
        Stage::TypeRest => {
            if type_next(s, REST) {
                s.stage = Stage::Multi;
            }
        }
        Stage::Fade if !s.state.base.killed => s.state.base.kill(),
        _ => {}
    }
    for (entity, _, _) in std::iter::once(&s.page).chain(&s.pool) {
        *visibility.get_mut(*entity).unwrap() = Visibility::Hidden;
    }
    // MessageBox.Update stops submitting the box once it is dead.
    if s.state.base.dead() {
        s.last = Some(
            serde_json::json!({"alpha": s.state.base.alpha, "draws": [], "background": false}),
        );
        return;
    }
    let message_height = s.presentation.message_height;
    let metrics = s.font.metrics.clone();
    let context = TextInputDrawContext {
        screen: SCREEN,
        daisy_wheel: false,
        base_font: &metrics,
        input_font: &metrics,
        message_height,
    };
    let mut host = PresentationDrawHost::new(&mut s.presentation, &s.strings, &s.font);
    s.state.draw_input(DT, context, &mut host).expect("draw");
    let (draws, page, background) = (host.draws, host.page, host.background);
    assert!(draws.len() <= POOL, "snapshot pool exhausted");
    if let (true, Some((transform, color))) = (background, page) {
        *meshes.get_mut(&s.page.1).unwrap() = placed(&s.page_base, transform);
        materials.get_mut(&s.page.2).unwrap().color = Vec4::from_array(color);
        *visibility.get_mut(s.page.0).unwrap() = Visibility::Visible;
    }
    let mut records = vec![];
    for (draw, (entity, mesh, handle)) in draws.iter().zip(&s.pool) {
        let base = geometry_mesh(&draw.geometry, draw.geometry.quads.len() as i32).unwrap();
        *meshes.get_mut(mesh).unwrap() = placed(&base, draw.transform);
        let mut material = materials.get_mut(handle).unwrap();
        material.color = Vec4::from_array(draw.color);
        material.flags.y = u32::from(draw.vertex_color);
        *visibility.get_mut(*entity).unwrap() = Visibility::Visible;
        records.push(serde_json::json!({"target": target_name(draw.target), "quads": draw.geometry.quads.len(),
            "color": draw.color, "vertex_color": draw.vertex_color, "bounds": bounds(draw), "transform": draw.transform}));
    }
    s.last = Some(
        serde_json::json!({"alpha": s.state.base.alpha, "blink_line": s.state.blink_line,
        "text_units": s.state.text().len(), "background": background, "page": page, "draws": records,
        "message_height": message_height}),
    );
}

fn capture(mut commands: Commands, mut session: ResMut<Session>) {
    let s = &mut *session;
    let Some(last) = s.last.take() else { return };
    let blink = s.state.blink_line;
    let full = s.state.base.alpha == 1.;
    let has = |name: &str| s.captured.iter().any(|c| c == name);
    let (prefix, next) = match s.stage {
        Stage::Empty => ("empty", Stage::TypeFirst),
        Stage::Single => ("single", Stage::TypeRest),
        Stage::Multi => ("multi", Stage::Fade),
        Stage::Fade => {
            let alpha = s.state.base.alpha;
            if alpha > 0.25 && alpha < 0.75 {
                ("fade", Stage::Hidden)
            } else {
                return;
            }
        }
        // Only a frame after death submits nothing; the frame reaching zero still draws.
        Stage::Hidden if last["draws"].as_array().is_some_and(Vec::is_empty) => {
            ("hidden", Stage::Done)
        }
        _ => return,
    };
    let phase = match s.stage {
        Stage::Empty | Stage::Single | Stage::Multi => {
            if !full {
                return;
            }
            let name = format!("{prefix}_{}", if blink { "on" } else { "off" });
            // Capture "on" first, then the following "off" frame.
            if has(&name) || (!blink && !has(&format!("{prefix}_on"))) {
                return;
            }
            if !blink {
                s.stage = next;
            }
            name
        }
        _ => {
            s.stage = next;
            prefix.to_string()
        }
    };
    let mut record = last;
    record["phase"] = serde_json::json!(phase);
    record["frame"] = serde_json::json!(s.frame);
    s.captured.push(phase.clone());
    s.states.push(record);
    std::fs::write(
        s.output.join("states.json"),
        serde_json::to_vec_pretty(&s.states).unwrap(),
    )
    .unwrap();
    let done = s.stage == Stage::Done;
    commands.spawn(Screenshot::image(s.target.clone())).observe(
        move |captured: On<ScreenshotCaptured>,
              session: Res<Session>,
              mut exit: MessageWriter<AppExit>| {
            std::fs::write(
                session.output.join(format!("{phase}.png")),
                encode_png(800, 600, captured.image.data.as_ref().unwrap()),
            )
            .unwrap();
            if done {
                exit.write(AppExit::Success);
            }
        },
    );
}
