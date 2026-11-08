//! The single-player revival fairy on retail Havindr (research/revival-fairy.md):
//! armed at level start, it revives the lone dead avatar at its last ground
//! position instead of the Defeat → Restart(Checkpoint) path, once per
//! checkpoint; multiplayer and Versus keep the restart path.
use magicka_runtime::audio::{banks, AudioRequest};
use magicka_runtime::end_game::{EndGameCondition, GameType};
use magicka_runtime::play_session::PlaySession;
use magicka_runtime::revival_fairy::{orbit_offset, FairyState, ETHEREAL_GRACE};
use magicka_runtime::scene_flow::SceneFlowHost;
use magicka_runtime::scene_loading::custom_hash;
use std::path::PathBuf;

const DT: f32 = 1.0 / 60.0;

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir().then_some(root)
}

fn checkpoint_action(spawn_fairy: bool) -> magicka_runtime::scene_flow::SceneFlowAction {
    use magicka_runtime::scene_script::{ActionDefinition, SceneScriptLayout};
    let xml = format!(
        r#"<Scene><Trigger id="checkpoint"><Then><TriggerCheckpoint spawnpoint="start" spawnfairy="{spawn_fairy}" savetodisk="false"/></Then></Trigger></Scene>"#
    );
    let layout = SceneScriptLayout::read(xml.as_bytes(), false).unwrap();
    match layout.actions.into_iter().next().unwrap() {
        ActionDefinition::Flow(action) => *action,
        other => panic!("expected checkpoint, got {other:?}"),
    }
}

fn snapped(s: &PlaySession, point: [f32; 3]) -> [f32; 3] {
    s.world
        .nav
        .as_ref()
        .and_then(|n| n.nearest(point, magicka_sim::navmesh::movement::DEFAULT))
        .map_or(point, |(_, p, _)| p)
}

fn cue_played(s: &PlaySession, bank: u16, name: &str) -> bool {
    s.audio.iter().any(|(request, _)| {
        matches!(request, AudioRequest::Start { bank: b, cue, .. }
            if *b == bank && *cue == custom_hash(name))
    })
}

/// Step until the avatar lives again or `frames` pass; the defeat path
/// must never start while the fairy works.
fn step_until_alive(s: &mut PlaySession, frames: usize) -> usize {
    for frame in 0..frames {
        s.step(DT);
        assert!(!s.end_game.restart, "fairy revival must not restart");
        assert_ne!(s.end_game.condition, EndGameCondition::Defeat);
        if !s.avatar().dead() {
            return frame + 1;
        }
    }
    panic!("avatar not revived within {frames} frames");
}

