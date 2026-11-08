use magicka_runtime::{
    level_collision::{SceneCollision, StaticLevelCollision},
    spray_world::*,
};
use magicka_sim::{
    character_impact::{ImpactCharacter, ImpactContext},
    character_status::{CueState, StatusContext},
    damage::{CastType, Damage},
    damage_delivery::{AttackerKind, DamagePayload, DeliveryContext, NetworkRole},
    impact_motion::MotionContext,
    resistance::Resistance,
    shield_intersection::{HollowSphere, ShieldGeometry},
    spells::Elements,
    spray_pulse::{HitList, SprayPulse, SpraySource},
};
use std::collections::BTreeMap;
const BASE: [Resistance; 11] = [Resistance::IDENTITY; 11];
fn context() -> ImpactContext<'static> {
    ImpactContext {
        status: StatusContext {
            immortal: false,
            ethereal: false,
            can_have_status: true,
            avatar: false,
            current_spell: false,
            base: &BASE,
            equipment: None,
            buffs: &[],
            cues: CueState::default(),
        },
        motion: MotionContext {
            mass: 70.0,
            ..Default::default()
        },
        attack_position: [0.0; 3],
        forward: [0.0, 0.0, 1.0],
        block_values: &[],
        block_item: -1,
        features: 15,
        network_client: false,
        attacker: None,
        attacker_is_self: false,
        attacker_avatar: false,
        attacker_local_avatar: false,
        attacker_is_gripped: false,
        grip_attack_state: false,
        hit_tolerance: 1000.0,
        resurrection_state: false,
        stun_time: 0.0,
        bleed_rate: 0.0,
        blood_and_gore: false,
        has_blood: false,
        bloat_killed: false,
        has_damage_trigger: false,
    }
}
#[derive(Default)]
struct Effects {
    combat: BTreeMap<u16, ImpactCharacter>,
    hits: Vec<SprayDamage>,
    order: Vec<(bool, u16)>,
    move_after_hit: Option<u16>,
    pushes: Vec<u16>,
    impulses: Vec<(u16, [f32; 3])>,
}
impl PushEffects for Effects {
    fn single_push(&mut self, _: &mut [WorldEntity], damage: SprayDamage) {
        self.pushes.push(damage.target);
    }
    fn impulse_push(&mut self, target: u16, direction: [f32; 3], _: f32, _: f32, _: f32) {
        self.impulses.push((target, direction));
    }
}
impl SprayEffects for Effects {
    fn damage(&mut self, entities: &mut [WorldEntity], damage: SprayDamage) {
        self.order.push((true, damage.target));
        let target = entities.iter().find(|e| e.handle == damage.target).unwrap();
        if let Some(combat) = self.combat.get_mut(&damage.target) {
            combat
                .deliver(
                    DamagePayload::Collection(damage.damages),
                    DeliveryContext {
                        network: NetworkRole::Offline,
                        attacker_kind: AttackerKind::Character,
                        target_handle: damage.target.into(),
                        attacker_handle: Some(damage.attacker.into()),
                        time_stamp: damage.timestamp,
                        target_position: target.position,
                        attack_position: damage.position,
                    },
                    &[],
                    &mut context(),
                    &mut vec![],
                )
                .unwrap();
        }
        self.hits.push(damage);
        if let Some(handle) = self.move_after_hit.take() {
            entities
                .iter_mut()
                .find(|e| e.handle == handle)
                .unwrap()
                .position = [0.0, 0.0, 100.0];
        }
    }
    fn electrocute(&mut self, _: &mut [WorldEntity], target: u16, _: u16, multiplier: f32) {
        assert_eq!(multiplier, 0.25);
        self.order.push((false, target));
    }
}
fn entity(handle: u16, z: f32) -> WorldEntity {
    WorldEntity {
        handle,
        position: [0.0, 0.0, z],
        radius: 0.5,
        dead: false,
        damageable: true,
        frozen: false,
        geometry: ArcGeometry::Character { ethereal: false },
    }
}
fn source() -> SpraySource {
    SpraySource {
        owner_handle: 1,
        owner_position: [0.0; 3],
        owner_direction: [0.0, 0.0, 1.0],
        cast_position: [0.0; 3],
        cast_direction: [0.0, 0.0, 1.0],
        weapon_position: [0.0; 3],
        weapon_direction: [0.0, 0.0, 1.0],
    }
}
fn pulse() -> SprayPulse {
    let mut damages = [Damage::default(); 5];
    damages[0] = Damage {
        attack: 1,
        element: Elements::EARTH.0,
        amount: 20.0,
        magnitude: 4.0,
    };
    damages[1].element = Elements::WATER.0;
    SprayPulse {
        damages,
        timestamp: 2.0,
        angle: 0.5,
        hits: HitList::default(),
    }
}

