//! Fafnir in Ruins ru_s6BF (research/boss-battles.md).
use magicka_runtime::{
    boss_fafnir::State, play_session::PlaySession, scene_world::boss_live::BossRef,
};
use magicka_sim::{
    damage::{attack, Damage},
    spells::Elements,
};
use std::path::Path;

const DT: f32 = 1.0 / 60.0;

fn host(s: &PlaySession) -> &magicka_runtime::scene_world::fafnir_live::FafnirHost {
    s.world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| match b {
            BossRef::Fafnir(f) => Some(f.as_ref()),
            _ => None,
        })
        .expect("no Fafnir boss")
}

fn step(s: &mut PlaySession, seconds: f32) {
    for _ in 0..(seconds / DT) as usize {
        s.step(DT);
    }
}

fn hit(s: &mut PlaySession, zone: u16, element: Elements, amount: f32) {
    let mut triggers = Vec::new();
    s.world.physics_damage.get_mut(&zone).unwrap().deliver(
        &[Damage {
            attack: attack::DAMAGE,
            element: element.0,
            amount,
            magnitude: 1.0,
        }],
        15,
        &mut triggers,
    );
}

#[test]
fn fafnir_wakes_fights_quakes_and_is_defeated() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Ruins.lvl", "Wizard").unwrap();
    s.change_scene_named("ru_s6BF", "start").unwrap();
    s.world.flags.cutscene = false;
    step(&mut s, 0.5);
    let h = host(&s);
    assert_eq!(h.fafnir.state, State::Sleep);
    eprintln!("fafnir parent {:?}", &h.fafnir.pose.parent[12..15]);
    let zones = h.zones;
    // Sleeping Fafnir deflects damage.
    hit(&mut s, zones[1], Elements::EARTH, 5000.0);
    step(&mut s, 0.1);
    assert_eq!(host(&s).fafnir.status.hit_points, 35000.0);
    // activateBoss: Intro, then fafnir_awake; the script answers with the
    // FafnirFight message (sent here directly).
    s.run_trigger("activate fafnir", true).unwrap();
    step(&mut s, 4.0);
    assert_eq!(host(&s).fafnir.state, State::Intro);
    for _ in 0..(20.0 / DT) as usize {
        s.step(DT);
        if host(&s).fafnir.controller.clip_name() == Some("idle") {
            break;
        }
    }
    assert_eq!(
        host(&s).fafnir.controller.clip_name(),
        Some("idle"),
        "intro never ended"
    );
    // The script's "fafnir intro" sends bossMessage fafnirfight (delay 1).
    s.run_trigger("fafnir intro", true).unwrap();
    step(&mut s, 1.5);
    assert_ne!(
        host(&s).fafnir.state,
        State::Intro,
        "FafnirFight not received"
    );
    // Only the body zone takes damage; fire heals (multiplier −3).
    hit(&mut s, zones[0], Elements::EARTH, 2000.0);
    step(&mut s, 0.05);
    assert_eq!(
        host(&s).fafnir.status.hit_points,
        35000.0,
        "head took damage"
    );
    hit(&mut s, zones[1], Elements::ARCANE, 1000.0);
    step(&mut s, 0.05);
    let hp = host(&s).fafnir.status.hit_points;
    assert!(hp < 35000.0, "body did not take damage");
    // Decisions run attacks; reaching 80% triggers the first earthquake.
    let mut states = std::collections::BTreeSet::new();
    for _ in 0..(30.0 / DT) as usize {
        s.step(DT);
        states.insert(format!("{:?}", host(&s).fafnir.state));
    }
    eprintln!("states seen: {states:?}");
    assert!(states.len() > 2, "no attacks chosen");
    hit(&mut s, zones[1], Elements::ARCANE, 8000.0);
    let mut quake = false;
    for _ in 0..(30.0 / DT) as usize {
        s.step(DT);
        if host(&s).fafnir.quakes > 0 {
            quake = true;
            break;
        }
    }
    assert!(quake, "no earthquake below 80%");
    // Defeat at HP <= 1 (damage clamps negative HP to 1).
    hit(&mut s, zones[1], Elements::ARCANE, 100_000.0);
    step(&mut s, 0.1);
    assert_eq!(host(&s).fafnir.state, State::Defeated);
}

