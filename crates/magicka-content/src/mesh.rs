//! Vertex/index buffer decoding using XNA 3.1 vertex declarations.
//!
//! Enum values from the decompiled XNA 3.1 `VertexElementFormat`
//! (Vector2 = 1, Vector3 = 2, ...) and `VertexElementUsage` (Position = 0,
//! Normal = 3, TextureCoordinate = 5, ...).

use crate::xna::VertexElement;

pub const FMT_SINGLE: u8 = 0;
pub const FMT_VECTOR2: u8 = 1;
pub const FMT_VECTOR3: u8 = 2;
pub const USAGE_POSITION: u8 = 0;
pub const USAGE_NORMAL: u8 = 3;
pub const USAGE_TEXCOORD: u8 = 5;
pub const USAGE_TANGENT: u8 = 6;
pub const USAGE_BINORMAL: u8 = 7;

fn find(decl: &[VertexElement], usage: u8, format: u8) -> Option<usize> {
    decl.iter()
        .find(|e| e.stream == 0 && e.usage == usage && e.usage_index == 0 && e.format == format)
        .map(|e| e.offset as usize)
}

fn floats<const N: usize>(
    vb: &[u8],
    stride: usize,
    offset: usize,
    count: usize,
) -> Option<Vec<[f32; N]>> {
    (0..count)
        .map(|i| {
            let base = i * stride + offset;
            let s = vb.get(base..base + 4 * N)?;
            let mut v = [0f32; N];
            for (k, x) in v.iter_mut().enumerate() {
                *x = f32::from_le_bytes([s[4 * k], s[4 * k + 1], s[4 * k + 2], s[4 * k + 3]]);
            }
            Some(v)
        })
        .collect()
}

/// Vertex count implied by a buffer and stride.
pub fn vertex_count(vb: &[u8], stride: usize) -> usize {
    vb.len().checked_div(stride).unwrap_or(0)
}

/// Position (usage 0, Vector3) of every vertex.
pub fn positions(decl: &[VertexElement], vb: &[u8], stride: usize) -> Option<Vec<[f32; 3]>> {
    floats::<3>(
        vb,
        stride,
        find(decl, USAGE_POSITION, FMT_VECTOR3)?,
        vertex_count(vb, stride),
    )
}

/// Normal (usage 3, Vector3), if present.
pub fn normals(decl: &[VertexElement], vb: &[u8], stride: usize) -> Option<Vec<[f32; 3]>> {
    floats::<3>(
        vb,
        stride,
        find(decl, USAGE_NORMAL, FMT_VECTOR3)?,
        vertex_count(vb, stride),
    )
}

/// First texture coordinate set (usage 5 index 0, Vector2), if present.
pub fn texcoords(decl: &[VertexElement], vb: &[u8], stride: usize) -> Option<Vec<[f32; 2]>> {
    floats::<2>(
        vb,
        stride,
        find(decl, USAGE_TEXCOORD, FMT_VECTOR2)?,
        vertex_count(vb, stride),
    )
}

pub const FMT_VECTOR4: u8 = 3;
pub const FMT_BYTE4: u8 = 5;
pub const USAGE_BLEND_WEIGHT: u8 = 1;
pub const USAGE_BLEND_INDICES: u8 = 2;

/// Blend indices (usage 2, Byte4).
pub fn blend_indices(decl: &[VertexElement], vb: &[u8], stride: usize) -> Option<Vec<[u8; 4]>> {
    let off = find(decl, USAGE_BLEND_INDICES, FMT_BYTE4)?;
    (0..vertex_count(vb, stride))
        .map(|i| {
            let b = vb.get(i * stride + off..i * stride + off + 4)?;
            Some([b[0], b[1], b[2], b[3]])
        })
        .collect()
}

/// Blend weights (usage 1, Vector4).
pub fn blend_weights(decl: &[VertexElement], vb: &[u8], stride: usize) -> Option<Vec<[f32; 4]>> {
    floats::<4>(
        vb,
        stride,
        find(decl, USAGE_BLEND_WEIGHT, FMT_VECTOR4)?,
        vertex_count(vb, stride),
    )
}

pub const USAGE_COLOR: u8 = 10;

/// Vertex color (usage 10 index 0, Vector4 floats; Magicka level geometry).
/// XNA VertexElementFormat.Color: packed RGBA bytes (Color.PackedValue =
/// R | G << 8 | B << 16 | A << 24, little-endian), normalized to 0..1.
pub const FMT_COLOR: u8 = 4;

/// COLOR0 as Vector4 floats or as a packed XNA Color.
pub fn colors(decl: &[VertexElement], vb: &[u8], stride: usize) -> Option<Vec<[f32; 4]>> {
    let count = vertex_count(vb, stride);
    if let Some(offset) = find(decl, USAGE_COLOR, FMT_VECTOR4) {
        return floats::<4>(vb, stride, offset, count);
    }
    let offset = find(decl, USAGE_COLOR, FMT_COLOR)?;
    (0..count)
        .map(|i| {
            let b = vb.get(i * stride + offset..i * stride + offset + 4)?;
            Some(std::array::from_fn(|c| f32::from(b[c]) / 255.0))
        })
        .collect()
}

