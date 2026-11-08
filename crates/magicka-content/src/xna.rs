//! XNA 3.1 framework object readers needed by Magicka content.
//!
//! Source: `Microsoft.Xna.Framework.dll` 3.1 from the game's bundled
//! `Dependencies/xnafx31_redist.msi` (SHA-256 c4f8f4c7…3e3b), decompiled with
//! ILSpy: `ContentReader.ReadObject/ReadSharedResource`, `Model.Read`,
//! `VertexBufferReader`, `IndexBufferReader`, `VertexDeclarationReader`,
//! `ListReader<T>`. PolygonHead readers: `RenderDeferredEffect.Read`.

use crate::common::*;

/// `ContentReader.ReadObject<T>()` result, dispatched on the XNB manifest.
#[derive(Debug, Clone, PartialEq)]
pub enum XnaObject {
    String(String),
    VertexBuffer(Vec<u8>),
    IndexBuffer {
        sixteen_bit: bool,
        data: Vec<u8>,
    },
    VertexDeclaration(Vec<VertexElement>),
    Model(Box<Model>),
    Vector3List(Vec<[f32; 3]>),
    RenderDeferredEffect(Box<RenderDeferredEffect>),
    RenderDeferredLiquidEffect(Box<LiquidEffect>),
    LavaEffect(Box<LavaEffect>),
    AdditiveEffect(Box<AdditiveEffect>),
    BiTreeModel(Box<BiTreeModel>),
    /// Standard XNA / XNAnimation / PolygonHead objects (see `assets.rs`).
    Asset(Box<crate::assets::Asset>),
}

/// PolygonHead `RenderDeferredLiquidEffect.Read`.
#[derive(Debug, Clone, PartialEq)]
pub struct LiquidEffect {
    pub reflection_map: Option<String>,
    pub wave_height: f32,
    pub wave_speed0: [f32; 2],
    pub wave_speed1: [f32; 2],
    pub water_reflectiveness: f32,
    pub bottom_color: [f32; 3],
    pub deep_bottom_color: [f32; 3],
    pub water_emissive: f32,
    pub water_spec_amount: f32,
    pub water_spec_power: f32,
    pub bottom_texture: Option<String>,
    pub water_normal_map: Option<String>,
    pub ice_reflectiveness: f32,
    pub ice_color: [f32; 3],
    pub ice_emissive: f32,
    pub ice_spec_amount: f32,
    pub ice_spec_power: f32,
    pub ice_diffuse_map: Option<String>,
    pub ice_normal_map: Option<String>,
}

/// PolygonHead `LavaEffect.Read`.
#[derive(Debug, Clone, PartialEq)]
pub struct LavaEffect {
    pub mask_distortion: f32,
    pub speed0: [f32; 2],
    pub speed1: [f32; 2],
    pub lava_hot_emissive: f32,
    pub lava_cold_emissive: f32,
    pub lava_spec_amount: f32,
    pub lava_spec_power: f32,
    pub temp_frequency: f32,
    pub tone_map: Option<String>,
    pub temp_map: Option<String>,
    pub mask_map: Option<String>,
    pub rock_color: [f32; 3],
    pub rock_emissive: f32,
    pub rock_spec_amount: f32,
    pub rock_spec_power: f32,
    pub rock_normal_power: f32,
    pub rock_texture: Option<String>,
    pub rock_normal_map: Option<String>,
}

/// PolygonHead `AdditiveEffect.Read`.
#[derive(Debug, Clone, PartialEq)]
pub struct AdditiveEffect {
    pub color_tint: [f32; 3],
    pub vertex_color_enabled: bool,
    pub texture_enabled: bool,
    pub texture: Option<String>,
}

/// PolygonHead `BiTreeNode` (spatial split of a root's index range).
#[derive(Debug, Clone, PartialEq)]
pub struct BiTreeNode {
    pub primitive_count: i32,
    pub start_index: i32,
    pub bounds: ([f32; 3], [f32; 3]),
    pub child_a: Option<Box<BiTreeNode>>,
    pub child_b: Option<Box<BiTreeNode>>,
}

