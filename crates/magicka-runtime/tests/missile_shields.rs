use magicka_runtime::{
    missiles::{self, LiveMissile, MissileShield, MissileVisual, MissileWorld},
    npc_combat::CombatView,
};
use magicka_sim::{
    damage::Damage,
    impact_motion::Vector,
    projectile_launch::ProjectileLaunch,
    rigid_body::Gravity,
    shield_entity::{ShieldEntity, ShieldKind, SHIELD_COLOR},
    spells::{Elements as E, Spell},
};

struct World {
    view: CombatView<'static>,
    shields: Vec<(u16, ShieldEntity)>,
    damage_calls: usize,
}
impl World {
    fn new(kind: ShieldKind) -> Self {
        let shield = ShieldEntity::initialize(
            None,
            [0.0; 3],
            5.5,
            [1.0, 0.0, 0.0],
            kind,
            1000.0,
            SHIELD_COLOR,
            &mut [],
        );
        Self {
            view: CombatView::new(&[], None).unwrap(),
            shields: vec![(10, shield)],
            damage_calls: 0,
        }
    }
}
impl MissileWorld for World {
    fn view(&self) -> &CombatView<'_> {
        &self.view
    }
    fn shields(&self) -> Vec<MissileShield> {
        self.shields
            .iter()
            .map(|(handle, s)| MissileShield {
                handle: *handle,
                position: s.position,
                radius: s.radius,
                geometry: s.geometry(),
                dead: s.dead,
            })
            .collect()
    }
    fn damage_shield(&mut self, target: u16, damage: Damage) -> u16 {
        self.damage_calls += 1;
        self.shields
            .iter_mut()
            .find(|(h, _)| *h == target)
            .unwrap()
            .1
            .internal_damage(&damage, true)
    }
    fn collision_random_degrees(&mut self) -> u16 {
        panic!("nondegenerate contact must not draw RNG")
    }
    fn damage_character(&mut self, _: u16, _: Damage, _: u16, _: Vector) -> u16 {
        panic!("no characters")
    }
    fn scene_hit(&self, _: Vector, _: Vector) -> bool {
        false
    }
}
fn rock(position: Vector, velocity: Vector) -> LiveMissile {
    LiveMissile::spawn(
        1,
        123,
        false,
        &ProjectileLaunch {
            position,
            velocity,
            spell: Spell::default_for_list(&[E::EARTH]),
            splash: 0.0,
            total_projectiles: 1,
        },
    )
    .unwrap()
}
fn step(m: &mut LiveMissile, w: &mut World, visuals: &mut Vec<MissileVisual>) {
    missiles::physics_step(m, 1.0 / 60.0, Gravity::new([0.0; 3]), w, None, visuals);
}

#[test]
fn incoming_and_outgoing_rocks_damage_shields_and_bounce_in_opposite_directions() {
    for (position, velocity) in [(5.6, -50.0), (5.4, 50.0)] {
        let mut w = World::new(ShieldKind::Sphere);
        let mut m = rock([position, 0.0, 0.0], [velocity, 0.0, 0.0]);
        let mut visuals = Vec::new();
        step(&mut m, &mut w, &mut visuals);
        assert_eq!(w.shields[0].1.hit_points, 850.0);
        assert!(m.state.hits.contains(10));
        assert!(!m.body.dead);
        assert!((m.body.velocity[0] + velocity * 0.1).abs() < 0.001);
        // Pure Earth's Hit slot has damage and a bounce counter; its
        // effects belong to the separate Collision slot, not this contact.
        assert!(visuals.is_empty());
    }
}

#[test]
fn ordinary_hit_list_suppresses_damage_but_keeps_shield_physical_contact() {
    let mut w = World::new(ShieldKind::Sphere);
    let mut m = rock([5.6, 0.0, 0.0], [-50.0, 0.0, 0.0]);
    let mut visuals = Vec::new();
    step(&mut m, &mut w, &mut visuals);
    let calls = w.damage_calls;
    m.body.position = [5.6, 0.0, 0.0];
    m.body.velocity = [-50.0, 0.0, 0.0];
    step(&mut m, &mut w, &mut visuals);
    assert_eq!(w.damage_calls, calls);
    assert_eq!(w.shields[0].1.hit_points, 850.0);
    assert_eq!(m.body.velocity, [5.0, 0.0, 0.0]);
    let bounced = m.body.position;
    step(&mut m, &mut w, &mut visuals);
    assert!(
        m.body.position[0] > bounced[0],
        "separating missile must leave the shell"
    );
    assert_eq!(w.damage_calls, calls);
}

