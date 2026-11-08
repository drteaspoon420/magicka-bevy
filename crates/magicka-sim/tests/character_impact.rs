use magicka_sim::character_health::SelfShield;
use magicka_sim::character_impact::{feature as F, result as R, *};
use magicka_sim::character_status::{CueState, StatusContext};
use magicka_sim::damage::{attack as A, Damage};
use magicka_sim::impact_motion::MotionContext;
use magicka_sim::resistance::Resistance;
use magicka_sim::spells::{Elements as E, Spell};
use magicka_sim::status::{Status as S, StatusEffect};
fn ctx(base: &[Resistance; 11]) -> ImpactContext<'_> {
    ImpactContext {
        status: StatusContext {
            immortal: false,
            ethereal: false,
            can_have_status: true,
            avatar: false,
            current_spell: false,
            base,
            equipment: None,
            buffs: &[],
            cues: CueState::default(),
        },
        motion: MotionContext {
            mass: 70.0,
            gravity_y: -19.64,
            ..Default::default()
        },
        attack_position: [0.0, 0.0, -1.0],
        forward: [0.0, 0.0, -1.0],
        block_values: &[],
        block_item: -1,
        features: 15,
        network_client: false,
        attacker: Some(7),
        attacker_is_self: false,
        attacker_avatar: false,
        attacker_local_avatar: false,
        attacker_is_gripped: false,
        grip_attack_state: false,
        hit_tolerance: 5.0,
        resurrection_state: false,
        stun_time: 2.0,
        bleed_rate: 20.0,
        blood_and_gore: false,
        has_blood: false,
        bloat_killed: false,
        has_damage_trigger: false,
    }
}
fn character() -> ImpactCharacter {
    let mut c = ImpactCharacter::default();
    c.health.hit_points = 1000.0;
    c.health.max_hit_points = 1000.0;
    c
}
fn d(attack: u16, element: E, amount: f32, magnitude: f32) -> Damage {
    Damage {
        attack,
        element: element.0,
        amount,
        magnitude,
    }
}
fn seed(c: &mut ImpactCharacter, kind: S) {
    c.statuses
        .set_effect(StatusEffect::new(kind, 1.0, 1.0, false))
        .unwrap();
    c.statuses.current_mask |= kind as u16;
}

#[test]
fn composite_immunity_blocks_whole_hit_but_collection_keeps_other_entries() {
    let mut base = [Resistance::IDENTITY; 11];
    base[E::FIRE.index() as usize].multiplier = 0.;
    let context = ctx(&base);
    let mut combined = character();
    let result = combined
        .impact(
            d(A::DAMAGE, E::FIRE | E::ARCANE, 100., 1.),
            &context,
            &mut vec![],
        )
        .unwrap();
    assert_ne!(result & R::DEFLECTED, 0);
    assert_eq!(combined.health.hit_points, 1000.);

    let mut separate = character();
    separate
        .impact_collection(
            [
                d(A::DAMAGE, E::FIRE, 50., 1.),
                d(A::DAMAGE, E::ARCANE, 50., 1.),
                Damage::default(),
                Damage::default(),
                Damage::default(),
            ],
            &context,
            &mut vec![],
        )
        .unwrap();
    assert_eq!(separate.health.hit_points, 950.);
}

#[test]
fn composite_resistances_multiply_and_armour_piercing_keeps_vulnerability() {
    let mut base = [Resistance::IDENTITY; 11];
    base[E::FIRE.index() as usize].multiplier = 0.5;
    base[E::ARCANE.index() as usize].multiplier = 2.;
    for (attack, fire, expected) in [
        (A::DAMAGE, 0.5, 900.),
        (A::DAMAGE | A::PIERCING, 0., 1000.),
        (A::DAMAGE | A::ARMOUR_PIERCING, 0., 800.),
    ] {
        base[E::FIRE.index() as usize].multiplier = fire;
        let mut c = character();
        c.impact(
            d(attack, E::FIRE | E::ARCANE, 100., 1.),
            &ctx(&base),
            &mut vec![],
        )
        .unwrap();
        assert_eq!(c.health.hit_points, expected);
    }
}

#[test]
fn status_immunity_and_damage_immunity_have_separate_gates() {
    let mut base = [Resistance::IDENTITY; 11];
    base[E::FIRE.index() as usize].status_resistance = true;
    let mut c = character();
    c.impact(
        d(A::DAMAGE | A::STATUS, E::FIRE, 100., 1.),
        &ctx(&base),
        &mut vec![],
    )
    .unwrap();
    assert_eq!(c.health.hit_points, 900.);
    assert!(!c.statuses.has(S::Burning));
}

