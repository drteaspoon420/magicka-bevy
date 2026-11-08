//! ParticleSystem.Draw: every slot up to the last active particle drawn as
//! camera-facing quads in slot order with the ported ParticleEffect shaders
//! (`particles.wgsl`). Blend One/SrcAlpha, no depth test or write, after the
//! scene. The simulation is `magicka_runtime::particles`; see
//! research/particles.md.
use crate::text_box_visual::read_asset;
use bevy::{
    asset::{embedded_asset, RenderAssetUsages},
    camera::visibility::NoFrustumCulling,
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, MeshVertexAttribute, MeshVertexBufferLayoutRef, PrimitiveTopology},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, CompareFunction,
        Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError, TextureDimension,
        TextureFormat, VertexFormat,
    },
    shader::ShaderRef,
};
use magicka_content::assets::Asset as XnbAsset;
use magicka_runtime::particles::ParticleGraphics;
use std::path::Path;

const ATTRIBUTE_PARTICLE: MeshVertexAttribute = MeshVertexAttribute::new(
    "ParticleCornerRotationRadiusLife",
    0x6d61_6701,
    VertexFormat::Float32x4,
);
const ATTRIBUTE_FLAGS: MeshVertexAttribute =
    MeshVertexAttribute::new("ParticleFlags", 0x6d61_6702, VertexFormat::Float32x4);
const ATTRIBUTE_VELOCITY: MeshVertexAttribute =
    MeshVertexAttribute::new("ParticleVelocity", 0x6d61_6703, VertexFormat::Float32x3);

/// The four sprite sheets (EffectTextures/ParticlesA..D, Texture3D).
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct ParticleMaterial {
    #[texture(0, dimension = "3d")]
    #[sampler(1)]
    pub a: Handle<Image>,
    #[texture(2, dimension = "3d")]
    #[sampler(3)]
    pub b: Handle<Image>,
    #[texture(4, dimension = "3d")]
    #[sampler(5)]
    pub c: Handle<Image>,
    #[texture(6, dimension = "3d")]
    #[sampler(7)]
    pub d: Handle<Image>,
    #[texture(8, filterable = false)]
    pub scene_depth: Handle<Image>,
    /// x: use the opaque-depth snapshot instead of the outline prepass.
    #[uniform(9)]
    pub depth_control: Vec4,
}

impl Material for ParticleMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://magicka_viewer/particles.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/particles.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers = vec![layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(1),
            ATTRIBUTE_PARTICLE.at_shader_location(2),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(3),
            ATTRIBUTE_FLAGS.at_shader_location(4),
            ATTRIBUTE_VELOCITY.at_shader_location(5),
        ])?];
        // RenderState: SourceBlend One, DestinationBlend SourceAlpha (no
        // separate alpha blend), depth test and writes off, no culling.
        let blend = BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::SrcAlpha,
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
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_compare = Some(CompareFunction::Always);
            depth.depth_write_enabled = Some(false);
        }
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// The particles to draw this frame (`ParticleSystem.active()`).
#[derive(Resource, Default)]
pub struct ParticleFrame(pub Vec<ParticleGraphics>);

/// The old prepass source is retained only for reproducible GPU audit controls.
#[derive(Resource, Default, PartialEq, Eq)]
pub enum ParticleDepthSource {
    #[default]
    FullScene,
    OutlinePrepass,
}

#[derive(Resource)]
struct ParticleRenderer {
    mesh: Handle<Mesh>,
}

pub struct ParticlePlugin {
    pub sheets: SheetSource,
}

#[derive(Clone)]
pub enum SheetSource {
    /// `Content` directory holding EffectTextures/ParticlesA..D.xnb.
    Content(std::path::PathBuf),
    /// Procedural single-mip sheets (`test_sheet_texel`) for the shader check.
    Test,
}