#[test]
fn disc_front_blocks_but_back_and_dead_shield_do_not() {
    for (position, dead, blocked) in [(5.6, false, true), (-5.6, false, false), (5.6, true, false)]
    {
        let mut w = World::new(ShieldKind::Disc);
        w.shields[0].1.dead = dead;
        let mut m = rock([position, 0.0, 0.0], [-position.signum() * 50.0, 0.0, 0.0]);
        step(&mut m, &mut w, &mut Vec::new());
        assert_eq!(
            w.shields[0].1.hit_points,
            if blocked { 850.0 } else { 1000.0 }
        );
        assert_eq!(m.state.hits.contains(10), blocked);
    }
}

#[test]
fn shield_entity_order_and_zero_hp_fade_preserve_native_contact_semantics() {
    let mut w = World::new(ShieldKind::Sphere);
    w.shields[0].1.hit_points = 0.0;
    // Coincident contact fixture without Initialize's overlap-kill path.
    w.shields.push((11, w.shields[0].1.clone()));
    let mut m = rock([5.6, 0.0, 0.0], [-50.0, 0.0, 0.0]);
    step(&mut m, &mut w, &mut Vec::new());
    assert!(m.state.hits.contains(10));
    assert!(!m.state.hits.contains(11));
    assert_eq!(w.shields[0].1.hit_points, -150.0);
    assert_eq!(w.shields[1].1.hit_points, 0.0);
    assert_eq!(m.body.velocity, [5.0, 0.0, 0.0]);
}

#[test]
fn ice_piercing_damages_shields_without_passing_through_them() {
    let mut w = World::new(ShieldKind::Sphere);
    let mut m = LiveMissile::spawn(
        1,
        123,
        false,
        &ProjectileLaunch {
            position: [5.6, 0.0, 0.0],
            velocity: [-50.0, 0.0, 0.0],
            spell: Spell::default_for_list(&[E::ICE]),
            splash: 0.0,
            total_projectiles: 3,
        },
    )
    .unwrap();
    let mut visuals = Vec::new();
    step(&mut m, &mut w, &mut visuals);
    assert!(w.shields[0].1.hit_points < 1000.0);
    assert!(m.state.hits.contains(10));
    assert!(m.state.gibbed_hits.is_empty());
    assert!(!m.body.dead);
    assert_eq!(m.body.velocity, [5.0, 0.0, 0.0]);
    assert!(visuals.iter().any(|v| matches!(
        v,
        MissileVisual::Effect {
            name: "projectilehit_ice",
            ..
        }
    )));
}

#[test]
fn missile_blast_reaches_shield_with_collection_falloff() {
    use magicka_sim::{
        damage::attack,
        missile_events::{MissileEvent, MissileEventHost},
    };
    let mut w = World::new(ShieldKind::Sphere);
    let mut m = rock([7.0, 0.0, 0.0], [-50.0, 0.0, 0.0]);
    let mut visuals = Vec::new();
    let mut damages = [Damage::default(); 5];
    damages[0] = Damage {
        attack: attack::DAMAGE,
        element: E::FIRE.0,
        amount: 100.0,
        magnitude: 1.0,
    };
    missiles::MissileHost {
        body: &mut m.body,
        world: &mut w,
        visuals: &mut visuals,
        target: None,
        entity_handle: None,
    }
    .event(
        &MissileEvent::Blast {
            radius: 2.0,
            damages,
        },
        &mut Some([7.0, 0.0, 0.0]),
    );
    // Skin clipping occurs before subtracting the entity radius, so this
    // sphere receives full damage. A disc facing away does not clip.
    assert_eq!(w.shields[0].1.hit_points, 900.0);
    let mut w = World::new(ShieldKind::Disc);
    w.shields[0].1.forward = [-1.0, 0.0, 0.0];
    missiles::MissileHost {
        body: &mut m.body,
        world: &mut w,
        visuals: &mut visuals,
        target: None,
        entity_handle: None,
    }
    .event(
        &MissileEvent::Blast {
            radius: 2.0,
            damages,
        },
        &mut Some([7.0, 0.0, 0.0]),
    );
    assert_eq!(w.shields[0].1.hit_points, 942.1875);
    // Native collection test disables falloff for Fire too when any Beam
    // slot is present; it does not scale each slot independently.
    damages[1] = Damage {
        attack: attack::DAMAGE,
        element: E::ARCANE.0,
        amount: 10.0,
        magnitude: 1.0,
    };
    missiles::MissileHost {
        body: &mut m.body,
        world: &mut w,
        visuals: &mut visuals,
        target: None,
        entity_handle: None,
    }
    .event(
        &MissileEvent::Blast {
            radius: 2.0,
            damages,
        },
        &mut Some([7.0, 0.0, 0.0]),
    );
    assert_eq!(w.shields[0].1.hit_points, 832.1875);
}
