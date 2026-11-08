use magicka_content::character::ActionKind;
use magicka_runtime::{
    animation_controller::animation_value, play_content::load_character, play_session::PlaySession,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
const DT: f32 = 1. / 60.;
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
}
fn session(name: &str) -> Option<(PlaySession, u16)> {
    if !root().is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root(), "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    for _ in 0..60 {
        s.step(DT);
    }
    let h = s.spawn_npc(name, "idle", 4.).unwrap();
    let n = s.world.npcs.get_mut(&h).unwrap();
    n.combat.abilities = Arc::new(Vec::new());
    n.character.definition.max_hit_points = 100000.;
    n.character.combat.health.max_hit_points = 100000.;
    n.character.combat.health.hit_points = 1000.;
    n.character.definition.regeneration = 0;
    Some((s, h))
}
#[test]
fn authored_regenerate_action_heals_once_without_resistance_or_boost_scaling() {
    use magicka_sim::{damage::Damage, resistance::Resistance};
    for name in [
        "Dungeons_noapron_wurstmacher",
        "Dungeons_wurstmacher",
        "Frost_wurstmacher",
    ] {
        let Some((mut s, h)) = session(name) else {
            return;
        };
        let template = load_character(&root(), name).unwrap();
        eprintln!(
            "{name}: regeneration abilities={:?}",
            template
                .abilities
                .iter()
                .filter(|a| a.animation_keys.iter().any(|key| key == "special4"))
                .collect::<Vec<_>>()
        );
        let action=template.animation_sets.iter().flatten().find(|clip| clip.animation=="special4").unwrap().actions.iter().find(|a| matches!(&a.kind,ActionKind::SpecialAbility{weapon:-1,ability:Some(a)} if a.type_name.eq_ignore_ascii_case("regenerate"))).unwrap();
        assert_eq!(action.start_time, 0.33);
        let n = s.world.npcs.get_mut(&h).unwrap();
        n.character.definition.resistances = [Resistance {
            multiplier: 0.,
            ..Resistance::IDENTITY
        }; 11];
        n.character.add_nonvisual_buff(
            42,
            magicka_runtime::buffs::NonvisualKind::BoostDamage(Damage {
                attack: 1,
                element: 0xffff,
                amount: 100.,
                magnitude: 10.,
            }),
            10.,
            false,
        );
        n.character.set_invisible(10.);
        n.motion.attack(animation_value("special4").unwrap(), false);
        let mut healed_at = None;
        for tick in 0..240 {
            s.step(DT);
            let hp = s.world.npcs[&h].character.combat.health.hit_points;
            if hp > 1000. {
                healed_at.get_or_insert(tick);
                assert_eq!(
                    hp, 14000.,
                    "scalar healing bypasses resistance and attacker buffs and executes once"
                );
            }
        }
        eprintln!(
            "{name}: regenerate frame={healed_at:?}, HP={}",
            s.world.npcs[&h].character.combat.health.hit_points
        );
        assert!(healed_at.is_some());
        assert_eq!(
            s.world.npcs[&h].character.combat.health.invisibility_timer,
            0.
        );
        assert!(!s
            .world
            .log
            .iter()
            .any(|l| l.contains("not reconstructed: regenerate")));
    }
}
fn invocation() -> magicka_runtime::item_special::SpecialInvocation {
    let template = load_character(&root(), "Frost_wurstmacher").unwrap();
    let ability = template
        .animation_sets
        .iter()
        .flatten()
        .flat_map(|c| &c.actions)
        .find_map(|a| match &a.kind {
            ActionKind::SpecialAbility {
                ability: Some(a), ..
            } if a.type_name.eq_ignore_ascii_case("regenerate") => Some(a.clone()),
            _ => None,
        })
        .unwrap();
    magicka_runtime::item_special::SpecialInvocation {
        ability,
        buff_id: None,
    }
}
#[test]
fn regenerate_respects_immortality_client_and_maximum_health() {
    for (client, immortal, start, expected) in [
        (false, false, 95000., 100000.),
        (false, true, 1000., 1000.),
        (true, false, 1000., 1000.),
    ] {
        let Some((mut s, h)) = session("Frost_wurstmacher") else {
            return;
        };
        let n = s.world.npcs.get_mut(&h).unwrap();
        n.character.combat.health.hit_points = start;
        n.character.immortal_time = if immortal { 3. } else { 0. };
        n.character.set_invisible(10.);
        n.special_requests.push(invocation());
        if client {
            s.world.role = magicka_sim::damage_delivery::NetworkRole::Client;
        }
        s.world.update_npcs(0.);
        let n = &s.world.npcs[&h];
        assert_eq!(n.character.combat.health.hit_points, expected);
        assert_eq!(
            n.character.combat.health.invisibility_timer,
            if immortal { 10. } else { 0. }
        );
        assert_eq!(s.world.physics.bodies[n.body].owner.hit_points, expected);
    }
}

#[test]
fn regenerate_uses_native_self_shield_scalar_healing_rules() {
    use magicka_sim::character_health::{SelfShield, SelfShieldType};
    for (kind, expected) in [
        (SelfShieldType::Shield, 14000.),
        (SelfShieldType::Earth, 1000.),
        (SelfShieldType::Ice, 1000.),
        (SelfShieldType::IcedEarth, 1000.),
    ] {
        let Some((mut s, h)) = session("Frost_wurstmacher") else {
            return;
        };
        let n = s.world.npcs.get_mut(&h).unwrap();
        n.character.combat.health.shield = SelfShield {
            kind,
            health: 5000.,
            max_health: 5000.,
            time_since_damage: 1.,
            ..Default::default()
        };
        n.special_requests.push(invocation());
        s.world.update_npcs(0.);
        let health = &s.world.npcs[&h].character.combat.health;
        assert_eq!(health.hit_points, expected);
        assert_eq!(health.shield.health, 5000.);
        assert_eq!(health.shield.time_since_damage, 0.);
    }
}