impl Plugin for ParticlePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "particles.wgsl");
        let sheets = self.sheets.clone();
        app.add_plugins((
            MaterialPlugin::<ParticleMaterial>::default(),
            crate::effect_depth::EffectDepthPlugin,
        ))
        .init_resource::<ParticleFrame>()
        .init_resource::<ParticleDepthSource>()
        .add_systems(
            Startup,
            move |mut commands: Commands,
                  mut images: ResMut<Assets<Image>>,
                  mut meshes: ResMut<Assets<Mesh>>,
                  mut materials: ResMut<Assets<ParticleMaterial>>,
                  depth: Res<crate::effect_depth::EffectDepth>| {
                let loaded = match &sheets {
                    SheetSource::Content(content) => load_sheets(content, &mut images),
                    SheetSource::Test => Ok(test_sheets(&mut images)),
                };
                match loaded {
                    Ok([a, b, c, d]) => {
                        let mesh = meshes.add(build_mesh(&[]));
                        let material = materials.add(ParticleMaterial {
                            a,
                            b,
                            c,
                            d,
                            scene_depth: depth.image.clone(),
                            depth_control: Vec4::ZERO,
                        });
                        commands.spawn((
                            Mesh3d(mesh.clone()),
                            MeshMaterial3d(material),
                            Transform::IDENTITY,
                            NoFrustumCulling,
                        ));
                        commands.insert_resource(ParticleRenderer { mesh });
                    }
                    Err(e) => warn!("particle sheets unavailable: {e}"),
                }
            },
        )
        .add_systems(PostUpdate, (upload_particles, sync_depth));
    }
}

fn sync_depth(
    cameras: Query<&Camera, With<crate::effect_depth::EffectDepthCamera>>,
    source: Res<ParticleDepthSource>,
    mut materials: ResMut<Assets<ParticleMaterial>>,
) {
    let enabled = *source == ParticleDepthSource::FullScene
        && cameras.single().is_ok_and(|camera| camera.is_active);
    for (_, material) in materials.iter_mut() {
        let control = Vec4::new(f32::from(enabled), 0.0, 0.0, 0.0);
        if material.depth_control != control {
            material.depth_control = control;
        }
    }
}

fn upload_particles(
    frame: Res<ParticleFrame>,
    renderer: Option<Res<ParticleRenderer>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(renderer) = renderer else {
        return;
    };
    if let Some(mut mesh) = meshes.get_mut(&renderer.mesh) {
        *mesh = build_mesh(&frame.0);
    }
}

/// ParticleSystem vertex stream 0: corner index and texture coordinate.
const CORNERS: [(f32, [f32; 2]); 4] = [
    (0.0, [1.0, 1.0]),
    (1.0, [0.0, 1.0]),
    (2.0, [0.0, 0.0]),
    (3.0, [1.0, 0.0]),
];

/// Four vertices per slot (instance data repeated per corner), indices
/// 0,1,2,0,2,3 per quad, in slot order.
pub fn build_mesh(particles: &[ParticleGraphics]) -> Mesh {
    let n = particles.len().max(1);
    let mut positions = Vec::with_capacity(n * 4);
    let mut uvs = Vec::with_capacity(n * 4);
    let mut data = Vec::with_capacity(n * 4);
    let mut colors = Vec::with_capacity(n * 4);
    let mut flags = Vec::with_capacity(n * 4);
    let mut velocities = Vec::with_capacity(n * 4);
    let mut indices = Vec::with_capacity(n * 6);
    // An empty frame keeps one degenerate quad so the mesh stays valid.
    let empty = [ParticleGraphics::default()];
    let list = if particles.is_empty() {
        &empty[..]
    } else {
        particles
    };
    for (i, p) in list.iter().enumerate() {
        for (corner, uv) in CORNERS {
            positions.push(p.position);
            uvs.push(uv);
            data.push([corner, p.rotation, p.radius, p.normalized_lifetime]);
            colors.push(p.color);
            flags.push([
                f32::from(p.sprite),
                f32::from(p.alpha_blended),
                f32::from(p.hsv_colorize),
                f32::from(p.rotation_aligned),
            ]);
            velocities.push(p.velocity);
        }
        let base = (i * 4) as u32;
        indices.extend([0, 1, 2, 0, 2, 3].map(|k| base + k));
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(ATTRIBUTE_PARTICLE, data);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_attribute(ATTRIBUTE_FLAGS, flags);
    mesh.insert_attribute(ATTRIBUTE_VELOCITY, velocities);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Texture3D (SurfaceFormat.Color, BGRA bytes) with its full mip chain as a
/// raw Rgba8Unorm 3D image: Clamp on U/V/W, linear min/mag/mip filtering.
fn load_sheets(content: &Path, images: &mut Assets<Image>) -> Result<[Handle<Image>; 4], String> {
    let mut out = Vec::new();
    for name in ["ParticlesA", "ParticlesB", "ParticlesC", "ParticlesD"] {
        let path = content.join(format!("EffectTextures/{name}.xnb"));
        let XnbAsset::Texture3D {
            format,
            width,
            height,
            depth,
            mips,
        } = read_asset(&path)?
        else {
            return Err(format!("{name}: not a Texture3D"));
        };
        if format != 1 {
            return Err(format!("{name}: surface format {format}"));
        }
        let (w, h, d) = (width as u32, height as u32, depth as u32);
        let mut data = Vec::new();
        for (level, mip) in mips.iter().enumerate() {
            let expect = (w >> level).max(1) * (h >> level).max(1) * (d >> level).max(1) * 4;
            if mip.len() != expect as usize {
                return Err(format!(
                    "{name}: mip {level} has {} bytes, expected {expect}",
                    mip.len()
                ));
            }
            for &[b, g, r, a] in mip.as_chunks::<4>().0 {
                data.extend_from_slice(&[r, g, b, a]);
            }
        }
        // Image::new validates the base level only (a debug assertion);
        // the full mip chain replaces it afterwards.
        let base = (w * h * d * 4) as usize;
        let mut image = Image::new(
            Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: d,
            },
            TextureDimension::D3,
            data[..base].to_vec(),
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.data = Some(data);
        image.texture_descriptor.mip_level_count = mips.len() as u32;
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            address_mode_w: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            ..default()
        });
        out.push(images.add(image));
    }
    Ok(out.try_into().expect("four sheets"))
}