/// PolygonHead `BiTreeRootNode`: one renderable batch of level geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct BiTreeRoot {
    pub visible: bool,
    pub cast_shadows: bool,
    pub sway: f32,
    pub entity_influence: f32,
    pub ground_level: f32,
    pub num_vertices: i32,
    pub vertex_stride: i32,
    pub vertex_declaration: Option<Vec<VertexElement>>,
    pub vertex_buffer: Option<Vec<u8>>,
    pub index_buffer: Option<(bool, Vec<u8>)>,
    pub effect: Option<XnaObject>,
    pub node: BiTreeNode,
}

/// PolygonHead `BiTreeModel.Read`: static level geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct BiTreeModel {
    pub roots: Vec<BiTreeRoot>,
}

pub fn vec2(r: &mut Reader) -> Result<[f32; 2]> {
    Ok([r.f32()?, r.f32()?])
}

fn read_bitree_node_body(r: &mut Reader, primitive_count: i32) -> Result<BiTreeNode> {
    let start_index = r.i32()?;
    let bounds = (r.vec3()?, r.vec3()?);
    let child_a = if r.bool()? {
        Some(Box::new(read_bitree_node(r)?))
    } else {
        None
    };
    let child_b = if r.bool()? {
        Some(Box::new(read_bitree_node(r)?))
    } else {
        None
    };
    Ok(BiTreeNode {
        primitive_count,
        start_index,
        bounds,
        child_a,
        child_b,
    })
}

fn read_bitree_node(r: &mut Reader) -> Result<BiTreeNode> {
    let primitive_count = r.i32()?;
    read_bitree_node_body(r, primitive_count)
}

pub fn read_bitree_model(r: &mut Reader) -> Result<BiTreeModel> {
    let n = r.count()?;
    let mut roots = Vec::with_capacity(n);
    for _ in 0..n {
        let visible = r.bool()?;
        let cast_shadows = r.bool()?;
        let sway = r.f32()?;
        let entity_influence = r.f32()?;
        let ground_level = r.f32()?;
        let num_vertices = r.i32()?;
        let vertex_stride = r.i32()?;
        let vertex_declaration = expect_vd(read_object(r)?)?;
        let vertex_buffer = expect_vb(read_object(r)?)?;
        let index_buffer = expect_ib(read_object(r)?)?;
        let effect = read_object(r)?;
        let primitive_count = r.i32()?;
        let node = read_bitree_node_body(r, primitive_count)?;
        roots.push(BiTreeRoot {
            visible,
            cast_shadows,
            sway,
            entity_influence,
            ground_level,
            num_vertices,
            vertex_stride,
            vertex_declaration,
            vertex_buffer,
            index_buffer,
            effect,
            node,
        });
    }
    Ok(BiTreeModel { roots })
}

pub fn expect_vb(o: Option<XnaObject>) -> Result<Option<Vec<u8>>> {
    match o {
        None => Ok(None),
        Some(XnaObject::VertexBuffer(b)) => Ok(Some(b)),
        Some(o) => invalid(format!("expected vertex buffer, got {o:?}")),
    }
}

pub fn expect_ib(o: Option<XnaObject>) -> Result<Option<(bool, Vec<u8>)>> {
    match o {
        None => Ok(None),
        Some(XnaObject::IndexBuffer { sixteen_bit, data }) => Ok(Some((sixteen_bit, data))),
        Some(o) => invalid(format!("expected index buffer, got {o:?}")),
    }
}

pub fn expect_vd(o: Option<XnaObject>) -> Result<Option<Vec<VertexElement>>> {
    match o {
        None => Ok(None),
        Some(XnaObject::VertexDeclaration(v)) => Ok(Some(v)),
        Some(o) => invalid(format!("expected vertex declaration, got {o:?}")),
    }
}