#[test]
fn single_player_death_is_revived_by_the_fairy_instead_of_restarting() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    assert_eq!(s.world.game_type, GameType::Campaign);
    assert!(
        s.world.level_spawn_fairy,
        "Havindr.lvl keeps Level.SpawnFairy"
    );
    // CreatePlayer → SetSpawnFairies; GameScene.Initialize → SpawnFairies.
    assert!(s.world.spawn_fairies[0]);
    assert!(s.world.revival_fairies[0].active);
    assert_eq!(s.world.revival_fairies[0].state, FairyState::Orbiting);
    assert_eq!(
        s.world.revival_fairies[0].greetings, 0,
        "no greeting at scene entry"
    );
    assert!(s.world.log.iter().any(|l| l == "fairy 0 initialize"));
    assert!(SceneFlowHost::single_player_non_versus(&s.world));

    for _ in 0..10 {
        s.step(DT);
    }
    let fairy = &s.world.revival_fairies[0];
    let body = &s.world.physics.bodies[s.avatar().body];
    assert_eq!(fairy.position, body.position, "the body rides the owner");
    let offset = orbit_offset(fairy.time - DT);
    for axis in 0..3 {
        assert!((fairy.effect_position[axis] - (body.position[axis] + offset[axis])).abs() < 1e-4);
    }
    assert_eq!(s.world.fairy_views().len(), 1);

    let index = s.avatar().body;
    let home = s.world.physics.bodies[index].position;
    let orientation = s.world.physics.bodies[index].orientation;
    s.world.physics.bodies[index].move_to([home[0] + 3.0, home[1], home[2]], orientation);
    for _ in 0..30 {
        s.step(DT);
    }
    let ground = s.world.physics.bodies[index].last_position_on_ground();
    let target = snapped(&s, ground);
    s.world.players[0].as_mut().unwrap().terminate();
    s.step(DT);
    let fairy = &s.world.revival_fairies[0];
    assert_eq!(fairy.state, FairyState::FlyToRevivePosition);
    assert_eq!(fairy.revival_target, target);
    assert!(fairy.camera_following);
    assert_eq!(
        s.world.camera.behaviour,
        magicka_runtime::camera::CameraBehaviour::FollowEntity
    );
    assert!(s.world.revival_fairy_blocks_defeat(0));

    let frames = step_until_alive(&mut s, 600);
    // 2.1 s flight, then Revive's 0.25 s timer and the avatar swap.
    assert!(
        (140..=180).contains(&frames),
        "revived after {frames} frames"
    );
    assert!(cue_played(&s, banks::ADDITIONAL, "fairy_death"));
    assert!(cue_played(&s, 16, "magick_revive_cast"));
    assert!(cue_played(&s, 16, "magick_revive"));
    assert!(s.world.log.iter().any(|l| l == "fairy 0 fly"));
    assert!(s.world.log.iter().any(|l| l == "fairy 0 revive"));
    let player = s.avatar();
    assert_eq!(
        player.motion.state,
        magicka_runtime::avatar::AvatarState::Resurrection
    );
    assert_eq!(
        player.character.combat.health.hit_points, player.character.combat.health.max_hit_points,
        "Revive.Execute(position, playState) uses health 1"
    );
    let p = s.body().position;
    let d = ((p[0] - target[0]).powi(2) + (p[2] - target[2]).powi(2)).sqrt();
    assert!(d < 0.05, "revived {d} m from the fairy's target");
    assert!(!s.world.spawn_fairies[0], "RemoveFairyFrom: one use");
    assert_eq!(s.world.revival_fairies[0].revivals, 1);
    // WaitingForAvatar ends on the next update: ethereal with a 5 s grace.
    s.step(DT);
    let fairy = &s.world.revival_fairies[0];
    assert!(!fairy.active);
    assert!(s.world.log.iter().any(|l| l == "fairy 0 done"));
    let character = &s.avatar().character;
    assert!(character.ethereal);
    // TimedEthereal(5, false): the timer runs down from the next tick.
    assert!(character.ethereal_timer > ETHEREAL_GRACE - 3.0 * DT);
    assert!(character.ethereal_timer <= ETHEREAL_GRACE);
    assert_eq!(
        s.world.camera.behaviour,
        magicka_runtime::camera::CameraBehaviour::FollowPlayers,
        "Release_NoMagReset after the crash"
    );
    assert!(s.world.fairy_views().is_empty());

    // The second death has no fairy: Defeat → Restart(Checkpoint).
    for _ in 0..60 {
        s.step(DT);
    }
    s.world.players[0].as_mut().unwrap().terminate();
    let mut restarted = false;
    for _ in 0..600 {
        s.step(DT);
        if s.end_game.restart {
            restarted = true;
            break;
        }
    }
    assert!(restarted, "the used fairy leaves the defeat path in place");
    assert!(s.avatar().dead());
}

#[test]
fn checkpoint_rearms_the_fairy_and_a_restart_restores_its_flag() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    // Spend the level-start fairy.
    s.world.players[0].as_mut().unwrap().terminate();
    step_until_alive(&mut s, 600);
    s.step(DT);
    assert!(!s.world.revival_fairies[0].active && !s.world.spawn_fairies[0]);
    for _ in 0..60 {
        s.step(DT);
    }

    // TriggerCheckpoint (SpawnFairy, one non-Versus player): Initialize
    // with the greeting, then UpdateState captures the active fairy.
    let mut action = checkpoint_action(true);
    let matrices = action.initialize(&mut s.world).unwrap().unwrap();
    action.execute(Some(&matrices), &mut s.world).unwrap();
    assert!(s.world.revival_fairies[0].active);
    assert_eq!(s.world.revival_fairies[0].greetings, 1);
    assert!(s.world.log.iter().any(|l| l == "fairy 0 greeting"));
    assert!(s.world.spawn_fairies[0]);
    s.step(DT);
    let cp = s.checkpoint.as_ref().expect("checkpoint captured");
    assert_eq!(cp.players()[0]["fairy"], true);

    // Die: the re-armed fairy revives once more; the flag is spent again.
    s.world.players[0].as_mut().unwrap().terminate();
    step_until_alive(&mut s, 600);
    s.step(DT);
    assert_eq!(s.world.revival_fairies[0].revivals, 2);
    assert!(!s.world.spawn_fairies[0]);
    for _ in 0..60 {
        s.step(DT);
    }
    // Die again: Defeat → Restart(Checkpoint) → State.ApplyState re-arms the
    // fairy from the checkpoint's flag (no greeting).
    s.world.players[0].as_mut().unwrap().terminate();
    let mut restarted = false;
    for _ in 0..600 {
        s.step(DT);
        if s.end_game.restart {
            assert!(s.restart_at_checkpoint().unwrap());
            restarted = true;
            break;
        }
    }
    assert!(restarted);
    assert!(!s.avatar().dead());
    assert!(s.world.spawn_fairies[0]);
    assert!(s.world.revival_fairies[0].active);
    assert_eq!(s.world.revival_fairies[0].greetings, 0);

    // A checkpoint without SpawnFairy keeps the current fairy state, and a
    // spent fairy stays spent across the restart.
    s.world.players[0].as_mut().unwrap().terminate();
    step_until_alive(&mut s, 600);
    s.step(DT);
    for _ in 0..60 {
        s.step(DT);
    }
    let mut action = checkpoint_action(false);
    let matrices = action.initialize(&mut s.world).unwrap().unwrap();
    action.execute(Some(&matrices), &mut s.world).unwrap();
    assert!(!s.world.revival_fairies[0].active);
    assert!(!s.world.spawn_fairies[0]);
    s.step(DT);
    assert_eq!(s.checkpoint.as_ref().unwrap().players()[0]["fairy"], false);
    s.world.players[0].as_mut().unwrap().terminate();
    let mut restarted = false;
    for _ in 0..600 {
        s.step(DT);
        if s.end_game.restart {
            assert!(s.restart_at_checkpoint().unwrap());
            restarted = true;
            break;
        }
    }
    assert!(restarted);
    assert!(!s.world.spawn_fairies[0]);
    assert!(!s.world.revival_fairies[0].active);
}

