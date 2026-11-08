use magicka_content::assets::{BitmapFont, Glyph};
use magicka_runtime::{text_box::TypingState, text_geometry::*};
fn font() -> GeometryFont {
    let content = BitmapFont {
        line_height: 10,
        base_line: 8,
        default_character: '?',
        texture: None,
        glyphs: [
            ('A', 5, -1, false),
            ('B', 7, 0, true),
            ('[', 3, 0, false),
            ('?', 4, 0, false),
        ]
        .into_iter()
        .map(
            |(character, advance_width, left_side_bearing, force_white)| Glyph {
                character,
                advance_width,
                left_side_bearing,
                force_white,
                origin: [10, 20],
                size: [4, 6],
            },
        )
        .collect(),
        kerning: vec![('A', 'B', -2), ('A', 'A', -1)],
    };
    let mut font = GeometryFont::with_atlas_size(&content, [100, 200]).unwrap();
    font.metrics.spacing = 1;
    font
}
fn build(text: &str, style: TextStyle) -> TextGeometry {
    TextGeometry::build(&font(), &text.encode_utf16().collect::<Vec<_>>(), style).unwrap()
}
#[test]
fn positions_atlas_uvs_spacing_kerning_and_index_order() {
    let g = build("AB", TextStyle::default());
    assert_eq!(
        g.quads[0].vertices.map(|v| v.position),
        [[0., 0.], [4., 0.], [4., 6.], [0., 6.]]
    );
    assert_eq!(g.quads[1].vertices[0].position, [5., 0.]);
    let uv = g.quads[0].vertices.map(|v| v.uv);
    assert!((uv[0][0] - 0.1).abs() < 1e-6);
    assert!((uv[0][1] - 0.1).abs() < 1e-6);
    assert!((uv[2][0] - 0.14).abs() < 1e-6);
    assert!((uv[2][1] - 0.13).abs() < 1e-6);
    assert_eq!(g.indices(), [0, 1, 2, 2, 3, 0, 4, 5, 6, 6, 7, 4]);
    assert_eq!(g.primitive_count(), 4);
}
#[test]
fn colors_restore_default_force_white_alpha_and_permissive_matching() {
    let default = [0.2, 0.3, 0.4, 0.5];
    let style = TextStyle {
        default_color: default,
        ..TextStyle::default()
    };
    let g = build("[c=-.5,2,+.25,.1]AB[/c]A", style);
    assert_eq!(g.quads[0].vertices[0].color, [-0.5, 2.0, 0.25, 0.1]);
    assert_eq!(g.quads[1].vertices[0].color, [1.; 4]);
    assert_eq!(g.quads[2].vertices[0].color, default);
    let g = build("[c=1,0,0]A[c=0,1,0]A[/c]A", style);
    assert_eq!(g.quads[2].vertices[0].color, default);
    let g = build("[cxxx=1,0,0]A[/][c]A", style);
    assert_eq!(g.quads[0].vertices[0].color, [1., 0., 0., 1.]);
    assert_eq!(g.quads[1].vertices[0].color, default); // Restore matcher persists across tags.
    assert_eq!(
        build("[C=1,0,0]A", style).quads[0].vertices[0].color,
        default
    );
}
#[test]
fn newline_geometry_alignment_and_formatting_disabled() {
    let g = build("A\nA", TextStyle::default());
    assert_eq!(g.quads.len(), 2);
    assert_eq!(g.quads[1].vertices[0].position, [-1., 10.]);
    let centered = build(
        "A\nB",
        TextStyle {
            align: TextAlign::Center,
            ..TextStyle::default()
        },
    );
    assert_eq!(centered.quads[0].vertices[0].position, [-3., 0.]);
    assert_eq!(centered.quads[1].vertices[0].position, [-4., 10.]); // Prior A->B kerning survives newline.
    let right = build(
        "A",
        TextStyle {
            align: TextAlign::Right,
            ..TextStyle::default()
        },
    );
    assert_eq!(right.quads[0].vertices[0].position, [-6., 0.]);
    assert_eq!(build("[[A", TextStyle::default()).quads.len(), 2);
    assert_eq!(
        build(
            "[A]",
            TextStyle {
                formatting: false,
                ..TextStyle::default()
            }
        )
        .quads
        .len(),
        3
    );
    assert_eq!(build("A\0B", TextStyle::default()).quads.len(), 1);
    assert_eq!(build("😀", TextStyle::default()).quads.len(), 2);
}
#[test]
fn float_parser_preserves_delimiters_and_native_nonvalidation() {
    let mut i = 0;
    let chars: Vec<_> = "-12.25,tail".encode_utf16().collect();
    assert_eq!(parse_float(&chars, &mut i).unwrap(), -12.25);
    assert_eq!(chars[i], 44);
    let mut i = 0;
    let chars: Vec<_> = "1e3".encode_utf16().collect();
    assert_eq!(parse_float(&chars, &mut i).unwrap(), 1.);
    assert_eq!(i, 1);
    let mut i = 0;
    let chars: Vec<_> = " ,".encode_utf16().collect();
    assert_eq!(parse_float(&chars, &mut i).unwrap(), 0.);
    assert_eq!(i, 0);
    let mut i = 0;
    assert_eq!(parse_float(&[], &mut i), Err(GeometryError::BufferIndex(0)));
}
#[test]
fn draw_publication_updates_actual_typing_count_and_failure_keeps_it() {
    let mut typing = TypingState::default();
    typing.set_text(Some("[c=1,0,0]A\n[[B"));
    assert!(typing.finished()); // Count stays zero until geometry publication.
    let g = TextGeometry::rebuild_typing(&font(), &mut typing, TextStyle::default()).unwrap();
    assert_eq!(g.primitive_count(), 6);
    assert!(!typing.finished());
    assert!(!typing.dirty);
    typing.update(1.).unwrap();
    assert!(typing.finished());
    // An exact-capacity malformed buffer reaches the native array boundary;
    // shorter input can instead read retained trailing buffer contents in a tag.
    let mut typing = TypingState::new(4, 40.0);
    typing.publish_primitives(6);
    typing.set_text(Some("[c=1"));
    assert!(TextGeometry::rebuild_typing(&font(), &mut typing, TextStyle::default()).is_err());
    assert_eq!(typing.primitive_count, 6);
    assert!(typing.dirty);
}
#[test]
fn native_ushort_indices_wrap_after_16384_glyphs() {
    let quad = build("A", TextStyle::default()).quads.remove(0);
    let geometry = TextGeometry {
        quads: vec![quad; 16385],
    };
    let indices = geometry.indices();
    assert_eq!(&indices[16384 * 6..], &[0, 1, 2, 2, 3, 0]);
}
