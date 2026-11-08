use super::*;
use crate::{npc_combat::MeleeWeapon, play_session::PlaySession};
use magicka_sim::shield_intersection::{HollowSphere, ShieldGeometry};

fn sphere(center: [f32; 3]) -> ShieldGeometry {
    ShieldGeometry::Sphere(HollowSphere {
        center,
        radius: 0.5,
        forward: [0., 0., 1.],
        max_angle: std::f32::consts::PI,
    })
}
#[test]
fn shield_contact_ignores_radius_keeps_native_second_segment_and_list_order() {
    let mut w = MeleeWeapon::new(100., false, Arc::new(Vec::new()));
    w.attach = [-3., 0., 0.];
    w.last_attach = w.attach;
    let hit = w
        .absorb_shield_contact([0.; 3], &[(7, sphere([2., 0., 0.]))])
        .unwrap();
    assert_eq!(hit.0, 7);
    assert!((hit.1[0] - 1.5).abs() < 1e-5);
    assert!(w
        .absorb_shield_contact([0.; 3], &[(8, sphere([0., 3., 0.]))])
        .is_none());
    w.last_attach = [3., 0., 0.];
    assert_eq!(
        w.absorb_shield_contact([0.; 3], &[(9, sphere([2., 0., 0.])), (10, sphere([0.; 3]))])
            .unwrap()
            .0,
        9
    );
}
#[test]
fn knockback_kill_and_deflection_control_persistent_continuation() {
    let mut w = MeleeWeapon::new(1., false, Arc::new(Vec::new()));
    assert!(w.absorb_can_execute());
    w.hitlist.push(5);
    assert!(!w.absorb_result(0x200));
    assert!(w.absorb_can_execute());
    assert!(w.absorb_result(0x100));
    assert!(!w.absorb_can_execute());
    w.clear_hitlist();
    assert!(w.absorb_can_execute());
    assert!(!w.absorb_result(0));
    w.hitlist.push(6);
    assert!(!w.absorb_can_execute());
}
fn session() -> Option<(PlaySession, u16)> {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "BattleField.lvl", "Wizard").unwrap();
    s.change_scene_named("bf_s4BF", "start").unwrap();
    s.disable_scripts();
    s.world
        .boss_action(
            &crate::boss_action::BossAction {
                delay: 0.,
                command: crate::boss_action::BossCommand::Spawn {
                    boss: "warlord".into(),
                    prop: false,
                    generic: false,
                    area: custom_hash("boss_spawn"),
                    unique_id: 0,
                    mesh: -1,
                    on_death: 0,
                    on_damage: 0,
                    setup: Default::default(),
                },
            },
            false,
        )
        .unwrap()
        .unwrap();
    let boss_live::BossRef::Warlord(b) = &s.world.boss_fight.bosses[0] else {
        panic!()
    };
    let handle = b.handle;
    Some((s, handle))
}
#[test]
fn shield_interception_damages_shield_recoils_and_preserves_absorbed_queue() {
    let Some((mut s, h)) = session() else { return };
    let position = s.world.physics.bodies[s.world.npcs[&h].body].position;
    let player = s.avatar().handle;
    let shield = s.world.spawn_shield(
        player,
        position,
        1.,
        [0., 0., 1.],
        magicka_sim::shield_entity::ShieldKind::Sphere,
        10000.,
        [1.; 3],
    );
    let npc = s.world.npcs.get_mut(&h).unwrap();
    npc.character.combat.absorb_shield.as_mut().unwrap().add(
        Spell::default_for(magicka_sim::spells::Elements::ARCANE),
        false,
        &mut Vec::new(),
    );
    let weapon = &mut npc
        .weapons
        .iter_mut()
        .find(|(slot, _)| *slot == 0)
        .unwrap()
        .1;
    weapon.attach = [position[0] - 2., position[1], position[2]];
    weapon.last_attach = [position[0] + 2., position[1], position[2]];
    weapon.clear_hitlist();
    npc.melee_requests
        .push(crate::npc_combat::MeleeRequest::Execute {
            weapon: 0,
            targets: 0,
            first: true,
        });
    s.world.execute_melee();
    let n = &s.world.npcs[&h];
    assert_eq!(n.motion.controller.current_animation, 34);
    assert_eq!(
        n.character
            .combat
            .absorb_shield
            .as_ref()
            .unwrap()
            .queue()
            .len(),
        1
    );
    assert!(n
        .weapons
        .iter()
        .find(|(slot, _)| *slot == 0)
        .unwrap()
        .1
        .hitlist
        .is_empty());
    assert!(
        s.world
            .shields
            .iter()
            .find(|s| s.handle == shield)
            .unwrap()
            .shield
            .hit_points
            < 10000.
    );
}
