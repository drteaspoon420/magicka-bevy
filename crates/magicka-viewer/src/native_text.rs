//! Bevy atlas upload and native geometry/reveal adapter. Preview files are private.
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin};
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
    window::ExitCondition,
    winit::WinitPlugin,
};
use magicka_content::{
    assets::{Asset, BitmapFont},
    parse_primary,
    texture::decode_rgba,
    xna::XnaObject,
    Content,
};
use magicka_runtime::{
    text_box::TypingState,
    text_geometry::{GeometryFont, TextGeometry, TextStyle},
};
use std::{sync::Arc, time::Duration};

/// GUIBasicEffect font path for the encoded play GUI buffer. Straight alpha
/// coverage stays separate from RGB, including during title/end-game fades.
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct OverlayFontMaterial {
    #[uniform(0)]
    pub color: Vec4,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
}
impl Material2d for OverlayFontMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/overlay_font.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

#[derive(Resource, Default)]
pub struct NativeTextDelta(pub Option<f32>);
#[derive(Component)]
pub struct NativeBitmapText {
    pub font: Arc<GeometryFont>,
    pub typing: TypingState,
    pub style: TextStyle,
    pub reveal_enabled: bool,
    pub suspended: bool,
    pub failure: Option<String>,
    geometry: Option<TextGeometry>,
    last_visible: Option<i32>,
}
impl NativeBitmapText {
    pub fn new(font: Arc<GeometryFont>, text: &str, style: TextStyle) -> Self {
        let mut typing = TypingState::default();
        typing.set_text(Some(text));
        Self {
            font,
            typing,
            style,
            reveal_enabled: true,
            suspended: false,
            failure: None,
            geometry: None,
            last_visible: None,
        }
    }
}
pub struct NativeTextPlugin;
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum NativeTextSet {
    Mesh,
}
impl Plugin for NativeTextPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "overlay_font.wgsl");
        app.add_plugins(Material2dPlugin::<OverlayFontMaterial>::default());
        app.init_resource::<NativeTextDelta>()
            .add_systems(PostUpdate, update_text_mesh.in_set(NativeTextSet::Mesh));
    }
}
pub fn geometry_mesh(geometry: &TextGeometry, visible: i32) -> Result<Mesh, String> {
    let visible = usize::try_from(visible).map_err(|_| "negative visible count")?;
    if visible > geometry.quads.len() {
        return Err("visible count exceeds geometry".into());
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    if visible == 0 {
        // Bevy 0.19 skips allocating an empty mesh, then still uploads its
        // vertex/index buffers. A degenerate transparent triangle allocates
        // both buffers and rasterizes nothing. Native reveal counts stay zero.
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0., 0., 0.]]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0., 0.]]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0., 0., 0., 0.]]);
        mesh.insert_indices(Indices::U16(vec![0, 0, 0]));
        return Ok(mesh);
    }
    // Native coordinates have Y down; Camera2d has Y up. Atlas UVs stay unchanged.
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        geometry
            .quads
            .iter()
            .flat_map(|q| {
                q.vertices
                    .iter()
                    .map(|v| [v.position[0], -v.position[1], 0.0])
            })
            .collect::<Vec<_>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        geometry
            .quads
            .iter()
            .flat_map(|q| q.vertices.iter().map(|v| v.uv))
            .collect::<Vec<_>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        geometry
            .quads
            .iter()
            .flat_map(|q| q.vertices.iter().map(|v| v.color))
            .collect::<Vec<_>>(),
    );
    let mut indices = geometry.indices();
    indices.truncate(visible * 6);
    mesh.insert_indices(Indices::U16(indices));
    Ok(mesh)
}
fn update_text_mesh(
    clock: Res<NativeTextDelta>,
    time: Res<Time<Real>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut text: Query<(&mut NativeBitmapText, &Mesh2d)>,
) {
    let dt = clock.0.unwrap_or(time.delta_secs());
    for (mut text, handle) in &mut text {
        if text.failure.is_some() || text.suspended {
            continue;
        }
        let result = (|| -> Result<(), String> {
            if text.reveal_enabled {
                text.typing
                    .update(dt)
                    .map_err(|e| format!("typing: {e:?}"))?;
            }
            let dirty = text.typing.dirty;
            let new_geometry = if dirty || text.geometry.is_none() {
                Some(
                    TextGeometry::build(&text.font, &text.typing.characters, text.style)
                        .map_err(|e| format!("geometry: {e:?}"))?,
                )
            } else {
                None
            };
            if dirty
                || new_geometry.is_some()
                || text.last_visible != Some(text.typing.visible_characters)
            {
                let geometry = new_geometry
                    .as_ref()
                    .or(text.geometry.as_ref())
                    .ok_or("missing geometry")?;
                let mesh = geometry_mesh(geometry, text.typing.visible_characters)?;
                *meshes.get_mut(&handle.0).ok_or("missing mesh asset")? = mesh;
                if let Some(geometry) = new_geometry {
                    text.typing.publish_primitives(geometry.primitive_count());
                    text.geometry = Some(geometry);
                }
                text.last_visible = Some(text.typing.visible_characters);
            }
            Ok(())
        })();
        if let Err(error) = result {
            error!("native bitmap text: {error}");
            text.failure = Some(error);
        }
    }
}
/// A font's decoded atlas re-packed with transparent glyph gutters
/// (`text_geometry::padded_atlas`) and the matching geometry font.
pub fn padded_font_atlas(font: &BitmapFont) -> Result<(GeometryFont, [u32; 2], Vec<u8>), String> {
    let Some(XnaObject::Asset(asset)) = &font.texture else {
        return Err("missing atlas".into());
    };
    let Asset::Texture2D(texture) = asset.as_ref() else {
        return Err("missing texture2d".into());
    };
    let data = decode_rgba(
        texture.format,
        texture.width as usize,
        texture.height as usize,
        texture.mips.first().ok_or("missing atlas mip")?,
    )
    .ok_or("unsupported atlas encoding")?;
    let (padded, data, size) = magicka_runtime::text_geometry::padded_atlas(
        font,
        &data,
        [texture.width, texture.height],
        2,
    );
    let geometry_font =
        GeometryFont::with_atlas_size(&padded, size).map_err(|e| format!("font: {e:?}"))?;
    Ok((geometry_font, [size[0] as u32, size[1] as u32], data))
}
pub fn upload_font(
    font: &BitmapFont,
    images: &mut Assets<Image>,
    materials: &mut Assets<ColorMaterial>,
) -> Result<(Arc<GeometryFont>, Handle<ColorMaterial>), String> {
    let (geometry_font, atlas) = upload_atlas(font, images)?;
    let material = materials.add(ColorMaterial {
        texture: Some(atlas),
        alpha_mode: AlphaMode2d::Blend,
        ..default()
    });
    Ok((geometry_font, material))
}

