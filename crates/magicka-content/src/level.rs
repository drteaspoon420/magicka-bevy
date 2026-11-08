//! `Magicka.ContentReaders.LevelModelReader` -> `Levels.LevelModel(ContentReader)`
//! (`Levels/<area>/<scene>.xnb`, paired with `<scene>.xml`).

use crate::common::*;
use crate::xna::{self, BiTreeModel, Model, VertexElement, XnaObject};

/// Collision triangle mesh: `ReadObject<List<Vector3>>` + `count × (i32,i32,i32)`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TriMesh {
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[i32; 3]>,
}

impl TriMesh {
    pub(crate) fn read(r: &mut Reader) -> Result<Self> {
        let vertices = xna::expect_vec3_list(xna::read_object(r)?)?;
        let n = r.count()?;
        let triangles = (0..n)
            .map(|_| Ok([r.i32()?, r.i32()?, r.i32()?]))
            .collect::<Result<_>>()?;
        Ok(TriMesh {
            vertices,
            triangles,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Locator {
    pub name: String,
    pub transform: [f32; 16],
    pub radius: f32,
}

impl Locator {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Locator {
            name: r.string()?,
            transform: xna::matrix(r)?,
            radius: r.f32()?,
        })
    }
}

/// `VisualEffectStorage` as read by `LevelModel` / `AnimatedLevelPart`; the
/// game builds `Matrix.CreateWorld(position, forward, Up)`.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectPlacement {
    pub id: String,
    pub position: [f32; 3],
    pub forward: [f32; 3],
    pub range: f32,
    pub effect: String,
}

impl EffectPlacement {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(EffectPlacement {
            id: r.string()?,
            position: r.vec3()?,
            forward: r.vec3()?,
            range: r.f32()?,
            effect: r.string()?,
        })
    }
}

/// `PathFinding.Triangle`.
#[derive(Debug, Clone, PartialEq)]
pub struct NavTriangle {
    pub vertices: [u16; 3],
    pub neighbours: [u16; 3],
    pub costs: [f32; 3],
    pub properties: u8,
}

/// `NavMesh` / `AnimatedNavMesh` (identical layouts; u16 counts).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NavMesh {
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<NavTriangle>,
}

