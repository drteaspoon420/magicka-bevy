use magicka_sim::{
    shield_boost::shield_to_boost,
    shield_entity::{ShieldEntity, ShieldKind},
};

fn shield(position: [f32; 3], kind: ShieldKind, hp: f32) -> ShieldEntity {
    ShieldEntity::initialize(
        None,
        position,
        1.0,
        [0.0, 0.0, 1.0],
        kind,
        hp,
        [1.0; 3],
        &mut [],
    )
}

#[test]
fn boost_selects_nearest_hit_not_nearest_center_and_keeps_first_tie() {
    let mut broad = shield([12.0, 0.0, 0.0], ShieldKind::Sphere, 100.0);
    broad.radius = 10.0;
    let narrow = shield([4.0, 0.0, 0.0], ShieldKind::Sphere, 100.0);
    assert_eq!(
        shield_to_boost([0.0; 3], [1.0, 0.0, 0.0], [&narrow, &broad]),
        Some(1)
    );
    assert_eq!(
        shield_to_boost([0.0; 3], [1.0, 0.0, 0.0], [&broad, &broad]),
        Some(0)
    );
}

#[test]
fn facing_ray_preserves_height_and_rejects_dead_behind_and_twenty_meter_hits() {
    let live = shield([6.0, 4.0, 0.0], ShieldKind::Sphere, 100.0);
    let dead = shield([3.0, 4.0, 0.0], ShieldKind::Sphere, 0.0);
    let behind = shield([-3.0, 4.0, 0.0], ShieldKind::Sphere, 100.0);
    let end = shield([21.0, 4.0, 0.0], ShieldKind::Sphere, 100.0);
    assert_eq!(
        shield_to_boost(
            [0.0, 4.0, 0.0],
            [1.0, 9.0, 0.0],
            [&dead, &behind, &end, &live]
        ),
        Some(3)
    );
    assert_eq!(shield_to_boost([0.0; 3], [1.0, 0.0, 0.0], [&live]), None);
    assert_eq!(shield_to_boost([0.0, 4.0, 0.0], [0.0; 3], [&live]), None);
    assert_eq!(
        shield_to_boost([0.0, 4.0, 0.0], [2.0, 0.0, 0.0], [&live]),
        Some(0)
    );
}

#[test]
fn wall_box_and_disc_cap_use_the_original_collision_primitives() {
    let wall = shield([6.0, 0.0, 0.0], ShieldKind::Wall, 100.0);
    assert_eq!(shield_to_boost([0.0; 3], [1.0, 0.0, 0.0], [&wall]), Some(0));
    let disc = shield([6.0, 0.0, 0.0], ShieldKind::Disc, 100.0);
    assert_eq!(shield_to_boost([0.0; 3], [1.0, 0.0, 0.0], [&disc]), Some(0));
    assert_eq!(
        shield_to_boost([10.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [&disc]),
        Some(0)
    );
}

#[test]
fn boosting_uses_both_enabled_skin_primitives_even_for_round_shields() {
    let round = shield([6.0, 0.0, 4.0], ShieldKind::Sphere, 100.0);
    let segment = magicka_sim::spray_pulse::Segment {
        origin: [0.0; 3],
        delta: [20.0, 0.0, 0.0],
    };
    assert_eq!(round.geometry().segment_fraction(segment), None);
    assert_eq!(
        shield_to_boost([0.0; 3], [1.0, 0.0, 0.0], [&round]),
        Some(0)
    );
}
