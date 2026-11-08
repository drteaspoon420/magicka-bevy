use magicka_sim::{
    character_impact::result as R,
    damage::{attack, Damage},
    impact_motion::Vector,
    missile_collision::*,
    missile_events::*,
    spells::{Elements as E, Spell},
};
#[derive(Default)]
struct Host {
    pos: Vector,
    velocity: Vector,
    result: u16,
    selected: Option<u16>,
    targets: Vec<(u16, Vector)>,
    actions: Vec<&'static str>,
    deliveries: usize,
    kills: usize,
    awards: usize,
    selected_events: Vec<Option<u16>>,
}
impl MissileEventHost for Host {
    fn damageable_target(&self) -> bool {
        self.selected.is_some()
    }
    fn normalized_velocity(&self) -> f32 {
        ((self.velocity.iter().map(|v| v * v).sum::<f32>()) as f64).sqrt() as f32 / 50.
    }
    fn damage(&mut self, _: Damage) -> u16 {
        self.deliveries += 1;
        self.actions.push("damage");
        self.result
    }
    fn kill(&mut self) {
        self.kills += 1;
        self.actions.push("kill");
    }
    fn event(&mut self, _: &MissileEvent, _: &mut Option<Vector>) {
        self.selected_events.push(self.selected);
    }
}
impl MissileCollisionHost for Host {
    fn position(&self) -> Vector {
        self.pos
    }
    fn velocity(&self) -> Vector {
        self.velocity
    }
    fn set_velocity(&mut self, v: Vector) {
        self.actions.push("velocity");
        self.velocity = v;
    }
    fn target_position(&self, h: u16) -> Vector {
        self.targets.iter().find(|(id, _)| *id == h).unwrap().1
    }
    fn select_target(&mut self, h: Option<u16>) {
        self.selected = h;
    }
    fn update_followed_effects(&mut self) {
        self.actions.push("follow");
    }
    fn award_deep_impact(&mut self) {
        self.awards += 1;
    }
}
fn state(ice: bool) -> MissileCollisionState {
    let s = Spell::default_for_list(&[if ice { E::ICE } else { E::EARTH }]);
    let r = MissileRecipe::from_spell(s, 0., 3).unwrap();
    MissileCollisionState::new(Some(1), true, r.conditions, false)
}
fn target(h: u16, kind: TargetKind) -> Contact {
    Contact::Body {
        physics_object: false,
        target: Some(CollisionTarget {
            handle: h,
            kind,
            dead: false,
            damageable: true,
        }),
    }
}
fn char_target(h: u16) -> Contact {
    target(h, TargetKind::Character { ethereal: false })
}
#[test]
fn owner_hit_list_expires_then_owner_can_be_hit_and_pierced() {
    let mut s = state(true);
    let mut h = Host {
        velocity: [0., 0., 50.],
        result: R::PIERCED,
        ..Default::default()
    };
    assert!(!s.on_collision(char_target(1), true, &mut h));
    assert_eq!(h.deliveries, 0);
    s.update_without_homing(0.25, true, &mut h);
    assert!(!s.on_collision(char_target(1), true, &mut h));
    assert_eq!(h.deliveries, 4);
    assert_eq!(s.gibbed_hits[&1], 5.);
    assert!(s.pending_velocity.is_none());
    assert!(!s.on_collision(char_target(1), true, &mut h));
    assert_eq!(h.deliveries, 4);
}
#[test]
fn hit_cooldown_allows_contact_for_earth_and_shield_but_not_piercing_body() {
    for ice in [false, true] {
        let mut s = state(ice);
        let mut h = Host {
            result: R::HIT,
            velocity: [1., 0., 0.],
            ..Default::default()
        };
        assert!(s.on_collision(char_target(2), true, &mut h));
        let count = h.deliveries;
        assert_eq!(s.on_collision(char_target(2), true, &mut h), !ice);
        assert_eq!(h.deliveries, count);
        assert!(s.on_collision(target(2, TargetKind::Shield), true, &mut h));
    }
}
#[test]
fn overkill_queues_velocity_and_follow_update_precedes_velocity_application() {
    let mut s = state(false);
    let mut h = Host {
        pos: [0.; 3],
        velocity: [10., 0., 0.],
        result: R::OVERKILLED,
        targets: vec![(2, [-1., 0., 0.])],
        ..Default::default()
    };
    assert!(!s.on_collision(char_target(2), true, &mut h));
    assert_eq!(h.velocity, [10., 0., 0.]);
    assert_eq!(s.pending_velocity, Some([12., 0., 0.]));
    assert_eq!(s.gibbed_hits[&2], 5.);
    h.actions.clear();
    s.update_without_homing(0.1, true, &mut h);
    assert_eq!(&h.actions[..2], &["follow", "velocity"]);
    assert_eq!(h.velocity, [12., 0., 0.]);
    assert!(s.pending_velocity.is_none());
    s.update_without_homing(5., true, &mut h);
    assert!(!s.gibbed_hits.contains_key(&2));
}
#[test]
fn level_collision_threshold_is_squared_speed_and_target_clears_before_events() {
    let mut s = state(false);
    let mut h = Host {
        velocity: [3., 4., 0.],
        ..Default::default()
    };
    assert!(!s.on_collision(
        Contact::Body {
            physics_object: true,
            target: Some(CollisionTarget {
                handle: 2,
                kind: TargetKind::Other,
                dead: false,
                damageable: false
            })
        },
        true,
        &mut h
    ));
    assert_eq!(s.collision_target, Some(2));
    assert_eq!(s.collision_velocity, 25.);
    let args = s.update_without_homing(0.05, true, &mut h);
    assert_eq!(args.threshold, 25.);
    assert_eq!(args.flags, condition::TIMER | condition::COLLISION);
    assert!(h.selected_events.iter().all(Option::is_none));
    assert_eq!(s.collision_target, None);
    assert!(s.on_collision(Contact::Static(StaticContact::Level), true, &mut h));
    let args = s.update_without_homing(0.05, true, &mut h);
    assert_eq!(args.flags, condition::TIMER);
    s.update_without_homing(0.05, true, &mut h);
    assert!(!s.has_collided);
    assert_eq!(s.level_timer, 0.);
    assert!(s.on_collision(Contact::Static(StaticContact::Water), true, &mut h));
    s.update_without_homing(0.01, true, &mut h);
    assert_eq!(h.kills, 1, "second distinct collision spends the counter");
}
#[test]
fn rejected_entities_and_network_gate_do_not_receive_hit_events() {
    let mut s = state(true);
    let mut h = Host::default();
    for kind in [
        TargetKind::Pickable,
        TargetKind::Character { ethereal: true },
        TargetKind::BossDamage { ethereal: true },
        TargetKind::Grease,
        TargetKind::SpellMine,
        TargetKind::ElementalEgg,
        TargetKind::Tornado,
        TargetKind::Barrier { solid: false },
        TargetKind::Missile { owner: Some(1) },
    ] {
        assert!(!s.on_collision(target(2, kind), true, &mut h));
    }
    assert_eq!(h.deliveries, 0);
    assert!(s.on_collision(char_target(2), false, &mut h));
    assert!(!s.hits.contains(2));
    assert!(s.on_collision(target(2, TargetKind::BossCollision), true, &mut h));
    assert_eq!(h.deliveries, 0);
    assert!(!s.on_collision(Contact::Static(StaticContact::Other), true, &mut h));
}
#[test]
fn signed_damage_accumulation_resets_values_but_keeps_damaged_flag_and_fall_kills_last() {
    let mut s = state(false);
    let mut h = Host {
        pos: [0., -50., 0.],
        ..Default::default()
    };
    for (amount, element, grease) in [
        (10., E::FIRE, false),
        (-4., E::COLD, false),
        (99., E::EARTH, true),
    ] {
        assert_eq!(
            s.receive_damage(
                Damage {
                    attack: attack::DAMAGE,
                    amount,
                    element: element.0,
                    magnitude: 1.
                },
                grease
            ),
            0
        );
    }
    let a = s.update_without_homing(0.1, true, &mut h);
    assert_eq!(a.hitpoints, 6.);
    assert_eq!(a.elements, (E::FIRE | E::COLD).0);
    assert_eq!(h.actions.last(), Some(&"kill"));
    assert!(s.is_damaged);
    assert_eq!(s.damage_amount, 0.);
    let a = s.update_without_homing(0.1, true, &mut h);
    assert_eq!(a.hitpoints, 0.);
    assert_eq!(a.elements, 0);
    assert!(a.flags & condition::DAMAGED != 0);
}
#[test]
fn deep_impact_awards_every_fifth_and_later_character_kill() {
    let mut s = state(false);
    s.deep_impact_possible = true;
    let mut h = Host {
        velocity: [50., 0., 0.],
        result: R::PIERCED | R::KILLED,
        ..Default::default()
    };
    for target in 2..8 {
        assert!(!s.on_collision(char_target(target), true, &mut h));
    }
    assert_eq!(s.deep_impact_count, 6);
    assert_eq!(h.awards, 2);
}
#[test]
fn default_initialization_and_permanent_exclusion_are_separate_from_hit_cooldown() {
    let r = MissileRecipe::from_spell(Spell::default_for_list(&[E::ICE]), 0., 3).unwrap();
    let mut s = MissileCollisionState::new(Some(1), false, r.conditions, false);
    assert_eq!(s.excluded_owner, Some(1));
    assert!(!s.hits.contains(1));
    assert!(s.piercing);
    let mut h = Host::default();
    s.time_alive = 99.;
    s.collision_target = Some(4);
    s.execute_default(&mut h);
    assert_eq!(s.time_alive, 0.);
    assert_eq!(s.collision_target, None);
    assert_eq!(h.selected, None);
    assert!(!h.selected_events.is_empty());
}
