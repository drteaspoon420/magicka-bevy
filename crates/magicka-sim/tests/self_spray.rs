use magicka_sim::{
    damage::Damage,
    self_spray::*,
    spell_cast::EffectOwner,
    spells::{Elements as E, Spell},
};
#[derive(Default)]
struct Host {
    ops: Vec<String>,
    damage: Option<[Damage; 5]>,
}
impl SelfSprayHost for Host {
    fn electrocute_self(&mut self, m: f32) {
        assert_eq!(m, 1.0);
        self.ops.push("electrocute".into());
    }
    fn start_effect(&mut self, _: usize, n: &'static str) {
        self.ops.push(n.into());
    }
    fn damage_self(&mut self, d: [Damage; 5], t: f64) {
        assert_eq!(t, 2.0);
        self.damage = Some(d);
        self.ops.push("damage".into());
    }
}
#[test]
fn water_calls_electrocute_then_visuals_then_damage_once() {
    let mut h = Host::default();
    let s = SelfSpray::cast(Spell::default_for(E::WATER), true, 2.0, &mut h).unwrap();
    assert_eq!(h.ops, ["electrocute", "self_water", "damage"]);
    assert_eq!(h.damage, Some(s.damages));
    assert_eq!(s.ttl, 1.0);
    assert_eq!(s.lifecycle.max_ttl, 0.2);
}
#[test]
fn fractional_magnitude_does_not_start_visual_and_steam_suppresses_electrocute() {
    let mut h = Host::default();
    let mut s = Spell::default_for(E::STEAM);
    s.mag[9] = 0.5;
    SelfSpray::cast(s, false, 2.0, &mut h).unwrap();
    assert_eq!(h.ops, ["damage"]);
    h.ops.clear();
    SelfSpray::cast(Spell::default_for(E::STEAM), false, 2.0, &mut h).unwrap();
    assert_eq!(h.ops, ["self_steam", "damage"]);
}
#[test]
fn expiry_checks_previous_maximum_and_returns_derived_turn_speed() {
    let mut s = SelfSpray::cast(
        Spell::default_for(E::FIRE),
        false,
        2.0,
        &mut Host::default(),
    )
    .unwrap();
    let owner = EffectOwner {
        avatar: true,
        ..Default::default()
    };
    let mut rumbles = 0;
    assert_eq!(
        s.update(
            0.25,
            owner,
            true,
            |a, b| {
                assert_eq!((a, b), (0.8, 0.8));
                rumbles += 1;
            },
            |_| panic!()
        ),
        (true, 0.6)
    );
    assert_eq!(s.ttl, 1.0);
    let ttl = s.lifecycle.max_ttl;
    assert_eq!(
        s.update(0.25, owner, false, |_, _| panic!(), |s| s.active = false),
        (false, 0.6)
    );
    assert_eq!(s.lifecycle.max_ttl, ttl);
    assert_eq!(rumbles, 1);
}
#[test]
fn release_at_minimum_uses_base_cleanup_in_same_update() {
    let mut s = SelfSpray::cast(
        Spell::default_for(E::FIRE),
        false,
        2.0,
        &mut Host::default(),
    )
    .unwrap();
    let owner = EffectOwner {
        avatar: true,
        button_none: true,
        ..Default::default()
    };
    assert_eq!(
        s.update(0.2, owner, false, |_, _| panic!(), |s| s.active = false),
        (false, 0.6)
    );
    assert!(!s.lifecycle.active);
}
