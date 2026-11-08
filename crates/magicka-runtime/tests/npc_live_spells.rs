use magicka_runtime::{
    avatar::AvatarEvent,
    npc_spellcasting::SpellAbility,
    play_content::load_character,
    play_session::{PlaySession, SessionEvent},
};
use magicka_sim::{damage::CastType, spells::Elements as E};
use std::path::Path;

#[test]
fn authored_enemy_projectile_cast_reaches_live_missiles() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(root.join("Data/Characters")).unwrap() {
        let path = entry.unwrap().path();
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("xnb"))
        {
            continue;
        }
        let name = path.file_stem().unwrap().to_str().unwrap();
        let template = load_character(&root, name).unwrap();
        if template.faction & 2 != 0 {
            continue;
        }
        for ability in &template.abilities {
            if ability.target != 2 || ability.fuzzy_expression.is_none() {
                continue;
            }
            let Some(spell) = SpellAbility::from_content(ability).unwrap() else {
                continue;
            };
            if spell.cast_type == CastType::Force && spell.elements.contains(&E::EARTH) {
                candidates.push(name.to_string());
            }
        }
    }
    candidates.sort();
    candidates.dedup();
    eprintln!("Authored enemy projectile casters: {candidates:?}");
    let name = candidates.first().expect("retail enemy Earth caster");
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    for _ in 0..60 {
        s.step(1. / 60.);
    }
    let npc = s.spawn_npc(name, "attack", 5.).unwrap();
    let mut conjured = false;
    let mut cast = false;
    let mut launched = false;
    let mut model = false;
    for _ in 0..900 {
        s.step(1. / 60.);
        let events = std::mem::take(&mut s.events);
        for event in events {
            if let SessionEvent::NpcSpell { owner, event } = event {
                if owner != npc {
                    continue;
                }
                match event {
                    AvatarEvent::NpcConjure(_) => conjured = true,
                    AvatarEvent::SpellCast { .. } => cast = true,
                    AvatarEvent::MissileLaunched(_) => launched = true,
                    _ => {}
                }
            }
        }
        model |= s
            .world
            .missiles
            .iter()
            .any(|m| m.body.owner == npc && m.body.model.is_some());
        if launched && model {
            break;
        }
    }
    eprintln!(
        "{name}: conjured={conjured} cast={cast} launched={launched} model={model}; log={:?}",
        s.world.log
    );
    assert!(conjured && cast && launched && model);
}

#[test]
fn item_projectiles_keep_travelling_effects_on_their_live_missile() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    for (template, item_model) in [("Beholder_benign", false), ("Goblin_bomber", true)] {
        let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        s.disable_scripts();
        s.stop_level_effects();
        s.world.flags.cutscene = false;
        s.world.flags.input_limited = false;
        for _ in 0..60 {
            s.step(1. / 60.);
        }
        let owner = s.spawn_npc(template, "attack", 8.).unwrap();
        let mut positions = std::collections::BTreeMap::<u32, Vec<[f32; 3]>>::new();
        let mut travelling = None;
        for _ in 0..600 {
            s.step(1. / 60.);
            s.events.clear();
            for m in s.world.missiles.iter().filter(|m| m.body.owner == owner) {
                assert_eq!(
                    m.body.item_model.is_some(),
                    item_model,
                    "{template} model recipe"
                );
                let refs = s.missile_effect_refs(m.body.id);
                let emitters: Vec<_> = s
                    .effects
                    .presentation()
                    .into_iter()
                    .filter(|p| refs.contains(&p.reference))
                    .collect();
                for emitter in &emitters {
                    assert_eq!(
                        &emitter.transform[12..15],
                        &m.body.position,
                        "{template} emitter must follow its missile"
                    );
                }
                let count = s
                    .effects
                    .particles
                    .active()
                    .iter()
                    .zip(s.effects.particles.active_sources())
                    .filter(|(p, source)| {
                        p.radius > 0.
                            && p.normalized_lifetime < 1.
                            && source.is_some_and(|r| refs.contains(&r))
                    })
                    .count();
                if count > 0 && !emitters.is_empty() {
                    let samples = positions.entry(m.body.id).or_default();
                    samples.push(m.body.position);
                    if samples.len() >= 3 && samples.last() != samples.first() {
                        travelling = Some(m.body.id);
                    }
                }
            }
            if travelling.is_some() {
                break;
            }
        }
        let missile = travelling.unwrap_or_else(|| {
            panic!(
                "{template} did not launch a moving missile with retained particles; log={:?}",
                s.world.log
            )
        });
        eprintln!(
            "{template}: travelling missile {missile}, {} moving samples",
            positions[&missile].len()
        );
        for _ in 0..600 {
            s.step(1. / 60.);
            s.events.clear();
            if s.world.missiles.iter().all(|m| m.body.id != missile) {
                break;
            }
        }
        assert!(
            s.world.missiles.iter().all(|m| m.body.id != missile),
            "{template}: original missile did not expire"
        );
        assert!(
            s.missile_effect_refs(missile).is_empty(),
            "{template}: removed missile retained its emitter"
        );
    }
}

