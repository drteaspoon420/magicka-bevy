use magicka_content::{level::ForceField, xna::VertexElement};
use magicka_runtime::force_field::*;
use magicka_sim::spray_pulse::Segment;
fn source(sixteen: bool) -> ForceField {
    let mut bytes = vec![];
    for point in [[0.0f32; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        bytes.extend_from_slice(&[0xff; 4]);
        for v in point {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    let indices = if sixteen {
        [0u16, 1, 2]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect()
    } else {
        [0i32, 1, 2]
            .into_iter()
            .flat_map(i32::to_le_bytes)
            .collect()
    };
    ForceField {
        color: [0.0; 3],
        width: 0.0,
        alpha_power: 0.0,
        alpha_falloff_power: 0.0,
        max_radius: 0.0,
        ripple_distortion: 0.0,
        map_distortion: 0.0,
        vertex_color_enabled: false,
        displacement_map: None,
        ttl: 0.0,
        vertex_buffer: Some(bytes),
        index_buffer: Some((sixteen, indices)),
        vertex_declaration: Some(vec![VertexElement {
            stream: 0,
            offset: 4,
            format: 2,
            method: 0,
            usage: 0,
            usage_index: 0,
        }]),
        vertex_stride: 16,
        num_vertices: 3,
        primitive_count: 1,
    }
}
#[test]
fn both_index_widths_use_position_offset_and_reverse_winding() {
    for width in [true, false] {
        let collision = force_field_collision(&source(width)).unwrap();
        let front = Segment {
            origin: [0.25, 0.25, -1.0],
            delta: [0.0, 0.0, 2.0],
        };
        let back = Segment {
            origin: [0.25, 0.25, 1.0],
            delta: [0.0, 0.0, -2.0],
        };
        let hit = collision.mesh.segment_intersect(front, true).unwrap();
        assert_eq!(hit.hit.fraction, 0.5);
        assert_eq!(hit.normal, [0.0, 0.0, -1.0]);
        assert!(collision.mesh.segment_intersect(back, true).is_none());
    }
}
#[test]
fn malformed_declarations_buffers_and_negative_indices_report_errors() {
    let mut s = source(true);
    s.vertex_declaration.as_mut().unwrap()[0].format = 1;
    assert!(matches!(
        force_field_collision(&s),
        Err(ForceFieldError::UnsupportedFormat(1))
    ));
    let mut s = source(true);
    s.vertex_buffer.as_mut().unwrap().pop();
    assert!(matches!(
        force_field_collision(&s),
        Err(ForceFieldError::InvalidLayout)
    ));
    let mut s = source(true);
    s.primitive_count = 2;
    assert!(matches!(
        force_field_collision(&s),
        Err(ForceFieldError::IndexCount)
    ));
    let mut s = source(false);
    s.index_buffer.as_mut().unwrap().1[..4].copy_from_slice(&(-1i32).to_le_bytes());
    assert!(matches!(
        force_field_collision(&s),
        Err(ForceFieldError::Mesh(_))
    ));
}