/// Depths of ParticlesA..D (Texture3D slices = lifetime frames).
pub const TEST_SHEET_DEPTHS: [u32; 4] = [1, 4, 4, 8];

/// Test texel of sheet `k`: constant over each 16×16 quadrant of each 32×32
/// sprite cell and each lifetime slice (mirrored by check_particle_shader.py).
pub fn test_sheet_texel(k: u32, z: u32, x: u32, y: u32) -> [u8; 4] {
    let (cx, cy, qx, qy) = (x / 32, y / 32, (x % 32) / 16, (y % 32) / 16);
    [
        ((k * 61 + z * 29 + cx * 17 + cy * 5 + qx * 90) % 256) as u8,
        ((k * 13 + z * 71 + cx * 3 + cy * 41 + qy * 120) % 256) as u8,
        ((k * 97 + z * 11 + cx * 29 + cy * 7 + 40) % 256) as u8,
        (64 + (k * 31 + z * 19 + cx * 11 + cy * 23 + (qx + qy) * 50) % 192) as u8,
    ]
}

fn test_sheets(images: &mut Assets<Image>) -> [Handle<Image>; 4] {
    std::array::from_fn(|k| {
        let d = TEST_SHEET_DEPTHS[k];
        let mut data = Vec::with_capacity((256 * 256 * d * 4) as usize);
        for z in 0..d {
            for y in 0..256 {
                for x in 0..256 {
                    data.extend_from_slice(&test_sheet_texel(k as u32, z, x, y));
                }
            }
        }
        let mut image = Image::new(
            Extent3d {
                width: 256,
                height: 256,
                depth_or_array_layers: d,
            },
            TextureDimension::D3,
            data,
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            address_mode_w: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            ..default()
        });
        images.add(image)
    })
}