#[test]
fn wounded_authored_caster_selects_and_presents_its_own_life_spell() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(root.join("Data/Characters")).unwrap() {
        let path = entry.unwrap().path();
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("xnb"))
        {
            continue;
        }
        let name = path.file_stem().unwrap().to_str().unwrap();
        if name.starts_with("Boss_") || name.starts_with("Wizard") || name.starts_with("Vlad") {
            continue;
        }
        let template = load_character(&root, name).unwrap();
        if template.abilities.iter().any(|a| {
            a.target == 1
                && a.fuzzy_expression.is_some()
                && SpellAbility::from_content(a).unwrap().is_some_and(|spell| {
                    spell.cast_type == CastType::SelfCast
                        && !spell.elements.is_empty()
                        && spell.elements.iter().all(|e| *e == E::LIFE)
                })
        }) {
            candidates.push(name.to_owned());
        }
    }
    candidates.sort();
    eprintln!("Authored Life self-casters: {candidates:?}");
    let template = candidates
        .iter()
        .find(|n| n.as_str() == "Warlock")
        .unwrap_or_else(|| candidates.first().expect("retail Life self-caster"));
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.stop_level_effects();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    for _ in 0..60 {
        s.step(1. / 60.);
    }
    let npc = s.spawn_npc(template, "attack", 8.).unwrap();
    // Damage through the public scene arc route, so hit reactions, aggro,
    // cooldowns and the caster's normal choice of ability all remain live.
    let at = s.world.physics.bodies[s.world.npcs[&npc].body].position;
    let max_hp = s.world.npcs[&npc].character.combat.health.max_hit_points;
    for _ in 0..30 {
        if s.world.npcs[&npc].character.combat.health.hit_points < max_hp * 0.45 {
            break;
        }
        s.world.arc_damage(
            Some(s.avatar().handle),
            Some(s.avatar().handle),
            at,
            [0., 0., -1.],
            0.1,
            std::f32::consts::PI,
            &[magicka_sim::damage::Damage {
                attack: magicka_sim::damage::attack::DAMAGE,
                element: E::EARTH.0,
                amount: max_hp * 0.05,
                magnitude: 1.,
            }],
            s.play_time,
        );
    }
    assert!(s.world.npcs[&npc].character.combat.health.hit_points < max_hp * 0.5);
    let mut healed = false;
    let mut effect = false;
    let mut particles = false;

    assert!(
        !s.world.npcs[&npc].dead(),
        "fixture damage must leave the caster alive"
    );
    for _ in 0..900 {
        s.step(1. / 60.);
        for event in std::mem::take(&mut s.events) {
            if let SessionEvent::NpcSpell { owner, event } = event {
                if owner != npc {
                    continue;
                }
                match event {
                    AvatarEvent::SelfCastApplied {
                        hit_points_before,
                        hit_points_after,
                        ..
                    } => healed |= hit_points_after > hit_points_before,
                    AvatarEvent::SelfEffectStarted {
                        name: "self_life", ..
                    } => effect = true,
                    _ => {}
                }
            }
        }
        let hash = magicka_runtime::scene_loading::custom_hash("self_life");
        let refs: Vec<_> = s
            .effects
            .presentation()
            .into_iter()
            .filter(|p| p.definition == hash)
            .map(|p| p.reference)
            .collect();
        particles |= s
            .effects
            .particles
            .active()
            .iter()
            .zip(s.effects.particles.active_sources())
            .any(|(p, source)| {
                p.radius > 0.
                    && p.normalized_lifetime < 1.
                    && source.is_some_and(|r| refs.contains(&r))
            });
        if healed && effect && particles {
            break;
        }
    }
    eprintln!("{template}: healed={healed} self_effect={effect} live_particles={particles}");
    assert!(
        healed && effect && particles,
        "authored NPC Life did not reach healing and presentation"
    );
}
