//! ForceField constructor's render-buffer to collision-mesh conversion.
use crate::level_collision::{collision_mesh, TransformedCollisionMesh};
use magicka_content::level::{ForceField, TriMesh};
use magicka_sim::mesh_octree::{MeshError, MeshTransform};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForceFieldError {
    MissingBuffer,
    MissingPosition,
    UnsupportedFormat(u8),
    InvalidLayout,
    IndexCount,
    Mesh(MeshError),
}
pub fn force_field_collision(
    source: &ForceField,
) -> Result<TransformedCollisionMesh, ForceFieldError> {
    let declaration = source
        .vertex_declaration
        .as_ref()
        .ok_or(ForceFieldError::MissingBuffer)?;
    let position = declaration
        .iter()
        .find(|e| e.usage == 0 && e.usage_index == 0)
        .ok_or(ForceFieldError::MissingPosition)?;
    if position.format != 2 {
        return Err(ForceFieldError::UnsupportedFormat(position.format));
    }
    let offset = usize::try_from(position.offset).map_err(|_| ForceFieldError::InvalidLayout)?;
    let stride =
        usize::try_from(source.vertex_stride).map_err(|_| ForceFieldError::InvalidLayout)?;
    let count = usize::try_from(source.num_vertices).map_err(|_| ForceFieldError::InvalidLayout)?;
    let primitives =
        usize::try_from(source.primitive_count).map_err(|_| ForceFieldError::InvalidLayout)?;
    let buffer = source
        .vertex_buffer
        .as_ref()
        .ok_or(ForceFieldError::MissingBuffer)?;
    let (sixteen, indices) = source
        .index_buffer
        .as_ref()
        .ok_or(ForceFieldError::MissingBuffer)?;
    if stride < 12 {
        return Err(ForceFieldError::InvalidLayout);
    }
    let mut vertices = Vec::with_capacity(count.min(buffer.len() / 12));
    for i in 0..count {
        let start = i
            .checked_mul(stride)
            .and_then(|x| x.checked_add(offset))
            .ok_or(ForceFieldError::InvalidLayout)?;
        let end = start
            .checked_add(12)
            .ok_or(ForceFieldError::InvalidLayout)?;
        let bytes = buffer
            .get(start..end)
            .ok_or(ForceFieldError::InvalidLayout)?;
        vertices.push(std::array::from_fn(|j| {
            f32::from_le_bytes(bytes[j * 4..j * 4 + 4].try_into().unwrap())
        }));
    }
    let width = if *sixteen { 2 } else { 4 };
    let expected = primitives
        .checked_mul(3)
        .and_then(|n| n.checked_mul(width))
        .ok_or(ForceFieldError::IndexCount)?;
    // The original reads every index into a primitive-count array; malformed
    // mismatches can throw or leave default triangles. Report rather than pad.
    if indices.len() != expected {
        return Err(ForceFieldError::IndexCount);
    }
    let mut triangles = Vec::with_capacity(primitives);
    for triple in indices.chunks_exact(width * 3) {
        let read = |i: usize| {
            if *sixteen {
                i32::from(u16::from_le_bytes(
                    triple[i * 2..i * 2 + 2].try_into().unwrap(),
                ))
            } else {
                i32::from_le_bytes(triple[i * 4..i * 4 + 4].try_into().unwrap())
            }
        };
        triangles.push([read(0), read(2), read(1)]);
    }
    let mesh = collision_mesh(&TriMesh {
        vertices,
        triangles,
    })
    .map_err(ForceFieldError::Mesh)?;
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    Ok(TransformedCollisionMesh {
        mesh,
        transform: MeshTransform {
            forward: identity,
            inverse: identity,
        },
    })
}