impl NavMesh {
    fn read(r: &mut Reader) -> Result<Self> {
        let n = r.u16()? as usize;
        let vertices = (0..n).map(|_| r.vec3()).collect::<Result<_>>()?;
        let n = r.u16()? as usize;
        let triangles = (0..n)
            .map(|_| {
                Ok(NavTriangle {
                    vertices: [r.u16()?, r.u16()?, r.u16()?],
                    neighbours: [r.u16()?, r.u16()?, r.u16()?],
                    costs: [r.f32()?, r.f32()?, r.f32()?],
                    properties: r.u8()?,
                })
            })
            .collect::<Result<_>>()?;
        Ok(NavMesh {
            vertices,
            triangles,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LiquidMaterial {
    Water(Box<xna::LiquidEffect>),
    Lava(Box<xna::LavaEffect>),
}

/// `Liquid.Read` -> `Water` / `Lava` (same stored layout after the effect).
#[derive(Debug, Clone, PartialEq)]
pub struct Liquid {
    pub material: LiquidMaterial,
    pub vertex_buffer: Option<Vec<u8>>,
    pub index_buffer: Option<(bool, Vec<u8>)>,
    pub vertex_declaration: Option<Vec<VertexElement>>,
    pub vertex_stride: i32,
    pub num_vertices: i32,
    pub primitive_count: i32,
    pub has_collision: bool,
    pub freezable: bool,
    pub auto_freeze: bool,
}

impl Liquid {
    /// Water constructor's CPU collision geometry. Stored winding is reversed
    /// as I0,I2,I1; 16-bit indices are read as signed shorts in the original.
    pub fn collision_geometry(&self) -> Result<TriMesh> {
        let vb = self
            .vertex_buffer
            .as_ref()
            .ok_or_else(|| ContentError::Invalid("liquid vertex buffer missing".into()))?;
        let (sixteen, ib) = self
            .index_buffer
            .as_ref()
            .ok_or_else(|| ContentError::Invalid("liquid index buffer missing".into()))?;
        let decl = self
            .vertex_declaration
            .as_ref()
            .ok_or_else(|| ContentError::Invalid("liquid declaration missing".into()))?;
        let stride = usize::try_from(self.vertex_stride)
            .map_err(|_| ContentError::Invalid("negative liquid stride".into()))?;
        let count = usize::try_from(self.num_vertices)
            .map_err(|_| ContentError::Invalid("negative liquid vertex count".into()))?;
        let triangles = usize::try_from(self.primitive_count)
            .map_err(|_| ContentError::Invalid("negative liquid triangle count".into()))?;
        let offset = decl.iter().rfind(|e| e.usage == 0).map_or(0, |e| e.offset);
        let offset = usize::try_from(offset)
            .map_err(|_| ContentError::Invalid("negative liquid position offset".into()))?;
        if stride == 0 || count.checked_mul(stride).is_none_or(|n| n > vb.len()) {
            return invalid("invalid liquid vertex extent");
        }
        let mut vertices = Vec::with_capacity(count);
        for i in 0..count {
            let start = i
                .checked_mul(stride)
                .and_then(|n| n.checked_add(offset))
                .ok_or_else(|| ContentError::Invalid("liquid position overflow".into()))?;
            let bytes = vb
                .get(
                    start
                        ..start.checked_add(12).ok_or_else(|| {
                            ContentError::Invalid("liquid position overflow".into())
                        })?,
                )
                .ok_or_else(|| ContentError::Invalid("truncated liquid position".into()))?;
            vertices.push(std::array::from_fn(|axis| {
                f32::from_le_bytes(bytes[axis * 4..axis * 4 + 4].try_into().unwrap())
            }));
        }
        let width = if *sixteen { 2 } else { 4 };
        let extent = triangles
            .checked_mul(3)
            .and_then(|n| n.checked_mul(width))
            .ok_or_else(|| ContentError::Invalid("liquid index overflow".into()))?;
        let bytes = ib
            .get(..extent)
            .ok_or_else(|| ContentError::Invalid("truncated liquid indices".into()))?;
        let indices: Vec<i32> = bytes
            .chunks_exact(width)
            .map(|b| {
                if *sixteen {
                    i16::from_le_bytes(b.try_into().unwrap()) as i32
                } else {
                    i32::from_le_bytes(b.try_into().unwrap())
                }
            })
            .collect();
        if indices.iter().any(|&i| i < 0 || i as usize >= count) {
            return invalid("liquid index out of range");
        }
        Ok(TriMesh {
            vertices,
            triangles: indices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|i| [i[0], i[2], i[1]])
                .collect(),
        })
    }

    fn read(r: &mut Reader) -> Result<Self> {
        let material = match xna::read_object(r)? {
            Some(XnaObject::RenderDeferredLiquidEffect(e)) => LiquidMaterial::Water(e),
            Some(XnaObject::LavaEffect(e)) => LiquidMaterial::Lava(e),
            o => {
                return invalid(format!(
                    "Liquid effect must be water or lava (original throws), got {o:?}"
                ))
            }
        };
        Ok(Liquid {
            material,
            vertex_buffer: xna::expect_vb(xna::read_object(r)?)?,
            index_buffer: xna::expect_ib(xna::read_object(r)?)?,
            vertex_declaration: xna::expect_vd(xna::read_object(r)?)?,
            vertex_stride: r.i32()?,
            num_vertices: r.i32()?,
            primitive_count: r.i32()?,
            has_collision: r.bool()?,
            freezable: r.bool()?,
            auto_freeze: r.bool()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForceField {
    pub color: [f32; 3],
    pub width: f32,
    pub alpha_power: f32,
    pub alpha_falloff_power: f32,
    pub max_radius: f32,
    pub ripple_distortion: f32,
    pub map_distortion: f32,
    pub vertex_color_enabled: bool,
    pub displacement_map: Option<String>,
    pub ttl: f32,
    pub vertex_buffer: Option<Vec<u8>>,
    pub index_buffer: Option<(bool, Vec<u8>)>,
    pub vertex_declaration: Option<Vec<VertexElement>>,
    pub vertex_stride: i32,
    pub num_vertices: i32,
    pub primitive_count: i32,
}

impl ForceField {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(ForceField {
            color: r.vec3()?,
            width: r.f32()?,
            alpha_power: r.f32()?,
            alpha_falloff_power: r.f32()?,
            max_radius: r.f32()?,
            ripple_distortion: r.f32()?,
            map_distortion: r.f32()?,
            vertex_color_enabled: r.bool()?,
            displacement_map: r.external()?,
            ttl: r.f32()?,
            vertex_buffer: xna::expect_vb(xna::read_object(r)?)?,
            index_buffer: xna::expect_ib(xna::read_object(r)?)?,
            vertex_declaration: xna::expect_vd(xna::read_object(r)?)?,
            vertex_stride: r.i32()?,
            num_vertices: r.i32()?,
            primitive_count: r.i32()?,
        })
    }
}

/// XNAnimation `AnimationChannel` keyframe: time, translation, rotation, scale.
#[derive(Debug, Clone, PartialEq)]
pub struct Keyframe {
    pub time: f32,
    pub translation: [f32; 3],
    pub orientation: [f32; 4],
    pub scale: [f32; 3],
}

/// `AnimatedLevelPart(ContentReader, LevelModel)` (recursive).
#[derive(Debug, Clone, PartialEq)]
pub struct AnimatedLevelPart {
    pub name: String,
    pub affect_shields: bool,
    pub model: Option<Model>,
    /// mesh name -> (bool, bool) settings.
    pub mesh_settings: Vec<(String, bool, bool)>,
    pub liquids: Vec<Liquid>,
    pub locators: Vec<Locator>,
    pub animation_duration: f32,
    pub animation: Vec<Keyframe>,
    pub effects: Vec<EffectPlacement>,
    pub lights: Vec<(String, [f32; 16])>,
    pub collision: Option<(u8, TriMesh)>,
    pub nav_mesh: Option<NavMesh>,
    pub children: Vec<AnimatedLevelPart>,
}

impl AnimatedLevelPart {
    fn read(r: &mut Reader) -> Result<Self> {
        let name = r.string()?;
        let affect_shields = r.bool()?;
        let model = match xna::read_object(r)? {
            None => None,
            Some(XnaObject::Model(m)) => Some(*m),
            Some(o) => return invalid(format!("expected Model, got {o:?}")),
        };
        let n = r.count()?;
        let mesh_settings = (0..n)
            .map(|_| Ok((r.string()?, r.bool()?, r.bool()?)))
            .collect::<Result<_>>()?;
        let n = r.count()?;
        let liquids = (0..n).map(|_| Liquid::read(r)).collect::<Result<_>>()?;
        let n = r.count()?;
        let locators = (0..n).map(|_| Locator::read(r)).collect::<Result<_>>()?;
        let animation_duration = r.f32()?;
        let n = r.count()?;
        let animation = (0..n)
            .map(|_| {
                Ok(Keyframe {
                    time: r.f32()?,
                    translation: r.vec3()?,
                    orientation: xna::quaternion(r)?,
                    scale: r.vec3()?,
                })
            })
            .collect::<Result<_>>()?;
        let n = r.count()?;
        let effects = (0..n)
            .map(|_| EffectPlacement::read(r))
            .collect::<Result<_>>()?;
        let n = r.count()?;
        let lights = (0..n)
            .map(|_| Ok((r.string()?, xna::matrix(r)?)))
            .collect::<Result<_>>()?;
        let collision = if r.bool()? {
            Some((r.u8()?, TriMesh::read(r)?))
        } else {
            None
        };
        let nav_mesh = if r.bool()? {
            Some(NavMesh::read(r)?)
        } else {
            None
        };
        let n = r.count()?;
        let children = (0..n)
            .map(|_| AnimatedLevelPart::read(r))
            .collect::<Result<_>>()?;
        Ok(AnimatedLevelPart {
            name,
            affect_shields,
            model,
            mesh_settings,
            liquids,
            locators,
            animation_duration,
            animation,
            effects,
            lights,
            collision,
            nav_mesh,
            children,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LevelLight {
    pub name: String,
    pub position: [f32; 3],
    pub direction: [f32; 3],
    /// `PolygonHead.Lights.LightType`; Point, Directional, Spot (others throw).
    pub light_type: i32,
    pub variation_type: i32,
    /// Radius (point) or range (spot).
    pub range: f32,
    pub use_attenuation: bool,
    pub cutoff_angle: f32,
    pub sharpness: f32,
    pub diffuse: [f32; 3],
    pub ambient: [f32; 3],
    pub specular: f32,
    pub variation_speed: f32,
    pub variation_amount: f32,
    /// As stored; scaled by the shadow-quality setting at load.
    pub shadow_map_size: i32,
    pub cast_shadows: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerArea {
    pub name: String,
    pub position: [f32; 3],
    pub sides: [f32; 3],
    pub orientation: [f32; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct LevelModel {
    pub model: Option<BiTreeModel>,
    pub animated_parts: Vec<AnimatedLevelPart>,
    pub lights: Vec<LevelLight>,
    pub effects: Vec<EffectPlacement>,
    /// Start transform + template name under `Data/PhysicsEntities/`.
    pub physics_entities: Vec<([f32; 16], String)>,
    pub liquids: Vec<Liquid>,
    pub force_fields: Vec<ForceField>,
    /// Exactly 10 collision layers (index = collision material slot); absent
    /// layers are empty.
    pub collision_layers: Vec<TriMesh>,
    pub camera_mesh: Option<TriMesh>,
    pub trigger_areas: Vec<TriggerArea>,
    pub locators: Vec<Locator>,
    pub nav_mesh: NavMesh,
}

impl LevelModel {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let model = match xna::read_object(r)? {
            None => None,
            Some(XnaObject::BiTreeModel(m)) => Some(*m),
            Some(o) => return invalid(format!("expected BiTreeModel, got {o:?}")),
        };
        let n = r.count()?;
        let animated_parts = (0..n)
            .map(|_| AnimatedLevelPart::read(r))
            .collect::<Result<_>>()?;
        let n = r.count()?;
        let mut lights = Vec::with_capacity(n);
        for _ in 0..n {
            let name = r.string()?;
            let position = r.vec3()?;
            let direction = r.vec3()?;
            let light_type = r.i32()?;
            let variation_type = r.i32()?;
            let range = r.f32()?;
            let use_attenuation = r.bool()?;
            let cutoff_angle = r.f32()?;
            let sharpness = r.f32()?;
            if !(0..=2).contains(&light_type) {
                return invalid(format!("Unknown light type {light_type}"));
            }
            lights.push(LevelLight {
                name,
                position,
                direction,
                light_type,
                variation_type,
                range,
                use_attenuation,
                cutoff_angle,
                sharpness,
                diffuse: r.vec3()?,
                ambient: r.vec3()?,
                specular: r.f32()?,
                variation_speed: r.f32()?,
                variation_amount: r.f32()?,
                shadow_map_size: r.i32()?,
                cast_shadows: r.bool()?,
            });
        }
        let n = r.count()?;
        let effects = (0..n)
            .map(|_| EffectPlacement::read(r))
            .collect::<Result<_>>()?;
        let n = r.count()?;
        let physics_entities = (0..n)
            .map(|_| Ok((xna::matrix(r)?, r.string()?)))
            .collect::<Result<_>>()?;
        let n = r.count()?;
        let liquids = (0..n).map(|_| Liquid::read(r)).collect::<Result<_>>()?;
        let n = r.count()?;
        let force_fields = (0..n).map(|_| ForceField::read(r)).collect::<Result<_>>()?;
        let collision_layers = (0..10)
            .map(|_| {
                if r.bool()? {
                    TriMesh::read(r)
                } else {
                    Ok(TriMesh::default())
                }
            })
            .collect::<Result<_>>()?;
        let camera_mesh = if r.bool()? {
            Some(TriMesh::read(r)?)
        } else {
            None
        };
        let n = r.count()?;
        let trigger_areas = (0..n)
            .map(|_| {
                Ok(TriggerArea {
                    name: r.string()?,
                    position: r.vec3()?,
                    sides: r.vec3()?,
                    orientation: xna::quaternion(r)?,
                })
            })
            .collect::<Result<_>>()?;
        let n = r.count()?;
        let locators = (0..n).map(|_| Locator::read(r)).collect::<Result<_>>()?;
        let nav_mesh = NavMesh::read(r)?;
        Ok(LevelModel {
            model,
            animated_parts,
            lights,
            effects,
            physics_entities,
            liquids,
            force_fields,
            collision_layers,
            camera_mesh,
            trigger_areas,
            locators,
            nav_mesh,
        })
    }
}
