//! Native PlaySound actions on real player and NPC animation clips.
use magicka_content::character::ActionKind;
use magicka_runtime::{
    animation_controller::{animation_value, AnimationSet},
    audio::{AudioRequest, SoundSource},
    dialog_world::DialogWorldCommand,
    message_playback::MessageEffect,
    play_session::PlaySession,
    scene_loading::custom_hash,
    spawn_action::SpawnOrder,
};

fn sounds(set: &AnimationSet, animation: u16) -> Vec<(u16, i32)> {
    set.lookup(animation)
        .unwrap()
        .actions
        .iter()
        .filter_map(|a| match &a.kind {
            ActionKind::PlaySound { bank, sound } => Some((*bank as u16, custom_hash(sound))),
            _ => None,
        })
        .collect()
}

#[test]
fn player_and_npc_hit_clips_play_each_authored_sound_once_at_their_emitters() {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        eprintln!("SKIP: retail content absent");
        return;
    }
    let mut session = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    session.disable_scripts();
    let mut position = session.body().position;
    position[0] += 10.0;
    let npc = session
        .world
        .summon_npc(
            None,
            "Goblin_warrior",
            position,
            [0.0, 0.0, -1.0],
            magicka_sim::summons::SummonKind::Normal,
            SpawnOrder {
                order: 0,
                react_to: 0,
                reaction: 0,
                priority_target: -1,
                priority_ability: -1,
                trigger: 0,
                events: &[],
            },
        )
        .expect("retail NPC");
    for _ in 0..60 {
        session.step(1.0 / 60.0);
    }
    let hit = animation_value("hit").unwrap();
    let player = session.avatar().handle;
    let player_slot = session
        .world
        .players
        .iter()
        .position(|p| p.as_ref().is_some_and(|p| p.handle == player))
        .unwrap();
    let player_sounds = sounds(&session.avatar().motion.animations, hit);
    let npc_sounds = sounds(&session.world.npcs[&npc].motion.animations, hit);
    assert!(!player_sounds.is_empty() && !npc_sounds.is_empty());
    session.world.players[player_slot]
        .as_mut()
        .unwrap()
        .go_to_animation(hit, 0.0);
    // NPC animations also pass through the dialog scene command route.
    let owner = custom_hash("animation_audio_npc");
    session
        .world
        .entities
        .iter_mut()
        .find(|e| e.handle == npc)
        .unwrap()
        .unique_id = owner;
    session
        .world
        .dialog_text
        .as_mut()
        .unwrap()
        .world
        .commands
        .push(DialogWorldCommand::Message(MessageEffect::Animate {
            owner,
            animation: hit,
            blend: 0.0,
        }));
    session.world.update_dialogs(0.0, [[0.0; 4]; 4]);
    session.world.audio_requests.clear();
    let mut observed = Vec::new();
    for _ in 0..60 {
        session.step(1.0 / 60.0);
        for (request, _) in std::mem::take(&mut session.audio) {
            if let AudioRequest::Start {
                key,
                bank,
                cue,
                source: SoundSource::Emitter(position),
                ..
            } = request
            {
                if player_sounds.contains(&(bank, cue)) || npc_sounds.contains(&(bank, cue)) {
                    observed.push((key, bank, cue, position));
                }
            }
        }
    }
    for (handle, expected) in [(player, player_sounds), (npc, npc_sounds)] {
        let body = if handle == player {
            session.avatar().body
        } else {
            session.world.npcs[&npc].body
        };
        let position = session.world.physics.bodies[body].position;
        for (bank, cue) in expected {
            let found: Vec<_> = observed
                .iter()
                .filter(|(_, b, c, p)| *b == bank && *c == cue && (p[0] - position[0]).abs() < 1.0)
                .collect();
            assert_eq!(
                found.len(),
                1,
                "actor {handle} cue {cue} must execute once: {observed:?}"
            );
        }
    }
    assert_ne!(observed[0].0, observed[1].0);
}
