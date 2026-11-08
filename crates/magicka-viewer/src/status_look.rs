//! Character.UpdateRenderData status looks on the skinned body material
//! (research/status-look.md): Cold Colorize and brightening, Wet diffuse
//! tint and specular, and the Frozen ice block (bloat, ice cube map, ice
//! normal cube map, emissive and specular). Robe and lit materials bind the
//! same uniform and cube maps (bindings 107–111) and import
//! `magicka_viewer::status_look`.
use bevy::{
    asset::RenderAssetUsages,
    image::{ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::render_resource::{
        Extent3d, ShaderType, TextureDimension, TextureViewDescriptor, TextureViewDimension,
    },
};

/// x: Colorize.W = min(10·Cold magnitude, 1); y: Wet (or drowning) look;
/// z: Frozen Bloat = (int)(10·Frozen magnitude)·0.02; w: Frozen look.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct StatusLook {
    pub look: Vec4,
}

impl StatusLook {
    /// UpdateRenderData's branch order: Frozen, else Wet/drowning.
    pub fn from_status(cold: f32, wet: bool, frozen: Option<f32>) -> Self {
        // Math.Min(magnitude * 10, 1).
        let colorize = (cold * 10.0).min(1.0);
        match frozen {
            Some(magnitude) => Self {
                look: Vec4::new(colorize, 0.0, (magnitude * 10.0).trunc() * 0.02, 1.0),
            },
            None => Self {
                look: Vec4::new(colorize, f32::from(u8::from(wet)), 0.0, 0.0),
            },
        }
    }
}

/// EffectTextures/iceCube and iceCube_NRM assembled from exported faces.
#[derive(Resource, Default)]
pub struct StatusCubes {
    faces: Vec<Handle<Image>>,
    pub ice: Option<Handle<Image>>,
    pub ice_normal: Option<Handle<Image>>,
}

pub struct StatusLookPlugin;
impl Plugin for StatusLookPlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "status_look.wgsl");
        bevy::asset::embedded_asset!(app, "status_vertex.wgsl");
        app.init_resource::<StatusCubes>()
            .add_systems(Update, build_cubes);
    }
}

fn build_cubes(
    server: Res<AssetServer>,
    mut cubes: ResMut<StatusCubes>,
    mut images: ResMut<Assets<Image>>,
) {
    if cubes.ice.is_some() {
        return;
    }
    if cubes.faces.is_empty() {
        for name in ["iceCube", "iceCube_NRM"] {
            for face in 0..6 {
                cubes
                    .faces
                    .push(server.load(format!("textures/EffectTextures/{name}.face{face}.png")));
            }
        }
        return;
    }
    let ready = cubes
        .faces
        .iter()
        .all(|h| images.get(h).is_some_and(|i| i.data.is_some()));
    if !ready {
        return;
    }
    let mut cube = |faces: &[Handle<Image>]| {
        let first = images.get(&faces[0])?;
        let (size, format) = (first.size(), first.texture_descriptor.format);
        let mut data = Vec::new();
        for f in faces {
            let image = images.get(f)?;
            if image.size() != size || image.texture_descriptor.format != format {
                return None;
            }
            data.extend_from_slice(image.data.as_ref()?);
        }
        let mut image = Image::new(
            Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 6,
            },
            TextureDimension::D2,
            data,
            format,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.texture_view_descriptor = Some(TextureViewDescriptor {
            dimension: Some(TextureViewDimension::Cube),
            ..default()
        });
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
        Some(images.add(image))
    };
    let faces = cubes.faces.clone();
    cubes.ice = cube(&faces[..6]);
    cubes.ice_normal = cube(&faces[6..]);
    if cubes.ice.is_none() || cubes.ice_normal.is_none() {
        warn!("ice cube faces could not be assembled");
        cubes.ice = Some(Handle::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_follow_update_render_data() {
        let cold = StatusLook::from_status(0.05, false, None);
        assert_eq!(cold.look, Vec4::new(0.5, 0.0, 0.0, 0.0));
        assert_eq!(StatusLook::from_status(0.3, true, None).look.x, 1.0);
        // Frozen takes precedence over Wet; bloat steps by 0.02 per 0.1.
        let frozen = StatusLook::from_status(0.0, true, Some(0.95));
        assert_eq!(frozen.look.y, 0.0);
        assert!((frozen.look.z - 0.18).abs() < 1e-6);
        assert_eq!(frozen.look.w, 1.0);
    }
}
