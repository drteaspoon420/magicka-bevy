//! PolygonHead LavaEffect shader 5. See research/lava-render.md.
use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct LavaParams {
    pub speeds: Vec4,
    /// Time, MaskDistortion, TempFrequency, RockNormalPower.
    pub motion: Vec4,
    /// Cold/Hot/Rock emissive amounts, RockNormalMapEnabled.
    pub emissive: Vec4,
    /// Lava/Rock spec amount, Lava/Rock spec power.
    pub spec: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct LavaMaterial {
    #[uniform(0)]
    pub params: LavaParams,
    #[texture(1)]
    #[sampler(2)]
    pub temperature: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    pub mask: Option<Handle<Image>>,
    #[texture(5)]
    #[sampler(6)]
    pub tone: Option<Handle<Image>>,
    #[texture(7)]
    #[sampler(8)]
    pub rock: Option<Handle<Image>>,
    #[texture(9)]
    #[sampler(10)]
    pub normal: Option<Handle<Image>>,
    #[storage(90, read_only)]
    pub lights: Handle<bevy::render::storage::ShaderBuffer>,
}

impl Material for LavaMaterial {
    fn enable_prepass() -> bool {
        false
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/lava.wgsl".into()
    }
}

pub fn from_extras(
    d: &serde_json::Value,
    assets: &AssetServer,
    lights: Handle<bevy::render::storage::ShaderBuffer>,
    resolve: impl Fn(&str) -> String,
) -> LavaMaterial {
    let f = |k: &str| d[k].as_f64().unwrap_or(0.0) as f32;
    let v = |k: &str, i: usize| d[k][i].as_f64().unwrap_or(0.0) as f32;
    let texture = |k: &str, srgb: bool| {
        d[k].as_str().map(|uri| {
            assets
                .load_builder()
                .with_settings(move |settings: &mut ImageLoaderSettings| {
                    settings.is_srgb = srgb;
                    settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                        address_mode_u: ImageAddressMode::Repeat,
                        address_mode_v: ImageAddressMode::Repeat,
                        ..ImageSamplerDescriptor::linear()
                    });
                })
                .load(resolve(uri))
        })
    };
    LavaMaterial {
        params: LavaParams {
            speeds: Vec4::new(
                v("speed0", 0),
                v("speed0", 1),
                v("speed1", 0),
                v("speed1", 1),
            ),
            motion: Vec4::new(
                0.0,
                f("mask_distortion"),
                f("temp_frequency"),
                f("rock_normal_power"),
            ),
            emissive: Vec4::new(
                f("cold_emissive"),
                f("hot_emissive"),
                f("rock_emissive"),
                if d["rock_normal_map"].is_string() {
                    1.0
                } else {
                    0.0
                },
            ),
            spec: Vec4::new(
                f("spec_amount"),
                f("rock_spec_amount"),
                f("spec_power"),
                f("rock_spec_power"),
            ),
        },
        temperature: texture("temp_map", false),
        mask: texture("mask_map", false),
        tone: texture("tone_map", true),
        rock: texture("rock_texture", true),
        normal: texture("rock_normal_map", false),
        lights,
    }
}

pub fn advance_time(time: Res<Time>, mut materials: ResMut<Assets<LavaMaterial>>) {
    for (_, material) in materials.iter_mut() {
        material.params.motion.x += time.delta_secs();
    }
}
