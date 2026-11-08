use magicka_sim::{spray_pulse::Segment, triangle_intersection::*};
fn tri() -> Triangle {
    Triangle::from_vertices([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0])
}
#[test]
fn crossing_returns_fraction_and_barycentrics() {
    let hit = tri()
        .segment_intersect(
            Segment {
                origin: [0.25, 0.25, 1.0],
                delta: [0.0, 0.0, -2.0],
            },
            true,
        )
        .unwrap();
    assert_eq!(
        hit,
        TriangleHit {
            fraction: 0.5,
            barycentric0: 0.25,
            barycentric1: 0.25
        }
    );
}
#[test]
fn edges_vertices_and_segment_endpoints_are_inclusive() {
    for p in [[0.0, 0.0, 0.0], [0.5, 0.5, 0.0], [1.0, 0.0, 0.0]] {
        assert_eq!(
            tri()
                .segment_intersect(
                    Segment {
                        origin: p,
                        delta: [0.0, 0.0, -1.0]
                    },
                    false
                )
                .unwrap()
                .fraction,
            0.0
        );
        let mut start = p;
        start[2] = 1.0;
        assert_eq!(
            tri()
                .segment_intersect(
                    Segment {
                        origin: start,
                        delta: [0.0, 0.0, -1.0]
                    },
                    false
                )
                .unwrap()
                .fraction,
            1.0
        );
    }
}
#[test]
fn backfaces_parallel_and_outside_are_rejected() {
    let back = Segment {
        origin: [0.25, 0.25, -1.0],
        delta: [0.0, 0.0, 2.0],
    };
    assert!(tri().segment_intersect(back, true).is_none());
    assert!(tri().segment_intersect(back, false).is_some());
    for s in [
        Segment {
            origin: [0.25, 0.25, 1.0],
            delta: [1.0, 0.0, 0.0],
        },
        Segment {
            origin: [0.75, 0.75, 1.0],
            delta: [0.0, 0.0, -2.0],
        },
        Segment {
            origin: [0.25, 0.25, 2.0],
            delta: [0.0, 0.0, -1.0],
        },
    ] {
        assert!(tri().segment_intersect(s, false).is_none());
    }
}