#[test]
fn pushes_classify_damageable_missiles_and_keep_force_arc_and_area_rules() {
    use magicka_sim::push_spell::PushSpell;
    for area in [false, true] {
        let mut entities = [entity(1, 0.), entity(2, 2.), entity(3, -2.), entity(4, 2.)];
        // A high airborne missile still uses MissileEntity's horizontal arc;
        // its full 3D position remains relevant to the radius query.
        entities[1].position[1] = 4.1;
        entities[1].geometry = ArcGeometry::Missile;
        entities[2].geometry = ArcGeometry::Missile;
        let mut effects = Effects::default();
        let mut world = RuntimeSprayWorld::new(None, &mut entities, &[], &mut effects).unwrap();
        let mut push = PushSpell::cast(
            if area {
                CastType::Area
            } else {
                CastType::Force
            },
            false,
            1.,
            &mut 1.,
            2.,
            [0., 0., 1.],
        )
        .unwrap();
        for _ in 0..14 {
            push.update(1. / 60., 1, [0.; 3], &mut world);
        }
        assert!(world.query_error.is_none());
        drop(world);
        assert_eq!(effects.pushes, [4], "characters retain public push damage");
        let mut targets: Vec<_> = effects.impulses.iter().map(|(h, _)| *h).collect();
        targets.sort();
        assert_eq!(targets, if area { vec![2, 3] } else { vec![2] });
        assert!(effects.impulses.iter().all(|(_, d)| d[1] == 0.));
    }
}
#[test]
fn real_delivery_changes_health_and_geometry_before_next_target_arc() {
    let mut entities = [entity(1, 0.0), entity(2, 2.0), entity(3, 3.0)];
    let mut effects = Effects {
        move_after_hit: Some(3),
        ..Default::default()
    };
    let mut character = ImpactCharacter::default();
    character.health.hit_points = 100.0;
    character.health.max_hit_points = 100.0;
    effects.combat.insert(2, character);
    let mut world = RuntimeSprayWorld::new(None, &mut entities, &[], &mut effects).unwrap();
    let mut p = pulse();
    p.execute(10.0, CastType::Force, false, source(), &mut world);
    assert!(world.query_error.is_none());
    drop(world);
    assert_eq!(effects.combat[&2].health.hit_points, 80.0);
    assert_eq!(effects.hits.len(), 1);
    assert_eq!(effects.hits[0].damages[0].magnitude, 1.0);
    assert_eq!(effects.hits[0].position, [0.0; 3]);
    assert_eq!(effects.order, [(false, 1), (true, 2), (false, 2)]);
    assert!(p.hits.contains(2) && !p.hits.contains(3));
}
#[test]
fn sphere_shield_receives_full_range_hit_and_removes_protected_character() {
    let mut entities = [entity(1, 0.0), entity(2, 4.0), entity(3, 6.0)];
    entities[1].radius = 1.0;
    entities[1].geometry = ArcGeometry::Shield(ShieldGeometry::Sphere(HollowSphere {
        center: [0.0, 0.0, 4.0],
        radius: 1.0,
        forward: [0.0, 0.0, 1.0],
        max_angle: std::f32::consts::PI,
    }));
    let mut effects = Effects::default();
    let mut world = RuntimeSprayWorld::new(None, &mut entities, &[2], &mut effects).unwrap();
    let mut p = pulse();
    p.execute(10.0, CastType::Force, false, source(), &mut world);
    p.execute(10.0, CastType::Force, false, source(), &mut world);
    drop(world);
    assert_eq!(
        effects.hits.iter().map(|d| d.target).collect::<Vec<_>>(),
        [2]
    );
    assert!(!p.hits.contains(3));
}
#[test]
fn scene_obstruction_reduces_actual_arc_and_ethereal_targets_are_rejected() {
    let mut layers = vec![magicka_content::level::TriMesh::default(); 10];
    layers[0] = magicka_content::level::TriMesh {
        vertices: vec![[-5.0, -5.0, 2.0], [5.0, -5.0, 2.0], [0.0, 5.0, 2.0]],
        triangles: vec![[0, 1, 2]],
    };
    let scene = SceneCollision {
        water: Vec::new(),
        static_skin: StaticLevelCollision::from_layers(&layers).unwrap(),
        force_fields: vec![],
        animated_parts: vec![],
    };
    let mut entities = [
        entity(1, 0.0),
        entity(2, 1.0),
        entity(3, 4.0),
        entity(4, 1.5),
    ];
    entities[3].geometry = ArcGeometry::Character { ethereal: true };
    let mut effects = Effects::default();
    let mut world = RuntimeSprayWorld::new(Some(&scene), &mut entities, &[], &mut effects).unwrap();
    pulse().execute(10.0, CastType::Force, false, source(), &mut world);
    drop(world);
    assert_eq!(
        effects.hits.iter().map(|d| d.target).collect::<Vec<_>>(),
        [2]
    );
}
#[test]
fn duplicate_identity_and_nonshield_membership_are_errors() {
    let mut effects = Effects::default();
    let mut entities = [entity(1, 0.0), entity(1, 1.0)];
    assert!(matches!(
        RuntimeSprayWorld::new(None, &mut entities, &[], &mut effects),
        Err(SprayWorldError::DuplicateHandle(1))
    ));
    let mut entities = [entity(1, 0.0)];
    assert!(matches!(
        RuntimeSprayWorld::new(None, &mut entities, &[1], &mut effects),
        Err(SprayWorldError::InvalidShield(1))
    ));
}
