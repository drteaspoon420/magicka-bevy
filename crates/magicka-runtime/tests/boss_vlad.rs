use magicka_runtime::{
    boss_action::{BossAction, BossCommand, BossSetup},
    boss_vlad::{Kind, Phase},
    play_session::PlaySession,
    scene_loading::custom_hash,
    scene_world::boss_live::BossRef,
    script_runtime::ScriptWorld,
};
use magicka_sim::status::{Status as S, StatusEffect};
use std::path::{Path, PathBuf};
const DT: f32 = 1. / 60.;
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
}
fn command(s: &mut PlaySession, command: BossCommand) {
    s.world
        .boss_action(&BossAction { delay: 0., command }, false)
        .unwrap()
        .unwrap();
}
fn spawn(class: &str) -> Option<(PlaySession, u16)> {
    if !root().is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root(), "Swamp.lvl", "Wizard").unwrap();
    s.change_scene_named("sw_s7BF", "start").unwrap();
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    for _ in 0..60 {
        s.step(DT);
    }
    let area = custom_hash("boss_spawn");
    command(
        &mut s,
        BossCommand::Spawn {
            boss: class.into(),
            prop: false,
            generic: false,
            area,
            unique_id: custom_hash("vlad_unique"),
            mesh: -1,
            on_death: 0,
            on_damage: 0,
            setup: BossSetup {
                delayed: true,
                ..Default::default()
            },
        },
    );
    let BossRef::Vlad(v) = &s.world.boss_fight.bosses[0] else {
        panic!("dedicated Vlad encounter")
    };
    let handle = v.handle;
    Some((s, handle))
}
fn phase(s: &PlaySession) -> Phase {
    let BossRef::Vlad(v) = &s.world.boss_fight.bosses[0] else {
        panic!()
    };
    v.phase
}
#[test]
fn delayed_encounters_idle_until_activated_and_keep_native_item_visibility() {
    for class in ["vlad", "vlad2", "vlad3"] {
        let Some((mut s, h)) = spawn(class) else {
            return;
        };
        let n = &s.world.npcs[&h];
        assert!(n.vlad_character && n.character.combat.cannot_die_without_explicit_kill);
        assert_eq!(
            n.equipment_visible,
            Kind::from_class(class).unwrap().equipment_visible()
        );
        assert_eq!(n.ai.order, 0);
        assert_eq!(n.react_to, 0);
        for _ in 0..60 {
            s.step(DT);
        }
        assert_eq!(phase(&s), Phase::Intro);
        assert_eq!(s.world.npcs[&h].ai.order, 0);
        command(&mut s, BossCommand::Activate);
        s.step(DT);
        let n = &s.world.npcs[&h];
        assert_eq!(phase(&s), Phase::Battle);
        assert_eq!(n.ai.order, 2);
        assert_eq!(n.react_to, 3);
        assert!((s.world.physics.bodies[n.body].speed_multiplier() - 1.5).abs() < 1e-5);
        let e = s.world.entities.iter().find(|e| e.handle == h).unwrap();
        assert_eq!(
            e.unique_id,
            if class == "vlad3" {
                custom_hash("vlad_unique")
            } else {
                0
            }
        );
    }
}
#[test]
fn zero_health_uses_distinct_native_outros_without_synthesizing_boss_dead() {
    for (class, expected) in [
        ("vlad", Phase::Outro),
        ("vlad2", Phase::Battle),
        ("vlad3", Phase::Outro),
    ] {
        let Some((mut s, h)) = spawn(class) else {
            return;
        };
        command(&mut s, BossCommand::Activate);
        s.world
            .npcs
            .get_mut(&h)
            .unwrap()
            .character
            .combat
            .health
            .hit_points = 0.;
        s.step(DT);
        assert_eq!(phase(&s), expected);
        assert!(!s.world.boss_fight.dead);
        assert_eq!(s.world.boss_fight.normalized_health, 0.);
        assert_eq!(s.world.npcs.contains_key(&h), class != "vlad3");
        if class == "vlad3" {
            assert_eq!(
                s.effects
                    .active_effects_by_hash(custom_hash("special_fairy_crash")),
                1
            );
        }
    }
}
#[test]
fn mortal_message_kills_swamp_and_diplomat_but_is_noop_for_spirit() {
    for class in ["vlad", "vlad2", "vlad3"] {
        let Some((mut s, h)) = spawn(class) else {
            return;
        };
        command(&mut s, BossCommand::Message(5));
        assert_eq!(s.world.npcs.contains_key(&h), class == "vlad3");
        assert!(!s.world.boss_fight.dead);
    }
}
#[test]
fn frozen_diplomat_and_spirit_stop_wet_frozen_and_cold_slots() {
    for class in ["vlad2", "vlad3"] {
        let Some((mut s, h)) = spawn(class) else {
            return;
        };
        command(&mut s, BossCommand::Activate);
        let n = s.world.npcs.get_mut(&h).unwrap();
        for kind in [S::Wet, S::Frozen, S::Cold] {
            n.character
                .combat
                .statuses
                .set_effect(StatusEffect {
                    kind,
                    magnitude: 1.,
                    ..Default::default()
                })
                .unwrap();
        }
        n.character.combat.statuses.current_mask =
            S::Wet as u16 | S::Frozen as u16 | S::Cold as u16;
        s.step(DT);
        let statuses = &s.world.npcs[&h].character.combat.statuses;
        for kind in [S::Wet, S::Frozen, S::Cold] {
            assert!(statuses.effect(kind).unwrap().dead(), "{class}: {kind:?}");
        }
    }
}
#[test]
fn vlad_overkill_preserves_character_and_stops_animation_instead_of_gibbing() {
    let Some((mut s, h)) = spawn("vlad") else {
        return;
    };
    s.world.overkill(h);
    let n = &s.world.npcs[&h];
    assert!(!n.character.combat.dead);
    assert!(n.character.combat.overkilled && n.character.combat.cannot_die_without_explicit_kill);
    assert_eq!(n.character.combat.health.hit_points, -1.);
    assert!(!n.motion.controller.is_playing());
    assert!(!s.world.overkilled.contains(&h));
}

