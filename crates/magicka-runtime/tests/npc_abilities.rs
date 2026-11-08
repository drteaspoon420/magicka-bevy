//! The DLC NPC special abilities (research/npc-abilities.md), each fired
//! from a spawned NPC through its special-ability request queue.
use magicka_content::common::SpecialAbility;
use magicka_runtime::item_special::SpecialInvocation;
use magicka_runtime::play_session::PlaySession;
const DT: f32 = 1. / 60.;

fn session(template: &str) -> Option<(PlaySession, u16)> {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    for _ in 0..30 {
        s.step(DT);
    }
    let h = s.spawn_npc(template, "attack", 4.).unwrap();
    // Keep the AI from casting its own abilities.
    s.world.npcs.get_mut(&h).unwrap().combat.abilities = std::sync::Arc::new(Vec::new());
    for _ in 0..30 {
        s.step(DT);
    }
    Some((s, h))
}

fn fire(s: &mut PlaySession, owner: u16, name: &str, elements: &[i32]) {
    s.world
        .npcs
        .get_mut(&owner)
        .unwrap()
        .special_requests
        .push(SpecialInvocation {
            ability: SpecialAbility {
                type_name: name.into(),
                animation: String::new(),
                hash_string: String::new(),
                elements: elements.to_vec(),
            },
            buff_id: None,
        });
}

/// The NPC targets and faces the avatar, as its AI does before casting.
fn aim(s: &mut PlaySession, npc: u16) {
    let p = s.world.players[0].as_ref().unwrap();
    let (avatar, at) = (p.handle, s.world.physics.bodies[p.body].position);
    let n = s.world.npcs.get_mut(&npc).unwrap();
    n.combat.change_target(avatar, None);
    let body = &mut s.world.physics.bodies[n.body];
    let d = [at[0] - body.position[0], 0., at[2] - body.position[2]];
    let l = (d[0] * d[0] + d[2] * d[2]).sqrt();
    let dir = [d[0] / l, 0., d[2] / l];
    body.orientation =
        magicka_sim::character_physics::Orientation::from_forward_up(dir, [0., 1., 0.]);
    body.set_desired_direction(dir);
}

fn run(s: &mut PlaySession, frames: usize) {
    for _ in 0..frames {
        s.step(DT);
    }
}

fn assert_supported(s: &PlaySession) {
    assert!(
        s.world.unsupported_specials.is_empty(),
        "{:?}",
        s.world.unsupported_specials
    );
}

#[test]
fn potion_heals_its_drinker_and_tosses_a_flask() {
    let Some((mut s, h)) = session("Dungeons_elf_adventurer") else {
        return;
    };
    s.world
        .npcs
        .get_mut(&h)
        .unwrap()
        .character
        .combat
        .health
        .hit_points -= 600.;
    let before = s.world.npcs[&h].character.combat.health.hit_points;
    let missiles = s.world.missiles.len();
    fire(&mut s, h, "PotionLeft", &[]);
    run(&mut s, 2);
    assert_supported(&s);
    let after = s.world.npcs[&h].character.combat.health.hit_points;
    assert!(after > before + 400., "{before} -> {after}");
    assert!(s.world.missiles.len() > missiles, "no flask");
}

#[test]
fn alucart_death_explosion_and_grease_lump_launch_missiles() {
    let Some((mut s, h)) = session("Dungeons_wurstmacher") else {
        return;
    };
    let missiles = s.world.missiles.len();
    fire(&mut s, h, "AlucartDeathExplosion", &[]);
    run(&mut s, 1);
    assert!(s.world.missiles.len() > missiles);
    let fields = s.world.grease.len();
    fire(&mut s, h, "GreaseLump", &[]);
    run(&mut s, 40);
    assert_supported(&s);
    assert!(
        s.world.grease.len() > fields,
        "the lump leaves no grease trail"
    );
}

#[test]
fn floor_stomp_runs_three_stomps_and_charges_end() {
    let Some((mut s, h)) = session("Dungeons_wurstmacher") else {
        return;
    };
    fire(&mut s, h, "FloorStomp", &[]);
    fire(&mut s, h, "StopCharge", &[]);
    fire(&mut s, h, "BreakBarriers", &[]);
    run(&mut s, 2);
    assert_supported(&s);
    assert_eq!(s.world.staff_effects.len(), 3);
    run(&mut s, 60 * 9);
    assert!(s.world.staff_effects.is_empty(), "abilities never ended");
}

#[test]
fn starfall_strikes_the_target_and_stargaze_charms() {
    let Some((mut s, h)) = session("Starspawn") else {
        return;
    };
    let avatar = s.world.players[0].as_ref().unwrap().handle;
    aim(&mut s, h);
    fire(&mut s, h, "StarGaze", &[]);
    run(&mut s, 2);
    assert_supported(&s);
    let p = s.world.players[0].as_ref().unwrap();
    assert!(
        p.character.is_charmed(),
        "StarGaze did not charm the avatar"
    );
    assert!(p.controls_inverted);
    let hp = p.character.combat.health.hit_points;
    aim(&mut s, h);
    fire(&mut s, h, "Starfall", &[]);
    run(&mut s, 120);
    let p = s.world.players[0].as_ref().unwrap();
    assert!(
        p.character.combat.health.hit_points < hp || p.dead(),
        "Starfall missed avatar {avatar}"
    );
}

#[test]
fn mutate_beastman_raises_a_skeleton() {
    let Some((mut s, h)) = session("Grimnirs_Starspawn") else {
        return;
    };
    aim(&mut s, h);
    fire(&mut s, h, "MutateBeastman", &[]);
    run(&mut s, 60);
    assert_supported(&s);
    assert!(
        s.world.npcs.values().any(|n| n
            .character
            .definition
            .id
            .eq_ignore_ascii_case("skeleton_darksoul_elite_noscore")),
        "no skeleton rose"
    );
}

#[test]
fn healing_rain_heals_outdoors_and_arrow_rain_falls() {
    let Some((mut s, h)) = session("Dungeons_dwarf_adventurer") else {
        return;
    };
    let body = s.world.players[0].as_ref().unwrap().body;
    let at = s.world.physics.bodies[body].position;
    s.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .combat
        .health
        .hit_points -= 300.;
    let hurt = s.world.players[0]
        .as_ref()
        .unwrap()
        .character
        .combat
        .health
        .hit_points;
    fire(&mut s, h, "HealingRain", &[]);
    run(&mut s, 120);
    assert_supported(&s);
    let healed = s.world.players[0]
        .as_ref()
        .unwrap()
        .character
        .combat
        .health
        .hit_points;
    assert!(healed > hurt, "{hurt} -> {healed} at {at:?}");
    aim(&mut s, h);
    let missiles = s.world.missiles.len();
    fire(&mut s, h, "ArrowRain", &[1024, 1]);
    let mut most = 0;
    for _ in 0..60 * 4 {
        run(&mut s, 1);
        most = most.max(s.world.missiles.len());
    }
    assert!(most > missiles + 2, "no arrows fell ({missiles} -> {most})");
}