#[test]
fn later_statuses_observe_cue_start_within_one_damage_or_collection() {
    use magicka_sim::character_status::StatusEvent;
    let base = [Resistance::IDENTITY; 11];
    for collection in [false, true] {
        let mut c = character();
        c.statuses.dry_timer = -1.0;
        let mut events = Vec::new();
        if collection {
            c.impact_collection(
                [
                    d(A::STATUS, E::FIRE, 100.0, 1.0),
                    d(A::STATUS, E::WATER, 0.0, 1.0),
                    Damage::default(),
                    Damage::default(),
                    Damage::default(),
                ],
                &ctx(&base),
                &mut events,
            )
            .unwrap();
        } else {
            c.impact(
                d(A::STATUS, E::FIRE | E::WATER, 100.0, 1.0),
                &ctx(&base),
                &mut events,
            )
            .unwrap();
        }
        assert!(c.statuses.effect(S::Burning).unwrap().dead(), "{events:?}");
        let status: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ImpactEvent::Status(event) => Some(*event),
                _ => None,
            })
            .collect();
        assert_eq!(
            status,
            [
                StatusEvent::PlayStatusCue(S::Burning),
                StatusEvent::StopCue(S::Burning),
                StatusEvent::DryingEffect,
                StatusEvent::PlaySteamCue
            ]
        );
    }
}
#[test]
fn wet_lightning_and_frozen_physical_multiply_after_resistance() {
    let mut base = [Resistance::IDENTITY; 11];
    base[4].multiplier = 0.5;
    let context = ctx(&base);
    let mut c = character();
    seed(&mut c, S::Wet);
    let r = c
        .impact(
            d(A::DAMAGE, E::LIGHTNING, 100.0, 2.0),
            &context,
            &mut vec![],
        )
        .unwrap();
    assert_eq!(c.health.hit_points, 800.0);
    assert_eq!(c.hit_by_lightning, 0.1);
    assert_eq!(r, R::DAMAGED | R::HIT);
    seed(&mut c, S::Frozen);
    c.impact(
        d(A::DAMAGE | A::PIERCING, E::EARTH, 100.0, 1.0),
        &context,
        &mut vec![],
    )
    .unwrap();
    assert_eq!(c.health.hit_points, 500.0);
}
#[test]
fn full_block_still_applies_status_before_early_return() {
    let base = [Resistance::IDENTITY; 11];
    let mut context = ctx(&base);
    context.block_item = 0;
    context.block_values = &[100];
    let mut c = character();
    let r = c
        .impact(
            d(A::DAMAGE | A::STATUS, E::EARTH | E::FIRE, 60.0, 1.0),
            &context,
            &mut vec![],
        )
        .unwrap();
    assert_eq!(r, R::DEFLECTED | 0x20);
    assert_eq!(c.health.hit_points, 1000.0);
    assert!(c.statuses.has(S::Burning));
    assert_eq!(c.last_attacker_timer, 0.0);
}
#[test]
fn pure_shield_removes_status_flags_before_status_addition() {
    let base = [Resistance::IDENTITY; 11];
    let context = ctx(&base);
    let mut c = character();
    c.health.shield = SelfShield::from_spell(&Spell::default_for(E::SHIELD), &mut vec![]);
    let r = c
        .impact(
            d(A::DAMAGE | A::STATUS, E::FIRE, 60.0, 1.0),
            &context,
            &mut vec![],
        )
        .unwrap();
    assert_eq!(r, R::DEFLECTED);
    assert_eq!(c.health.shield.health, 440.0);
    assert!(!c.statuses.has(S::Burning));
    assert_eq!(c.health.hit_points, 1000.0);
}
#[test]
fn solid_overflow_is_multiplied_by_magnitude_again() {
    let base = [Resistance::IDENTITY; 11];
    let context = ctx(&base);
    let mut c = character();
    let mut spell = Spell::default_for(E::SHIELD);
    spell.mag[0] = 1.0;
    c.health.shield = SelfShield::from_spell(&spell, &mut vec![]);
    c.health.shield.health = 100.0;
    c.impact(d(A::DAMAGE, E::EARTH, 110.0, 2.0), &context, &mut vec![])
        .unwrap();
    assert_eq!(c.health.hit_points, 860.0);
}
#[test]
fn client_and_disabled_damage_report_hits_without_reducing_hp() {
    let base = [Resistance::IDENTITY; 11];
    for client in [false, true] {
        let mut context = ctx(&base);
        context.network_client = client;
        if !client {
            context.features = F::NOTIFY;
        }
        let mut c = character();
        let r = c
            .impact(d(A::DAMAGE, E::FIRE, 50.0, 1.0), &context, &mut vec![])
            .unwrap();
        assert_eq!(r, R::HIT | R::DAMAGED);
        assert_eq!(c.health.hit_points, 1000.0);
        assert!(c.is_hit);
        assert_eq!(c.last_attacker, Some(7));
    }
}
#[test]
fn life_damage_reduces_poison_and_heals_without_status_constructor() {
    let base = [Resistance::IDENTITY; 11];
    let context = ctx(&base);
    let mut c = character();
    c.health.hit_points = 500.0;
    seed(&mut c, S::Poisoned);
    assert_eq!(
        c.impact(d(A::DAMAGE, E::LIFE, -80.0, 2.0), &context, &mut vec![])
            .unwrap(),
        R::HEALED
    );
    assert_eq!(c.health.hit_points, 660.0);
    assert!(c.statuses.effect(S::Poisoned).unwrap().dead());
}
#[test]
fn stun_and_bleed_are_not_gated_by_damage_feature() {
    let base = [Resistance::IDENTITY; 11];
    let mut context = ctx(&base);
    context.features = 0;
    let mut c = character();
    let r = c
        .impact(
            d(A::STUN | A::BLEED, E::EARTH, 1.0, 1.0),
            &context,
            &mut vec![],
        )
        .unwrap();
    assert_eq!(r, 0x20);
    assert_eq!(c.stun_timer, 2.0);
    assert!(!c.statuses.effect(S::Bleeding).unwrap().dead());
    assert_eq!(c.health.hit_points, 1000.0);
}
#[test]
fn entangle_returns_before_shield_and_attribution() {
    let base = [Resistance::IDENTITY; 11];
    let context = ctx(&base);
    let mut c = character();
    let mut events = vec![];
    assert_eq!(
        c.impact(
            d(A::ENTANGLEMENT, E::EARTH, 50.0, 2.0),
            &context,
            &mut events
        )
        .unwrap(),
        R::HIT
    );
    assert_eq!(events, [ImpactEvent::Entangle(2.0)]);
    assert_eq!(c.last_attacker, None);
}
#[test]
fn lethal_and_overkill_paths_differ_and_pure_arcane_does_not_execute_overkill() {
    let base = [Resistance::IDENTITY; 11];
    let mut context = ctx(&base);
    context.blood_and_gore = true;
    let mut c = character();
    let r = c
        .impact(d(A::DAMAGE, E::FIRE, 1001.0, 1.0), &context, &mut vec![])
        .unwrap();
    assert_ne!(r & R::KILLED, 0);
    assert!(!c.dead);
    assert_eq!(c.health.hit_points, -1.0);
    for (element, dead, hp) in [(E::EARTH, true, -2000.0), (E::ARCANE, false, -1100.0)] {
        let mut c = character();
        let r = c
            .impact(d(A::DAMAGE, element, 2100.0, 1.0), &context, &mut vec![])
            .unwrap();
        assert_ne!(r & R::OVERKILLED, 0);
        assert_eq!(c.dead, dead);
        assert_eq!(c.health.hit_points, hp);
    }
}
#[test]
fn collection_includes_zero_entries_and_preserves_unusual_result_formula() {
    let base = [Resistance::IDENTITY; 11];
    let context = ctx(&base);
    let mut c = character();
    let mut entries = [Damage::default(); 5];
    entries[0] = d(A::DAMAGE, E::FIRE, 10.0, 1.0);
    assert_eq!(
        c.impact_collection(entries, &context, &mut vec![]).unwrap(),
        R::DAMAGED | R::HIT
    );
    assert_eq!(c.health.hit_points, 990.0);
    let mut c = character();
    let entries = [d(A::DAMAGE, E::FIRE, 10.0, 1.0); 5];
    assert_eq!(
        c.impact_collection(entries, &context, &mut vec![]).unwrap(),
        0
    );
    assert_eq!(c.health.hit_points, 950.0);
}

