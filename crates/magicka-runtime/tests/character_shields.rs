use magicka_runtime::{character::ActiveAura, play_session::PlaySession};
use magicka_sim::{
    character_health::{SelfShield, SelfShieldType},
    resistance::Resistance,
    shield_entity::{ShieldKind, SHIELD_COLOR},
    spells::{Elements as E, Spell},
};
use std::path::{Path, PathBuf};

const DT: f32 = 1.0 / 60.0;
fn session() -> Option<PlaySession> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    Some(s)
}
fn fixture(s: &mut PlaySession, x: f32, velocity: f32) -> u16 {
    let index = s.avatar().body;
    let b = &mut s.world.physics.bodies[index];
    b.position = [x, 100.0, 0.0];
    b.velocity = [0.0; 3];
    b.apply_gravity = false;
    // Let EnableBody's native 0.2 s ground grace expire above terrain.
    for _ in 0..20 {
        s.step(DT);
    }
    s.world.physics.bodies[index].velocity = [velocity, 0.0, 0.0];
    s.world.spawn_shield(
        s.avatar().handle,
        [0.0, 100.0, 0.0],
        5.5,
        [1.0, 0.0, 0.0],
        ShieldKind::Sphere,
        1000.0,
        SHIELD_COLOR,
    )
}

#[test]
fn round_shield_blocks_both_sides_without_terrain_damage_or_ground_contact() {
    for (x, velocity, outward) in [(6.0, -20.0, true), (5.0, 20.0, false)] {
        let Some(mut s) = session() else { return };
        fixture(&mut s, x, velocity);
        let hp = s.avatar().character.combat.health.hit_points;
        s.step(DT);
        assert!(!s.world.physics.contact_points(s.avatar().body).is_empty());
        if outward {
            assert!(s.body().velocity[0] > 0.0);
        } else {
            assert!(s.body().velocity[0] < 0.0);
        }
        assert_eq!(s.avatar().character.combat.health.hit_points, hp);
        assert!(!s.body().is_touching_ground());
    }
}

#[test]
fn disc_blocks_front_and_allows_back_contacts() {
    for (x, velocity, blocked) in [(6.0, -20.0, true), (-6.0, 20.0, false)] {
        let Some(mut s) = session() else { return };
        fixture(&mut s, x, velocity);
        s.world.shields[0].shield.kind = ShieldKind::Disc;
        s.step(DT);
        assert_eq!(
            !s.world.physics.contact_points(s.avatar().body).is_empty(),
            blocked
        );
        assert_eq!(
            s.body().velocity[0].signum(),
            if blocked { 1.0 } else { velocity.signum() }
        );
    }
}

#[test]
fn dead_character_does_not_nullify_but_zero_health_self_shield_type_still_does() {
    let Some(mut s) = session() else { return };
    fixture(&mut s, 6.0, -20.0);
    let mut character = s.avatar().character.clone();
    character.combat.health.shield.kind = SelfShieldType::Shield;
    character.combat.health.shield.health = 0.0;
    let mut body = s.body().clone();
    let mut shield = s.world.shields[0].shield.clone();
    let (accept, effect) =
        magicka_runtime::character_shield::contact(&mut character, &mut body, true, &mut shield);
    assert!(!accept);
    assert!(effect.is_none());
    assert_eq!(character.combat.health.shield.kind, SelfShieldType::Shield);
    assert_eq!(shield.intersect_kill_timer, 0.0);
    let (accept, effect) =
        magicka_runtime::character_shield::contact(&mut character, &mut body, false, &mut shield);
    assert!(!accept);
    assert!(effect.is_some());
    assert_eq!(character.combat.health.shield.kind, SelfShieldType::None);
    assert_eq!(shield.intersect_kill_timer, 0.25);
}

#[test]
fn fading_intersecting_and_ethereal_contacts_do_not_block() {
    for (hp, timer, ethereal) in [
        (0.0, 0.0, false),
        (1000.0, 0.25, false),
        (1000.0, 0.0, true),
    ] {
        let Some(mut s) = session() else { return };
        fixture(&mut s, 6.0, -20.0);
        s.world.shields[0].shield.hit_points = hp;
        s.world.shields[0].shield.intersect_kill_timer = timer;
        s.world.shields[0].shield.tint[3] = 1.0;
        s.world.players[0].as_mut().unwrap().character.ethereal = ethereal;
        s.step(DT);
        assert!(s.world.physics.contact_points(s.avatar().body).is_empty());
        assert!(s.body().velocity[0] < 0.0);
    }
}