pub fn upload_overlay_font(
    font: &BitmapFont,
    images: &mut Assets<Image>,
    materials: &mut Assets<OverlayFontMaterial>,
) -> Result<(Arc<GeometryFont>, Handle<OverlayFontMaterial>), String> {
    let (geometry_font, texture) = upload_atlas(font, images)?;
    let material = materials.add(OverlayFontMaterial {
        color: Vec4::ONE,
        texture,
    });
    Ok((geometry_font, material))
}

pub(crate) fn upload_atlas(
    font: &BitmapFont,
    images: &mut Assets<Image>,
) -> Result<(Arc<GeometryFont>, Handle<Image>), String> {
    let (geometry_font, [width, height], data) = padded_font_atlas(font)?;
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    let atlas = images.add(image);
    Ok((Arc::new(geometry_font), atlas))
}
#[derive(Resource)]
struct Preview {
    font: BitmapFont,
    text: String,
    output: String,
    sample_time: f32,
    size: UVec2,
    target: Handle<Image>,
    frames: u32,
}
pub fn preview(path: &str, args: &[String]) -> Result<(), String> {
    let get = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let xnb = magicka_xnb::xnb::parse(&bytes).map_err(|e| e.to_string())?;
    let parsed = parse_primary(&xnb)
        .ok_or("null font")?
        .map_err(|e| e.to_string())?;
    let Content::Xna(object) = parsed.content else {
        return Err("expected font object".into());
    };
    let XnaObject::Asset(asset) = *object else {
        return Err("expected font asset".into());
    };
    let Asset::BitmapFont(font) = *asset else {
        return Err("expected bitmap font".into());
    };
    let sample_time = get("--text-time")
        .unwrap_or_else(|| "10".into())
        .parse::<f32>()
        .map_err(|e| e.to_string())?;
    if !sample_time.is_finite() || sample_time < 0.0 {
        return Err("invalid text time".into());
    }
    let size = if let Some(i) = args.iter().position(|a| a == "--size") {
        UVec2::new(
            args.get(i + 1)
                .ok_or("missing width")?
                .parse::<u32>()
                .map_err(|e| e.to_string())?,
            args.get(i + 2)
                .ok_or("missing height")?
                .parse::<u32>()
                .map_err(|e| e.to_string())?,
        )
    } else {
        UVec2::new(800, 240)
    };
    if size.x == 0 || size.y == 0 {
        return Err("preview dimensions must be positive".into());
    }
    let output = get("--render").ok_or("--render <private output.png> required")?;
    let text = get("--text").unwrap_or_else(|| {
        "Native atlas: [c=1,0,0]red[/c], [c=0,1,0]green[/c].\n[p=.5]A recovered typing pause."
            .into()
    });
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
    .add_plugins(NativeTextPlugin)
    .insert_resource(ClearColor(Color::BLACK))
    .insert_resource(Preview {
        font,
        text,
        output,
        sample_time,
        size,
        target: Handle::default(),
        frames: 0,
    })
    .add_systems(Startup, setup_preview)
    .add_systems(Update, capture_preview)
    .run();
    Ok(())
}
fn setup_preview(
    mut commands: Commands,
    mut preview: ResMut<Preview>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let (font, material) =
        upload_font(&preview.font, &mut images, &mut materials).expect("font atlas upload");
    let mut target = Image::new_target_texture(
        preview.size.x,
        preview.size.y,
        TextureFormat::Rgba8Unorm,
        None,
    );
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
    commands.spawn((
        NativeBitmapText::new(font, &preview.text, TextStyle::dialogue()),
        Mesh2d(meshes.add(geometry_mesh(&TextGeometry::default(), 0).unwrap())),
        MeshMaterial2d(material),
        Transform::from_xyz(
            32.0 - preview.size.x as f32 * 0.5,
            preview.size.y as f32 * 0.5 - 32.0,
            0.0,
        ),
    ));
}
fn capture_preview(
    mut commands: Commands,
    mut preview: ResMut<Preview>,
    mut clock: ResMut<NativeTextDelta>,
    texts: Query<&NativeBitmapText>,
) {
    preview.frames += 1;
    clock.0 = Some(if preview.frames == 3 {
        preview.sample_time
    } else {
        0.0
    });
    if let Some(error) = texts.iter().find_map(|t| t.failure.as_ref()) {
        panic!("preview adapter failed: {error}");
    }
    if preview.frames == 30 {
        for text in &texts {
            info!(
                "native preview: {} visible / {} glyphs",
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
                    // Bevy's generic saver rejects Rgba8Unorm. Keep the captured
                    // channel bytes unchanged and encode them with the private
                    // content PNG writer rather than applying a color conversion.
                    let bytes = captured.image.data.as_ref().expect("capture pixels");
                    let png = magicka_content::texture::encode_png(
                        preview.size.x as usize,
                        preview.size.y as usize,
                        bytes,
                    );
                    std::fs::write(&preview.output, png).expect("save private font preview");
                    exit.write(AppExit::Success);
                },
            );
    }
    if preview.frames > 240 {
        panic!("font preview capture timeout");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hidden_text_has_allocatable_buffers_and_no_rasterizable_triangle() {
        let mesh = geometry_mesh(&TextGeometry::default(), 0).unwrap();
        assert_eq!(mesh.count_vertices(), 1);
        assert!(matches!(mesh.indices(), Some(Indices::U16(indices)) if indices == &[0, 0, 0]));
        assert!(matches!(mesh.attribute(Mesh::ATTRIBUTE_COLOR),
            Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) if colors == &[[0.; 4]]));
        assert!(geometry_mesh(&TextGeometry::default(), -1).is_err());
    }
    #[test]
    fn mesh_uses_actual_uv_color_and_prefix_indices() {
        use magicka_runtime::text_geometry::{GlyphQuad, TextVertex};
        let vertex = TextVertex {
            position: [2., 3.],
            uv: [0.25, 0.5],
            color: [0.1, 0.2, 0.3, 0.4],
        };
        let geometry = TextGeometry {
            quads: vec![
                GlyphQuad {
                    input: 65,
                    vertices: [vertex; 4]
                };
                2
            ],
        };
        let mesh = geometry_mesh(&geometry, 1).unwrap();
        assert_eq!(mesh.indices().unwrap().len(), 6);
        assert_eq!(mesh.count_vertices(), 8);
        assert_eq!(
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
                .unwrap()
                .as_float3()
                .unwrap()[0],
            [2., -3., 0.]
        );
        assert!(geometry_mesh(&geometry, 3).is_err());
    }
    #[test]
    fn ecs_draw_phase_publishes_geometry_and_rebuilds_reveal_prefix() {
        use magicka_content::assets::Glyph;
        let content = BitmapFont {
            line_height: 10,
            base_line: 8,
            default_character: 'A',
            texture: None,
            glyphs: vec![Glyph {
                character: 'A',
                origin: [0; 2],
                size: [4, 5],
                advance_width: 5,
                left_side_bearing: 0,
                force_white: false,
            }],
            kerning: vec![],
        };
        let font = Arc::new(GeometryFont::with_atlas_size(&content, [16, 16]).unwrap());
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_resource::<Assets<Mesh>>()
            .add_plugins(NativeTextPlugin);
        app.world_mut().resource_mut::<NativeTextDelta>().0 = Some(0.0);
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(geometry_mesh(&TextGeometry::default(), 0).unwrap());
        let entity = app
            .world_mut()
            .spawn((
                NativeBitmapText::new(font, "AA", TextStyle::default()),
                Mesh2d(mesh.clone()),
            ))
            .id();
        app.update();
        let text = app.world().get::<NativeBitmapText>(entity).unwrap();
        assert_eq!(text.typing.primitive_count, 4);
        assert_eq!(text.typing.visible_characters, 0);
        assert!(!text.typing.dirty);
        app.world_mut().resource_mut::<NativeTextDelta>().0 = Some(0.001);
        app.update();
        assert_eq!(
            app.world()
                .get::<NativeBitmapText>(entity)
                .unwrap()
                .typing
                .visible_characters,
            1
        );
        assert_eq!(
            app.world()
                .resource::<Assets<Mesh>>()
                .get(&mesh)
                .unwrap()
                .indices()
                .unwrap()
                .len(),
            6
        );
        app.world_mut()
            .get_mut::<NativeBitmapText>(entity)
            .unwrap()
            .typing
            .set_text(Some("A"));
        app.world_mut().resource_mut::<NativeTextDelta>().0 = Some(0.0);
        app.update();
        let text = app.world().get::<NativeBitmapText>(entity).unwrap();
        assert_eq!(text.typing.primitive_count, 2);
        assert!(text.failure.is_none());
        assert_eq!(
            app.world()
                .resource::<Assets<Mesh>>()
                .get(&mesh)
                .unwrap()
                .indices()
                .unwrap()
                .len(),
            3
        );
    }
}
