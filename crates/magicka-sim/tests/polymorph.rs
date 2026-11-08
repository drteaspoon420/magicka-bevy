use magicka_sim::{
    entity_query::{EntityQueryGrid, QueryEntity},
    polymorph::{self, Candidate, PolymorphEffect, PolymorphEffects},
};

fn candidate(handle: u16, position: [f32; 3]) -> Candidate<'static> {
    Candidate {
        handle,
        position,
        character: true,
        ethereal: false,
        name: "goblin_miner",
    }
}

#[test]
fn selection_excludes_owner_ethereal_noncharacters_and_named_vlads() {
    let mut excluded = vec![candidate(1, [0.0; 3])];
    excluded.push(Candidate {
        ethereal: true,
        ..candidate(2, [0.0; 3])
    });
    excluded.push(Candidate {
        character: false,
        ..candidate(3, [0.0; 3])
    });
    for (i, name) in ["BOSS_VLAD_SWAMP", "Vlad_Vietnam", "vlad_future"]
        .into_iter()
        .enumerate()
    {
        excluded.push(Candidate {
            name,
            ..candidate(4 + i as u16, [0.0; 3])
        });
    }
    assert_eq!(
        polymorph::find_closest([0.0; 3], Some(1), excluded.clone()),
        None
    );
    excluded.push(candidate(7, [2.0, 0.0, 0.0]));
    assert_eq!(
        polymorph::find_closest([0.0; 3], Some(1), excluded),
        Some(7)
    );
    // The position-based overload has no owner exclusion.
    assert_eq!(
        polymorph::find_closest([0.0; 3], None, [candidate(1, [0.0; 3])]),
        Some(1)
    );
}

#[test]
fn facing_query_uses_body_radius_and_ignores_height_but_ranking_does_not() {
    let center = polymorph::query_center([1.0, 2.0, 3.0], [0.0, 0.0, 2.0]);
    assert_eq!(center, [1.0, 2.0, 9.0]);
    let candidates = [
        candidate(1, [1.0, 12.0, 9.0]),
        candidate(2, [7.5, 2.0, 9.0]), // Center beyond 6, body within reach.
        candidate(3, [8.1, 2.0, 9.0]), // Outside radius + body radius.
        candidate(4, [1.0, 2.0, 9.0]), // Dead: query rejects, selector doesn't.
    ];
    let geometry: Vec<_> = candidates
        .iter()
        .map(|c| QueryEntity {
            position: c.position,
            radius: 1.0,
            dead: c.handle == 4,
            shield: false,
            damageable: true,
        })
        .collect();
    let grid = EntityQueryGrid::build(&geometry).unwrap();
    let indices = grid
        .query(
            &geometry,
            center,
            polymorph::QUERY_RADIUS,
            false,
            true,
            |_, _| panic!("protected-entity filtering must be disabled"),
        )
        .unwrap();
    assert!(indices.contains(&0));
    assert!(indices.contains(&1));
    assert!(!indices.contains(&2));
    assert!(!indices.contains(&3));
    assert_eq!(
        polymorph::find_closest(center, None, indices.into_iter().map(|i| candidates[i])),
        Some(2)
    );
}

#[test]
fn equal_distances_keep_first_query_result_and_nan_does_not_win() {
    let candidates = [
        candidate(8, [f32::NAN, 0.0, 0.0]),
        candidate(9, [-1.0, 0.0, 0.0]),
        candidate(2, [1.0, 0.0, 0.0]),
    ];
    assert_eq!(polymorph::find_closest([0.0; 3], None, candidates), Some(9));
}

#[test]
fn replacement_and_reversal_preserve_feet_and_current_health_fraction() {
    let position = polymorph::replacement_position([3.0, 5.0, 7.0], 0.5, 1.0, 1.5, 2.0);
    assert_eq!(position, [3.0, 6.5, 7.0]);
    assert_eq!(
        polymorph::replacement_position(position, 1.5, 2.0, 0.5, 1.0),
        [3.0, 5.0, 7.0]
    );
    assert_eq!(
        polymorph::transferred_hit_points(40.0, 100.0, 1000.0),
        400.0
    );
    // Damage during the transformation carries back instead of healing to 40.
    assert_eq!(
        polymorph::transferred_hit_points(200.0, 1000.0, 100.0),
        20.0
    );
}

#[test]
fn recast_cancels_current_target_and_leaves_other_effects_untouched() {
    let mut effects = PolymorphEffects::default();
    effects.add(PolymorphEffect::new(Some(10), 100));
    effects.add(PolymorphEffect::new(Some(20), 200));
    assert!(effects.update(5.0, |_| Some(50.0)).is_empty());
    // The replaced NPC's new identity owns the effect, not its old handle.
    assert_eq!(effects.take_for_recast(9), None);
    let cancelled = effects.take_for_recast(10).unwrap();
    assert_eq!(cancelled.ttl, 15.0);
    assert_eq!(cancelled.restoration(Some(50.0), true), Some((10, 100)));
    assert_eq!(effects.active().len(), 1);
    assert_eq!(effects.active()[0].target, Some(20));
    assert_eq!(effects.take_for_recast(10), None);
}

#[test]
fn expiry_and_nullify_restore_living_targets_only_once() {
    let mut effects = PolymorphEffects::default();
    effects.add(PolymorphEffect::new(Some(10), 100));
    assert!(effects.update(19.75, |_| Some(20.0)).is_empty());
    let removed = effects.update(0.25, |_| Some(20.0));
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].restoration(Some(20.0), true), Some((10, 100)));
    assert_eq!(removed[0].restoration(Some(20.0), false), None);
    assert!(effects.update(1.0, |_| Some(20.0)).is_empty());
    effects.add(PolymorphEffect::new(Some(20), 200));
    let cleared = effects.clear();
    assert_eq!(cleared[0].restoration(Some(10.0), true), Some((20, 200)));
    assert!(effects.clear().is_empty());
}

#[test]
fn death_missing_and_null_targets_expire_without_restoring() {
    let mut effects = PolymorphEffects::default();
    for target in [Some(1), Some(2), None] {
        effects.add(PolymorphEffect::new(target, 100));
    }
    let removed = effects.update(0.0, |target| if target == 1 { Some(0.0) } else { None });
    assert_eq!(removed.len(), 3);
    assert_eq!(
        removed.iter().map(|e| e.target).collect::<Vec<_>>(),
        [Some(1), Some(2), None]
    );
    for effect in removed {
        assert!(effect.is_dead());
        assert_eq!(effect.restoration(Some(0.0), true), None);
        assert_eq!(effect.restoration(None, true), None);
    }
}

#[test]
fn unordered_health_preserves_timer_but_cannot_restore() {
    let mut effect = PolymorphEffect::new(Some(1), 100);
    effect.update(1.0, Some(f32::NAN));
    assert_eq!(effect.ttl, 19.0);
    assert!(!effect.is_dead());
    assert_eq!(effect.restoration(Some(f32::NAN), true), None);
}
