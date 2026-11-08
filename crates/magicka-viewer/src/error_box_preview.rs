//! Private original-atlas message-box lifecycle and localization GPU fixture.
use crate::{
    box_devices::{BoxDevices, BoxDevicesPlugin},
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
    window::{CursorOptions, ExitCondition, PrimaryWindow, WindowResolution},
    winit::WinitPlugin,
};
use magicka_content::{
    assets::{Asset, BitmapFont, Texture2D},
    texture::{decode_rgba, encode_png},
};
use magicka_runtime::{
    bitmap_font::FontMetrics,
    dialog_definition::MessageDefinition,
    dialog_frame::{DialogFrame, DialogFrameHost, DialogFrameTarget},
    dialog_input::DialogInputKind,
    dialog_manager::{
        ActiveInteraction, DialogManager, DialogManagerEffect, DialogManagerHost, DialogSink,
    },
    error_message_box::{default_message_id as error_message_id, ErrorMessageBoxState},
    interaction_playback::InteractionHost,
    language_state::{Language, LanguageHost, LanguageListener, LanguageState},
    menu_text_item::MENU_COLOR,
    message_box::background_vertices,
    message_box_collection::{
        ErrorMessageBoxes, MessageBoxDrawKind, MessageBoxHandle, MessageBoxInput,
    },
    message_box_device::{BoxDeviceEvent, BoxDeviceState},
    message_playback::{MessagePlacement, MessagePlayback},
    text_geometry::{GeometryFont, TextAlign, TextGeometry, TextStyle},
    waiting_message_box::{default_message_id, WaitingMessageBoxState},
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
const SCREEN: [f32; 2] = [800., 600.];
#[derive(Resource)]
struct Preview {
    interactive: bool,
    input_audit: bool,
    root: PathBuf,
    font: MenuFont,
    languages: LanguageState,
    switch_language: Option<Language>,
    language_events: usize,
    page: Texture2D,
    output: PathBuf,
    target: Handle<Image>,
    frame: u32,
    states: Vec<serde_json::Value>,
    waiting: bool,
}
pub(crate) struct MenuFont {
    pub(crate) content: BitmapFont,
    pub(crate) metrics: FontMetrics,
    pub(crate) source_language: Language,
}
pub(crate) fn load_menu_font(root: &Path, language: Language) -> Result<MenuFont, String> {
    // Native Windows Directory.Exists uses lower-case "font" while assets use
    // "Font". This fixture resolves the actual case explicitly on Linux.
    let language = if root
        .join("Languages")
        .join(language.directory_name())
        .join("Font")
        .is_dir()
    {
        language
    } else {
        Language::ENGLISH
    };
    let Asset::BitmapFont(content) = read_asset(
        &root
            .join("Languages")
            .join(language.directory_name())
            .join("Font/MenuDefault.xnb"),
    )?
    else {
        return Err("expected menu font".into());
    };
    let mut metrics = FontMetrics::from_content(&content).map_err(|e| format!("{e:?}"))?;
    metrics.spacing = 1; // FontManager.LoadFont custom MenuDefault spacing.
    Ok(MenuFont {
        content,
        metrics,
        source_language: language,
    })
}
pub(crate) struct PreviewLanguageHost<'a> {
    pub(crate) root: &'a Path,
    pub(crate) font: Option<&'a mut MenuFont>,
    binding: Option<(&'a mut World, MessageBoxHandle)>,
    pub(crate) events: &'a mut usize,
}
impl<'a> PreviewLanguageHost<'a> {
    /// Table/font loading only; no box listener is bound.
    pub(crate) fn unbound(root: &'a Path, events: &'a mut usize) -> Self {
        Self {
            root,
            font: None,
            binding: None,
            events,
        }
    }
}
impl LanguageHost for PreviewLanguageHost<'_> {
    type Error = String;
    type Table = PathBuf;
    fn enumerate_tables(&mut self, language: Language) -> Result<Vec<PathBuf>, String> {
        let mut paths = vec![];
        for entry in std::fs::read_dir(self.root.join("Languages").join(language.directory_name()))
            .map_err(|e| e.to_string())?
        {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_file()
                && path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with(".loctable.xml")
            {
                paths.push(path);
            }
        }
        // Deterministic fixture host order; native GetFiles ordering is unknown.
        paths.sort();
        Ok(paths)
    }
    fn read_table(&mut self, table: &PathBuf) -> Result<Vec<u8>, String> {
        std::fs::read(table).map_err(|e| e.to_string())
    }
    fn reload_fonts(&mut self, language: Language) -> Result<(), String> {
        if let Some(font) = self.font.as_deref_mut() {
            let fresh = load_menu_font(self.root, language)?;
            font.metrics.read_from(&fresh.metrics); // Native Read resets spacing.
            font.content = fresh.content;
            font.source_language = fresh.source_language;
        }
        Ok(())
    }
    fn language_changed(
        &mut self,
        listener: LanguageListener,
        state: &mut LanguageState,
    ) -> Result<(), String> {
        if listener != LanguageListener(1) {
            return Err("unknown box language listener".into());
        }
        *self.events += 1;
        let (world, handle) = self
            .binding
            .as_mut()
            .ok_or("language callback without box binding")?;
        world
            .boxes
            .language_changed(
                *handle,
                &state.strings,
                &self
                    .font
                    .as_ref()
                    .ok_or("language callback without font")?
                    .metrics,
            )
            .map_err(|e| format!("{e:?}"))
    }
}
pub(crate) fn language_argument(args: &[String], flag: &str) -> Result<Option<Language>, String> {
    let Some(index) = args.iter().position(|a| a == flag) else {
        return Ok(None);
    };
    let value = args
        .get(index + 1)
        .ok_or_else(|| format!("{flag} requires a language code"))?;
    Language::from_code(value)
        .or_else(|| value.parse::<i32>().ok().map(Language))
        .map(Some)
        .ok_or_else(|| format!("invalid language code: {value}"))
}
struct World {
    boxes: ErrorMessageBoxes,
}
impl InteractionHost for World {
    type Error = String;
    fn advance_message(
        &mut self,
        _: usize,
        _: &MessageDefinition,
        _: &mut MessagePlayback,
        _: MessagePlacement,
    ) -> Result<bool, String> {
        Err("no gameplay message host".into())
    }
    fn lock_input(&mut self, _: i32) -> Result<(), String> {
        Err("no controller lock host".into())
    }
    fn unlock_input(&mut self, _: i32) -> Result<(), String> {
        Err("no controller unlock host".into())
    }
    fn end_message(&mut self, _: usize, _: &MessageDefinition) -> Result<(), String> {
        Err("no gameplay cue host".into())
    }
}
impl DialogManagerHost for World {
    fn bind(&mut self, _: DialogSink, _: ActiveInteraction) {}
    fn visible(&self, _: DialogSink) -> bool {
        false
    }
    fn animating(&self, _: DialogSink) -> bool {
        false
    }
    fn manager_effect(&mut self, _: DialogManagerEffect) -> Result<(), String> {
        Err("no campaign effect host".into())
    }
}
impl DialogFrameHost for World {
    fn update_textbox(
        &mut self,
        _: &mut DialogManager,
        _: DialogFrameTarget,
        _: DialogFrame,
        _: &mut [[f32; 4]; 4],
    ) -> Result<(), String> {
        Ok(())
    }
    fn slot_owner_dead(&mut self, _: usize) -> Result<bool, String> {
        Ok(false)
    }
    fn additional_count(&self) -> usize {
        0
    }
    fn additional_visible(&self, _: usize) -> bool {
        false
    }
    fn remove_additional(&mut self, _: usize) -> Result<(), String> {
        Err("no additional boxes".into())
    }
    fn message_box_count(&self) -> usize {
        self.boxes.active().len()
    }
    fn update_message_box(
        &mut self,
        _: &mut DialogManager,
        index: usize,
        frame: DialogFrame,
    ) -> Result<(), String> {
        self.boxes
            .update_index(index, frame.channel, SCREEN, false)
            .map_err(|e| format!("{e:?}"))
    }
    fn message_box_dead(&self, index: usize) -> bool {
        self.boxes.dead_index(index).expect("valid index")
    }
    fn remove_message_box(&mut self, index: usize) -> Result<(), String> {
        self.boxes.remove_index(index).map_err(|e| format!("{e:?}"))
    }
    fn play_state_in_cutscene(&self) -> bool {
        false
    }
    fn submit_subtitles(&mut self, _: usize, _: f32) -> Result<(), String> {
        Err("no subtitles".into())
    }
}
#[derive(Resource)]
struct Session {
    devices: BoxDeviceState,
    device_events: usize,
    last_device_event: Option<BoxDeviceEvent>,
    manager: DialogManager,
    world: World,
    handle: MessageBoxHandle,
    entities: [Entity; 3],
    meshes: [Handle<Mesh>; 3],
    base_meshes: [Mesh; 3],
    materials: [Handle<GuiBasicMaterial>; 3],
    geometry_font: std::sync::Arc<GeometryFont>,
    aborts: usize,
    last_dot_count: i32,
    geometry_spacing: [i32; 3],
}
pub fn preview(root: &str, args: &[String]) -> Result<(), String> {
    let interactive = args.iter().any(|a| a == "--interactive");
    let input_audit = args.iter().any(|a| a == "--input-audit");
    let out = args
        .iter()
        .position(|x| x == "--render")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
        .or(if interactive {
            Some("output/.cache/message-box-interactive")
        } else {
            None
        })
        .ok_or("--render <private capture directory> or --interactive required")?;
    let root = Path::new(root);
    let mut available = vec![];
    for entry in std::fs::read_dir(root.join("Languages")).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let Some(language) = Language::from_code(&entry.file_name().to_string_lossy()) else {
            continue;
        };
        if entry.path().is_dir() {
            available.push(language);
        }
    }
    let mut languages = LanguageState::new(available);
    let mut language_events = 0;
    languages
        .set_language(
            language_argument(args, "--language")?.unwrap_or(Language::ENGLISH),
            &mut PreviewLanguageHost {
                root,
                font: None,
                binding: None,
                events: &mut language_events,
            },
        )
        .map_err(|e| format!("{e:?}"))?;
    let font = load_menu_font(root, languages.current())?;
    let switch_language = language_argument(args, "--switch-language")?;
    let Asset::Texture2D(page) = read_asset(&root.join("UI/Menu/pages.xnb"))? else {
        return Err("expected menu pages".into());
    };
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let mut app = App::new();
    if interactive {
        app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Magicka message box".into(),
                resolution: WindowResolution::new(800, 600).with_scale_factor_override(1.),
                resizable: false,
                ..default()
            }),
            ..default()
        }));
    } else {
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
            1. / 120.,
        )));
    }
    app.add_plugins(BoxDevicesPlugin)
        .add_plugins(GuiBasicPlugin)
        .insert_resource(ClearColor(Color::linear_rgb(0.2, 0.2, 0.2)))
        .insert_resource(Preview {
            interactive,
            input_audit,
            root: root.into(),
            font,
            languages,
            switch_language,
            language_events,
            page,
            output: out.into(),
            target: Handle::default(),
            frame: 0,
            states: vec![],
            waiting: args.iter().any(|a| a == "--waiting"),
        })
        .add_systems(Startup, setup)
        .add_systems(Update, drive)
        .add_systems(Last, capture)
        .run();
    Ok(())
}
fn setup(
    mut commands: Commands,
    mut preview: ResMut<Preview>,
    mut images: ResMut<Assets<Image>>,
    mut colors: ResMut<Assets<ColorMaterial>>,
    mut materials: ResMut<Assets<GuiBasicMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let (mut geometry_font, color) =
        upload_font(&preview.font.content, &mut images, &mut colors).expect("font upload");
    std::sync::Arc::make_mut(&mut geometry_font)
        .metrics
        .clone_from(&preview.font.metrics);
    let atlas = colors.get(&color).unwrap().texture.clone().unwrap();
    let page = &preview.page;
    let bytes = decode_rgba(
        page.format,
        page.width as usize,
        page.height as usize,
        &page.mips[0],
    )
    .expect("page decode");
    std::fs::write(
        preview.output.join("pages-atlas.png"),
        encode_png(page.width as usize, page.height as usize, &bytes),
    )
    .expect("private page cache");
    let mut image = Image::new(
        Extent3d {
            width: page.width as u32,
            height: page.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        bytes,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    let page_texture = images.add(image);
    let metrics = &preview.font.metrics;
    let mut world = World {
        boxes: ErrorMessageBoxes::default(),
    };
    let mut manager = DialogManager::new(vec![]).unwrap();
    let handle = if preview.waiting {
        let h = world
            .boxes
            .insert_waiting(WaitingMessageBoxState::new(SCREEN));
        world
            .boxes
            .show_waiting(
                h,
                default_message_id(),
                &preview.languages.strings,
                metrics,
                &mut manager,
                false,
            )
            .unwrap();
        world.boxes.get_waiting_mut(h).unwrap().on_abort = Some(7);
        h
    } else {
        world.boxes.insert(
            ErrorMessageBoxState::new_localized(SCREEN, &preview.languages.strings, metrics)
                .unwrap(),
        )
    };
    if !preview.waiting {
        world
            .boxes
            .show(
                handle,
                &preview
                    .languages
                    .strings
                    .get_string(error_message_id())
                    .unwrap(),
                metrics,
                &mut manager,
                false,
            )
            .unwrap();
    }
    let style = TextStyle {
        align: TextAlign::Center,
        ..Default::default()
    };
    let (body_characters, secondary_characters) = if preview.waiting {
        (
            world.boxes.get_waiting(handle).unwrap().characters.clone(),
            vec![46, 0], // Retained backing glyph; zero-dot frames submit no draw.
        )
    } else {
        let s = world.boxes.get(handle).unwrap();
        (
            s.message.encode_utf16().collect(),
            s.ok_button.as_ref().unwrap().characters.clone(),
        )
    };
    let body = TextGeometry::build(&geometry_font, &body_characters, style).unwrap();
    let ok = TextGeometry::build(
        &geometry_font,
        &secondary_characters,
        if preview.waiting {
            TextStyle::default()
        } else {
            style
        },
    )
    .unwrap();
    let mut background = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    let vertices = if preview.waiting {
        [
            [-0.5, -0.5, 0., 0.],
            [0.5, -0.5, 0., 0.],
            [0.5, 0.5, 0., 0.],
            [-0.5, 0.5, 0., 0.],
        ]
    } else {
        background_vertices([page.width as f32, page.height as f32])
    };
    background.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vertices
            .iter()
            .map(|v| [v[0], -v[1], 0.])
            .collect::<Vec<_>>(),
    );
    background.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vertices.iter().map(|v| [v[2], v[3]]).collect::<Vec<_>>(),
    );
    background.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        vec![
            if preview.waiting {
                [0., 0., 0., 1.]
            } else {
                [1.; 4]
            };
            4
        ],
    );
    background.insert_indices(Indices::U16(vec![0, 1, 2, 0, 2, 3]));
    let base_meshes = [
        background,
        geometry_mesh(&body, body.quads.len() as i32).unwrap(),
        geometry_mesh(&ok, ok.quads.len() as i32).unwrap(),
    ];
    let mesh_handles = std::array::from_fn(|i| meshes.add(base_meshes[i].clone()));
    let material_handles = [
        materials.add(GuiBasicMaterial::textured(page_texture, Vec4::ONE)),
        materials.add(GuiBasicMaterial::textured(
            atlas.clone(),
            Vec4::from_array(MENU_COLOR),
        )),
        materials.add(GuiBasicMaterial::textured(
            atlas,
            Vec4::from_array(MENU_COLOR),
        )),
    ];
    if preview.waiting {
        materials.get_mut(&material_handles[0]).unwrap().flags = UVec4::new(0, 1, 0, 1);
        for material in &material_handles[1..] {
            materials.get_mut(material).unwrap().flags = UVec4::new(1, 1, 0, 1);
        }
    }
    let entities = std::array::from_fn(|i| {
        commands
            .spawn((
                Mesh2d(mesh_handles[i].clone()),
                MeshMaterial2d(material_handles[i].clone()),
                Transform::from_xyz(0., 0., i as f32),
                Visibility::Hidden,
            ))
            .id()
    });
    let mut target = Image::new_target_texture(800, 600, TextureFormat::Rgba8Unorm, None);
    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    preview.target = images.add(target);
    let mut camera = commands.spawn((Camera2d, Msaa::Off, Tonemapping::None));
    if !preview.interactive {
        camera.insert(RenderTarget::Image(ImageRenderTarget {
            handle: preview.target.clone(),
            scale_factor: 1.,
        }));
    }
    preview.languages.subscribe(LanguageListener(1)); // MessageBox constructor subscription.
    commands.insert_resource(Session {
        devices: BoxDeviceState::default(),
        device_events: 0,
        last_device_event: None,
        manager,
        world,
        handle,
        entities,
        meshes: mesh_handles,
        base_meshes,
        materials: material_handles,
        geometry_font,
        aborts: 0,
        last_dot_count: 0,
        geometry_spacing: [
            0,
            preview.font.metrics.spacing,
            preview.font.metrics.spacing,
        ],
    });
}
fn placed(mesh: &Mesh, matrix: [[f32; 4]; 4]) -> Mesh {
    let mut result = mesh.clone();
    let Some(VertexAttributeValues::Float32x3(points)) =
        result.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("positions")
    };
    for p in points {
        p[0] = p[0] * matrix[0][0] + matrix[3][0] - 0.5 - SCREEN[0] * 0.5;
        p[1] = SCREEN[1] * 0.5 - (-p[1] * matrix[1][1] + matrix[3][1] - 0.5);
    }
    result
}
fn translation(point: [f32; 2]) -> [[f32; 4]; 4] {
    [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [point[0], point[1], 0., 1.],
    ]
}
#[derive(bevy::ecs::system::SystemParam)]
struct PreviewInput<'w, 's> {
    visibility: Query<'w, 's, &'static mut Visibility>,
    devices: ResMut<'w, BoxDevices>,
    time: Res<'w, Time>,
    cursors: Query<'w, 's, &'static mut CursorOptions, With<PrimaryWindow>>,
}
fn dispatch_device(
    session: &mut Session,
    event: BoxDeviceEvent,
    cursors: &mut Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    session.device_events += 1;
    session.last_device_event = Some(event);
    if event.show_mouse {
        for mut cursor in cursors.iter_mut() {
            cursor.visible = true;
        }
    }
    let inside_ok = session
        .world
        .boxes
        .get(session.handle)
        .and_then(|s| s.ok_button.as_ref())
        .is_some_and(|b| b.inside_bounds(event.point.map(|v| v as f32)));
    let Session {
        world,
        manager,
        aborts,
        ..
    } = session;
    world
        .boxes
        .dispatch_native(
            manager,
            MessageBoxInput {
                kind: event.kind,
                left_released: event.left_released,
                inside_ok,
            },
            || {
                for mut cursor in cursors.iter_mut() {
                    cursor.visible = false;
                }
                Ok::<_, String>(())
            },
            |_| Err("unexpected gameplay fallback".into()),
            |id, _| {
                assert_eq!(id, 7);
                *aborts += 1;
                Ok(())
            },
        )
        .expect("native device dispatch");
}
fn drive(
    mut preview: ResMut<Preview>,
    mut session: ResMut<Session>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GuiBasicMaterial>>,
    input: PreviewInput,
    mut images: ResMut<Assets<Image>>,
    mut colors: ResMut<Assets<ColorMaterial>>,
) {
    let PreviewInput {
        mut visibility,
        mut devices,
        time,
        mut cursors,
    } = input;
    preview.frame += 1;
    let frame = preview.frame;
    if let (45, Some(language)) = (frame, preview.switch_language) {
        let Preview {
            root,
            font,
            languages,
            language_events,
            ..
        } = &mut *preview;
        let Session { world, handle, .. } = &mut *session;
        let changed = languages
            .set_language(
                language,
                &mut PreviewLanguageHost {
                    root,
                    font: Some(font),
                    binding: Some((world, *handle)),
                    events: language_events,
                },
            )
            .expect("language switch");
        if changed {
            let (mut geometry_font, color) =
                upload_font(&preview.font.content, &mut images, &mut colors)
                    .expect("font reload upload");
            std::sync::Arc::make_mut(&mut geometry_font)
                .metrics
                .clone_from(&preview.font.metrics);
            let atlas = colors.get(&color).unwrap().texture.clone().unwrap();
            for handle in &session.materials[1..] {
                let mut material = materials.get_mut(handle).unwrap();
                material.texture = atlas.clone();
                material.overlay = atlas.clone();
            }
            if preview.waiting {
                let text = &session
                    .world
                    .boxes
                    .get_waiting(session.handle)
                    .unwrap()
                    .characters;
                let body = TextGeometry::build(
                    &geometry_font,
                    text,
                    TextStyle {
                        align: TextAlign::Center,
                        ..default()
                    },
                )
                .unwrap();
                session.base_meshes[1] = geometry_mesh(&body, body.quads.len() as i32).unwrap();
                session.geometry_spacing[1] = preview.font.metrics.spacing;
            }
            // Error caption/OK and waiting dots keep old geometry until dirtied;
            // Draw still binds the reloaded font's new texture and line height.
            session.geometry_font = geometry_font;
        }
    }
    if preview.interactive {
        for vk in devices.key_downs.drain(..) {
            if let Some(event) =
                BoxDeviceState::key_down(vk, !session.world.boxes.active().is_empty())
            {
                dispatch_device(&mut session, event, &mut cursors);
            }
        }
        let enabled = !session.world.boxes.active().is_empty();
        if let Some(event) = session.devices.update(devices.sample, enabled) {
            dispatch_device(&mut session, event, &mut cursors);
        }
    }
    let dt = if preview.interactive {
        time.delta_secs()
    } else if preview.waiting {
        match frame {
            10 => 0.25,
            30 | 50 => 0.5,
            70 | 90 => 0.125,
            _ => 0.,
        }
    } else {
        match frame {
            10 => 0.25,
            50 | 70 => 0.125,
            _ => 0.,
        }
    };
    let Session {
        manager,
        world,
        aborts,
        ..
    } = &mut *session;
    if !preview.interactive && preview.waiting && frame == 70 {
        world
            .boxes
            .dispatch_native(
                manager,
                MessageBoxInput {
                    kind: DialogInputKind::Escape,
                    left_released: false,
                    inside_ok: false,
                },
                || Ok::<_, String>(()),
                |_| Err("unexpected gameplay fallback".into()),
                |id, _| {
                    assert_eq!(id, 7);
                    *aborts += 1;
                    Ok(())
                },
            )
            .unwrap();
    } else if !preview.interactive && !preview.waiting && frame == 30 {
        world
            .boxes
            .dispatch(
                manager,
                DialogInputKind::Select,
                false,
                false,
                || Ok::<_, String>(()),
                |_| Err("unexpected gameplay fallback".into()),
            )
            .unwrap();
    }
    world.boxes.begin_frame(0).unwrap();
    manager
        .update(DialogFrame { channel: 0, dt }, &mut [[0.; 4]; 4], world)
        .unwrap();
    let draws = world
        .boxes
        .draw_submitted_all(0, dt, SCREEN, false)
        .unwrap();
    for entity in session.entities {
        *visibility.get_mut(entity).unwrap() = if draws.is_empty() {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
    assert!(draws.len() <= 1, "fixture requires one message box");
    if let Some((_, MessageBoxDrawKind::Waiting(draw))) = draws.first() {
        let mut backdrop = translation([SCREEN[0] * 0.5, SCREEN[1] * 0.5]);
        backdrop[0][0] = SCREEN[0];
        backdrop[1][1] = SCREEN[1];
        *meshes.get_mut(&session.meshes[0]).unwrap() = placed(&session.base_meshes[0], backdrop);
        *meshes.get_mut(&session.meshes[1]).unwrap() =
            placed(&session.base_meshes[1], translation(draw.message_position));
        if draw.dot_count == 0 {
            // Native Text.Draw skips all device commands for zero primitives.
            *visibility.get_mut(session.entities[2]).unwrap() = Visibility::Hidden;
        } else {
            if draw.dot_count != session.last_dot_count {
                let dots = TextGeometry::build(
                    &session.geometry_font,
                    &session
                        .world
                        .boxes
                        .get_waiting(session.handle)
                        .unwrap()
                        .dots()
                        .encode_utf16()
                        .collect::<Vec<_>>(),
                    TextStyle::default(),
                )
                .unwrap();
                session.base_meshes[2] = geometry_mesh(&dots, dots.quads.len() as i32).unwrap();
                session.geometry_spacing[2] = preview.font.metrics.spacing;
            }
            *meshes.get_mut(&session.meshes[2]).unwrap() =
                placed(&session.base_meshes[2], translation(draw.dot_position));
        }
        session.last_dot_count = draw.dot_count;
        materials.get_mut(&session.materials[0]).unwrap().color =
            Vec4::from_array(draw.overlay_color);
        for material in &session.materials[1..] {
            materials.get_mut(material).unwrap().color = Vec4::from_array(draw.text_color);
        }
    } else if let Some((_, MessageBoxDrawKind::Error(draw))) = draws.first() {
        let state = session.world.boxes.get(session.handle).unwrap();
        let button = state
            .ok_button
            .as_ref()
            .unwrap()
            .draw_plan(1., &preview.font.metrics)
            .unwrap();
        let transforms = [
            translation(draw.background.unwrap().center),
            translation(draw.message_position),
            button.transform,
        ];
        for (i, matrix) in transforms.into_iter().enumerate() {
            *meshes.get_mut(&session.meshes[i]).unwrap() = placed(&session.base_meshes[i], matrix);
        }
        materials.get_mut(&session.materials[0]).unwrap().color =
            Vec4::from_array(draw.background.unwrap().color);
        materials.get_mut(&session.materials[2]).unwrap().color = Vec4::from_array(button.color);
    }
}
fn capture(mut commands: Commands, mut preview: ResMut<Preview>, session: Res<Session>) {
    let phase = if preview.interactive {
        if !preview.input_audit {
            return;
        }
        let base = if preview.waiting {
            &session
                .world
                .boxes
                .get_waiting(session.handle)
                .unwrap()
                .base
        } else {
            &session.world.boxes.get(session.handle).unwrap().base
        };
        let has = |phase: &str| preview.states.iter().any(|s| s["phase"] == phase);
        if !has("full") && base.alpha == 1. && !base.killed {
            "full"
        } else if session.world.boxes.active().is_empty()
            && session.world.boxes.submissions(0).unwrap().is_empty()
            && !has("hidden")
        {
            "hidden"
        } else if !has("selected") && base.killed {
            "selected"
        } else if base.killed && base.alpha > 0. && base.alpha < 0.75 && !has("fade") {
            "fade"
        } else if !preview.waiting
            && !has("hover")
            && session.world.boxes.get(session.handle).unwrap().ok_selected
        {
            "hover"
        } else if !preview.waiting
            && !has("pressed")
            && !base.killed
            && session
                .last_device_event
                .is_some_and(|e| e.kind == DialogInputKind::MouseClick && !e.left_released)
        {
            "pressed"
        } else {
            return;
        }
    } else if preview.waiting {
        match preview.frame {
            20 => "full",
            40 => "one_dot",
            48 if preview.switch_language.is_some() => "language_changed",
            60 => "two_dots",
            80 => "fade",
            100 => "hidden",
            _ => return,
        }
    } else {
        match preview.frame {
            20 => "full",
            40 => "selected",
            48 if preview.switch_language.is_some() => "language_changed",
            60 => "fade",
            80 => "hidden",
            _ => return,
        }
    };
    let mut record = if preview.waiting {
        let state = session.world.boxes.get_waiting(session.handle).unwrap();
        serde_json::json!({"phase":phase,"alpha":state.base.alpha,"timer":state.timer,"killed":state.base.killed,"dot_count":state.dot_count(),
            "active":session.world.boxes.active().len(),"submitted":session.world.boxes.submissions(0).unwrap().len(),"aborts":session.aborts,
            "caption_id":state.message_id,"caption_matches_lookup":state.message==preview.languages.strings.get_string(state.message_id),
            "caption_units":state.message.as_deref().unwrap_or("").encode_utf16().count(),"dot_position":state.dot_position})
    } else {
        let state = session.world.boxes.get(session.handle).unwrap();
        let button = state.ok_button.as_ref().unwrap();
        let title = preview.languages.strings.get_string(button.hash);
        let current_units = title
            .as_deref()
            .unwrap_or("")
            .encode_utf16()
            .collect::<Vec<_>>();
        let display_units = button
            .characters
            .iter()
            .copied()
            .take_while(|&u| u != 0)
            .collect::<Vec<_>>();
        serde_json::json!({"phase":phase,"alpha":state.base.alpha,"timer":state.timer,"killed":state.base.killed,
            "active":session.world.boxes.active().len(),"submitted":session.world.boxes.submissions(0).unwrap().len(),
            "caption_id":error_message_id(),"caption_matches_lookup":Some(&state.message)==preview.languages.strings.get_string(error_message_id()).as_ref(),
            "caption_units":state.message.encode_utf16().count(),"ok_id":button.hash,"ok_matches_lookup":display_units==current_units,
            "ok_units":display_units.len(),"ok_name_matches_lookup":button.text==title,"ok_bounds":[button.top_left,button.bottom_right],"ok_selected":state.ok_selected})
    };
    record["language"] = serde_json::json!(preview.languages.current().0);
    record["font_source_language"] = serde_json::json!(preview.font.source_language.0);
    record["font_spacing"] = serde_json::json!(preview.font.metrics.spacing);
    record["language_events"] = serde_json::json!(preview.language_events);
    record["geometry_spacing"] = serde_json::json!(session.geometry_spacing);
    record["device_events"] = serde_json::json!(session.device_events);
    record["device_kind"] =
        serde_json::json!(session.last_device_event.map(|e| format!("{:?}", e.kind)));
    preview.states.push(record);
    std::fs::write(
        preview.output.join("states.json"),
        serde_json::to_vec_pretty(&preview.states).unwrap(),
    )
    .unwrap();
    let screenshot = if preview.interactive {
        Screenshot::primary_window()
    } else {
        Screenshot::image(preview.target.clone())
    };
    commands.spawn(screenshot).observe(
        move |captured: On<ScreenshotCaptured>,
              preview: Res<Preview>,
              mut exit: MessageWriter<AppExit>| {
            let pixels = if preview.interactive {
                captured
                    .image
                    .clone()
                    .try_into_dynamic()
                    .expect("window screenshot conversion")
                    .to_rgba8()
                    .into_raw()
            } else {
                captured.image.data.as_ref().unwrap().clone()
            };
            std::fs::write(
                preview.output.join(format!("{phase}.png")),
                encode_png(800, 600, &pixels),
            )
            .unwrap();
            if phase == "hidden" {
                exit.write(AppExit::Success);
            }
        },
    );
}
