//! Authored terrain policy. Heights and materials are design inputs, not retail constants.
use super::{attr, scalar, vector};
use crate::scene_script::XmlElement;
use magicka_content::{
    level::{Liquid, LiquidMaterial, TriMesh},
    xna::{LiquidEffect, VertexElement},
};
#[derive(Clone, Debug, Default)]
pub struct Terrain {
    pub river: Option<River>,
    pub rises: Vec<Rise>,
}
#[derive(Clone, Debug)]
pub struct River {
    pub half_width: f32,
    pub surface: f32,
    pub deep_depth: f32,
    pub shallow_depth: f32,
    pub ford_half_length: f32,
    pub fords: Vec<f32>,
}
#[derive(Clone, Debug)]
pub struct Rise {
    pub center: [f32; 2],
    pub size: [f32; 2],
    pub height: f32,
    pub apron: f32,
}
impl Terrain {
    pub(super) fn read(root: &XmlElement) -> Result<Self, String> {
        let mut out = Self::default();
        for n in &root.children {
            match n.name.as_str() {
                "River" => {
                    if out.river.is_some() {
                        return Err("only one diagonal River supported".into());
                    }
                    let river = River {
                        half_width: scalar(n, "width")? / 2.,
                        surface: scalar(n, "surface")?,
                        deep_depth: scalar(n, "deepDepth")?,
                        shallow_depth: scalar(n, "shallowDepth")?,
                        ford_half_length: scalar(n, "fordHalfLength")?,
                        fords: attr(n, "fords")?
                            .split(',')
                            .map(|v| v.trim().parse::<f32>().map_err(|_| "invalid ford".into()))
                            .collect::<Result<Vec<_>, String>>()?,
                    };
                    if [
                        river.half_width,
                        river.deep_depth,
                        river.shallow_depth,
                        river.ford_half_length,
                    ]
                    .iter()
                    .any(|v| *v <= 0.)
                        || river.deep_depth <= river.shallow_depth
                        || river.fords.is_empty()
                        || river.fords.iter().any(|v| !v.is_finite())
                    {
                        return Err("invalid river dimensions".into());
                    }
                    out.river = Some(river);
                }
                "Rise" => {
                    let rise = Rise {
                        center: vector(n, "center")?,
                        size: vector(n, "size")?,
                        height: scalar(n, "height")?,
                        apron: scalar(n, "apron")?,
                    };
                    if rise.size.iter().any(|v| *v <= 0.) || rise.height <= 0. || rise.apron <= 0. {
                        return Err("invalid rise".into());
                    }
                    out.rises.push(rise);
                }
                _ => {}
            }
        }
        Ok(out)
    }
    /// Channel runs along x=z; ford coordinates are world x/z, not arc lengths.
    pub fn water_depth(&self, x: f32, z: f32) -> Option<f32> {
        let r = self.river.as_ref()?;
        if (x - z).abs() * std::f32::consts::FRAC_1_SQRT_2 > r.half_width {
            return None;
        }
        let along = (x + z) * 0.5;
        Some(
            if r.fords
                .iter()
                .any(|f| (along - f).abs() <= r.ford_half_length)
            {
                r.shallow_depth
            } else {
                r.deep_depth
            },
        )
    }
    pub fn deep(&self, x: f32, z: f32) -> bool {
        self.water_depth(x, z)
            .is_some_and(|v| self.river.as_ref().is_some_and(|r| v == r.deep_depth))
    }
    pub fn height(&self, x: f32, z: f32, base: f32) -> f32 {
        if let Some(depth) = self.water_depth(x, z) {
            return self.river.as_ref().unwrap().surface - depth;
        }
        base + self
            .rises
            .iter()
            .map(|r| {
                let edge = (x - r.center[0]).abs() - r.size[0] * 0.5;
                let edge = edge.max((z - r.center[1]).abs() - r.size[1] * 0.5).max(0.);
                r.height * (1. - edge / r.apron).clamp(0., 1.)
            })
            .fold(0.0, f32::max)
    }
    pub(super) fn liquid(&self, floor: &TriMesh) -> Option<Liquid> {
        let r = self.river.as_ref()?;
        // A regular river-aligned grid gives smooth banks and local freeze vertices.
        let min = floor
            .vertices
            .iter()
            .map(|v| v[0].min(v[2]))
            .fold(f32::INFINITY, f32::min);
        let max = floor
            .vertices
            .iter()
            .map(|v| v[0].max(v[2]))
            .fold(f32::NEG_INFINITY, f32::max);
        let along_cells = ((max - min) * std::f32::consts::SQRT_2 / 2.).ceil() as usize;
        let across_cells = (r.half_width * 2. / 2.).ceil() as usize;
        let mut vertices = Vec::new();
        let mut triangles = Vec::new();
        for z in 0..=along_cells {
            for x in 0..=across_cells {
                let along = min + (max - min) * z as f32 / along_cells as f32;
                let side = (-r.half_width + 2. * r.half_width * x as f32 / across_cells as f32)
                    * std::f32::consts::FRAC_1_SQRT_2;
                vertices.push([
                    (along + side).clamp(min, max),
                    r.surface,
                    (along - side).clamp(min, max),
                ]);
            }
        }
        for z in 0..along_cells {
            for x in 0..across_cells {
                let a = (z * (across_cells + 1) + x) as i32;
                let b = a + (across_cells + 1) as i32;
                let c = b + 1;
                let d = a + 1;
                triangles.extend([[a, b, c], [a, c, d]]);
            }
        }
        let vb = vertices
            .iter()
            .flatten()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        // Liquid::collision_geometry reverses the stored winding.
        let ib = triangles
            .iter()
            .flat_map(|t| [t[0], t[2], t[1]])
            .flat_map(|i| i.to_le_bytes())
            .collect();
        Some(Liquid {
            material: LiquidMaterial::Water(Box::new(LiquidEffect {
                reflection_map: None,
                wave_height: 0.05,
                wave_speed0: [0.01, 0.02],
                wave_speed1: [-0.02, 0.01],
                water_reflectiveness: 0.35,
                bottom_color: [0.28, 0.46, 0.4],
                deep_bottom_color: [0.035, 0.12, 0.15],
                water_emissive: 0.1,
                water_spec_amount: 0.6,
                water_spec_power: 40.,
                bottom_texture: Some("Levels/Textures/Surface/Nature/Ground/pebbles00_0".into()),
                water_normal_map: Some("Levels/Textures/Liquids/WaterNormals_0".into()),
                ice_reflectiveness: 0.4,
                ice_color: [0.65, 0.85, 0.9],
                ice_emissive: 0.1,
                ice_spec_amount: 0.6,
                ice_spec_power: 40.,
                ice_diffuse_map: Some("Levels/Textures/Surface/Nature/Ground/ice_0".into()),
                ice_normal_map: Some("Levels/Textures/Liquids/IceNrm_0".into()),
            })),
            vertex_buffer: Some(vb),
            index_buffer: Some((false, ib)),
            vertex_declaration: Some(vec![VertexElement {
                stream: 0,
                offset: 0,
                format: 2,
                method: 0,
                usage: 0,
                usage_index: 0,
            }]),
            vertex_stride: 12,
            num_vertices: vertices.len() as i32,
            primitive_count: triangles.len() as i32,
            has_collision: true,
            freezable: true,
            auto_freeze: false,
        })
    }
}
