use magicka_runtime::{
    audio::{AudioLibrary, AudioRequest},
    play_session::PlaySession,
    scene_loading::custom_hash,
};
use std::collections::BTreeMap;
#[path = "support/ffmpeg.rs"]
mod ffmpeg;

#[test]
fn authored_bomb_plays_throw_following_fuse_and_surviving_explosion_cues() {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() || !ffmpeg::require() {
        return;
    }
    let library = AudioLibrary::load(&root);
    for name in ["wep_bomb_throw", "wep_bomb_fuse", "wep_bomb_explode"] {
        let voices = library
            .resolve(128, custom_hash(name), &mut || 0.5)
            .expect(name);
        assert!(!voices.is_empty());
        for voice in voices {
            let pcm = library.try_decode(&voice.wave_bank, voice.track).unwrap();
            assert!(
                pcm.frames() > 100 && pcm.samples.iter().any(|v| *v != 0),
                "{name} silent PCM"
            );
            eprintln!(
                "{name}: track={} frames={} loops={}",
                voice.track,
                pcm.frames(),
                voice.loop_count
            );
        }
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    for _ in 0..60 {
        s.step(1. / 60.);
    }
    let owner = s.spawn_npc("Goblin_bomber", "attack", 8.).unwrap();
    let mut starts = BTreeMap::new();
    let mut moved = Vec::new();
    let mut stopped = Vec::new();
    let mut exploded = false;
    let mut explosion_particles = false;
    for _ in 0..600 {
        s.step(1. / 60.);
        for (request, _) in std::mem::take(&mut s.audio) {
            match request {
                AudioRequest::Start {
                    key,
                    bank: 128,
                    cue,
                    source,
                    magnitude,
                    ..
                } => {
                    if magnitude.is_some()
                        && ["wep_bomb_throw", "wep_bomb_fuse", "wep_bomb_explode"]
                            .iter()
                            .any(|n| custom_hash(n) == cue)
                    {
                        assert_eq!(magnitude, Some(0.), "authored Magnitude lost");
                        starts.insert(key, (cue, source));
                        if cue == custom_hash("wep_bomb_explode") {
                            exploded = true;
                        }
                    }
                }
                AudioRequest::Move { key, position }
                    if starts
                        .get(&key)
                        .is_some_and(|(cue, _)| *cue == custom_hash("wep_bomb_fuse")) =>
                {
                    moved.push((key, position))
                }
                AudioRequest::Stop { key, .. } if starts.contains_key(&key) => stopped.push(key),
                _ => {}
            }
        }
        let refs: Vec<_> = s
            .effects
            .presentation()
            .into_iter()
            .filter(|p| p.definition == custom_hash("goblin_bomb_explosion"))
            .map(|p| p.reference)
            .collect();
        explosion_particles |= s
            .effects
            .particles
            .active()
            .iter()
            .zip(s.effects.particles.active_sources())
            .any(|(p, r)| {
                p.radius > 0. && p.normalized_lifetime < 1. && r.is_some_and(|r| refs.contains(&r))
            });
        if exploded
            && explosion_particles
            && stopped
                .iter()
                .any(|key| starts[key].0 == custom_hash("wep_bomb_fuse"))
        {
            break;
        }
    }
    assert!(
        explosion_particles,
        "authored bomb explosion produced no live particles"
    );
    assert!(starts
        .values()
        .any(|(cue, _)| *cue == custom_hash("wep_bomb_throw")));
    assert!(
        exploded,
        "ordinary authored bomb did not play its timed explosion; owner={owner}"
    );
    assert!(moved.iter().any(|(key,p)| matches!(starts[key].1,magicka_runtime::audio::SoundSource::Emitter(initial) if initial != *p)),"fuse did not follow moving bomb");
    assert!(
        stopped
            .iter()
            .any(|key| starts[key].0 == custom_hash("wep_bomb_fuse")),
        "fuse was not stopped at removal"
    );
    assert!(
        !stopped
            .iter()
            .any(|key| starts[key].0 == custom_hash("wep_bomb_explode")),
        "removal cut off the explosion"
    );
    eprintln!(
        "ordinary bomb: throw, moving fuse, explosion, fuse stop; {} starts {} emitter moves",
        starts.len(),
        moved.len()
    );
}

#[test]
fn a_real_player_fire_spray_detonates_an_authored_bomb_before_its_timer() {
    use magicka_runtime::avatar::PlayerInput;
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    for _ in 0..60 {
        s.step(1. / 60.);
    }
    // Move the caster onto open road; the Havindr start locator is beside a
    // cart which can obstruct the spray before it reaches the projectile.
    let body = s.avatar().body;
    s.world.physics.bodies[body].position[0] = 0.;
    s.world.physics.bodies[body].position[2] = 0.;
    for _ in 0..60 {
        s.step(1. / 60.);
    }
    // Five authored Fire elements extend the ordinary spray to the launcher;
    // short single-element flames can miss the bomb's high ballistic flight.
    for _ in 0..5 {
        let mut elements = [false; 8];
        elements[7] = true;
        s.handle_input(
            PlayerInput {
                cursor_ndc: Some([0., 0.5]),
                elements,
                ..Default::default()
            },
            16. / 9.,
        );
        for _ in 0..10 {
            s.step(1. / 60.);
            s.handle_input(
                PlayerInput {
                    cursor_ndc: Some([0., 0.5]),
                    ..Default::default()
                },
                16. / 9.,
            );
        }
    }
    for _ in 0..60 {
        s.step(1. / 60.);
        s.handle_input(
            PlayerInput {
                cursor_ndc: Some([0., 0.5]),
                ..Default::default()
            },
            16. / 9.,
        );
    }
    let owner = s.spawn_npc("Goblin_bomber", "attack", 8.).unwrap();
    // Keep the launcher alive so its own fire-overkill explosion cannot be
    // mistaken for the projectile explosion under test.
    s.world
        .npcs
        .get_mut(&owner)
        .unwrap()
        .character
        .immortal_time = 100.;
    let mut first = None;
    for _ in 0..180 {
        s.step(1. / 60.);
        if let Some(m) = s.world.missiles.iter().find(|m| m.body.owner == owner) {
            first = Some(m.body.id);
            break;
        }
    }
    let first = first.expect("ordinary bomber launch before casting Fire");
    let launch_time = s.play_time;
    let mut exploded_at = None;
    let mut removed_by_fire = None;
    let mut cue_missiles = BTreeMap::new();
    for _ in 0..600 {
        s.events.clear();
        s.step(1. / 60.);
        s.handle_input(
            PlayerInput {
                cursor_ndc: Some([0., 0.5]),
                cast_down: true,
                ..Default::default()
            },
            16. / 9.,
        );
        for event in &s.events {
            if let magicka_runtime::play_session::SessionEvent::MissileSound { missile, key } =
                event
            {
                cue_missiles.insert(*key, *missile);
            }
        }
        let sound = s.audio.iter().find_map(|(a, _)| {
            let AudioRequest::Start {
                key,
                bank: 128,
                cue,
                ..
            } = a
            else {
                return None;
            };
            (cue_missiles.get(key) == Some(&first) && *cue == custom_hash("wep_bomb_explode"))
                .then_some(first)
        });
        if let Some(missile) = sound {
            removed_by_fire = Some(missile);
            exploded_at = Some(s.play_time);
            break;
        }
        s.audio.clear();
    }
    let age = exploded_at.expect("player Fire spray did not detonate any bomb") - launch_time;
    assert!(
        age < 3.9,
        "bomb only reached its four-second timer: age={age}"
    );
    assert!(
        !s.world
            .missiles
            .iter()
            .any(|m| Some(m.body.id) == removed_by_fire),
        "fire explosion did not remove its bomb"
    );
    eprintln!("player Fire spray detonated ordinary bomb {removed_by_fire:?} {age} seconds after first launch, before four-second timer");
}

#[test]
fn bomb_damage_condition_uses_the_authored_fire_threshold() {
    use magicka_sim::{
        damage::{attack, Damage},
        spells::Elements as E,
    };
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    for _ in 0..60 {
        s.step(1. / 60.);
    }
    let owner = s.spawn_npc("Goblin_bomber", "attack", 8.).unwrap();
    let mut id = None;
    for _ in 0..180 {
        s.step(1. / 60.);
        id = s
            .world
            .missiles
            .iter()
            .find(|m| m.body.owner == owner)
            .map(|m| m.body.id);
        if id.is_some() {
            break;
        }
    }
    let id = id.expect("ordinary bomb launch");
    for (element, amount, expected_alive) in [
        (E::EARTH, 20., true),
        (E::FIRE, 9., true),
        (E::FIRE, 10., false),
    ] {
        let bomb = s.world.missiles.iter().find(|m| m.body.id == id).unwrap();
        let at = bomb.body.position;
        let age = bomb.state.time_alive;
        assert!(age < 1., "fixture reached timer");
        let damage = Damage {
            attack: attack::DAMAGE,
            element: element.0,
            amount,
            magnitude: 1.,
        };
        s.world.arc_damage(
            Some(s.avatar().handle),
            Some(owner),
            at,
            [0., 0., -1.],
            1.,
            std::f32::consts::PI,
            &[damage],
            s.play_time,
        );
        s.step(1. / 60.);
        assert_eq!(
            s.world.missiles.iter().any(|m| m.body.id == id),
            expected_alive,
            "bomb response to {element:?} amount={amount}"
        );
    }
}
