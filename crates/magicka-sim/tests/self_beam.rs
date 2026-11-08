use magicka_sim::{
    damage::Damage,
    self_beam::*,
    spell_cast::{EffectLifecycle, EffectOwner},
    spells::{Elements as E, Spell},
};
#[derive(Default)]
struct Host {
    ops: Vec<String>,
    deliveries: Vec<([Damage; 5], f64)>,
}
impl SelfBeamHost for Host {
    fn start_effect(&mut self, _: usize, name: &'static str) {
        self.ops.push(name.into());
    }
    fn damage_self(&mut self, d: [Damage; 5], t: f64) {
        self.ops.push("damage".into());
        self.deliveries.push((d, t));
    }
    fn update_effects(&mut self) {
        self.ops.push("position".into());
    }
    fn deinitialize(&mut self, l: &mut EffectLifecycle) {
        self.ops.push("stop".into());
        l.active = false;
    }
}
#[test]
fn damage_is_deferred_once_and_precedes_position_update() {
    let mut h = Host::default();
    let mut s = SelfBeam::cast(Spell::default_for(E::LIFE), false, 7.5, &mut h).unwrap();
    assert_eq!(h.ops, ["self_life"]);
    assert!(h.deliveries.is_empty());
    assert_eq!(
        s.update(0.1, EffectOwner::default(), &mut h),
        (true, 0.0325)
    );
    assert_eq!(h.ops, ["self_life", "damage", "position"]);
    assert_eq!(h.deliveries, [(s.damages, 7.5)]);
    s.update(0.1, EffectOwner::default(), &mut h);
    assert_eq!(h.deliveries.len(), 1);
}
#[test]
fn visual_membership_uses_mask_instead_of_truncated_magnitude() {
    let mut h = Host::default();
    let mut spell = Spell::default_for(E::LIFE);
    spell.mag[E::LIFE.index() as usize] = 0.25;
    SelfBeam::cast(spell, false, 0., &mut h).unwrap();
    assert_eq!(h.ops, ["self_life"]);
}
#[test]
fn held_expiry_is_derived_one_second_not_shared_maximum() {
    let mut h = Host::default();
    let mut s = SelfBeam::cast(Spell::default_for(E::ARCANE), true, 0., &mut h).unwrap();
    let owner = EffectOwner {
        avatar: true,
        ..Default::default()
    };
    assert_eq!(s.update(0.25, owner, &mut h), (true, 0.0325));
    assert!(s.lifecycle.max_ttl < 0.);
    assert_eq!(s.update(0.75, owner, &mut h), (false, 0.0325));
    assert_eq!(h.deliveries.len(), 1);
    assert!(!s.lifecycle.active);
}
#[test]
fn release_and_cancellation_keep_native_delivery_order() {
    for owner in [
        EffectOwner {
            avatar: true,
            dead: true,
            ..Default::default()
        },
        EffectOwner {
            avatar: true,
            cast_none: true,
            ..Default::default()
        },
    ] {
        let mut h = Host::default();
        let mut s = SelfBeam::cast(Spell::default_for(E::LIFE), false, 0., &mut h).unwrap();
        assert_eq!(s.update(0.01, owner, &mut h), (false, 0.0325));
        assert_eq!(h.ops, ["self_life", "damage", "position", "stop"]);
    }
    let mut h = Host::default();
    let mut s = SelfBeam::cast(Spell::default_for(E::LIFE), false, 0., &mut h).unwrap();
    let released = EffectOwner {
        avatar: true,
        button_none: true,
        ..Default::default()
    };
    assert!(s.update(0.1, released, &mut h).0);
    assert!(!s.update(0.1, released, &mut h).0);
    assert_eq!(h.deliveries.len(), 1);
}
