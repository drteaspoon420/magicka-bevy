use magicka_sim::{
    damage::Damage,
    spells::{Elements as E, Spell},
    spray_area::{greased_damage, plan},
};

#[test]
fn shared_cache_is_lifo_retains_timer_and_exposes_sixteen_entry_exhaustion() {
    use magicka_sim::spray_area::SprayTimerCache;
    let mut cache = SprayTimerCache::default();
    for _ in 0..16 {
        assert_eq!(cache.acquire(), Some(0.0));
    }
    assert_eq!(cache.acquire(), None);
    cache.release(1.0); // Self leaves its private TTL at one.
    cache.release(0.25); // Force can finish with positive private TTL.
    assert_eq!(cache.acquire(), Some(0.25));
    assert_eq!(cache.acquire(), Some(1.0));
    assert_eq!(cache.acquire(), None);
}

#[test]
fn fresh_area_deinitializes_on_first_update_without_decrementing_base_timer() {
    use magicka_sim::{spell_cast::EffectOwner, spray_area::SprayArea};
    let mut area = SprayArea::begin(false, 7.0, 0.0);
    assert_eq!(area.lifecycle.max_ttl, 0.5);
    assert_eq!(
        area.update(0.1, EffectOwner::default(), |l| l.active = false),
        (false, 0.6)
    );
    assert_eq!(area.lifecycle.max_ttl, 0.5);
    assert!(!area.lifecycle.active);
}

#[test]
fn reused_area_preserves_private_timer_and_checks_expiry_before_base_decrement() {
    use magicka_sim::{spell_cast::EffectOwner, spray_area::SprayArea};
    let mut area = SprayArea::begin(true, 8.0, 1.0);
    assert_eq!(
        area.update(0.5, EffectOwner::default(), |l| l.active = false),
        (true, 0.6)
    );
    assert_eq!(area.private_ttl, 1.0);
    assert_eq!(area.lifecycle.max_ttl, 0.0);
    assert_eq!(
        area.update(0.01, EffectOwner::default(), |l| l.active = false),
        (false, 0.6)
    );
    assert!(!area.lifecycle.active);
}

#[test]
fn dispatch_preserves_blast_electrocution_then_greased_damage_order() {
    use magicka_sim::spray_area::{SprayAreaHost, SprayAreaPlan};
    struct Host(Vec<&'static str>);
    impl SprayAreaHost for Host {
        fn full_blast(&mut self, radius: f32, damages: [Damage; 5]) {
            assert_eq!(radius, 3.0);
            assert_eq!(damages[0].element, E::WATER.0);
            self.0.push("blast");
        }
        fn electrocute_owner(&mut self, multiplier: f32) {
            assert_eq!(multiplier, 1.0);
            self.0.push("electrocute");
        }
        fn damage_greased_owner(&mut self, damage: Damage) {
            assert_eq!(damage.element, E::FIRE.0);
            self.0.push("grease");
        }
    }
    let mut damages = [Damage::default(); 5];
    damages[0].element = E::WATER.0;
    let fire = Damage {
        element: E::FIRE.0,
        ..Default::default()
    };
    // Synthetic plan exercises all callback branches independently of recipes.
    let mut plan = SprayAreaPlan {
        radius: 3.0,
        damages,
        electrocute_owner: true,
        greased_owner_damage: Some(fire),
    };
    let mut host = Host(Vec::new());
    plan.dispatch(&mut host);
    assert_eq!(host.0, ["blast", "electrocute", "grease"]);
    plan.electrocute_owner = false;
    plan.greased_owner_damage = None;
    host.0.clear();
    plan.dispatch(&mut host);
    assert_eq!(host.0, ["blast"]);
}

#[test]
fn fire_area_has_native_radius_and_only_greased_owner_receives_fire_slot() {
    let spell = Spell::default_for_list(&[E::FIRE, E::FIRE]);
    let plain = plan(&spell, false).unwrap();
    let greased = plan(&spell, true).unwrap();
    assert!((plain.radius - (2.0f32 / 5.0).sqrt() * 10.0).abs() < 1e-6);
    assert_eq!(plain.damages, greased.damages);
    assert!(plain.greased_owner_damage.is_none());
    assert!(greased.greased_owner_damage.unwrap().element & E::FIRE.0 != 0);
    assert!(!plain.electrocute_owner);
}

#[test]
fn grease_selects_first_fire_slot_and_ignores_fifth_slot() {
    let mut slots = [Damage::default(); 5];
    slots[4].element = E::FIRE.0;
    assert!(greased_damage(&slots, true).is_none());
    slots[2].element = E::FIRE.0;
    slots[2].amount = 7.0;
    slots[3].element = E::FIRE.0;
    slots[3].amount = 9.0;
    assert_eq!(greased_damage(&slots, true), Some(slots[2]));
    assert!(greased_damage(&slots, false).is_none());
}

#[test]
fn water_requests_owner_electrocution_but_steam_does_not() {
    assert!(
        plan(&Spell::default_for_list(&[E::WATER]), false)
            .unwrap()
            .electrocute_owner
    );
    assert!(
        !plan(&Spell::default_for_list(&[E::STEAM]), false)
            .unwrap()
            .electrocute_owner
    );
}