pub fn expect_vec3_list(o: Option<XnaObject>) -> Result<Vec<[f32; 3]>> {
    match o {
        None => Ok(Vec::new()),
        Some(XnaObject::Vector3List(v)) => Ok(v),
        Some(o) => invalid(format!("expected List<Vector3>, got {o:?}")),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VertexElement {
    pub stream: i16,
    pub offset: i16,
    pub format: u8,
    pub method: u8,
    pub usage: u8,
    pub usage_index: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bone {
    pub name: Option<String>,
    pub transform: [f32; 16],
    pub parent: Option<usize>,
    pub children: Vec<Option<usize>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeshPart {
    pub stream_offset: i32,
    pub base_vertex: i32,
    pub num_vertices: i32,
    pub start_index: i32,
    pub primitive_count: i32,
    pub vertex_declaration: i32,
    /// 1-based shared-resource index of the part's effect (0 = none).
    pub effect: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    pub name: Option<String>,
    pub parent_bone: Option<usize>,
    pub bounding_sphere: ([f32; 3], f32),
    pub vertex_buffer: Option<Vec<u8>>,
    pub index_buffer: Option<(bool, Vec<u8>)>,
    pub parts: Vec<MeshPart>,
}

/// `Microsoft.Xna.Framework.Graphics.Model` (XNA 3.1 layout).
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub bones: Vec<Bone>,
    pub vertex_declarations: Vec<Vec<VertexElement>>,
    pub meshes: Vec<Mesh>,
    pub root: Option<usize>,
}

/// PolygonHead `RenderDeferredEffect.Read` material layer.
#[derive(Debug, Clone, PartialEq)]
pub struct DeferredLayer {
    pub diffuse_alpha_disabled: bool,
    pub alpha_mask_enabled: bool,
    pub diffuse_color: [f32; 3],
    pub spec_amount: f32,
    pub spec_power: f32,
    pub emissive_amount: f32,
    pub normal_power: f32,
    pub reflectiveness: f32,
    pub diffuse_texture: Option<String>,
    pub material_texture: Option<String>,
    pub normal_texture: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderDeferredEffect {
    pub alpha: f32,
    pub sharpness: f32,
    pub vertex_color_enabled: bool,
    pub use_material_texture_for_reflectiveness: bool,
    pub reflection_map: Option<String>,
    pub layer0: DeferredLayer,
    pub layer1: Option<DeferredLayer>,
}

impl DeferredLayer {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(DeferredLayer {
            diffuse_alpha_disabled: r.bool()?,
            alpha_mask_enabled: r.bool()?,
            diffuse_color: r.vec3()?,
            spec_amount: r.f32()?,
            spec_power: r.f32()?,
            emissive_amount: r.f32()?,
            normal_power: r.f32()?,
            reflectiveness: r.f32()?,
            diffuse_texture: r.external()?,
            material_texture: r.external()?,
            normal_texture: r.external()?,
        })
    }
}

impl RenderDeferredEffect {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(RenderDeferredEffect {
            alpha: r.f32()?,
            sharpness: r.f32()?,
            vertex_color_enabled: r.bool()?,
            use_material_texture_for_reflectiveness: r.bool()?,
            reflection_map: r.external()?,
            layer0: DeferredLayer::read(r)?,
            layer1: if r.bool()? {
                Some(DeferredLayer::read(r)?)
            } else {
                None
            },
        })
    }
}

pub fn matrix(r: &mut Reader) -> Result<[f32; 16]> {
    let mut m = [0f32; 16];
    for v in &mut m {
        *v = r.f32()?;
    }
    Ok(m)
}

pub fn quaternion(r: &mut Reader) -> Result<[f32; 4]> {
    Ok([r.f32()?, r.f32()?, r.f32()?, r.f32()?])
}

fn bytes(r: &mut Reader, n: usize) -> Result<Vec<u8>> {
    Ok(r.c.bytes(n)?.to_vec())
}

/// `ContentReader.ReadObject<T>()`: 7-bit type id (0 = null, else manifest
/// index + 1), then the named reader's body.
pub fn read_object(r: &mut Reader) -> Result<Option<XnaObject>> {
    let id = r.c.var_u32()? as usize;
    if id == 0 {
        return Ok(None);
    }
    let Some(name) = r.types.get(id - 1).cloned() else {
        return invalid(format!("type id {id} out of range"));
    };
    read_body(r, &name).map(Some)
}

/// The body of an object whose reader is `name` (assembly-qualified type
/// reader name from the manifest). Used for `ReadObject` and for the primary
/// object of standard (non-Magicka) XNB files.
pub fn read_body(r: &mut Reader, name: &str) -> Result<XnaObject> {
    // Strip the assembly qualification, keeping generic arguments intact.
    let short = match name.find("]]") {
        Some(i) => &name[..i + 2],
        None => name.split(',').next().unwrap_or(""),
    };
    let obj = match short {
        "Microsoft.Xna.Framework.Content.StringReader" => XnaObject::String(r.string()?),
        "Microsoft.Xna.Framework.Content.VertexBufferReader" => {
            let n = r.count()?;
            XnaObject::VertexBuffer(bytes(r, n)?)
        }
        "Microsoft.Xna.Framework.Content.IndexBufferReader" => {
            let sixteen_bit = r.bool()?;
            let n = r.count()?;
            XnaObject::IndexBuffer {
                sixteen_bit,
                data: bytes(r, n)?,
            }
        }
        "Microsoft.Xna.Framework.Content.VertexDeclarationReader" => {
            let n = r.count()?;
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                let b = r.c.bytes(4)?;
                let (stream, offset) = (
                    i16::from_le_bytes([b[0], b[1]]),
                    i16::from_le_bytes([b[2], b[3]]),
                );
                v.push(VertexElement {
                    stream,
                    offset,
                    format: r.u8()?,
                    method: r.u8()?,
                    usage: r.u8()?,
                    usage_index: r.u8()?,
                });
            }
            XnaObject::VertexDeclaration(v)
        }
        "Microsoft.Xna.Framework.Content.ModelReader" => XnaObject::Model(Box::new(read_model(r)?)),
        s if s.starts_with(
            "Microsoft.Xna.Framework.Content.ListReader`1[[Microsoft.Xna.Framework.Vector3",
        ) =>
        {
            let n = r.count()?;
            XnaObject::Vector3List((0..n).map(|_| r.vec3()).collect::<Result<_>>()?)
        }
        "PolygonHead.Pipeline.RenderDeferredEffectReader" => {
            XnaObject::RenderDeferredEffect(Box::new(RenderDeferredEffect::read(r)?))
        }
        "PolygonHead.Pipeline.RenderDeferredLiquidEffectReader" => {
            XnaObject::RenderDeferredLiquidEffect(Box::new(LiquidEffect {
                reflection_map: r.external()?,
                wave_height: r.f32()?,
                wave_speed0: vec2(r)?,
                wave_speed1: vec2(r)?,
                water_reflectiveness: r.f32()?,
                bottom_color: r.vec3()?,
                deep_bottom_color: r.vec3()?,
                water_emissive: r.f32()?,
                water_spec_amount: r.f32()?,
                water_spec_power: r.f32()?,
                bottom_texture: r.external()?,
                water_normal_map: r.external()?,
                ice_reflectiveness: r.f32()?,
                ice_color: r.vec3()?,
                ice_emissive: r.f32()?,
                ice_spec_amount: r.f32()?,
                ice_spec_power: r.f32()?,
                ice_diffuse_map: r.external()?,
                ice_normal_map: r.external()?,
            }))
        }
        "PolygonHead.Pipeline.LavaEffectReader" => XnaObject::LavaEffect(Box::new(LavaEffect {
            mask_distortion: r.f32()?,
            speed0: vec2(r)?,
            speed1: vec2(r)?,
            lava_hot_emissive: r.f32()?,
            lava_cold_emissive: r.f32()?,
            lava_spec_amount: r.f32()?,
            lava_spec_power: r.f32()?,
            temp_frequency: r.f32()?,
            tone_map: r.external()?,
            temp_map: r.external()?,
            mask_map: r.external()?,
            rock_color: r.vec3()?,
            rock_emissive: r.f32()?,
            rock_spec_amount: r.f32()?,
            rock_spec_power: r.f32()?,
            rock_normal_power: r.f32()?,
            rock_texture: r.external()?,
            rock_normal_map: r.external()?,
        })),
        "PolygonHead.Pipeline.AdditiveEffectReader" => {
            XnaObject::AdditiveEffect(Box::new(AdditiveEffect {
                color_tint: r.vec3()?,
                vertex_color_enabled: r.bool()?,
                texture_enabled: r.bool()?,
                texture: r.external()?,
            }))
        }
        "PolygonHead.Pipeline.BiTreeModelReader" => {
            XnaObject::BiTreeModel(Box::new(read_bitree_model(r)?))
        }
        other => match crate::assets::read_body(r, other)? {
            Some(o) => o,
            None => return invalid(format!("unsupported object reader {other}")),
        },
    };
    Ok(obj)
}

fn expect_string(o: Option<XnaObject>) -> Result<Option<String>> {
    match o {
        None => Ok(None),
        Some(XnaObject::String(s)) => Ok(Some(s)),
        Some(other) => invalid(format!("expected string, got {other:?}")),
    }
}

/// A `ReadObject<object>()` tag (any object; usually null).
fn expect_null_tag(r: &mut Reader) -> Result<()> {
    read_object(r).map(|_| ())
}

/// `Model.Read` (XNA 3.1).
pub fn read_model(r: &mut Reader) -> Result<Model> {
    let n = r.count()?;
    let mut bones = Vec::with_capacity(n);
    for _ in 0..n {
        bones.push(Bone {
            name: expect_string(read_object(r)?)?,
            transform: matrix(r)?,
            parent: None,
            children: Vec::new(),
        });
    }
    // Bone references: byte if count + 1 <= 255, else Int32; 0 = null.
    let wide = n + 1 > 255;
    let bone_ref = |r: &mut Reader| -> Result<Option<usize>> {
        let v = if wide {
            r.i32()? as i64
        } else {
            r.u8()? as i64
        };
        if v == 0 {
            Ok(None)
        } else if (v as usize) <= n {
            Ok(Some(v as usize - 1))
        } else {
            invalid(format!("bone reference {v} out of range"))
        }
    };
    for bone in &mut bones {
        bone.parent = bone_ref(r)?;
        let k = r.count()?;
        bone.children = (0..k).map(|_| bone_ref(r)).collect::<Result<_>>()?;
    }
    let k = r.count()?;
    let mut vertex_declarations = Vec::with_capacity(k);
    for _ in 0..k {
        match read_object(r)? {
            Some(XnaObject::VertexDeclaration(v)) => vertex_declarations.push(v),
            other => return invalid(format!("expected vertex declaration, got {other:?}")),
        }
    }
    let m = r.count()?;
    let mut meshes = Vec::with_capacity(m);
    for _ in 0..m {
        let name = expect_string(read_object(r)?)?;
        let parent_bone = bone_ref(r)?;
        let bounding_sphere = (r.vec3()?, r.f32()?);
        let vertex_buffer = match read_object(r)? {
            None => None,
            Some(XnaObject::VertexBuffer(b)) => Some(b),
            Some(o) => return invalid(format!("expected vertex buffer, got {o:?}")),
        };
        let index_buffer = match read_object(r)? {
            None => None,
            Some(XnaObject::IndexBuffer { sixteen_bit, data }) => Some((sixteen_bit, data)),
            Some(o) => return invalid(format!("expected index buffer, got {o:?}")),
        };
        expect_null_tag(r)?;
        let p = r.count()?;
        let mut parts = Vec::with_capacity(p);
        for _ in 0..p {
            let part = MeshPart {
                stream_offset: r.i32()?,
                base_vertex: r.i32()?,
                num_vertices: r.i32()?,
                start_index: r.i32()?,
                primitive_count: r.i32()?,
                vertex_declaration: r.i32()?,
                effect: 0,
            };
            if part.vertex_declaration < 0
                || part.vertex_declaration as usize >= vertex_declarations.len()
            {
                return invalid("mesh part vertex declaration index out of range");
            }
            expect_null_tag(r)?;
            let effect = r.c.var_u32()?;
            parts.push(MeshPart { effect, ..part });
        }
        meshes.push(Mesh {
            name,
            parent_bone,
            bounding_sphere,
            vertex_buffer,
            index_buffer,
            parts,
        });
    }
    let root = bone_ref(r)?;
    expect_null_tag(r)?;
    Ok(Model {
        bones,
        vertex_declarations,
        meshes,
        root,
    })
}
