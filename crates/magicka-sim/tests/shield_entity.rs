use magicka_sim::{
    damage::{attack, Damage},
    shield_entity::{pure_shield, ShieldEntity, ShieldKind, RESULT_DEFLECTED, RESULT_HIT},
    spray_pulse::Segment,
};

fn shield(
    position: [f32; 3],
    kind: ShieldKind,
    hp: f32,
    others: &mut [ShieldEntity],
) -> ShieldEntity {
    ShieldEntity::initialize(
        None,
        position,
        5.5,
        [0.0, 0.0, -1.0],
        kind,
        hp,
        [2.0, 1.5, 1.0],
        others,
    )
}

#[test]
fn pure_force_and_area_shields_follow_shield_spell() {
    let (p, r, k, hp, _) = pure_shield(false, [0.0; 3], [0.0, 0.0, -1.0], 1.0, 0.0, 0.0);
    assert_eq!(
        (p, r, k, hp),
        ([0.0, 0.0, 3.5], 5.5, ShieldKind::Disc, 800.0)
    );
    let (p, _, k, hp, c) = pure_shield(true, [1.0, 0.0, 2.0], [0.0, 0.0, -1.0], 1.0, 0.0, 0.0);
    assert_eq!(
        (p, k, hp, c),
        ([1.0, 0.0, 2.0], ShieldKind::Sphere, 1000.0, [4.0, 3.0, 2.0])
    );
}

#[test]
fn decays_ten_per_tenth_second_then_fades_out_and_dies() {
    let mut s = shield([0.0; 3], ShieldKind::Sphere, 1000.0, &mut []);
    // First update: timer 0 - dt < 0 → one 10 HP step immediately.
    s.update(0.05);
    assert_eq!(s.hit_points, 990.0);
    for _ in 0..19 {
        s.update(0.05);
    }
    assert_eq!(s.hit_points, 900.0);
    assert!(s.tint[3] > 0.9, "faded in {}", s.tint[3]);
    s.kill();
    let mut frames = 0;
    while !s.update(1.0 / 60.0) {
        frames += 1;
        assert!(frames < 200);
    }
    assert!(s.dead);
}

#[test]
fn damage_and_deflection() {
    let mut s = shield([0.0; 3], ShieldKind::Disc, 800.0, &mut []);
    let hit = Damage {
        attack: attack::DAMAGE,
        element: 8,
        amount: 50.0,
        magnitude: 2.0,
    };
    assert_eq!(s.internal_damage(&hit, true), RESULT_HIT);
    assert_eq!(s.hit_points, 700.0);
    let push = Damage {
        attack: attack::PUSHED,
        ..hit
    };
    assert_eq!(s.internal_damage(&push, true), RESULT_DEFLECTED);
    let heal = Damage {
        amount: -50.0,
        ..hit
    };
    s.internal_damage(&heal, true);
    assert_eq!(s.hit_points, 700.0, "negative amounts are ignored");
}

#[test]
fn overlapping_round_shields_kill_the_older_one() {
    let mut first = vec![shield([0.0; 3], ShieldKind::Sphere, 1000.0, &mut [])];
    let second = shield([3.0, 0.0, 0.0], ShieldKind::Sphere, 1000.0, &mut first);
    assert_eq!(first[0].hit_points, 0.0);
    assert_eq!(second.intersect_kill_timer, 0.25);
    let mut far = vec![shield([0.0; 3], ShieldKind::Sphere, 1000.0, &mut [])];
    shield([20.0, 0.0, 0.0], ShieldKind::Sphere, 1000.0, &mut far);
    assert_eq!(far[0].hit_points, 1000.0);
}

#[test]
fn disc_geometry_blocks_only_its_front_arc() {
    let s = shield([0.0; 3], ShieldKind::Disc, 800.0, &mut []);
    // Through the front (forward -Z) surface.
    let front = Segment {
        origin: [0.0, 0.0, -10.0],
        delta: [0.0, 0.0, 8.0],
    };
    assert!(s.geometry().segment_fraction(front).is_some());
    // Through the back: outside the π/4 opening.
    let back = Segment {
        origin: [0.0, 0.0, 10.0],
        delta: [0.0, 0.0, -8.0],
    };
    assert!(s.geometry().segment_fraction(back).is_none());
}
