//! `LiquidMaterial`: PolygonHead's RenderDeferredLiquidEffect (Water) for
//! glTF primitives whose material carries `extras.magicka_liquid` (written by
//! `level-gltf`). See `liquid.wgsl` and research/water-render.md.
use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

/// RenderDeferredLiquidEffect `ReflectColor` compiled default (fx parameter
/// table, `tools/fx_params.py`); no material sets it.
pub const REFLECT_COLOR: [f32; 3] = [0.6, 0.9, 1.0];

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct LiquidParams {
    pub bottom: Vec4,
    pub deep: Vec4,
    pub reflect: Vec4,
    pub speeds: Vec4,
    pub misc: Vec4,
    /// xyz IceColor, w IceDiffuseMap present.
    pub ice: Vec4,
}

/// A liquid surface mesh; its UV_1.x carries the per-vertex freeze value
/// (Water vertex stream 1), written by play mode from the runtime water.
#[derive(Component)]
pub struct LiquidSurface;

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct LiquidMaterial {
    #[uniform(0)]
    pub params: LiquidParams,
    #[texture(1)]
    #[sampler(2)]
    pub normal_map: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    pub bottom: Option<Handle<Image>>,
    #[texture(5)]
    #[sampler(6)]
    pub ice: Option<Handle<Image>>,
    #[storage(90, read_only)]
    pub lights: Handle<bevy::render::storage::ShaderBuffer>,
}

impl Material for LiquidMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/liquid.wgsl".into()
    }
}

fn wrap(srgb: bool) -> impl Fn(&mut ImageLoaderSettings) {
    move |settings: &mut ImageLoaderSettings| {
        // Normal maps are sampled as raw bytes by the game (no sRGB).
        settings.is_srgb = srgb;
        settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            ..ImageSamplerDescriptor::linear()
        });
    }
}

/// Build the material from `magicka_liquid` extras; `resolve` maps texture
/// URIs (relative to the scene .glb) to asset paths.
pub fn from_extras(
    d: &serde_json::Value,
    assets: &AssetServer,
    lights: Handle<bevy::render::storage::ShaderBuffer>,
    resolve: impl Fn(&str) -> String,
) -> LiquidMaterial {
    let f = |k: &str| d.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    let v = |k: &str, n: usize| -> Vec<f32> {
        let a = d
            .get(k)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        (0..n)
            .map(|i| a.get(i).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32)
            .collect()
    };
    let texture = |k: &str, srgb: bool| {
        d.get(k).and_then(|v| v.as_str()).map(|u| {
            assets
                .load_builder()
                .with_settings(wrap(srgb))
                .load(resolve(u))
        })
    };
    let bottom = texture("bottom_texture", true);
    let ice = texture("ice_diffuse_map", true);
    let ice_color = v("ice_color", 3);
    let (b, deep, s0, s1) = (
        v("bottom_color", 3),
        v("deep_bottom_color", 3),
        v("wave_speed0", 2),
        v("wave_speed1", 2),
    );
    LiquidMaterial {
        params: LiquidParams {
            bottom: Vec4::new(b[0], b[1], b[2], if bottom.is_some() { 1.0 } else { 0.0 }),
            deep: Vec4::new(deep[0], deep[1], deep[2], f("wave_height")),
            reflect: Vec4::new(
                REFLECT_COLOR[0],
                REFLECT_COLOR[1],
                REFLECT_COLOR[2],
                f("reflectiveness"),
            ),
            speeds: Vec4::new(s0[0], s0[1], s1[0], s1[1]),
            misc: Vec4::new(0.0, f("emissive"), f("spec_amount"), f("spec_power")),
            ice: Vec4::new(
                ice_color[0],
                ice_color[1],
                ice_color[2],
                if ice.is_some() { 1.0 } else { 0.0 },
            ),
        },
        ice,
        normal_map: texture("normal_map", false),
        bottom,
        lights,
    }
}

/// Give liquid meshes a zero freeze stream so the pipeline layout is stable.
pub fn add_freeze_stream(
    surfaces: Query<&Mesh3d, Added<LiquidSurface>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    for mesh in &surfaces {
        if let Some(mut m) = meshes.get_mut(&mesh.0) {
            if !m.contains_attribute(Mesh::ATTRIBUTE_UV_1) {
                let n = m.count_vertices();
                m.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0f32, 0.0]; n]);
            }
        }
    }
}

/// Water.Update: `mTime += dt` drives the wave preshader.
pub fn advance_time(time: Res<Time>, mut materials: ResMut<Assets<LiquidMaterial>>) {
    let dt = time.delta_secs();
    for (_, m) in materials.iter_mut() {
        m.params.misc.x += dt;
    }
}
