use magicka_sim::{
    damage::{CastType, Damage},
    impact_motion::Vector,
    spells::Elements as E,
    spray_pulse::*,
};
#[derive(Default)]
struct World {
    segments: Vec<Segment>,
    arcs: Vec<(u16, f32)>,
    order: Vec<u16>,
    damaged: Vec<[Damage; 5]>,
    origin: Vector,
}
impl SprayWorld for World {
    fn scene_intersection(&mut self, s: Segment) -> Option<f32> {
        self.segments.push(s);
        Some(0.5)
    }
    fn shields(&mut self) -> Vec<u16> {
        vec![10, 11]
    }
    fn shield_intersection(&mut self, _: u16, s: Segment) -> Option<f32> {
        self.segments.push(s);
        Some(0.5)
    }
    fn targets(&mut self, c: Vector, r: f32) -> Vec<SprayTarget> {
        assert_eq!(c, [1.0; 3]);
        assert_eq!(r, 8.0);
        vec![
            SprayTarget {
                handle: 1,
                shield: false,
                damageable: true,
            },
            SprayTarget {
                handle: 2,
                shield: false,
                damageable: true,
            },
            SprayTarget {
                handle: 3,
                shield: true,
                damageable: true,
            },
        ]
    }
    fn arc_intersects(&mut self, t: u16, o: Vector, _: Vector, r: f32, _: f32, h: f32) -> bool {
        self.origin = o;
        assert_eq!(h, 5.0);
        self.arcs.push((t, r));
        true
    }
    fn damage(&mut self, t: u16, a: u16, d: [Damage; 5], time: f64, p: Vector) {
        assert_eq!(a, 1);
        assert_eq!(time, 4.0);
        assert_eq!(p, self.origin);
        self.order.push(t);
        self.damaged.push(d);
    }
    fn electrocute(&mut self, t: u16, a: u16, m: f32) {
        assert_eq!(a, 1);
        assert_eq!(m, 0.25);
        self.order.push(t + 100);
    }
}
fn source() -> SpraySource {
    SpraySource {
        owner_handle: 1,
        owner_position: [1.0; 3],
        owner_direction: [0.0, 0.0, 1.0],
        cast_position: [2.0; 3],
        cast_direction: [0.0, 1.0, 0.0],
        weapon_position: [3.0; 3],
        weapon_direction: [1.0, 0.0, 0.0],
    }
}
fn pulse() -> SprayPulse {
    let mut d = [Damage::default(); 5];
    d[0] = Damage {
        attack: 32,
        element: E::WATER.0,
        amount: 10.0,
        magnitude: 2.0,
    };
    SprayPulse {
        damages: d,
        timestamp: 4.0,
        angle: 0.3,
        hits: HitList::default(),
    }
}
#[test]
fn sequential_obstruction_shield_exception_and_damage_order() {
    let mut p = pulse();
    let mut w = World::default();
    p.execute(8.0, CastType::Force, true, source(), &mut w);
    assert_eq!(
        w.segments.iter().map(|s| s.delta).collect::<Vec<_>>(),
        [[0.0, 8.0, 0.0], [0.0, 4.0, 0.0], [0.0, 2.0, 0.0]]
    );
    assert_eq!(w.arcs, [(2, 0.8), (3, 8.0)]);
    assert_eq!(w.order, [101, 2, 102, 3, 103]);
    assert_eq!(w.damaged[0][0].magnitude, 0.5);
    assert_eq!(w.damaged[0][0].amount, 10.0);
    assert_eq!(w.damaged[0][4], Damage::default());
    assert_eq!(w.origin, [2.0; 3]);
    assert_eq!(p.hits.remaining(2), Some(0.25));
    w.arcs.clear();
    p.execute(8.0, CastType::Force, true, source(), &mut w);
    assert!(w.arcs.is_empty());
    p.hits.update(0.25);
    p.execute(8.0, CastType::Force, true, source(), &mut w);
    assert_eq!(w.arcs.len(), 2);
}
#[test]
fn source_selection_and_steam_exclusion() {
    for (ct, staff, origin, delta) in [
        (CastType::Weapon, true, [3.0; 3], [8.0, 0.0, 0.0]),
        (CastType::Force, false, [2.0; 3], [0.0, 0.0, 8.0]),
    ] {
        let mut p = pulse();
        p.damages[1].element = E::STEAM.0;
        let mut w = World::default();
        p.execute(8.0, ct, staff, source(), &mut w);
        assert_eq!(w.segments[0], Segment { origin, delta });
        assert_eq!(w.order, [2, 3]);
    }
}
#[test]
fn cooldown_refresh_boundary_and_nan_retention() {
    let mut h = HitList::default();
    h.add(5);
    h.update(0.125);
    assert_eq!(h.remaining(5), Some(0.125));
    h.add(5);
    assert_eq!(h.remaining(5), Some(0.25));
    h.add(1);
    h.update(0.25);
    assert!(!h.contains(1));
    assert!(!h.contains(5));
    h.add(8);
    h.update(f32::NAN);
    assert!(h.remaining(8).unwrap().is_nan());
    h.update(100.0);
    assert!(h.contains(8));
    h.clear();
    assert!(!h.contains(8));
}
