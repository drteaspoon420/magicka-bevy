//! Deterministic retained-session GPU fixture; originals are read as data only.
use crate::{
    native_dialog::{
        NativeDialogBox, NativeDialogCommand, NativeDialogFrameSet, NativeDialogPlugin,
        NativeDialogSession,
    },
    native_text::{
        geometry_mesh, upload_font, NativeBitmapText, NativeTextDelta, NativeTextPlugin,
    },
    text_box_visual::{read_asset, upload_background, TextBoxBackgroundPlugin, TextBoxMaterial},
};
use bevy::{
    app::{AppExit, ScheduleRunnerPlugin},
    camera::{ImageRenderTarget, RenderTarget},
    core_pipeline::tonemapping::Tonemapping,
    prelude::*,
    render::{
        render_resource::{TextureFormat, TextureUsages},
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
    window::ExitCondition,
    winit::WinitPlugin,
};
use magicka_content::{
    assets::{Asset, BitmapFont, Texture2D},
    texture::encode_png,
};
use magicka_runtime::{
    dialog_definition::{DialogDefinition, MessageDefinition},
    dialog_frame::DialogFrame,
    dialog_manager::{DialogManager, DialogManagerEffect},
    dialog_text_host::{DialogMessageKey, DialogTextFrameWorld, DialogTextHost, DialogTextWorld},
    interaction_playback::InteractionLocation,
    localization::LocalizationTable,
    message_playback::{MessageEffect, MessageHost, MessageOwner},
    message_textbox::MessageTextBoxWorld,
    scene_loading::custom_hash,
    text_box::TextBoxOwner,
    text_box_initialize::TextBoxInitHost,
    text_geometry::{TextGeometry, TextStyle},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

struct PreviewWorld {
    background: Handle<TextBoxMaterial>,
    loads: usize,
    ends: usize,
    effects: Vec<String>,
    cutscene: bool,
}
impl MessageHost for PreviewWorld {
    type Error = String;
    fn resolve_owner(&mut self, _: i32) -> Result<Option<MessageOwner>, String> {
        Ok(None)
    }
    fn cue_stopping(&self) -> Option<bool> {
        None
    }
    fn effect(&mut self, _: &MessageDefinition, _: MessageEffect) -> Result<(), String> {
        Err("fixture has no entity/audio/camera effect host".into())
    }
}
impl TextBoxInitHost for PreviewWorld {
    type Error = String;
    fn load_background(&mut self, path: &str) -> Result<(), String> {
        if path != "UI/HUD/Dialog_Say" || self.background == Handle::default() {
            return Err("invalid cached background".into());
        }
        self.loads += 1;
        Ok(())
    }
}
impl MessageTextBoxWorld for PreviewWorld {
    fn entity_scene(&mut self, _: i32) -> Result<Option<i32>, String> {
        Err("fixture has no entity scene".into())
    }
}
impl DialogTextWorld for PreviewWorld {
    fn bind_message(&mut self, _: DialogMessageKey) -> Result<(), String> {
        Ok(())
    }
    fn lock_controller(&mut self, _: i32) -> Result<(), String> {
        Err("fixture has no controllers".into())
    }
    fn unlock_controller(&mut self, _: i32) -> Result<(), String> {
        Err("fixture has no controllers".into())
    }
    fn stop_message_cue(&mut self) -> Result<(), String> {
        self.ends += 1;
        Ok(())
    } // null cue: native no-op
    fn manager_effect(&mut self, effect: DialogManagerEffect) -> Result<(), String> {
        self.effects.push(format!("{effect:?}"));
        Ok(())
    }
}
impl DialogTextFrameWorld for PreviewWorld {
    fn text_owner(&self, _: i32) -> Option<TextBoxOwner> {
        None
    }
    fn owner_dead(&self, _: i32) -> bool {
        false
    }
    fn in_cutscene(&self) -> bool {
        self.cutscene
    }
    fn message_box_count(&self) -> usize {
        0
    }
    fn update_message_box(&mut self, _: usize, _: DialogFrame) -> Result<(), String> {
        Err("fixture has no message boxes".into())
    }
    fn message_box_dead(&self, _: usize) -> bool {
        false
    }
    fn remove_message_box(&mut self, _: usize) -> Result<(), String> {
        Err("fixture has no message boxes".into())
    }
    fn submit_subtitles(&mut self, _: usize, _: f32) -> Result<(), String> {
        Err("fixture has no subtitles".into())
    }
}
#[derive(Resource)]
struct Preview {
    body: BitmapFont,
    name: BitmapFont,
    background: Texture2D,
    output: PathBuf,
    target: Handle<Image>,
    frame: u32,
    states: Vec<serde_json::Value>,
    cutscene: bool,
}
pub fn preview(root: &str, args: &[String]) -> Result<(), String> {
    let output = args
        .iter()
        .position(|a| a == "--render")
        .and_then(|i| args.get(i + 1))
        .ok_or("--render <private capture directory> required")?;
    let root = Path::new(root);
    let Asset::BitmapFont(body) = read_asset(&root.join("Languages/eng/Font/Maiandra14.xnb"))?
    else {
        return Err("expected body font".into());
    };
    let Asset::BitmapFont(name) = read_asset(&root.join("Languages/eng/Font/Maiandra16.xnb"))?
    else {
        return Err("expected name font".into());
    };
    let Asset::Texture2D(background) = read_asset(&root.join("UI/HUD/Dialog_Say.xnb"))? else {
        return Err("expected background".into());
    };
    std::fs::create_dir_all(output).map_err(|e| e.to_string())?;
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
        .add_plugins((
            NativeTextPlugin,
            TextBoxBackgroundPlugin,
            NativeDialogPlugin::<PreviewWorld>::default(),
        ))
        .insert_resource(ClearColor(Color::linear_rgb(0.2, 0.2, 0.2)))
        .insert_resource(Preview {
            body,
            name,
            background,
            output: output.into(),
            target: Handle::default(),
            frame: 0,
            states: vec![],
            cutscene: args.iter().any(|a| a == "--cutscene"),
        })
        .add_systems(Startup, setup)
        .add_systems(Update, drive.before(NativeDialogFrameSet::Update))
        .add_systems(Last, capture)
        .run();
    Ok(())
}
fn setup(
    mut commands: Commands,
    mut preview: ResMut<Preview>,
    mut images: ResMut<Assets<Image>>,
    mut text_materials: ResMut<Assets<ColorMaterial>>,
    mut box_materials: ResMut<Assets<TextBoxMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let (body, body_material) =
        upload_font(&preview.body, &mut images, &mut text_materials).unwrap();
    let (mut name, name_material) =
        upload_font(&preview.name, &mut images, &mut text_materials).unwrap();
    Arc::get_mut(&mut name).unwrap().metrics.spacing = 1;
    // Prewarm the original private asset cache; Initialize then resolves this
    // cached background synchronously on both text entries.
    let material = upload_background(&preview.background, &mut images, &mut box_materials).unwrap();
    let mut strings = LocalizationTable::default();
    for (id, text) in [
        ("#live_owner", "Archivist"),
        (
            "#live_one",
            "Recovered dialog box.\n[c=1,0,0]Red[/c] and [c=0,1,0]green[/c] text.",
        ),
        (
            "#live_two",
            "Replacement dialog box.\n[c=1,0,0]Red[/c] and [c=0,1,0]green[/c] text.",
        ),
    ] {
        strings.strings.insert(custom_hash(id), text.into());
    }
    let definitions = DialogDefinition::read_collection(&[br##"<Dialogs><Dialog id="live" repeatFrom="-1"><Interact><Message character="#live_owner"><Text ttl="0.1">#live_one</Text><Text ttl="0.2">#live_two</Text></Message></Interact></Dialog></Dialogs>"##]).unwrap();
    let world = PreviewWorld {
        background: material.clone(),
        loads: 0,
        ends: 0,
        effects: vec![],
        cutscene: preview.cutscene,
    };
    let mut host = DialogTextHost::new(
        world,
        strings,
        body.metrics.clone(),
        name.metrics.clone(),
        100,
    );
    host.scene = Some(1);
    host.initialize_layouts(&definitions).unwrap();
    let mut session =
        NativeDialogSession::new(DialogManager::new(definitions).unwrap(), host, [800., 240.]);
    session.commands.push_back(NativeDialogCommand::Start {
        dialog: custom_hash("live"),
        location: InteractionLocation::Screen([0.; 2]),
        controller: None,
    });
    commands.insert_resource(session);
    let empty = || geometry_mesh(&TextGeometry::default(), 0).unwrap();
    let background_mesh = meshes.add(empty());
    let background = commands
        .spawn((
            Mesh2d(background_mesh.clone()),
            MeshMaterial2d(material),
            Transform::default(),
            Visibility::Hidden,
        ))
        .id();
    let owner_mesh = meshes.add(empty());
    let owner = commands
        .spawn((
            Mesh2d(owner_mesh.clone()),
            MeshMaterial2d(name_material.clone()),
            Transform::default(),
            Visibility::Hidden,
        ))
        .id();
    let binding = if preview.cutscene {
        NativeDialogBox::for_cutscene(background, background_mesh, owner, owner_mesh, name.clone())
    } else {
        NativeDialogBox::new(
            0,
            background,
            background_mesh,
            owner,
            owner_mesh,
            name.clone(),
        )
    };
    let (body, body_material) = if preview.cutscene {
        (name, name_material.clone())
    } else {
        (body, body_material)
    };
    commands.spawn((
        NativeBitmapText::new(body, "", TextStyle::dialogue()),
        Mesh2d(meshes.add(empty())),
        MeshMaterial2d(body_material),
        Transform::default(),
        Visibility::Hidden,
        binding,
    ));
    let mut target = Image::new_target_texture(800, 240, TextureFormat::Rgba8Unorm, None);
    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    preview.target = images.add(target);
    commands.spawn((
        Camera2d,
        RenderTarget::Image(ImageRenderTarget {
            handle: preview.target.clone(),
            scale_factor: 1.,
        }),
        Msaa::Off,
        Tonemapping::None,
    ));
}
fn drive(
    mut preview: ResMut<Preview>,
    mut clock: ResMut<NativeTextDelta>,
    mut session: ResMut<NativeDialogSession<PreviewWorld>>,
) {
    preview.frame += 1;
    clock.0 = Some(match preview.frame {
        10 | 30 | 100 => 0.125,
        50 | 90 => 10.,
        70 => 0.11,
        _ => 0.,
    });
    if preview.frame == 100 {
        session.commands.push_back(NativeDialogCommand::EndAll);
    }
    if let Some(error) = &session.failure {
        panic!("live dialog preview: {error}");
    }
    if preview.frame > 180 {
        panic!("live dialog capture timeout");
    }
}
fn capture(
    mut commands: Commands,
    mut preview: ResMut<Preview>,
    session: Res<NativeDialogSession<PreviewWorld>>,
) {
    if let Some(error) = &session.failure {
        panic!("live dialog preview: {error}");
    }
    let phase = match preview.frame {
        20 => "growth",
        40 => "partial",
        60 => "full",
        80 => "replacement",
        110 => "hidden",
        _ => return,
    };
    let b = if preview.cutscene {
        &session.host.cutscene.widget.state
    } else {
        &session.host.boxes[0].state
    };
    let state = serde_json::json!({"phase":phase,"cutscene":preview.cutscene,"frame":preview.frame,"scale":b.scale,"grow":b.grow,"ttl":b.ttl,
        "visible":b.typing.visible_characters,"glyphs":b.typing.primitive_count/2,"text":b.text,"size":b.size,
        "active":session.manager.dialog_active(custom_hash("live")),"background_loads":session.host.world.loads,
        "end_calls":session.host.world.ends,"effects":session.host.world.effects});
    info!("live dialog preview: {state}");
    preview.states.push(state);
    std::fs::write(
        preview.output.join("states.json"),
        serde_json::to_vec_pretty(&preview.states).unwrap(),
    )
    .unwrap();
    commands
        .spawn(Screenshot::image(preview.target.clone()))
        .observe(
            move |captured: On<ScreenshotCaptured>,
                  preview: Res<Preview>,
                  mut exit: MessageWriter<AppExit>| {
                let png = encode_png(
                    800,
                    240,
                    captured.image.data.as_ref().expect("capture pixels"),
                );
                std::fs::write(preview.output.join(format!("{phase}.png")), png)
                    .expect("save private live preview");
                if phase == "hidden" {
                    exit.write(AppExit::Success);
                }
            },
        );
}
