use magicka_sim::character_health::{SelfShield, SelfShieldType as T};
use magicka_sim::damage::{attack as A, Damage};
use magicka_sim::self_shield::{
    ShieldEvent as Event, ShieldImpactContext as Context, ShieldResult as R,
};
use magicka_sim::spells::{Elements as E, Spell};
fn shield(earth: f32, ice: f32) -> SelfShield {
    let mut spell = Spell::default_for(E::SHIELD);
    spell.mag[E::EARTH.index() as usize] = earth;
    spell.mag[E::ICE.index() as usize] = ice;
    SelfShield::from_spell(&spell, &mut vec![])
}
fn ctx() -> Context {
    Context {
        damage_enabled: true,
        network_client: false,
    }
}
fn damage(attack: u16, element: E, amount: f32, magnitude: f32) -> Damage {
    Damage {
        attack,
        element: element.0,
        amount,
        magnitude,
    }
}
#[test]
fn creation_sets_capacity_and_emits_ice_burst_before_earth_cue() {
    for (earth, ice, kind, hp) in [
        (0.0, 0.0, T::Shield, 500.0),
        (1.0, 0.0, T::Earth, 1500.0),
        (0.0, 2.0, T::Ice, 900.0),
        (1.0, 2.0, T::IcedEarth, 1500.0),
    ] {
        let s = shield(earth, ice);
        assert_eq!(s.kind, kind);
        assert_eq!(s.health, hp);
        assert_eq!(s.max_health, hp);
    }
    let mut spell = Spell::default_for(E::SHIELD);
    spell.mag[0] = 1.0;
    spell.mag[8] = 2.0;
    let mut events = vec![];
    let s = SelfShield::from_spell(&spell, &mut events);
    assert_eq!(s.magnitude, 3.0);
    assert_eq!(
        events,
        [
            Event::SetMass(1001.0),
            Event::ResetBarrierMeshes,
            Event::CircleDamage {
                radius: 3.0,
                damage: damage(A::DAMAGE | A::PIERCING, E::EARTH, 275.0, 3.0)
            },
            Event::PlayCreationCue(T::Ice),
            Event::PlayCreationCue(T::Earth)
        ]
    );
}
#[test]
fn pure_shield_blocks_damage_and_status_and_retains_physical_knockback() {
    let mut s = shield(0.0, 0.0);
    let mut d = damage(A::DAMAGE | A::STATUS, E::EARTH, 100.0, 2.0);
    let mut events = vec![];
    assert_eq!(s.filter_impact(&mut d, ctx(), &mut events), R::Deflected);
    assert_eq!(s.health, 300.0);
    assert_eq!(s.time_since_damage, 0.0);
    assert_eq!(d, damage(A::KNOCKBACK, E::EARTH, 100.0, 0.6980619));
    assert_eq!(events, [Event::HitEffect(T::Shield)]);
    let mut d = damage(A::STATUS, E::FIRE, 60.0, 1.0);
    events.clear();
    s.filter_impact(&mut d, ctx(), &mut events);
    assert_eq!(s.health, 240.0);
    assert_eq!(d.attack, 0);
    assert!(events.is_empty());
}
#[test]
fn pure_push_is_reported_deflected_without_changing_the_push() {
    let mut s = shield(0.0, 0.0);
    let original = damage(A::PUSHED, E::WATER, 70.0, 1.0);
    let mut d = original;
    assert_eq!(s.filter_impact(&mut d, ctx(), &mut vec![]), R::Deflected);
    assert_eq!(d, original);
    assert_eq!(s.time_since_damage, 1.0);
    assert_eq!(s.health, 500.0);
}
#[test]
fn solid_physical_reduction_and_overflow_keep_incoming_magnitude() {
    let mut s = shield(1.0, 0.0);
    s.health = 100.0;
    let mut d = damage(A::DAMAGE, E::EARTH, 110.0, 2.0);
    assert_eq!(s.filter_impact(&mut d, ctx(), &mut vec![]), R::Hit);
    assert_eq!(s.health, -70.0);
    assert_eq!(d.amount, 70.0);
    assert_eq!(d.magnitude, 2.0);
    // The full impact pipeline will multiply 70 by 2 again, not use 70 total.
    let mut s = shield(0.0, 2.0);
    let mut d = damage(A::DAMAGE, E::EARTH, 60.0, 1.0);
    assert_eq!(s.filter_impact(&mut d, ctx(), &mut vec![]), R::Deflected);
    assert_eq!(s.health, 900.0);
    assert_eq!(d.amount, 0.0);
    assert_eq!(d.attack, 0);
}
#[test]
fn disabled_damage_and_client_still_transform_impacts_without_spending_shield() {
    for context in [
        Context {
            damage_enabled: false,
            network_client: false,
        },
        Context {
            damage_enabled: true,
            network_client: true,
        },
    ] {
        let mut s = shield(1.0, 0.0);
        s.health = 100.0;
        let mut d = damage(A::DAMAGE, E::FIRE, 150.0, 1.0);
        assert_eq!(s.filter_impact(&mut d, context, &mut vec![]), R::Hit);
        assert_eq!(s.health, 100.0);
        assert_eq!(d.amount, 50.0);
    }
}
#[test]
fn solid_blocks_push_magnitude_but_passes_status_without_damage_flag() {
    let mut s = shield(1.0, 1.0);
    let mut d = damage(A::PUSHED, E::WATER, 70.0, 2.0);
    assert_eq!(s.filter_impact(&mut d, ctx(), &mut vec![]), R::None);
    assert_eq!(d, damage(A::PUSHED, E::WATER, 0.0, 0.0));
    let original = damage(A::STATUS, E::FIRE, 60.0, 1.0);
    let mut d = original;
    s.filter_impact(&mut d, ctx(), &mut vec![]);
    assert_eq!(d, original);
}
#[test]
fn timed_shield_updates_noise_before_decay_and_removes_after_healthbar() {
    let mut s = shield(0.0, 0.0);
    s.health = 25.0;
    let mut speed = 1.0;
    let mut events = vec![];
    s.update(0.5, &mut speed, 70.0, true, &mut events);
    assert_eq!(s.health, -25.0);
    assert_eq!(s.kind, T::None);
    assert!((s.noise_offsets[2][1] + 0.2625).abs() < 1e-6);
    assert_eq!(
        events,
        [
            Event::HealthBar {
                ratio: -0.05,
                time_since_damage: 1.5
            },
            Event::SetMass(70.0),
            Event::RemoveTrackedBuff(T::Shield),
            Event::ClearAura,
            Event::UpdateTrackedBuff {
                kind: T::None,
                health: -25.0
            }
        ]
    );
}
#[test]
fn permanent_setter_resets_health_and_solid_speed_overwrites_previous_limit() {
    let mut s = shield(0.0, 0.0);
    s.set_time_sensitive(false);
    assert_eq!(s.health, 250.0);
    s.health = 100.0;
    s.set_time_sensitive(false);
    assert_eq!(s.health, 250.0);
    let mut speed = 0.1;
    s.update(1.0, &mut speed, 70.0, false, &mut vec![]);
    assert_eq!(s.health, 250.0);
    let mut s = shield(2.0, 2.0);
    s.update(1.0, &mut speed, 70.0, false, &mut vec![]);
    assert!((speed - 0.7).abs() < 1e-6);
    assert_eq!(s.health, 1500.0);
    let mut events = vec![];
    s.remove(70.0, false, &mut events);
    assert_eq!(
        events,
        [
            Event::SetMass(70.0),
            Event::DeathEffect(T::Earth),
            Event::DeathEffect(T::Ice),
            Event::ClearAura
        ]
    );
}
