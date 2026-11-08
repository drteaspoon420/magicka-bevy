use magicka_sim::character_health::*;
use magicka_sim::character_status::CharacterStatuses;
use magicka_sim::spells::Elements as E;
use magicka_sim::status::{Status as S, StatusEffect};
fn health() -> CharacterHealth {
    CharacterHealth {
        hit_points: 100.0,
        max_hit_points: 100.0,
        invisibility_timer: 5.0,
        ..Default::default()
    }
}
fn seed(s: &mut CharacterStatuses, kind: S, dps: f32, magnitude: f32) {
    s.set_effect(StatusEffect::new(kind, dps, magnitude, false))
        .unwrap();
    s.current_mask |= kind as u16;
}
#[test]
fn lethal_damage_retains_negative_hp_and_omits_hypnosis_stop() {
    let mut h = health();
    let mut events = vec![];
    let ctx = HealthContext {
        has_last_damager: true,
        entangled: true,
        ..Default::default()
    };
    assert_eq!(
        h.damage(110.0, E::FIRE, 0, ctx, &mut events),
        DirectResult::Killed
    );
    assert_eq!(h.hit_points, -10.0);
    assert_eq!(h.fire_damage, 110.0);
    assert_eq!(h.invisibility_timer, 0.0);
    assert_eq!(
        events,
        [
            HealthEvent::Statistics {
                amount: 110,
                element: E::FIRE,
                result: DirectResult::Killed
            },
            HealthEvent::DecreaseEntanglement {
                amount: 110.0,
                element: E::FIRE
            }
        ]
    );
}
#[test]
fn client_records_prediction_but_never_writes_health_or_entanglement() {
    let mut h = health();
    let mut events = vec![];
    let ctx = HealthContext {
        network_client: true,
        entangled: true,
        ..Default::default()
    };
    assert_eq!(
        h.damage(110.0, E::FIRE, 0, ctx, &mut events),
        DirectResult::Killed
    );
    assert_eq!(h.hit_points, 100.0);
    assert_eq!(h.fire_damage, 110.0);
    assert!(events.is_empty());
}
#[test]
fn healing_clamps_hp_but_accumulates_only_when_predicted_health_is_below_max() {
    let mut h = health();
    h.hit_points = 90.0;
    let ctx = HealthContext {
        avatar: true,
        has_last_damager: true,
        ..Default::default()
    };
    let mut events = vec![];
    h.damage(-10.0, E::LIFE, 0, ctx, &mut events);
    assert_eq!(h.hit_points, 100.0);
    assert_eq!(h.healing, 0.0);
    assert_eq!(events, [HealthEvent::ProfileHealed(-10.0)]);
    h.hit_points = 50.0;
    h.damage(-10.0, E::LIFE, 0, ctx, &mut events);
    assert_eq!(h.healing, -10.0);
    h.damage(-100.0, E::LIFE, 0, ctx, &mut events);
    assert_eq!(h.hit_points, 100.0);
    assert_eq!(h.healing, -10.0);
}
#[test]
fn shield_is_processed_before_grease_and_does_not_absorb_direct_damage() {
    let mut h = health();
    h.shield = DirectShield {
        kind: SelfShieldType::Ice,
        health: 200.0,
        ice_magnitude: 2.0,
        ..Default::default()
    };
    h.damage(
        10.0,
        E::FIRE,
        S::Greased as u16,
        HealthContext::default(),
        &mut vec![],
    );
    assert_eq!(h.shield.health, 170.0);
    assert_eq!(h.hit_points, 40.0);
    let mut h = health();
    h.shield.kind = SelfShieldType::Shield;
    h.shield.health = 200.0;
    h.damage(10.0, E::FIRE, 0, HealthContext::default(), &mut vec![]);
    assert_eq!(h.shield.health, 190.0);
    assert_eq!(h.hit_points, 90.0);
}
#[test]
fn solid_shield_blocks_healing_and_pure_shield_repair_is_halved_when_not_timed() {
    let mut h = health();
    h.hit_points = 50.0;
    h.shield.kind = SelfShieldType::Earth;
    let mut events = vec![];
    assert_eq!(
        h.damage(-20.0, E::LIFE, 0, HealthContext::default(), &mut events),
        DirectResult::Damaged
    );
    assert_eq!(h.hit_points, 50.0);
    assert_eq!(events, [HealthEvent::StopHypnotize]);
    h.shield.kind = SelfShieldType::Shield;
    h.shield.health = 100.0;
    h.damage(-20.0, E::SHIELD, 0, HealthContext::default(), &mut events);
    assert_eq!(h.shield.health, 110.0);
    assert_eq!(h.hit_points, 70.0);
}
#[test]
fn immortal_guard_precedes_all_side_effects_and_combined_elements_skip_accumulators() {
    let mut h = health();
    let mut events = vec![];
    assert_eq!(
        h.damage(
            10.0,
            E::FIRE,
            0,
            HealthContext {
                immortal: true,
                ..Default::default()
            },
            &mut events
        ),
        DirectResult::None
    );
    assert_eq!(h.invisibility_timer, 5.0);
    assert!(events.is_empty());
    h.damage(
        10.5,
        E::FIRE | E::POISON,
        0,
        HealthContext {
            has_last_damager: true,
            gripping_last_damager: true,
            ..Default::default()
        },
        &mut events,
    );
    assert_eq!(h.fire_damage, 0.0);
    assert_eq!(h.poison_damage, 0.0);
    assert_eq!(h.grip_damage, 10.5);
    assert!(matches!(
        events[1],
        HealthEvent::Statistics { amount: 10, .. }
    ));
}
#[test]
fn poison_before_healing_changes_the_health_seen_by_healing() {
    let mut h = health();
    let mut statuses = CharacterStatuses::default();
    seed(&mut statuses, S::Poisoned, 20.0, 1.0);
    seed(&mut statuses, S::Healing, -4.0, 3.0);
    let mut frame = StatusFrame {
        speed_multiplier: 1.0,
        ..Default::default()
    };
    statuses.update(
        0.5,
        &mut h,
        &mut frame,
        StatusUpdateContext::default(),
        &mut vec![],
    );
    assert_eq!(h.hit_points, 92.0);
    assert_eq!(statuses.effect(S::Healing).unwrap().magnitude, 1.5); // not stopped at old full HP
    assert_eq!(frame.speed_multiplier, 0.5);
    assert!(statuses.has(S::Poisoned));
    assert!(statuses.has(S::Healing));
}
#[test]
fn expiry_cleanup_uses_old_mask_for_light_then_new_mask_on_next_frame() {
    let mut h = health();
    let mut statuses = CharacterStatuses::default();
    seed(&mut statuses, S::Burning, 1.0, 0.01);
    let mut frame = StatusFrame {
        speed_multiplier: 1.0,
        ..Default::default()
    };
    let ctx = StatusUpdateContext {
        cues_exist: S::Burning as u16,
        ..Default::default()
    };
    let mut events = vec![];
    statuses.update(0.5, &mut h, &mut frame, ctx, &mut events);
    assert!(!statuses.has(S::Burning));
    assert!(frame.light_present);
    assert!(events.contains(&StatusUpdateEvent::StopCue(S::Burning)));
    assert_eq!(events.last(), Some(&StatusUpdateEvent::StartBurningLight));
    events.clear();
    statuses.update(0.5, &mut h, &mut frame, ctx, &mut events);
    assert_eq!(
        events,
        [StatusUpdateEvent::MoveLight, StatusUpdateEvent::StopLight]
    );
}
#[test]
fn explicit_dead_state_clears_slots_without_damage_or_buff_removal() {
    let mut h = health();
    let mut statuses = CharacterStatuses::default();
    seed(&mut statuses, S::Poisoned, 50.0, 1.0);
    let mut frame = StatusFrame::default();
    let mut events = vec![];
    statuses.update(
        1.0,
        &mut h,
        &mut frame,
        StatusUpdateContext {
            dead: true,
            buff_tracker: true,
            ..Default::default()
        },
        &mut events,
    );
    assert_eq!(h.hit_points, 100.0);
    assert_eq!(statuses.current_mask, 0);
    assert_eq!(
        statuses.effect(S::Poisoned).unwrap(),
        &StatusEffect::default()
    );
    assert!(events.is_empty());
    assert_eq!(statuses.dry_timer, -1.0);
}
#[test]
fn alive_empty_slots_remove_tracked_buffs_and_panic_can_remain_without_light() {
    let mut h = health();
    let mut statuses = CharacterStatuses::default();
    let mut frame = StatusFrame {
        panic: 2.0,
        ..Default::default()
    };
    let mut events = vec![];
    statuses.update(
        0.1,
        &mut h,
        &mut frame,
        StatusUpdateContext {
            buff_tracker: true,
            ..Default::default()
        },
        &mut events,
    );
    assert_eq!(events.len(), 9);
    assert_eq!(frame.panic, 2.0);
    assert_eq!(events[0], StatusUpdateEvent::RemoveTrackedBuff(S::Burning));
    assert_eq!(events[8], StatusUpdateEvent::RemoveTrackedBuff(S::Bleeding));
}