/// User report: Fafnir's fire attacks did not land. FirelanceHigh's own
/// mLanceTime counts up while Fafnir.Update counts a separate mLanceTime
/// down; the rewrite shared one field, so the lance froze at 8% length and
/// its CircleDamage stayed beside the head. With the Wizard standing still
/// after the natural intro (dialog advanced), both the fireball and the
/// lance damage it.
#[test]
fn fafnir_fireball_and_lance_reach_the_wizard() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Ruins.lvl", "Wizard").unwrap();
    s.change_scene_named("ru_s6BF", "start").unwrap();
    s.world.flags.cutscene = false;
    step(&mut s, 0.5);
    s.run_trigger("activate fafnir", true).unwrap();
    let mut lost = std::collections::BTreeMap::<String, f32>::new();
    let mut lance = 0.0f32;
    let (mut spray, mut trail, mut weld) = (false, false, false);
    // One looping weld at the lance end; none once the lance is gone.
    let (mut most_welds, mut stale_welds) = (0usize, 0usize);
    // The intro dialog outlasts fafnir_awake's delayed 8 s cutscene; when it
    // is done, "fafnir intro" ends the cutscene (run here 10 s into the
    // fight instead of reading the dialog).
    let mut fighting = 0.0;
    let mut released = false;
    for _ in 0..(150.0 / DT) as usize {
        if !matches!(host(&s).fafnir.state, State::Sleep | State::Intro) {
            fighting += DT;
        }
        if fighting > 10.0 && !released {
            s.run_trigger("fafnir intro", true).unwrap();
            released = true;
        }
        let before = s.avatar().character.combat.health.hit_points;
        s.step(DT);
        let f = &host(&s).fafnir;
        lance = lance.max(f.lance_scale);
        let hash = magicka_runtime::scene_loading::custom_hash;
        spray |= s
            .world
            .effects
            .effects
            .get(&hash("fafnir_live_fire_spray"))
            .is_some_and(|e| e.effect.active);
        trail |= s
            .effects
            .active_effects_by_hash(hash("fafnir_fireball_trail"))
            > 0;
        let welds = s.effects.active_effects_by_hash(hash("fafnir_lance_weld"));
        weld |= welds > 0;
        most_welds = most_welds.max(welds);
        if !f.draw_lance {
            stale_welds = stale_welds.max(welds);
        }
        let after = s.avatar().character.combat.health.hit_points;
        *lost.entry(format!("{:?}", f.state)).or_default() += (before - after).max(0.0);
        if after < 500.0 {
            let p = s.world.players[0].as_mut().unwrap();
            p.character.combat.health.hit_points = 1000.0;
        }
    }
    assert!(!s.world.flags.cutscene, "the intro cutscene must end");
    assert!(spray, "the lance must draw fafnir_fire_spray");
    assert!(weld, "the lance end must draw fafnir_lance_weld");
    assert_eq!(most_welds, 1, "one weld effect at a time");
    assert_eq!(stale_welds, 0, "the weld stops with the lance");
    assert!(trail, "a flying fireball must show its trail");
    assert!(lance > 0.99, "the lance must extend fully ({lance})");
    assert!(
        lost.get("Fireball").copied().unwrap_or(0.0) > 0.0,
        "{lost:?}"
    );
    assert!(
        lost.get("LanceHigh").copied().unwrap_or(0.0) > 0.0,
        "{lost:?}"
    );
}

/// Confuse's two eye charms and EarthQuake's head effect live exactly as long
/// as their states (native handles: start, update, Stop on exit). Started
/// through the one-shot path they stayed for the rest of the fight.
#[test]
fn fafnir_charm_and_earthquake_effects_end_with_their_states() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Ruins.lvl", "Wizard").unwrap();
    s.change_scene_named("ru_s6BF", "start").unwrap();
    s.world.flags.cutscene = false;
    step(&mut s, 0.5);
    s.run_trigger("activate fafnir", true).unwrap();
    let zones = host(&s).zones;
    let hash = magicka_runtime::scene_loading::custom_hash;
    let (mut quake_seen, mut confuse_seen) = (false, false);
    let (mut most_charms, mut most_quakes) = (0usize, 0usize);
    let (mut stale_charms, mut stale_quakes) = (0usize, 0usize);
    let mut hit_done = false;
    let mut fighting = 0.0;
    let mut released = false;
    // The Stop reaches the particle manager one step after the state exit.
    let mut before = host(&s).fafnir.state;
    for _ in 0..(240.0 / DT) as usize {
        let state = host(&s).fafnir.state;
        if !matches!(state, State::Sleep | State::Intro) {
            fighting += DT;
            if !hit_done {
                // 50%: below the 80% and 60% quake thresholds and Confuse's 60%.
                hit(&mut s, zones[1], Elements::ARCANE, 17500.0);
                hit_done = true;
            }
        }
        if fighting > 10.0 && !released {
            s.run_trigger("fafnir intro", true).unwrap();
            released = true;
        }
        s.step(DT);
        let f = &host(&s).fafnir;
        let charms = s.effects.active_effects_by_hash(hash("fafnir_charm"));
        let quakes = s.effects.active_effects_by_hash(hash("fafnir_earthquake"));
        most_charms = most_charms.max(charms);
        most_quakes = most_quakes.max(quakes);
        match f.state {
            State::Confuse => confuse_seen |= charms == 2,
            State::EarthQuake => quake_seen |= quakes == 1,
            _ => {}
        }
        if f.state != State::Confuse && before != State::Confuse {
            stale_charms = stale_charms.max(charms);
        }
        if f.state != State::EarthQuake && before != State::EarthQuake {
            stale_quakes = stale_quakes.max(quakes);
        }
        before = f.state;
        if quake_seen
            && confuse_seen
            && f.state != State::Confuse
            && f.state != State::EarthQuake
            && before == f.state
        {
            break;
        }
        if s.avatar().character.combat.health.hit_points < 500.0 {
            let p = s.world.players[0].as_mut().unwrap();
            p.character.combat.health.hit_points = 1000.0;
        }
    }
    assert!(quake_seen, "the earthquake must draw fafnir_earthquake");
    assert!(confuse_seen, "Confuse must draw a charm on each eye");
    assert_eq!(most_charms, 2, "one charm per eye");
    assert_eq!(most_quakes, 1, "one earthquake effect");
    assert_eq!(stale_charms, 0, "charms stop with Confuse");
    assert_eq!(
        stale_quakes, 0,
        "the earthquake effect stops with the state"
    );
}

