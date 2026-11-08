use magicka_sim::{
    damage::{CastType, Damage},
    impact_motion::Vector,
    push_spell::{PushSpell, PushTarget, PushWorld},
    spray_pulse::Segment,
};
#[derive(Default)]
struct World {
    targets: Vec<PushTarget>,
    ranges: Vec<f32>,
    arcs: Vec<(u16, Vector, f32)>,
    blocked: Vec<u16>,
    deliveries: Vec<(u16, Damage, f64, Vector)>,
    impulses: Vec<(u16, Vector)>,
}
impl PushWorld for World {
    fn push_targets(&mut self, _: Vector, range: f32) -> Vec<PushTarget> {
        self.ranges.push(range);
        self.targets.clone()
    }
    fn push_arc(
        &mut self,
        target: u16,
        _: Vector,
        direction: Vector,
        _: f32,
        angle: f32,
        _: f32,
    ) -> bool {
        self.arcs.push((target, direction, angle));
        target != 3
    }
    fn push_obstructed(&mut self, s: Segment) -> bool {
        self.blocked.contains(&(s.delta[0] as u16))
    }
    fn push_damage(&mut self, target: u16, _: u16, d: Damage, t: f64, p: Vector) {
        self.deliveries.push((target, d, t, p))
    }
    fn push_impulse(&mut self, target: u16, d: Vector, _: f32, _: f32, _: f32) {
        self.impulses.push((target, d))
    }
}
fn target(handle: u16) -> PushTarget {
    PushTarget {
        handle,
        position: [handle as f32, 0., 0.],
        dead: false,
        damageable: true,
        missile: false,
    }
}
fn spell(kind: CastType) -> PushSpell {
    PushSpell::cast(kind, false, 42., &mut 0., 0., [0., 0., -1.]).unwrap()
}
#[test]
fn pulse_expands_once_per_target_and_expires_without_base_update() {
    let mut s = spell(CastType::Force);
    let mut w = World {
        targets: vec![target(1), target(2)],
        ..Default::default()
    };
    s.update(0.05, 1, [0.; 3], &mut w);
    s.update(0.05, 1, [1., 0., 0.], &mut w);
    assert!((w.ranges[0] - 4.).abs() < 1e-5);
    assert!((w.ranges[1] - 5.5).abs() < 1e-5);
    assert_eq!(w.deliveries.len(), 1);
    assert_eq!(w.deliveries[0].0, 2);
    assert_eq!(w.deliveries[0].2, 42.);
    assert_eq!(w.deliveries[0].1.amount, 50.);
    assert_eq!(w.deliveries[0].1.magnitude, 1.5);
    assert_eq!(w.arcs[0].1, [0., 0., -1.]);
    for _ in 0..8 {
        s.update(0.05, 1, [0.; 3], &mut w);
    }
    assert!(!s.lifecycle.active);
    assert!(s.hits.is_empty());
}
#[test]
fn area_skips_arc_but_respects_dead_obstruction_and_zero_length() {
    let mut s = spell(CastType::Area);
    let mut dead = target(4);
    dead.dead = true;
    let mut w = World {
        targets: vec![target(0), target(1), target(2), target(3), dead],
        blocked: vec![2],
        ..Default::default()
    };
    s.update(0.1, 1, [0.; 3], &mut w);
    assert!(w.arcs.is_empty());
    assert_eq!(w.deliveries.len(), 1);
    assert_eq!(w.deliveries[0].0, 3);
    assert!(!s.hits.contains(&0));
    assert!(!s.hits.contains(&2));
}
#[test]
fn missile_and_non_damageable_use_impulse_with_full_length_then_zero_y() {
    let mut s = spell(CastType::Area);
    let mut t = target(2);
    t.missile = true;
    t.position = [3., 4., 0.];
    let mut w = World {
        targets: vec![t],
        ..Default::default()
    };
    s.update(0.1, 1, [0.; 3], &mut w);
    assert!(w.deliveries.is_empty());
    assert_eq!(w.impulses, vec![(2, [0.6, 0., 0.])]);
}
#[test]
fn charge_is_consumed_and_long_frame_skips_the_hit_window() {
    let mut power = 0.7;
    let mut s = PushSpell::cast(CastType::Area, true, 5., &mut power, 2., [1., 0., 0.]).unwrap();
    assert_eq!(power, 0.);
    assert_eq!(s.force, 400.);
    assert_eq!(s.range, 5.5);
    let mut w = World {
        targets: vec![target(2)],
        ..Default::default()
    };
    assert!(s.update(0.25, 1, [0.; 3], &mut w).0);
    assert!(w.ranges.is_empty());
    assert!(!s.update(0.16, 1, [0.; 3], &mut w).0);
    assert!(w.deliveries.is_empty());
}
#[test]
fn weapon_cast_never_queries_targets_and_ends_after_min_ttl() {
    let mut s = PushSpell::cast_weapon(true, 7., [0., 0., -1.]);
    assert_eq!(s.lifecycle.cast_type, CastType::Weapon);
    let mut w = World {
        targets: vec![target(1), target(2)],
        ..Default::default()
    };
    // mMinTTL 0.4: still active at 0.35 s, inactive after 0.45 s.
    for _ in 0..7 {
        s.update(0.05, 9, [0.; 3], &mut w);
    }
    assert!(s.lifecycle.active);
    for _ in 0..2 {
        s.update(0.05, 9, [0.; 3], &mut w);
    }
    assert!(!s.lifecycle.active);
    assert!(w.ranges.is_empty() && w.deliveries.is_empty() && w.impulses.is_empty());
}
