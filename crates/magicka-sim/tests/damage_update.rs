use magicka_sim::{character_impact::ImpactCharacter, damage_update::*, status::Status};
fn character() -> ImpactCharacter {
    let mut c = ImpactCharacter::default();
    c.health.hit_points = 50.0;
    c.health.max_hit_points = 100.0;
    c
}
fn context() -> DamageUpdateContext {
    DamageUpdateContext {
        regeneration: 10,
        is_play_state: true,
        in_cutscene: false,
        dead: false,
    }
}
#[test]
fn regeneration_saturates_and_suppresses_overflow_number() {
    let mut c = character();
    c.health.hit_points = 95.0;
    let mut state = DamageBookkeeping::default();
    let mut events = vec![];
    state.update(&mut c, 1.0, context(), &mut events);
    assert_eq!(c.health.hit_points, 100.0);
    assert!(events.is_empty());
    assert_eq!(state.regen_timer, 1.0);
}
#[test]
fn cached_burning_or_poison_pauses_regen_timer() {
    for status in [Status::Burning, Status::Poisoned] {
        let mut c = character();
        c.statuses.current_mask = status as u16;
        let mut b = DamageBookkeeping::default();
        b.update(&mut c, 0.5, context(), &mut vec![]);
        assert_eq!(c.health.hit_points, 50.0);
        assert_eq!(b.regen_timer, 1.0);
    }
}
#[test]
fn regen_timer_resets_while_status_timers_add_one_only() {
    let mut c = character();
    let mut b = DamageBookkeeping::default();
    c.health.fire_damage = 20.0;
    let mut events = vec![];
    b.update(&mut c, 2.5, context(), &mut events);
    assert_eq!(b.regen_timer, 1.0);
    assert_eq!(b.accumulation_timers, [-1.25, -1.5, -1.25, -1.5]);
    assert_eq!(b.total_regen, 15.0);
    assert_eq!(
        events,
        [
            DamageUpdateEvent::Number {
                kind: NumberKind::Regeneration,
                amount: -10.0,
                ttl: 0.7
            },
            DamageUpdateEvent::Number {
                kind: NumberKind::Fire,
                amount: 20.0,
                ttl: 0.7
            }
        ]
    );
}
#[test]
fn accumulator_order_and_staggered_timers() {
    let mut c = character();
    c.health.healing = -4.0;
    c.health.fire_damage = 3.0;
    c.health.poison_damage = 2.0;
    c.health.bleed_damage = 1.0;
    let mut b = DamageBookkeeping::default();
    let mut events = vec![];
    b.update(&mut c, 0.1, context(), &mut events);
    assert_eq!(
        events
            .iter()
            .map(|e| match e {
                DamageUpdateEvent::Number { kind, .. } => *kind,
                _ => panic!(),
            })
            .collect::<Vec<_>>(),
        [NumberKind::Fire, NumberKind::Bleeding]
    );
    events.clear();
    b.update(&mut c, 0.15, context(), &mut events);
    assert_eq!(
        events
            .iter()
            .map(|e| match e {
                DamageUpdateEvent::Number { kind, .. } => *kind,
                _ => panic!(),
            })
            .collect::<Vec<_>>(),
        [NumberKind::Healing, NumberKind::Poison]
    );
    assert_eq!(b.time_since_status_damage, 0.0);
}
#[test]
fn damage_label_moves_at_threshold_then_releases() {
    let mut c = character();
    let mut b = DamageBookkeeping {
        last_damage_index: 7,
        ..Default::default()
    };
    let mut events = vec![];
    b.update(&mut c, 0.333, context(), &mut events);
    assert_eq!(events, [DamageUpdateEvent::MoveNumber(7)]);
    events.clear();
    b.update(&mut c, 0.001, context(), &mut events);
    assert_eq!(events, [DamageUpdateEvent::ReleaseNumber(7)]);
    assert_eq!(b.last_damage_index, -1);
}
#[test]
fn cutscene_suppresses_regen_text_but_not_accumulated_damage() {
    let mut c = character();
    c.health.fire_damage = 2.0;
    let mut b = DamageBookkeeping::default();
    let mut events = vec![];
    b.update(
        &mut c,
        1.0,
        DamageUpdateContext {
            in_cutscene: true,
            ..context()
        },
        &mut events,
    );
    assert_eq!(c.health.hit_points, 60.0);
    assert_eq!(
        events,
        [DamageUpdateEvent::Number {
            kind: NumberKind::Fire,
            amount: 2.0,
            ttl: 0.7
        }]
    );
}