/// `--particle-test <scene.json> <out.png>`: draw the listed particles with
/// the given XNA-style look-at camera over black (no scene depth), for
/// output/evidence/particles/check_particle_shader.py.
pub fn particle_test(scene: &str, out: String) -> Result<(), String> {
    use bevy::{
        app::ScheduleRunnerPlugin,
        camera::{ImageRenderTarget, RenderTarget},
        core_pipeline::tonemapping::{DebandDither, Tonemapping},
        render::{
            render_resource::TextureUsages,
            view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured},
        },
        window::ExitCondition,
        winit::WinitPlugin,
    };
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(scene).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let v3 = |v: &serde_json::Value| -> Vec3 {
        Vec3::new(
            v[0].as_f64().unwrap_or(0.0) as f32,
            v[1].as_f64().unwrap_or(0.0) as f32,
            v[2].as_f64().unwrap_or(0.0) as f32,
        )
    };
    let f = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0) as f32;
    let (eye, target, up) = (v3(&json["eye"]), v3(&json["target"]), v3(&json["up"]));
    let (fov, near, far) = (f(&json["fov"]), f(&json["near"]), f(&json["far"]));
    let size = json["size"].as_u64().unwrap_or(256) as u32;
    let full_depth = json["full_depth"].as_bool().unwrap_or(false);
    let msaa = if json["msaa"].as_u64().unwrap_or(1) == 4 {
        Msaa::Sample4
    } else {
        Msaa::Off
    };
    let occluders: Vec<(Vec3, Vec3)> = json["occluders"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|o| (v3(&o["position"]), v3(&o["size"])))
        .collect();
    let particles: Vec<ParticleGraphics> = json["particles"]
        .as_array()
        .ok_or("particles")?
        .iter()
        .map(|p| ParticleGraphics {
            position: v3(&p["position"]).to_array(),
            rotation: f(&p["rotation"]),
            radius: f(&p["radius"]),
            normalized_lifetime: f(&p["life"]),
            color: [
                f(&p["color"][0]),
                f(&p["color"][1]),
                f(&p["color"][2]),
                f(&p["color"][3]),
            ],
            sprite: p["sprite"].as_u64().unwrap_or(0) as u8,
            alpha_blended: p["alpha_blended"].as_u64().unwrap_or(0) as u8,
            hsv_colorize: p["hsv_colorize"].as_u64().unwrap_or(0) as u8,
            rotation_aligned: p["rotation_aligned"].as_u64().unwrap_or(0) as u8,
            velocity: v3(&p["velocity"]).to_array(),
        })
        .collect();

    #[derive(Resource)]
    struct Shot(String, Handle<Image>);
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
    .add_plugins(ScheduleRunnerPlugin::run_loop(
        std::time::Duration::from_secs_f64(1.0 / 120.0),
    ))
    .add_plugins(ParticlePlugin {
        sheets: SheetSource::Test,
    })
    .add_plugins(MaterialPlugin::<DepthTestOccluderMaterial>::default())
    .insert_resource(ClearColor(Color::BLACK))
    .insert_resource(ParticleFrame(particles))
    .insert_resource(Shot(out, Handle::default()))
    .add_systems(
        Startup,
        move |mut commands: Commands,
              mut images: ResMut<Assets<Image>>,
              mut shot: ResMut<Shot>,
              mut meshes: ResMut<Assets<Mesh>>,
              mut materials: ResMut<Assets<DepthTestOccluderMaterial>>| {
            let mut image =
                Image::new_target_texture(size, size, TextureFormat::Rgba8UnormSrgb, None);
            image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
            let handle = images.add(image);
            shot.1 = handle.clone();
            let mut camera = commands.spawn((
                Camera3d::default(),
                Tonemapping::None,
                DebandDither::Disabled,
                msaa,
                Projection::Perspective(PerspectiveProjection {
                    fov,
                    near,
                    far,
                    aspect_ratio: 1.0,
                    ..default()
                }),
                Transform::from_translation(eye).looking_at(target, up),
                RenderTarget::Image(ImageRenderTarget {
                    handle,
                    scale_factor: 1.0,
                }),
            ));
            if full_depth || !occluders.is_empty() {
                camera.insert(bevy::core_pipeline::prepass::DepthPrepass);
            }
            if full_depth {
                camera.insert(crate::effect_depth::EffectDepthCamera);
            }
            for (position, size) in &occluders {
                commands.spawn((
                    Mesh3d(meshes.add(Cuboid::new(size.x, size.y, size.z))),
                    MeshMaterial3d(materials.add(bevy::pbr::ExtendedMaterial {
                        base: StandardMaterial {
                            base_color: Color::BLACK,
                            unlit: true,
                            ..default()
                        },
                        extension: DepthTestOccluderExtension { unused: Vec4::ZERO },
                    })),
                    Transform::from_translation(*position),
                ));
            }
        },
    )
    .add_systems(
        Update,
        |mut commands: Commands, shot: Res<Shot>, mut n: Local<u32>| {
            *n += 1;
            if *n == 60 {
                commands
                    .spawn(Screenshot::image(shot.1.clone()))
                    .observe(save_to_disk(shot.0.clone()))
                    .observe(
                        |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                            exit.write(AppExit::Success);
                        },
                    );
            }
        },
    )
    .run();
    Ok(())
}

// The regression occluder models the player's depth participation exactly:
// opaque main-pass depth, but no depth in the outline's prepass.
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
struct DepthTestOccluderExtension {
    #[uniform(100)]
    unused: Vec4,
}
impl bevy::pbr::MaterialExtension for DepthTestOccluderExtension {
    fn enable_prepass() -> bool {
        false
    }
}
type DepthTestOccluderMaterial =
    bevy::pbr::ExtendedMaterial<StandardMaterial, DepthTestOccluderExtension>;
