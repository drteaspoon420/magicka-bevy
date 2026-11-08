use magicka_sim::{collision_skin::*, spray_pulse::Segment};
#[test]
fn reverse_order_shortening_and_fraction_accumulation_are_literal() {
    let mut order = vec![];
    let hit = segment_intersect(
        3,
        Segment {
            origin: [0.0; 3],
            delta: [0.0, 0.0, 8.0],
        },
        true,
        |i, s, b| {
            assert!(b);
            order.push((i, s.delta[2]));
            Some(PrimitiveHit {
                fraction: 0.5,
                position: [0.0, 0.0, s.delta[2] * 0.5],
                normal: [0.0, 0.0, -1.0],
            })
        },
    )
    .unwrap();
    assert_eq!(order, [(2, 8.0), (1, 4.0), (0, 2.0)]);
    assert_eq!(hit.primitive, 0);
    assert_eq!(hit.hit.fraction, 0.0625);
    assert_eq!(hit.hit.position, [0.0, 0.0, 1.0]);
}
#[test]
fn misses_leave_segment_unchanged_and_endpoint_is_valid() {
    let mut calls = 0;
    let hit = segment_intersect(
        2,
        Segment {
            origin: [0.0; 3],
            delta: [1.0, 0.0, 0.0],
        },
        false,
        |i, s, _| {
            calls += 1;
            assert_eq!(s.delta, [1.0, 0.0, 0.0]);
            if i == 1 {
                None
            } else {
                Some(PrimitiveHit {
                    fraction: 1.0,
                    position: [1.0, 0.0, 0.0],
                    normal: [0.0; 3],
                })
            }
        },
    )
    .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(hit.hit.fraction, 1.0);
    assert!(segment_intersect(
        0,
        Segment {
            origin: [0.0; 3],
            delta: [0.0; 3]
        },
        false,
        |_, _, _| panic!()
    )
    .is_none());
}
