//! Original TextBoxEffect background, plus named-box integration render fixture.
use crate::native_text::{
    geometry_mesh, upload_font, NativeBitmapText, NativeTextDelta, NativeTextPlugin,
};
use bevy::{
    app::{AppExit, ScheduleRunnerPlugin},
    asset::{embedded_asset, RenderAssetUsages},
    camera::{ImageRenderTarget, RenderTarget},
    core_pipeline::tonemapping::Tonemapping,
    image::{ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, PrimitiveTopology},
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
    parse_primary,
    texture::{decode_rgba, encode_png},
    xna::XnaObject,
    Content,
};
use magicka_runtime::{
    dialog_definition::DialogDefinition,
    dialog_manager::{DialogManager, DialogManagerEffect},
    dialog_text_host::{DialogMessageKey, DialogTextHost, DialogTextWorld},
    interaction_playback::InteractionLocation,
    localization::LocalizationTable,
    message_playback::{MessageEffect, MessageHost, MessageOwner},
    message_textbox::MessageTextBoxWorld,
    scene_loading::custom_hash,
    text_box_background,
    text_box_initialize::TextBoxInitHost,
    text_geometry::{TextGeometry, TextStyle},
};
use std::{path::Path, sync::Arc, time::Duration};

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct TextBoxMaterial {
    #[uniform(0)]
    pub color: Vec4,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
}
impl Material2d for TextBoxMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/text_box_background.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}
pub struct TextBoxBackgroundPlugin;
impl Plugin for TextBoxBackgroundPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "text_box_background.wgsl");
        app.add_plugins(Material2dPlugin::<TextBoxMaterial>::default());
    }
}
pub fn background_mesh(center: [f32; 2], size: [f32; 2], scale: f32, screen: [f32; 2]) -> Mesh {
    let vertices = text_box_background::vertices(center, size, scale);
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vertices
            .map(|v| {
                [
                    v.pixel_position[0] - screen[0] * 0.5,
                    screen[1] * 0.5 - v.pixel_position[1],
                    0.0,
                ]
            })
            .to_vec(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vertices.map(|v| v.uv).to_vec());
    mesh.insert_indices(Indices::U16(text_box_background::INDICES.to_vec()));
    mesh
}
pub(crate) fn read_asset(path: &Path) -> Result<Asset, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let xnb = magicka_xnb::xnb::parse(&bytes).map_err(|e| e.to_string())?;
    let parsed = parse_primary(&xnb)
        .ok_or("null content")?
        .map_err(|e| e.to_string())?;
    let Content::Xna(object) = parsed.content else {
        return Err("expected XNA object".into());
    };
    let XnaObject::Asset(asset) = *object else {
        return Err("expected asset".into());
    };
    Ok(*asset)
}
struct BackgroundUpload<'a> {
    texture: &'a Texture2D,
    images: &'a mut Assets<Image>,
    materials: &'a mut Assets<TextBoxMaterial>,
    material: Option<Handle<TextBoxMaterial>>,
}
pub(crate) fn upload_background(
    texture: &Texture2D,
    images: &mut Assets<Image>,
    materials: &mut Assets<TextBoxMaterial>,
) -> Result<Handle<TextBoxMaterial>, String> {
    let mut upload = BackgroundUpload {
        texture,
        images,
        materials,
        material: None,
    };
    upload.load_background("UI/HUD/Dialog_Say")?;
    upload.material.ok_or("missing background material".into())
}
impl TextBoxInitHost for BackgroundUpload<'_> {
    type Error = String;
    fn load_background(&mut self, path: &str) -> Result<(), String> {
        if path != "UI/HUD/Dialog_Say" {
            return Err("unexpected background".into());
        }
        let texture = self.texture;
        let data = decode_rgba(
            texture.format,
            texture.width as usize,
            texture.height as usize,
            texture.mips.first().ok_or("missing background mip")?,
        )
        .ok_or("unsupported background encoding")?;
        let mut image = Image::new(
            Extent3d {
                width: texture.width as u32,
                height: texture.height as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
        let handle = self.images.add(image);
        self.material = Some(self.materials.add(TextBoxMaterial {
            color: Vec4::ONE,
            texture: handle,
        }));
        Ok(())
    }
}
impl MessageHost for BackgroundUpload<'_> {
    type Error = String;
    fn resolve_owner(&mut self, _: i32) -> Result<Option<MessageOwner>, String> {
        Ok(None)
    }
    fn cue_stopping(&self) -> Option<bool> {
        None
    }
    fn effect(
        &mut self,
        _: &magicka_runtime::dialog_definition::MessageDefinition,
        _: MessageEffect,
    ) -> Result<(), String> {
        Err("preview has no world/audio effect host".into())
    }
}
impl MessageTextBoxWorld for BackgroundUpload<'_> {
    fn entity_scene(&mut self, _: i32) -> Result<Option<i32>, String> {
        Err("preview has no entity scene".into())
    }
}
impl DialogTextWorld for BackgroundUpload<'_> {
    fn bind_message(&mut self, _: DialogMessageKey) -> Result<(), String> {
        Ok(())
    }
    fn lock_controller(&mut self, _: i32) -> Result<(), String> {
        Err("preview has no controller host".into())
    }
    fn unlock_controller(&mut self, _: i32) -> Result<(), String> {
        Err("preview has no controller host".into())
    }
    fn stop_message_cue(&mut self) -> Result<(), String> {
        Err("preview has no cue host".into())
    }
    fn manager_effect(&mut self, _: DialogManagerEffect) -> Result<(), String> {
        Err("preview has no manager world host".into())
    }
}
#[derive(Resource)]
struct Preview {
    body: BitmapFont,
    name: BitmapFont,
    background: Texture2D,
    output: String,
    time: f32,
    scale: f32,
    target: Handle<Image>,
    frames: u32,
}
pub fn preview(root: &str, args: &[String]) -> Result<(), String> {
    let get = |name: &str| {
        args.iter()
            .position(|x| x == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
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
    let scale = get("--box-scale")
        .unwrap_or_else(|| "1".into())
        .parse::<f32>()
        .map_err(|e| e.to_string())?;
    let time = get("--text-time")
        .unwrap_or_else(|| "10".into())
        .parse::<f32>()
        .map_err(|e| e.to_string())?;
    if !scale.is_finite() || !(0.0..=1.0).contains(&scale) || !time.is_finite() || time < 0.0 {
        return Err("invalid preview scale/time".into());
    }
    let output = get("--render").ok_or("--render <private output.png> required")?;
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
    .add_plugins((NativeTextPlugin, TextBoxBackgroundPlugin))
    .insert_resource(ClearColor(Color::linear_rgb(0.2, 0.2, 0.2)))
    .insert_resource(Preview {
        body,
        name,
        background,
        output,
        time,
        scale,
        target: Handle::default(),
        frames: 0,
    })
    .add_systems(Startup, setup)
    .add_systems(Update, capture)
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
    let mut strings = LocalizationTable::default();
    strings
        .strings
        .insert(custom_hash("#preview_owner"), "Archivist".into());
    strings.strings.insert(
        custom_hash("#preview_body"),
        "Recovered dialog box.\n[c=1,0,0]Red[/c] and [c=0,1,0]green[/c] text.".into(),
    );
    let dialogs=DialogDefinition::read_collection(&[b"<Dialogs><Dialog><Interact><Message character=\"#preview_owner\"><Text>#preview_body</Text></Message></Interact></Dialog></Dialogs>"]).unwrap();
    let world = BackgroundUpload {
        texture: &preview.background,
        images: &mut images,
        materials: &mut box_materials,
        material: None,
    };
    let mut host = DialogTextHost::new(
        world,
        strings,
        body.metrics.clone(),
        name.metrics.clone(),
        100,
    );
    host.scene = Some(1);
    host.initialize_layouts(&dialogs).unwrap();
    let dialog_id = dialogs[0].id;
    let mut manager = DialogManager::new(dialogs).unwrap();
    assert_eq!(
        manager
            .start(
                dialog_id,
                InteractionLocation::Screen([0.; 2]),
                None,
                false,
                &mut host
            )
            .unwrap(),
        Some(0)
    );
    let material = host.world.material.take().unwrap();
    let mut widget = std::mem::take(&mut host.boxes[0]);
    widget.state.scale = preview.scale;
    let center = [400., 120.];
    let size = widget.state.size.map(|v| v as f32);
    let scale = preview.scale;
    commands.spawn((
        Mesh2d(meshes.add(background_mesh(center, size, scale, [800., 240.]))),
        MeshMaterial2d(material),
    ));
    let origins = widget.draw_origins(center, scale, name.metrics.line_height);
    let transform = |p: [f32; 2]| {
        Transform::from_xyz(p[0] - 400., 120. - p[1], 2.).with_scale(Vec3::new(scale, scale, 1.))
    };
    let mut text = NativeBitmapText::new(
        body,
        widget.state.text.as_deref().unwrap(),
        TextStyle::dialogue(),
    );
    text.typing = widget.state.typing;
    text.reveal_enabled = scale > 0.99;
    commands.spawn((
        text,
        Mesh2d(meshes.add(geometry_mesh(&TextGeometry::default(), 0).unwrap())),
        MeshMaterial2d(body_material),
        transform(origins.body),
    ));
    if let Some(position) = origins.owner_name {
        let geometry =
            TextGeometry::build(&name, &widget.name_characters, TextStyle::dialogue()).unwrap();
        commands.spawn((
            Mesh2d(meshes.add(geometry_mesh(&geometry, geometry.quads.len() as i32).unwrap())),
            MeshMaterial2d(name_material),
            transform(position),
        ));
    }
    info!(
        "named box preview: size {}x{}, scale {scale}",
        size[0], size[1]
    );
    let mut target = Image::new_target_texture(800, 240, TextureFormat::Rgba8Unorm, None);
    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    preview.target = images.add(target);
    commands.spawn((
        Camera2d,
        RenderTarget::Image(ImageRenderTarget {
            handle: preview.target.clone(),
            scale_factor: 1.0,
        }),
        Msaa::Off,
        Tonemapping::None,
    ));
}
fn capture(
    mut commands: Commands,
    mut preview: ResMut<Preview>,
    mut clock: ResMut<NativeTextDelta>,
    texts: Query<&NativeBitmapText>,
) {
    preview.frames += 1;
    clock.0 = Some(if preview.frames == 3 {
        preview.time
    } else {
        0.0
    });
    if let Some(error) = texts.iter().find_map(|t| t.failure.as_ref()) {
        panic!("named box preview: {error}")
    }
    if preview.frames == 30 {
        for text in &texts {
            info!(
                "named box preview: {} visible / {} body glyphs",
                text.typing.visible_characters,
                text.typing.primitive_count / 2
            );
        }
        commands
            .spawn(Screenshot::image(preview.target.clone()))
            .observe(
                |captured: On<ScreenshotCaptured>,
                 preview: Res<Preview>,
                 mut exit: MessageWriter<AppExit>| {
                    let png = encode_png(
                        800,
                        240,
                        captured.image.data.as_ref().expect("capture pixels"),
                    );
                    std::fs::write(&preview.output, png).expect("save private named box preview");
                    exit.write(AppExit::Success);
                },
            );
    }
    if preview.frames > 240 {
        panic!("named box capture timeout")
    }
}
