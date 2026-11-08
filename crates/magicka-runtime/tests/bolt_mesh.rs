//! LightningBolt geometry (research/lightning.md).
use magicka_runtime::bolt_mesh::{draw_count, strip, triangles};
use magicka_runtime::particles::DotNetRandom;

#[test]
fn strip_matches_the_constructor_layout() {
    let s = strip(&mut DotNetRandom::new(1));
    // First pair has no jitter: x = +0.5 / −0.5 at z = 0.
    assert_eq!(s[0].position, [0.5, 0.0, 0.0]);
    assert_eq!(s[1].position, [-0.5, 0.0, 0.0]);
    assert_eq!(s[33].position[2], -32.0);
    for p in s.chunks(2) {
        // Pairs share a jitter: one unit apart.
        assert!((p[0].position[0] - p[1].position[0] - 1.0).abs() < 1e-6);
        assert!(p[0].position[0].abs() <= 1.25 + 1e-6);
    }
    assert_eq!((s[0].uv[1], s[1].uv[1]), (0.375, 0.5));
}

#[test]
fn bolt_spans_source_to_target() {
    let s = strip(&mut DotNetRandom::new(2));
    let (from, to) = ([1.0, 0.5, 2.0], [1.0, 0.5, 9.0]);
    assert_eq!(draw_count(from, to, 1.0), 8);
    assert_eq!(draw_count(from, [0.0, 0.0, 200.0], 1.0), 32);
    let tri = triangles(&s, from, to, [1.0, 20.0, 2.0], 1.0);
    assert_eq!(tri.len(), 8 * 6);
    let far = tri.iter().map(|v| v.position[2]).fold(f32::MIN, f32::max);
    assert!((far - 9.0).abs() < 1e-4, "{far}");
    // Seen from above: the ribbon lies in the horizontal plane.
    assert!(tri.iter().all(|v| (v.position[1] - 0.5).abs() < 1e-5));
}