#[test]
fn swamp_taunts_use_the_live_dialog_manager_and_native_timer_range() {
    let Some((mut s, h)) = spawn("vlad") else {
        return;
    };
    command(&mut s, BossCommand::Activate);
    s.step(DT);
    s.world.boss_fight.vlad_taunt_timers[Kind::Swamp as usize] = 0.;
    s.step(DT);
    let manager = s.world.dialogs.as_ref().unwrap();
    assert!(["vladbattle1", "vladbattle2", "vladbattle3", "vladbattle4"]
        .iter()
        .any(|name| manager.dialog_active(custom_hash(name))));
    assert!((5.0..10.0).contains(&s.world.boss_fight.vlad_taunt_timers[Kind::Swamp as usize]));
    let position = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == h)
        .unwrap()
        .position;
    assert!(s
        .world
        .dialog_text
        .as_ref()
        .unwrap()
        .world
        .owners
        .values()
        .any(|facts| facts.position == position && facts.character));
    assert!(!s
        .world
        .log
        .iter()
        .any(|line| line.starts_with("Vlad taunt:")));
}

#[test]
fn simultaneous_swamp_bosses_advance_the_same_native_singleton_timer() {
    let Some((mut s, _)) = spawn("vlad") else {
        return;
    };
    command(
        &mut s,
        BossCommand::Spawn {
            boss: "vlad".into(),
            prop: false,
            generic: false,
            area: custom_hash("boss_spawn"),
            unique_id: 0,
            mesh: -1,
            on_death: 0,
            on_damage: 0,
            setup: BossSetup {
                delayed: true,
                ..Default::default()
            },
        },
    );
    command(&mut s, BossCommand::Activate);
    s.step(DT);
    assert_eq!(s.world.boss_fight.bosses.len(), 2);
    s.world.boss_fight.vlad_taunt_timers[Kind::Swamp as usize] = 2.;
    s.step(DT);
    assert!(
        (s.world.boss_fight.vlad_taunt_timers[Kind::Swamp as usize] - (2. - 2. * DT)).abs() < 1e-5
    );
}

#[test]
fn diplomat_fall_recovery_uses_actual_teleport_nav_and_effects() {
    let Some((mut s, h)) = spawn("vlad2") else {
        return;
    };
    command(&mut s, BossCommand::Activate);
    s.step(DT);
    let body = s.world.npcs[&h].body;
    s.world.physics.bodies[body].position[1] = -6.;
    s.step(DT);
    assert!(s.world.physics.bodies[body].position[1] > -5.);
    assert!(
        s.effects
            .active_effects_by_hash(custom_hash("magick_teleport_disappear"))
            > 0
    );
    assert!(
        s.effects
            .active_effects_by_hash(custom_hash("magick_teleport_appear"))
            > 0
    );
}
