//! Standard (non-Magicka) content readers: XNA 3.1 framework types,
//! XNAnimation skinned models, and the remaining PolygonHead types.
//!
//! Sources (ILSpy): `Microsoft.Xna.Framework.dll` 3.1 (Texture2D/Cube/3D,
//! Effect, BasicEffect, EffectMaterial, Dictionary, ExternalReference,
//! Texture, Single, Boolean, Matrix, Vector3, List readers); `XNAnimation.dll`
//! (`SkinnedModel`, `SkinnedModelBone`, `AnimationClip`, `AnimationChannel`,
//! `SkinnedModelBasicEffect`); `PolygonHead.dll` (`BitmapFont`, `Glyph`,
//! `SkinnedModelDeferredNormalMappedEffect`).

use crate::common::*;
use crate::xna::{self, Model, XnaObject};

#[derive(Debug, Clone, PartialEq)]
pub enum Asset {
    Texture2D(Texture2D),
    TextureCube {
        format: i32,
        size: i32,
        faces: Vec<Vec<Vec<u8>>>,
    },
    Texture3D {
        format: i32,
        width: i32,
        height: i32,
        depth: i32,
        mips: Vec<Vec<u8>>,
    },
    /// `TextureReader` reads nothing (returns the existing instance).
    TextureBase,
    /// `ExternalReferenceReader`: an asset path.
    External(Option<String>),
    /// `EffectReader`: compiled D3D9 effect bytecode.
    Effect(Vec<u8>),
    BasicEffect(BasicEffect),
    EffectMaterial {
        effect: Option<String>,
        parameters: Vec<(Option<XnaObject>, Option<XnaObject>)>,
    },
    Dictionary(Vec<(Option<XnaObject>, Option<XnaObject>)>),
    StringList(Vec<Option<String>>),
    Single(f32),
    Boolean(bool),
    Matrix([f32; 16]),
    Vector3([f32; 3]),
    SkinnedModel(SkinnedModel),
    SkinnedModelBone(SkinnedModelBone),
    AnimationClip(AnimationClip),
    SkinnedModelBasicEffect(SkinnedModelBasicEffect),
    SkinnedModelDeferredNormalMappedEffect(SkinnedNormalMappedEffect),
    BitmapFont(BitmapFont),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Texture2D {
    /// XNA 3.1 `SurfaceFormat`.
    pub format: i32,
    pub width: i32,
    pub height: i32,
    pub mips: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BasicEffect {
    pub texture: Option<String>,
    pub diffuse: [f32; 3],
    pub emissive: [f32; 3],
    pub specular: [f32; 3],
    pub specular_power: f32,
    pub alpha: f32,
    pub vertex_color_enabled: bool,
}

/// XNAnimation `SkinnedModel.Read`: inner XNA model + shared-resource
/// indices (1-based, 0 = none) of its bones and animation clips.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinnedModel {
    pub model: Option<Model>,
    pub bones: Vec<u32>,
    pub clips: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkinnedModelBone {
    pub index: u16,
    pub name: String,
    pub translation: [f32; 3],
    pub orientation: [f32; 4],
    pub scale: [f32; 3],
    pub inverse_bind_pose: [f32; 16],
    /// Shared-resource index of the parent bone (0 = root).
    pub parent: u32,
    pub children: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Keyframe {
    pub time: f32,
    pub translation: [f32; 3],
    pub orientation: [f32; 4],
    pub scale: [f32; 3],
}

/// XNAnimation `AnimationClip.Read`: name, duration, per-bone channels.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationClip {
    pub name: String,
    pub duration: f32,
    pub channels: Vec<(String, Vec<Keyframe>)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkinnedModelBasicEffect {
    pub technique: u8,
    pub emissive_amount: f32,
    pub diffuse_color: [f32; 3],
    pub specular_amount: f32,
    pub specular_power: f32,
    pub diffuse1_alpha: f32,
    pub use_soft_light_blend: bool,
    /// DiffuseMap0, DiffuseMap1, SpecularMap, DamageMap0, DamageMap1, NormalMap enable flags.
    pub map_enabled: [bool; 6],
    /// The same six maps, embedded inline as `ReadObject<Texture2D>`.
    pub maps: Vec<Option<XnaObject>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkinnedNormalMappedEffect {
    pub diffuse_color: [f32; 3],
    pub specular_amount: f32,
    pub specular_power: f32,
    pub emissive_amount: f32,
    pub normal_power: f32,
    /// Diffuse, Material, Damage, Normal, NormalDamage maps.
    pub maps: [Option<String>; 5],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    pub character: char,
    pub origin: [i32; 2],
    pub size: [i32; 2],
    pub advance_width: i32,
    pub left_side_bearing: i32,
    pub force_white: bool,
}

/// PolygonHead `BitmapFont.Read`.
#[derive(Debug, Clone, PartialEq)]
pub struct BitmapFont {
    pub line_height: i32,
    pub base_line: i32,
    pub default_character: char,
    pub texture: Option<XnaObject>,
    pub glyphs: Vec<Glyph>,
    pub kerning: Vec<(char, char, i32)>,
}

fn bytes(r: &mut Reader) -> Result<Vec<u8>> {
    let n = r.count()?;
    Ok(r.c.bytes(n)?.to_vec())
}

/// .NET `BinaryReader.ReadChar` with UTF-8 encoding (1-3 byte BMP char).
fn read_char(r: &mut Reader) -> Result<char> {
    let b0 = r.u8()? as u32;
    let (len, init) = match b0 {
        0x00..=0x7F => (0, b0),
        0xC0..=0xDF => (1, b0 & 0x1F),
        0xE0..=0xEF => (2, b0 & 0x0F),
        _ => return invalid(format!("unsupported UTF-8 lead byte {b0:#x} in ReadChar")),
    };
    let mut cp = init;
    for _ in 0..len {
        let b = r.u8()? as u32;
        if b & 0xC0 != 0x80 {
            return invalid("bad UTF-8 continuation byte in ReadChar");
        }
        cp = (cp << 6) | (b & 0x3F);
    }
    char::from_u32(cp).map_or_else(|| invalid(format!("invalid char {cp:#x}")), Ok)
}

fn shared_ref(r: &mut Reader) -> Result<u32> {
    Ok(r.c.var_u32()?)
}

fn keyframes(r: &mut Reader) -> Result<Vec<Keyframe>> {
    let n = r.count()?;
    (0..n)
        .map(|_| {
            Ok(Keyframe {
                time: r.f32()?,
                translation: r.vec3()?,
                orientation: xna::quaternion(r)?,
                scale: r.vec3()?,
            })
        })
        .collect()
}

fn dictionary(r: &mut Reader) -> Result<Vec<(Option<XnaObject>, Option<XnaObject>)>> {
    // Only Dictionary<string, object> occurs: both sides are reference types,
    // so each key and value carries its own type id.
    let n = r.count()?;
    (0..n)
        .map(|_| Ok((xna::read_object(r)?, xna::read_object(r)?)))
        .collect()
}

/// Bodies for the readers in this module; `None` if `short` is not one.
pub fn read_body(r: &mut Reader, short: &str) -> Result<Option<XnaObject>> {
    const X: &str = "Microsoft.Xna.Framework.Content.";
    let a = match short.strip_prefix(X).unwrap_or(short) {
        "Texture2DReader" => {
            let format = r.i32()?;
            let width = r.i32()?;
            let height = r.i32()?;
            let n = r.count()?;
            let mips = (0..n).map(|_| bytes(r)).collect::<Result<_>>()?;
            Asset::Texture2D(Texture2D {
                format,
                width,
                height,
                mips,
            })
        }
        "TextureCubeReader" => {
            let format = r.i32()?;
            let size = r.i32()?;
            let mips = r.count()?;
            let faces = (0..6)
                .map(|_| (0..mips).map(|_| bytes(r)).collect::<Result<_>>())
                .collect::<Result<_>>()?;
            Asset::TextureCube {
                format,
                size,
                faces,
            }
        }
        "Texture3DReader" => {
            let format = r.i32()?;
            let (width, height, depth) = (r.i32()?, r.i32()?, r.i32()?);
            let n = r.count()?;
            let mips = (0..n).map(|_| bytes(r)).collect::<Result<_>>()?;
            Asset::Texture3D {
                format,
                width,
                height,
                depth,
                mips,
            }
        }
        "TextureReader" => Asset::TextureBase,
        "ExternalReferenceReader" => Asset::External(r.external()?),
        "EffectReader" => Asset::Effect(bytes(r)?),
        "BasicEffectReader" => Asset::BasicEffect(BasicEffect {
            texture: r.external()?,
            diffuse: r.vec3()?,
            emissive: r.vec3()?,
            specular: r.vec3()?,
            specular_power: r.f32()?,
            alpha: r.f32()?,
            vertex_color_enabled: r.bool()?,
        }),
        "EffectMaterialReader" => {
            let effect = r.external()?;
            let parameters = match xna::read_object(r)? {
                Some(XnaObject::Asset(b)) => match *b {
                    Asset::Dictionary(d) => d,
                    other => {
                        return invalid(format!(
                            "EffectMaterial: expected dictionary, got {other:?}"
                        ))
                    }
                },
                None => Vec::new(),
                Some(o) => {
                    return invalid(format!("EffectMaterial: expected dictionary, got {o:?}"))
                }
            };
            Asset::EffectMaterial { effect, parameters }
        }
        s if s.starts_with("DictionaryReader`2[[System.String") && s.contains("System.Object") => {
            Asset::Dictionary(dictionary(r)?)
        }
        s if s.starts_with("ListReader`1[[System.String") => {
            let n = r.count()?;
            let items = (0..n)
                .map(|_| match xna::read_object(r)? {
                    None => Ok(None),
                    Some(XnaObject::String(s)) => Ok(Some(s)),
                    Some(o) => invalid(format!("expected string, got {o:?}")),
                })
                .collect::<Result<_>>()?;
            Asset::StringList(items)
        }
        "SingleReader" => Asset::Single(r.f32()?),
        "BooleanReader" => Asset::Boolean(r.bool()?),
        "MatrixReader" => Asset::Matrix(xna::matrix(r)?),
        "Vector3Reader" => Asset::Vector3(r.vec3()?),
        "XNAnimation.Pipeline.SkinnedModelReader" => {
            let model = match xna::read_object(r)? {
                None => None,
                Some(XnaObject::Model(m)) => Some(*m),
                Some(o) => return invalid(format!("SkinnedModel: expected Model, got {o:?}")),
            };
            let n = r.count()?;
            let bones = (0..n).map(|_| shared_ref(r)).collect::<Result<_>>()?;
            let n = r.count()?;
            let clips = (0..n).map(|_| shared_ref(r)).collect::<Result<_>>()?;
            Asset::SkinnedModel(SkinnedModel {
                model,
                bones,
                clips,
            })
        }
        "XNAnimation.Pipeline.SkinnedModelBoneReader" => {
            let index = r.u16()?;
            let name = r.string()?;
            let translation = r.vec3()?;
            let orientation = xna::quaternion(r)?;
            let scale = r.vec3()?;
            let inverse_bind_pose = xna::matrix(r)?;
            let parent = shared_ref(r)?;
            let n = r.count()?;
            let children = (0..n).map(|_| shared_ref(r)).collect::<Result<_>>()?;
            Asset::SkinnedModelBone(SkinnedModelBone {
                index,
                name,
                translation,
                orientation,
                scale,
                inverse_bind_pose,
                parent,
                children,
            })
        }
        "XNAnimation.Pipeline.AnimationClipReader" => {
            let name = r.string()?;
            let duration = r.f32()?;
            let n = r.count()?;
            let channels = (0..n)
                .map(|_| Ok((r.string()?, keyframes(r)?)))
                .collect::<Result<_>>()?;
            Asset::AnimationClip(AnimationClip {
                name,
                duration,
                channels,
            })
        }
        "XNAnimation.Pipeline.SkinnedModelBasicEffectReader" => {
            let technique = r.u8()?;
            let emissive_amount = r.f32()?;
            let diffuse_color = r.vec3()?;
            let specular_amount = r.f32()?;
            let specular_power = r.f32()?;
            let diffuse1_alpha = r.f32()?;
            let use_soft_light_blend = r.bool()?;
            let mut map_enabled = [false; 6];
            for e in &mut map_enabled {
                *e = r.bool()?;
            }
            let maps = (0..6).map(|_| xna::read_object(r)).collect::<Result<_>>()?;
            Asset::SkinnedModelBasicEffect(SkinnedModelBasicEffect {
                technique,
                emissive_amount,
                diffuse_color,
                specular_amount,
                specular_power,
                diffuse1_alpha,
                use_soft_light_blend,
                map_enabled,
                maps,
            })
        }
        "PolygonHead.Pipeline.SkinnedModelDeferredNormalMappedEffectReader" => {
            Asset::SkinnedModelDeferredNormalMappedEffect(SkinnedNormalMappedEffect {
                diffuse_color: r.vec3()?,
                specular_amount: r.f32()?,
                specular_power: r.f32()?,
                emissive_amount: r.f32()?,
                normal_power: r.f32()?,
                maps: [
                    r.external()?,
                    r.external()?,
                    r.external()?,
                    r.external()?,
                    r.external()?,
                ],
            })
        }
        "PolygonHead.Pipeline.BitmapFontReader" => {
            let line_height = r.i32()?;
            let base_line = r.i32()?;
            let default_character = read_char(r)?;
            let texture = xna::read_object(r)?;
            let n = r.count()?;
            let glyphs = (0..n)
                .map(|_| {
                    Ok(Glyph {
                        character: read_char(r)?,
                        origin: [r.i32()?, r.i32()?],
                        size: [r.i32()?, r.i32()?],
                        advance_width: r.i32()?,
                        left_side_bearing: r.i32()?,
                        force_white: r.bool()?,
                    })
                })
                .collect::<Result<_>>()?;
            let n = r.count()?;
            let kerning = (0..n)
                .map(|_| Ok((read_char(r)?, read_char(r)?, r.i32()?)))
                .collect::<Result<_>>()?;
            Asset::BitmapFont(BitmapFont {
                line_height,
                base_line,
                default_character,
                texture,
                glyphs,
                kerning,
            })
        }
        _ => return Ok(None),
    };
    Ok(Some(XnaObject::Asset(Box::new(a))))
}