/// Wing (clip 11) pushes damageables within 30 m that lie on the world +Z
/// side of the spine base with Pushed Earth 150 from 2 m behind them, inside
/// ANIMATION_TIMES[11]'s windows. Replays the viewer's `fafnir-wing` script
/// (hall walk, intro, dialog, run to the spine base's +Z side) through
/// handle_input. The wizard is healed, not made immortal: an immortal
/// character ignores every impact, pushes included, which is why the first
/// `ruins-fafnir-wing` captures showed him standing still through Wing.
#[test]
fn fafnir_wing_pushes_the_wizard_behind_its_spine() {
    use magicka_runtime::avatar::PlayerInput;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Ruins.lvl", "Wizard").unwrap();
    s.change_scene_named("ru_s6BF", "start").unwrap();
    s.headless_dialogs = Some([1280.0, 720.0]);
    s.run_trigger("activate fafnir", true).unwrap();
    let body = s.avatar().body;
    // (frames, cursor ndc, move held): FAFNIR_WING_SCRIPT's steps.
    let script: [(usize, [f32; 2], bool); 4] = [
        (300, [0.0, 0.9], true),
        (240, [0.0, 0.5], false),
        (900, [0.0, 0.5], false),
        (95, [-0.24, 0.87], true),
    ];
    let scripted: usize = script.iter().map(|s| s.0).sum();
    let (mut wing_seen, mut fastest) = (false, 0.0f32);
    let mut after = 0;
    let mut placed_at = [0.0; 3];
    for frame in 0..(240.0 / DT) as usize {
        if s.avatar().character.combat.health.hit_points < 500.0 {
            let p = s.world.players[0].as_mut().unwrap();
            p.character.combat.health.hit_points = 1000.0;
        }
        let mut at = 0;
        let mut input = PlayerInput {
            cursor_ndc: Some([0.0, 0.5]),
            ..Default::default()
        };
        for &(frames, ndc, moving) in &script {
            if frame < at + frames {
                input.cursor_ndc = Some(ndc);
                input.move_down = moving;
                break;
            }
            at += frames;
        }
        s.handle_input(input, 16.0 / 9.0);
        s.step(DT);
        // The viewer types the dialog out and its end runs "fafnir intro",
        // which ends the cutscene; headless, run it after the dialog window.
        if frame == 1440 && s.world.flags.cutscene {
            s.run_trigger("fafnir intro", true).unwrap();
        }
        if frame == scripted {
            placed_at = s.world.physics.bodies[body].position;
            eprintln!(
                "wing: scripted frames done, cutscene {} wizard {placed_at:?}",
                s.world.flags.cutscene
            );
        }
        if frame < scripted {
            continue;
        }
        let f = &host(&s).fafnir;
        if f.state == State::Wing {
            wing_seen = true;
            after = 0;
            let b = &s.world.physics.bodies[body];
            fastest = fastest.max((b.velocity[0].powi(2) + b.velocity[2].powi(2)).sqrt());
        } else if wing_seen {
            after += 1;
        } else {
            // The other attacks throw a mortal wizard out of Wing's zone;
            // put him back between them.
            let b = &mut s.world.physics.bodies[body];
            let d = [0, 2].map(|i| b.position[i] - placed_at[i]);
            if d[0] * d[0] + d[1] * d[1] > 1.0 {
                b.position = placed_at;
                b.velocity = [0.0; 3];
            }
        }
        if after > (2.0 / DT) as usize {
            break;
        }
    }
    let p = s.world.physics.bodies[body].position;
    let spine = host(&s).fafnir.spine_base_position;
    eprintln!(
        "wing: placed at {placed_at:?}, wing seen {wing_seen}, fastest {fastest} m/s, wizard {p:?}, spine {spine:?}"
    );
    assert!(
        wing_seen,
        "Wing was never chosen; wizard {p:?} spine {spine:?}"
    );
    assert!(
        fastest > 1.0,
        "the wing push must move the wizard (fastest {fastest} m/s, wizard {p:?})"
    );
}
