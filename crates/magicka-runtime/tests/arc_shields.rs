//! Helper.ArcDamage must use EntityManager's shield-protection filter.
use magicka_runtime::play_session::PlaySession;
use magicka_sim::{
    damage::{attack, Damage},
    resistance::Resistance,
    shield_entity::{ShieldKind, SHIELD_COLOR},
    spells::Elements,
};
use std::path::Path;

#[test]
fn arc_shields_filter_obstructed_targets_but_take_damage_themselves() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let npc = s.spawn_npc("Goblin_miner", "idle", 4.).unwrap();
    let body = s.world.npcs[&npc].body;
    let orientation = s.world.physics.bodies[body].orientation;
    s.world.physics.bodies[body].move_to([6., 100., 0.], orientation);
    let mut props = Vec::new();
    for x in [1., 6.] {
        let mut transform = magicka_content::skinned::IDENTITY;
        transform[12..15].copy_from_slice(&[x, 100., 0.]);
        let handle = s.spawn_physics_prop("dungeon_wall", transform).unwrap();
        let state = s.world.physics_damage.get_mut(&handle).unwrap();
        state.resistances.fill(Resistance::IDENTITY);
        // Retain the decoded prop geometry while isolating repeated damage.
        state.health = 1000.;
        state.maximum = 1000.;
        props.push(handle);
    }
    let damage = Damage {
        attack: attack::DAMAGE,
        element: Elements::FIRE.0,
        amount: 10.,
        magnitude: 1.,
    };
    for kind in [ShieldKind::Sphere, ShieldKind::Disc, ShieldKind::Wall] {
        s.world.shields.clear();
        let direction = if kind == ShieldKind::Wall {
            [0., 0., -1.]
        } else {
            [-1., 0., 0.]
        };
        let shield =
            s.world
                .spawn_shield(0, [3., 100., 0.], 1., direction, kind, 5000., SHIELD_COLOR);
        let before: Vec<_> = props
            .iter()
            .map(|h| s.world.physics_damage[h].health)
            .collect();
        let npc_before = s.world.npcs[&npc].character.combat.health.hit_points;
        assert!(s.world.arc_damage(
            None,
            None,
            [0., 100., 0.],
            [1., 0., 0.],
            10.,
            std::f32::consts::PI,
            &[damage],
            0.
        ));
        assert!(
            s.world.physics_damage[&props[0]].health < before[0],
            "{kind:?}: exposed prop"
        );
        assert_eq!(
            s.world.physics_damage[&props[1]].health, before[1],
            "{kind:?}: protected prop"
        );
        assert!(
            s.world.shields[0].shield.hit_points < 5000.,
            "{kind:?}: shield receives damage"
        );
        assert_eq!(
            s.world.npcs[&npc].character.combat.health.hit_points, npc_before,
            "{kind:?}: protected character"
        );
        // Native GetEntities filters protection before ArcDamage removes ignore.
        let shield_health = s.world.shields[0].shield.hit_points;
        s.world.arc_damage(
            None,
            Some(shield),
            [0., 100., 0.],
            [1., 0., 0.],
            10.,
            std::f32::consts::PI,
            &[damage],
            0.,
        );
        assert_eq!(s.world.shields[0].shield.hit_points, shield_health);
        assert_eq!(s.world.physics_damage[&props[1]].health, before[1]);
        // One query snapshot protects the target even when this hit depletes
        // the shield before damage delivery reaches later candidates.
        s.world.shields[0].shield.hit_points = 1.;
        s.world.arc_damage(
            None,
            None,
            [0., 100., 0.],
            [1., 0., 0.],
            10.,
            std::f32::consts::PI,
            &[damage],
            0.,
        );
        assert!(s.world.shields[0].shield.hit_points <= 0.);
        assert_eq!(s.world.physics_damage[&props[1]].health, before[1]);
        assert_eq!(
            s.world.npcs[&npc].character.combat.health.hit_points,
            npc_before
        );
        // A removed/dead shield no longer participates in GetEntities.
        s.world.shields[0].shield.dead = true;
        s.world.arc_damage(
            None,
            None,
            [0., 100., 0.],
            [1., 0., 0.],
            10.,
            std::f32::consts::PI,
            &[damage],
            0.,
        );
        assert!(
            s.world.physics_damage[&props[1]].health < before[1],
            "{kind:?}: dead-shield control"
        );
        assert!(
            s.world.npcs[&npc].character.combat.health.hit_points < npc_before,
            "{kind:?}: character dead-shield control"
        );
    }
}