#[test]
fn pure_self_shield_nullifies_even_when_shield_callback_rejects_and_character_is_ethereal() {
    for (hp, timer, ethereal) in [(1000.0, 0.0, false), (0.0, 0.1, true)] {
        let Some(mut s) = session() else { return };
        fixture(&mut s, 6.0, -20.0);
        s.world.shields[0].shield.hit_points = hp;
        s.world.shields[0].shield.intersect_kill_timer = timer;
        s.world.shields[0].shield.tint[3] = 1.0;
        let avatar = s.world.players[0].as_mut().unwrap();
        avatar.character.ethereal = ethereal;
        avatar.character.combat.health.shield =
            SelfShield::from_spell(&Spell::default_for_list(&[E::SHIELD]), &mut Vec::new());
        avatar.character.auras.push(ActiveAura {
            resistance: Resistance::IDENTITY,
            ttl: 10.0,
            start_ttl: 10.0,
            self_casted: true,
            self_only: false,
            visual: magicka_runtime::buffs::AuraVisual::new(1.5, [1.0; 3]),
        });
        let body = avatar.body;
        let mass = avatar.character.definition.mass;
        s.world.physics.bodies[body].mass = 1234.0;
        s.world.step_live(None, DT).unwrap();
        assert_eq!(
            s.avatar().character.combat.health.shield.kind,
            SelfShieldType::None
        );
        assert!(s.avatar().character.auras.is_empty());
        assert_eq!(s.body().mass, mass);
        assert!(s.world.physics.contact_points(body).is_empty());
        assert_eq!(s.world.shield_nullifications.len(), 1);
        let effect = s.world.shield_nullifications[0];
        assert!((effect.position[0] - 5.5).abs() < 0.001);
        assert!((effect.position[1] - 100.0).abs() < 0.001);
        assert!((s.world.shields[0].shield.intersect_kill_timer - (0.25 - DT)).abs() < 0.001);
        // The queued native effect reaches PlaySession's effect system.
        let effects = s.effects.active_effects();
        s.step(DT);
        assert!(s.world.shield_nullifications.is_empty());
        assert!(s.effects.active_effects() > effects);
    }
}

fn wall_fixture(s: &mut PlaySession, x: f32, velocity: f32) {
    fixture(s, x, velocity);
    // Use the native local box orientation: X thickness, Z long axis.
    let shield = &mut s.world.shields[0].shield;
    shield.kind = ShieldKind::Wall;
    shield.forward = [0.0, 0.0, -1.0];
    shield.right = [1.0, 0.0, 0.0];
    shield.up = [0.0, 1.0, 0.0];
}

#[test]
fn wall_blocks_both_faces_and_respects_ends_without_terrain_side_effects() {
    for (x, velocity, z, blocked) in [
        (0.9, -10.0, 0.0, true),
        (-0.9, 10.0, 0.0, true),
        (0.9, -10.0, 5.8, true),
        (0.9, -10.0, 7.0, false),
    ] {
        let Some(mut s) = session() else { return };
        wall_fixture(&mut s, x, velocity);
        let index = s.avatar().body;
        s.world.physics.bodies[index].position[2] = z;
        let hp = s.avatar().character.combat.health.hit_points;
        s.step(DT);
        assert_eq!(!s.world.physics.contact_points(index).is_empty(), blocked);
        if blocked {
            assert!(s.body().velocity[0] * x > 0.0, "{:?}", s.body().velocity);
        } else {
            assert_eq!(s.body().velocity[0], velocity);
        }
        assert_eq!(s.avatar().character.combat.health.hit_points, hp);
        assert!(!s.body().is_touching_ground());
    }
}