#[test]
fn multiplayer_and_versus_keep_the_checkpoint_restart_path() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    assert!(s.world.revival_fairies[0].active);
    s.join_player_at_start(1, "Wizard").unwrap();
    s.step(DT);
    // A second connected player: no fairy gate holds (hot join kills it).
    assert!(!s.world.revival_fairies[0].active);
    assert!(!s.world.spawn_fairies[0] && !s.world.spawn_fairies[1]);
    assert!(!SceneFlowHost::single_player_non_versus(&s.world));
    let mut action = checkpoint_action(true);
    let matrices = action.initialize(&mut s.world).unwrap().unwrap();
    action.execute(Some(&matrices), &mut s.world).unwrap();
    assert!(!s.world.revival_fairies[0].active && !s.world.revival_fairies[1].active);
    for slot in 0..2 {
        s.world.players[slot].as_mut().unwrap().terminate();
    }
    let mut restarted = false;
    for _ in 0..600 {
        s.step(DT);
        assert!(s.world.players[0].as_ref().unwrap().dead());
        if s.end_game.restart {
            restarted = true;
            break;
        }
    }
    assert!(restarted, "two dead players: Defeat → Restart(Checkpoint)");

    // Versus: GameType.Versus fails the checkpoint gate and SetSpawnFairies.
    let mut v = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    v.disable_scripts();
    v.world.game_type = GameType::Versus;
    v.world.kill_revival_fairy(0);
    v.world.spawn_fairies[0] = false;
    assert!(!SceneFlowHost::single_player_non_versus(&v.world));
    v.world.set_spawn_fairies();
    assert!(!v.world.spawn_fairies[0]);
    let mut action = checkpoint_action(true);
    let matrices = action.initialize(&mut v.world).unwrap().unwrap();
    action.execute(Some(&matrices), &mut v.world).unwrap();
    assert!(!v.world.revival_fairies[0].active);
}

#[test]
fn level_spawn_fairy_flag_controls_the_level_start_fairy() {
    let Some(root) = content() else { return };
    let castle =
        magicka_runtime::play_content::read_level(&root.join("Levels/Wizard_Castle.lvl")).unwrap();
    assert!(
        !castle.spawn_fairy,
        "Wizard_Castle.lvl: spawnFairy=\"false\""
    );
    let havindr =
        magicka_runtime::play_content::read_level(&root.join("Levels/Havindr.lvl")).unwrap();
    assert!(havindr.spawn_fairy, "Level.ctor default");
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.world.kill_revival_fairy(0);
    s.world.spawn_fairies[0] = false;
    s.world.level_spawn_fairy = false;
    s.world.set_spawn_fairies();
    assert!(!s.world.spawn_fairies[0]);
    s.world.spawn_fairies();
    assert!(!s.world.revival_fairies[0].active);
    s.world.level_spawn_fairy = true;
    s.world.set_spawn_fairies();
    s.world.spawn_fairies();
    assert!(s.world.revival_fairies[0].active);
    assert_eq!(s.world.revival_fairies[0].greetings, 0);
}