#[test]
fn wet_conjuring_penalty_routes_through_public_delivery_into_live_hp() {
    use magicka_sim::{
        avatar_input::{AvatarInput, ConjureContext},
        damage_delivery::*,
        spells::{Direction, MagickTree},
    };
    let base = [Resistance::IDENTITY; 11];
    for (network, expected_hp) in [(NetworkRole::Offline, 750.0), (NetworkRole::Client, 1000.0)] {
        let mut context = ctx(&base);
        context.status.avatar = true;
        context.status.equipment = Some(&base);
        let mut character = character();
        seed(&mut character, S::Wet);
        let mut input = AvatarInput::default();
        let mut events = vec![];
        let mut delivery_events = vec![];
        let conjure = ConjureContext {
            status_mask: S::Wet as u16,
            networked: network != NetworkRole::Offline,
            can_chant: true,
            ..Default::default()
        };
        input
            .direction(
                Direction::Left,
                conjure,
                &MagickTree::default(),
                (|_| true, |_| true),
                |_, _| panic!(),
                &mut events,
            )
            .unwrap();
        input
            .direction(
                Direction::Down,
                conjure,
                &MagickTree::default(),
                (|_| true, |_| true),
                |damage, _| {
                    character
                        .deliver(
                            DamagePayload::Collection(damage),
                            DeliveryContext {
                                network,
                                attacker_kind: AttackerKind::LocalAvatar,
                                target_handle: 2,
                                attacker_handle: Some(2),
                                time_stamp: 1.0,
                                target_position: [0.0; 3],
                                attack_position: [0.0; 3],
                            },
                            &[],
                            &mut context,
                            &mut delivery_events,
                        )
                        .unwrap();
                },
                &mut events,
            )
            .unwrap();
        assert_eq!(character.health.hit_points, expected_hp);
        assert!(input.queue.is_empty());
        assert_eq!(
            delivery_events
                .iter()
                .filter(|e| matches!(e, DeliveryEvent::Request(_)))
                .count(),
            usize::from(network == NetworkRole::Client)
        );
        if network == NetworkRole::Client {
            assert!(matches!(delivery_events[0], DeliveryEvent::Request(_)));
        }
    }
}