#[test]
fn wall_prediction_contact_and_rejected_contacts_preserve_native_acceptance() {
    for (hp, timer, ethereal, dead, blocked) in [
        (1000.0, 0.0, false, false, true),
        (0.0, 0.0, false, false, false),
        (1000.0, 0.25, false, false, false),
        (1000.0, 0.0, true, false, false),
        (1000.0, 0.0, false, true, false),
    ] {
        let Some(mut s) = session() else { return };
        wall_fixture(&mut s, 1.2, -30.0);
        s.world.shields[0].shield.hit_points = hp;
        s.world.shields[0].shield.intersect_kill_timer = timer;
        s.world.shields[0].shield.tint[3] = 1.0;
        let avatar = s.world.players[0].as_mut().unwrap();
        avatar.character.ethereal = ethereal;
        avatar.character.combat.dead = dead;
        let index = avatar.body;
        s.world.step_live(None, DT).unwrap();
        assert_eq!(!s.world.physics.contact_points(index).is_empty(), blocked);
        if blocked {
            assert!(s.body().velocity[0] > -30.0);
        }
    }
}

#[test]
fn pure_self_shield_nullifies_wall_at_line_projection_with_character_height() {
    for ethereal in [false, true] {
        let Some(mut s) = session() else { return };
        wall_fixture(&mut s, 0.9, -10.0);
        let avatar = s.world.players[0].as_mut().unwrap();
        avatar.character.ethereal = ethereal;
        avatar.character.combat.health.shield =
            SelfShield::from_spell(&Spell::default_for_list(&[E::SHIELD]), &mut Vec::new());
        let index = avatar.body;
        let mass = avatar.character.definition.mass;
        s.world.physics.bodies[index].mass = 1234.0;
        s.world.physics.bodies[index].position[1] = 101.0;
        s.world.physics.bodies[index].position[2] = 2.0;
        s.world.step_live(None, DT).unwrap();
        assert_eq!(
            s.avatar().character.combat.health.shield.kind,
            SelfShieldType::None
        );
        assert!(s.world.physics.contact_points(index).is_empty());
        assert_eq!(s.body().mass, mass);
        assert_eq!(s.world.shield_nullifications.len(), 1);
        let point = s.world.shield_nullifications[0].position;
        assert!((point[0]).abs() < 0.001);
        assert!((point[1] - 101.0).abs() < 0.001);
        assert!((point[2] - 2.0).abs() < 0.001);
        assert!((s.world.shields[0].shield.intersect_kill_timer - (0.25 - DT)).abs() < 0.001);
    }
}

#[test]
fn npc_capsule_is_blocked_by_wall_through_the_same_live_solver() {
    use magicka_runtime::{
        play_content::load_start_scene,
        scene_script::{ActionDefinition, SceneScriptLayout},
        spawn_action::SpawnState,
    };
    let Some(mut s) = session() else { return };
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    let area = load_start_scene(&root, "Havindr.lvl")
        .unwrap()
        .start
        .area
        .unwrap();
    let xml = format!(
        r#"<Scene><trigger id="t"><then><spawn type="Goblin_miner" area="{area}1" order="idle"/></then></trigger></Scene>"#
    );
    let ActionDefinition::Spawn(action) = SceneScriptLayout::read(xml.as_bytes(), false)
        .unwrap()
        .actions
        .remove(0)
    else {
        panic!()
    };
    let mut state = SpawnState::default();
    action.initialize(&mut state, &mut s.world).unwrap();
    action.execute(&mut state, &mut s.world).unwrap();
    let handle = state.spawned[0].handle.unwrap();
    wall_fixture(&mut s, 20.0, 0.0);
    let index = s.world.npcs[&handle].body;
    let body = &mut s.world.physics.bodies[index];
    body.apply_gravity = false;
    body.position = [0.2 + body.radius - 0.05, 100.0, 0.0];
    body.velocity = [-10.0, 0.0, 0.0];
    body.add_external_forces(0.3, None);
    let hp = s.world.npcs[&handle].character.combat.health.hit_points;
    s.world.step_live(None, DT).unwrap();
    assert!(!s.world.physics.contact_points(index).is_empty());
    assert!(s.world.physics.bodies[index].velocity[0] > 0.0);
    assert!(!s.world.physics.bodies[index].is_touching_ground());
    assert_eq!(s.world.npcs[&handle].character.combat.health.hit_points, hp);
}
