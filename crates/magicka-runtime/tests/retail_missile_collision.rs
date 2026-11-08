//! Original Ice recipe and contact state -> live avatar single-damage delivery.
//! This verifies the damage adapter independently of pending missile flight.
use magicka_runtime::{
    avatar::{PlayerAvatar, PlayerInput},
    play_session::PlaySession,
};
use magicka_sim::{
    character_physics::CharacterBody,
    damage::Damage,
    impact_motion::Vector,
    missile_collision::{
        CollisionTarget, Contact, MissileCollisionHost, MissileCollisionState, TargetKind,
    },
    missile_events::{MissileEvent, MissileEventHost, MissileRecipe},
    spells::{Elements as E, Spell},
};
struct Host<'a> {
    avatar: &'a mut PlayerAvatar,
    body: &'a mut CharacterBody,
    selected: Option<u16>,
    pos: Vector,
    velocity: Vector,
    time: f64,
    deliveries: usize,
    killed: bool,
}
impl MissileEventHost for Host<'_> {
    fn damageable_target(&self) -> bool {
        self.selected == Some(self.avatar.handle)
    }
    fn normalized_velocity(&self) -> f32 {
        ((self.velocity.iter().map(|v| v * v).sum::<f32>()) as f64).sqrt() as f32 / 50.
    }
    fn damage(&mut self, d: Damage) -> u16 {
        self.deliveries += 1;
        self.avatar
            .receive_damage(self.body, d, 123, self.pos, self.time)
    }
    fn kill(&mut self) {
        self.killed = true;
    }
    fn event(&mut self, _: &MissileEvent, _: &mut Option<Vector>) {} // presentation outside this fixture
}
impl MissileCollisionHost for Host<'_> {
    fn position(&self) -> Vector {
        self.pos
    }
    fn velocity(&self) -> Vector {
        self.velocity
    }
    fn set_velocity(&mut self, v: Vector) {
        self.velocity = v;
    }
    fn target_position(&self, handle: u16) -> Vector {
        assert_eq!(handle, self.avatar.handle);
        self.body.position
    }
    fn select_target(&mut self, target: Option<u16>) {
        self.selected = target;
    }
    fn update_followed_effects(&mut self) {}
    fn award_deep_impact(&mut self) {
        panic!("Ice cannot award Earth achievement")
    }
}
#[test]
fn live_scene_missile_flight_damages_round_shield() {
    use magicka_runtime::missiles::LiveMissile;
    use magicka_sim::{
        projectile_launch::ProjectileLaunch,
        shield_entity::{ShieldKind, SHIELD_COLOR},
    };
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../../target/Magicka/Content")
        });
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        s.step(1.0 / 60.0);
    }
    // Place the controlled contact away from Havindr's terrain/characters.
    // This exercises SceneHitWorld, physics and shield update together.
    let mut center = s.body().position;
    center[1] += 20.0;
    let owner = s.avatar().handle;
    let shield = s.world.spawn_shield(
        owner,
        center,
        5.5,
        [1.0, 0.0, 0.0],
        ShieldKind::Sphere,
        1000.0,
        SHIELD_COLOR,
    );
    let mut position = center;
    position[0] += 5.6;
    s.world.missiles.push(
        LiveMissile::spawn(
            999,
            owner,
            true,
            &ProjectileLaunch {
                position,
                velocity: [-50.0, 0.0, 0.0],
                spell: Spell::default_for_list(&[E::EARTH]),
                splash: 0.0,
                total_projectiles: 1,
            },
        )
        .unwrap(),
    );
    s.step(1.0 / 60.0);
    let m = s.world.missiles.iter().find(|m| m.body.id == 999).unwrap();
    assert!(m.state.hits.contains(shield));
    assert!((m.body.velocity[0] - 5.0).abs() < 0.001);
    // 150 damage (slightly more from gravity's speed), then 10 decay.
    let hp = s
        .world
        .shields
        .iter()
        .find(|s| s.handle == shield)
        .unwrap()
        .shield
        .hit_points;
    assert!((hp - 840.0).abs() < 0.01, "shield HP {hp}");
}

#[test]
fn ice_missile_contact_damages_a_retail_wizard_and_pierces_without_second_hit() {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../../target/Magicka/Content")
        });
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        s.step(1. / 60.);
        s.handle_input(PlayerInput::default(), 16. / 9.);
    }
    let index = s.avatar().body;
    let handle = s.avatar().handle;
    let hp = s.avatar().character.combat.health.hit_points;
    let mut pos = s.body().position;
    pos[2] += 1.;
    let recipe = MissileRecipe::from_spell(Spell::default_for_list(&[E::ICE]), 0., 3).unwrap();
    let mut missile = MissileCollisionState::new(Some(123), true, recipe.conditions, false);
    let mut host = Host {
        avatar: s.world.players[0].as_mut().unwrap(),
        body: &mut s.world.physics.bodies[index],
        selected: None,
        pos,
        velocity: [0., 0., -50.],
        time: s.play_time,
        deliveries: 0,
        killed: false,
    };
    let contact = Contact::Body {
        physics_object: false,
        target: Some(CollisionTarget {
            handle,
            kind: TargetKind::Character { ethereal: false },
            dead: false,
            damageable: true,
        }),
    };
    assert!(!missile.on_collision(contact, true, &mut host));
    let after = host.avatar.character.combat.health.hit_points;
    assert!(after < hp);
    assert_eq!(host.deliveries, 4);
    assert!(!host.killed);
    assert!(missile.gibbed_hits.contains_key(&handle));
    assert!(missile.pending_velocity.is_none());
    assert!(!missile.on_collision(contact, true, &mut host));
    assert_eq!(host.deliveries, 4);
    assert_eq!(host.avatar.character.combat.health.hit_points, after);
    eprintln!("Ice missile: Wizard {hp} -> {after} HP; four single damage events, pierced contact, duplicate suppressed");
}