/// Per-vertex tangents and binormals.
pub type TangentFrames = (Vec<[f32; 3]>, Vec<[f32; 3]>);

/// Tangent and binormal (usage 6/7, Vector3), as liquid vertices carry them.
pub fn tangent_frames(decl: &[VertexElement], vb: &[u8], stride: usize) -> Option<TangentFrames> {
    let n = vertex_count(vb, stride);
    Some((
        floats::<3>(vb, stride, find(decl, USAGE_TANGENT, FMT_VECTOR3)?, n)?,
        floats::<3>(vb, stride, find(decl, USAGE_BINORMAL, FMT_VECTOR3)?, n)?,
    ))
}

/// Single-float COLOR0 (liquid vertices: the deep-bottom blend weight).
pub fn color_single(decl: &[VertexElement], vb: &[u8], stride: usize) -> Option<Vec<f32>> {
    floats::<1>(
        vb,
        stride,
        find(decl, USAGE_COLOR, FMT_SINGLE)?,
        vertex_count(vb, stride),
    )
    .map(|v| v.into_iter().map(|[x]| x).collect())
}

/// Second texture coordinate set (usage 5 index 1, Vector2).
pub fn texcoords1(decl: &[VertexElement], vb: &[u8], stride: usize) -> Option<Vec<[f32; 2]>> {
    let off = decl
        .iter()
        .find(|e| {
            e.stream == 0
                && e.usage == USAGE_TEXCOORD
                && e.usage_index == 1
                && e.format == FMT_VECTOR2
        })
        .map(|e| e.offset as usize)?;
    floats::<2>(vb, stride, off, vertex_count(vb, stride))
}

/// Decode an index buffer (16- or 32-bit) to u32.
pub fn indices(sixteen_bit: bool, data: &[u8]) -> Vec<u32> {
    if sixteen_bit {
        data.as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c) as u32)
            .collect()
    } else {
        data.as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c))
            .collect()
    }
}

/// XNA `ContentReader.GetPathToReference`: directory of the referencing asset
/// joined with the reference, then normalized (`\` -> `/`, `.`/`..` resolved).
pub fn resolve_reference(asset: &str, reference: &str) -> String {
    let asset = asset.replace('\\', "/");
    let dir = asset.rfind('/').map_or("", |i| &asset[..i]);
    let joined = format!("{dir}/{}", reference.replace('\\', "/"));
    let mut parts: Vec<&str> = Vec::new();
    for p in joined.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The R'lyeh beam globes declare COLOR0 as XNA VertexElementFormat
    /// Color (4): packed R, G, B, A bytes, not a Vector4.
    #[test]
    fn packed_xna_color_vertices_decode_as_normalized_rgba() {
        use crate::xna::VertexElement;
        let decl = [
            VertexElement {
                stream: 0,
                offset: 0,
                format: FMT_VECTOR3,
                method: 0,
                usage: USAGE_POSITION,
                usage_index: 0,
            },
            VertexElement {
                stream: 0,
                offset: 12,
                format: FMT_COLOR,
                method: 0,
                usage: USAGE_COLOR,
                usage_index: 0,
            },
        ];
        let mut vb = Vec::new();
        for (p, c) in [
            ([0f32; 3], [255u8, 0, 51, 255]),
            ([1.0, 2.0, 3.0], [0, 128, 255, 0]),
        ] {
            for v in p {
                vb.extend_from_slice(&v.to_le_bytes());
            }
            vb.extend_from_slice(&c);
        }
        let colors = colors(&decl, &vb, 16).unwrap();
        assert_eq!(colors[0], [1.0, 0.0, 0.2, 1.0]);
        assert_eq!(colors[1], [0.0, 128.0 / 255.0, 1.0, 0.0]);
    }

    #[test]
    fn reference_resolution_matches_xna() {
        assert_eq!(
            resolve_reference(
                "Levels/BattleField/bf_s1",
                "..\\Textures\\Surface\\snow00_0"
            ),
            "Levels/Textures/Surface/snow00_0"
        );
        assert_eq!(
            resolve_reference("Data/Items/staff", "staff_0"),
            "Data/Items/staff_0"
        );
    }

    #[test]
    fn decodes_interleaved_positions() {
        let decl = [VertexElement {
            stream: 0,
            offset: 4,
            format: FMT_VECTOR3,
            method: 0,
            usage: USAGE_POSITION,
            usage_index: 0,
        }];
        let mut vb = vec![0u8; 16];
        vb[4..8].copy_from_slice(&1.5f32.to_le_bytes());
        vb[12..16].copy_from_slice(&(-2.0f32).to_le_bytes());
        assert_eq!(positions(&decl, &vb, 16), Some(vec![[1.5, 0.0, -2.0]]));
        assert_eq!(indices(true, &[1, 0, 2, 0]), vec![1, 2]);
    }
}
