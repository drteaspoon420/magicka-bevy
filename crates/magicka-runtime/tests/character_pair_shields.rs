use magicka_runtime::{
    character_shield,
    play_content::load_start_scene,
    play_session::PlaySession,
    scene_script::{ActionDefinition, SceneScriptLayout},
    spawn_action::SpawnState,
};
use magicka_sim::{
    character_health::{SelfShield, SelfShieldType},
    spells::{Elements as E, Spell},
};
use std::path::PathBuf;
const DT: f32 = 1.0 / 60.0;

fn session() -> Option<(PlaySession, PathBuf)> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    Some((s, root))
}
fn pure_shield() -> SelfShield {
    SelfShield::from_spell(&Spell::default_for_list(&[E::SHIELD]), &mut Vec::new())
}

#[test]
fn live_player_npc_shields_nullify_once_even_when_ethereal_contact_is_rejected() {
    for ethereal in [false, true] {
        let Some((mut s, root)) = session() else {
            return;
        };
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
        let player_body = s.avatar().body;
        let npc = s.world.npcs.get_mut(&handle).unwrap();
        npc.character.combat.health.shield = pure_shield();
        let npc_body = npc.body;
        let player = s.world.players[0].as_mut().unwrap();
        player.character.ethereal = ethereal;
        player.character.combat.health.shield = pure_shield();
        for (i, x) in [(player_body, 0.0), (npc_body, 0.4)] {
            let body = &mut s.world.physics.bodies[i];
            body.position = [x, 100.0, 0.0];
            body.velocity = [0.0; 3];
            body.apply_gravity = false;
        }
        s.world.step_live(None, DT).unwrap();
        assert_eq!(
            s.avatar().character.combat.health.shield.kind,
            SelfShieldType::None
        );
        assert_eq!(
            s.world.npcs[&handle].character.combat.health.shield.kind,
            SelfShieldType::None
        );
        assert_eq!(s.world.shield_nullifications.len(), 1);
        let effect = s.world.shield_nullifications[0];
        assert!((effect.position[0] - s.world.physics.bodies[player_body].radius).abs() < 0.001);
        assert_eq!(effect.emitter_position, [0.0, 100.0, 0.0]);
        assert_eq!(
            !s.world.physics.contact_points(player_body).is_empty(),
            !ethereal
        );
        let effects = s.effects.active_effects();
        s.step(DT);
        assert!(s.world.shield_nullifications.is_empty());
        assert!(s.effects.active_effects() > effects);
    }
}

#[test]
fn second_callback_nullifies_after_first_dead_callback_rejects() {
    let Some((s, _)) = session() else { return };
    for dead in [[false, false], [true, false], [false, true], [true, true]] {
        let mut a = s.avatar().character.clone();
        let mut b = a.clone();
        a.combat.health.shield = pure_shield();
        b.combat.health.shield = pure_shield();
        let mut body_a = s.body().clone();
        let mut body_b = body_a.clone();
        body_a.position = [0.0, 100.0, 0.0];
        body_b.position = [0.4, 100.0, 0.0];
        body_a.mass = 1234.0;
        body_b.mass = 2345.0;
        let (accepted, effect) =
            character_shield::pair([&mut a, &mut b], [&mut body_a, &mut body_b], dead);
        assert_eq!(accepted, dead == [false, false]);
        if dead == [true, true] {
            assert!(effect.is_none());
            assert_eq!(a.combat.health.shield.kind, SelfShieldType::Shield);
            assert_eq!(b.combat.health.shield.kind, SelfShieldType::Shield);
        } else {
            let source = if dead[0] {
                body_b.position
            } else {
                body_a.position
            };
            assert_eq!(effect.unwrap().emitter_position, source);
            assert_eq!(a.combat.health.shield.kind, SelfShieldType::None);
            assert_eq!(b.combat.health.shield.kind, SelfShieldType::None);
            assert_eq!(body_a.mass, a.definition.mass);
            assert_eq!(body_b.mass, b.definition.mass);
        }
    }
}

#[test]
fn one_pure_shield_does_not_nullify_a_solid_or_absent_self_shield() {
    let Some((s, _)) = session() else { return };
    for kind in [
        SelfShieldType::None,
        SelfShieldType::Earth,
        SelfShieldType::Ice,
        SelfShieldType::IcedEarth,
    ] {
        let mut a = s.avatar().character.clone();
        let mut b = a.clone();
        a.combat.health.shield = pure_shield();
        b.combat.health.shield.kind = kind;
        let mut body_a = s.body().clone();
        let mut body_b = body_a.clone();
        let (accepted, effect) =
            character_shield::pair([&mut a, &mut b], [&mut body_a, &mut body_b], [false, false]);
        assert!(accepted);
        assert!(effect.is_none());
        assert_eq!(a.combat.health.shield.kind, SelfShieldType::Shield);
        assert_eq!(b.combat.health.shield.kind, kind);
    }
}
