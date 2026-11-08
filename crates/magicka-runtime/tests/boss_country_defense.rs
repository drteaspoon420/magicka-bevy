//! CountryDefense in Highlands hl_s5BF (research/boss-battles.md).
use magicka_runtime::{
    play_session::PlaySession, scene_loading::custom_hash, scene_world::boss_live::BossRef,
};

fn defense(
    s: &PlaySession,
) -> &magicka_runtime::scene_world::country_defense_live::CountryDefenseHost {
    s.world
        .boss_fight
        .bosses
        .iter()
        .find_map(|b| match b {
            BossRef::CountryDefense(c) => Some(c),
            _ => None,
        })
        .expect("no CountryDefense boss")
}

#[test]
fn houses_drive_the_icons_and_the_chieftain_drives_the_bar() {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Highlands.lvl", "Wizard").unwrap();
    s.change_scene_named("hl_s5BF", "start").unwrap();
    for _ in 0..30 {
        s.step(1. / 60.);
    }
    let c = defense(&s);
    let chieftain = c.chieftain.expect("chieftain not resolved");
    assert_eq!(
        s.world
            .entities
            .iter()
            .find(|e| e.handle == chieftain)
            .unwrap()
            .unique_id,
        custom_hash("#ent_beast_chieftain")
    );
    assert_eq!(c.slices, [0; 5], "undamaged houses use slice 0");
    let house0 = s
        .world
        .entities
        .iter()
        .find(|e| e.unique_id == custom_hash("house0"))
        .unwrap()
        .handle;
    let max = s.world.physics_damage[&house0].maximum;
    // Half a house's health (its template resists most elements).
    s.world.physics_damage.get_mut(&house0).unwrap().health = max * 0.5;
    s.step(1. / 60.);
    let fraction = s.world.physics_damage[&house0].health / s.world.physics_damage[&house0].maximum;
    let c = defense(&s);
    eprintln!("house0 at {fraction}: slices {:?}", c.slices);
    assert!(fraction < 1.0);
    assert_eq!(
        c.slices[0],
        magicka_runtime::scene_world::country_defense_live::house_slice(fraction)
    );
    assert_eq!(&c.slices[1..], &[0; 4]);
    // The boss bar is the chieftain's health.
    let n = &s.world.npcs[&chieftain];
    let expected = n.character.combat.health.hit_points / n.character.combat.health.max_hit_points;
    assert!(s.world.boss_fight.active);
    assert!(
        (s.world.boss_fight.normalized_health - expected).abs() < 1e-4
            || !s.world.boss_fight.running
    );
}

fn house(s: &PlaySession, i: usize) -> Option<u16> {
    s.world
        .entities
        .iter()
        .find(|e| e.unique_id == custom_hash(&format!("house{i}")) && !e.removed && !e.dead)
        .map(|e| e.handle)
}

#[test]
fn restored_house_icons_follow_the_restored_entities() {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Highlands.lvl", "Wizard").unwrap();
    s.player_ids[0] = Some("00000000000000000000000000000001".into());
    s.change_scene_named("hl_s5BF", "start").unwrap();
    s.step(1. / 60.);
    s.run_trigger("skip_intro", true).unwrap();
    for _ in 0..300 {
        s.step(1. / 60.);
    }
    assert!(s.checkpoint.is_some());
    for disk in [false, true] {
        if disk {
            let saved = s.disk_save().unwrap();
            s = PlaySession::load_disk_save(&root, &saved).unwrap();
        }
        assert!(s.restart_at_checkpoint().unwrap());
        s.step(1. / 60.);
        eprintln!("restored disk={disk}: {:?}", defense(&s));
        for i in 0..5 {
            let h = house(&s, i).expect("restored house");
            let state = s.world.physics_damage.get_mut(&h).unwrap();
            assert_eq!(state.health, state.maximum);
            state.health = state.maximum * [0., 0.2, 0.5, 0.9, 1.][i];
        }
        s.step(1. / 60.);
        eprintln!("damaged disk={disk}: {:?}", defense(&s));
        assert_eq!(defense(&s).slices, [3, 2, 1, 0, 0]);
    }
}

/// A torched house explodes (onDeath "houseN explodes": the houses counter
/// drops and its icon shows the burnt slice); dying after the "Start"
/// checkpoint restores every house, the counter and the icons.
#[test]
fn destroyed_houses_return_on_checkpoint_restart() {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Highlands.lvl", "Wizard").unwrap();
    s.change_scene_named("hl_s5BF", "start").unwrap();
    for _ in 0..30 {
        s.step(1. / 60.);
    }
    // Damage taken before the checkpoint is not kept: PhysicsEntity.State
    // re-initializes restored props at full health.
    let h1 = house(&s, 1).unwrap();
    let d1 = s.world.physics_damage.get_mut(&h1).unwrap();
    d1.health = d1.maximum * 0.5;
    s.run_trigger("Start", true).unwrap();
    for _ in 0..120 {
        s.step(1. / 60.);
    }
    assert!(s.checkpoint.is_some(), "Start did not take a checkpoint");
    let houses = custom_hash("houses");
    assert_eq!(s.runtime.variables.counter(houses), 5);
    // Burn house0 down.
    let h0 = house(&s, 0).unwrap();
    s.world.physics_damage.get_mut(&h0).unwrap().health = 1.0;
    s.world
        .damage_physics(
            h0,
            magicka_sim::damage::Damage {
                attack: magicka_sim::damage::attack::DAMAGE,
                element: magicka_sim::spells::Elements::FIRE.0,
                amount: 10_000.0,
                magnitude: 1.0,
            },
            15,
        )
        .unwrap();
    for _ in 0..60 {
        s.step(1. / 60.);
    }
    eprintln!(
        "after torching: house0 {:?}, counter {}, slices {:?}",
        house(&s, 0),
        s.runtime.variables.counter(houses),
        defense(&s).slices
    );
    assert!(house(&s, 0).is_none(), "house0 survived");
    assert_eq!(s.runtime.variables.counter(houses), 4, "onDeath not run");
    assert_eq!(defense(&s).slices[0], 3, "burnt icon");
    // Death → checkpoint restart.
    assert!(s.restart_at_checkpoint().unwrap());
    for _ in 0..30 {
        s.step(1. / 60.);
    }
    for i in 0..5 {
        let h = house(&s, i).unwrap_or_else(|| panic!("house{i} not restored"));
        let d = &s.world.physics_damage[&h];
        assert_eq!(d.health, d.maximum, "house{i} health");
    }
    assert_eq!(s.runtime.variables.counter(houses), 5);
    assert_eq!(defense(&s).slices, [0; 5]);
    assert!(s.world.boss_fight.active);
}

/// The raid: "Trickle" spawns beastman_torchers and "Torch house0" orders
/// them to attack house0 (priorityTarget, a non-Character IDamageable);
/// once it burns, "Torch house1" moves them on.
#[test]
fn torchers_attack_the_first_house() {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Highlands.lvl", "Wizard").unwrap();
    s.change_scene_named("hl_s5BF", "start").unwrap();
    for _ in 0..30 {
        s.step(1. / 60.);
    }
    s.run_trigger("Start", true).unwrap();
    let h0 = house(&s, 0).unwrap();
    let max = s.world.physics_damage[&h0].maximum;
    let mut damaged_at = None;
    for frame in 0..(90 * 60) {
        s.step(1. / 60.);
        // Keep the wizard alive and out of the way.
        let p = s.world.players[0].as_mut().unwrap();
        p.character.combat.health.hit_points = p.character.combat.health.max_hit_points;
        let health = s.world.physics_damage.get(&h0).map_or(0.0, |d| d.health);
        if health < max {
            damaged_at = Some(frame as f32 / 60.0);
            break;
        }
    }
    eprintln!(
        "house0 first damaged at {damaged_at:?} s; slices {:?}",
        defense(&s).slices
    );
    assert!(damaged_at.is_some(), "no torcher damaged house0 in 90 s");
    // House0 burns down, then "Torch house1" retargets the torchers.
    let h1 = house(&s, 1).unwrap();
    let mut burnt = None;
    for frame in 0..(240 * 60) {
        s.step(1. / 60.);
        let p = s.world.players[0].as_mut().unwrap();
        p.character.combat.health.hit_points = p.character.combat.health.max_hit_points;
        if burnt.is_none() && house(&s, 0).is_none() {
            burnt = Some(frame as f32 / 60.0);
        }
        if burnt.is_some()
            && s.world.physics_damage[&h1].health < s.world.physics_damage[&h1].maximum
        {
            break;
        }
    }
    let h1_health = s.world.physics_damage[&h1].health;
    eprintln!(
        "house0 burnt after {burnt:?} s more; house1 {h1_health}; slices {:?}; houses counter {}",
        defense(&s).slices,
        s.runtime.variables.counter(custom_hash("houses"))
    );
    assert!(burnt.is_some(), "house0 never burnt down");
    assert!(
        h1_health < s.world.physics_damage[&h1].maximum,
        "house1 not attacked next"
    );
}

/// The skip-intro path into the raid (as the viewer QA scenario runs it).
#[test]
fn skip_intro_starts_the_raid() {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Highlands.lvl", "Wizard").unwrap();
    s.change_scene_named("hl_s5BF", "start").unwrap();
    for _ in 0..30 {
        s.step(1. / 60.);
    }
    s.run_trigger("skip_intro", true).unwrap();
    let h0 = house(&s, 0).unwrap();
    for frame in 0..(60 * 60) {
        s.step(1. / 60.);
        let p = s.world.players[0].as_mut().unwrap();
        p.character.combat.health.hit_points = p.character.combat.health.max_hit_points;
        if frame % 600 == 0 {
            let t: Vec<_> = s
                .world
                .npcs
                .iter()
                .filter(|(_, n)| !n.dead())
                .map(|(h, n)| (*h, n.combat.priority_target))
                .collect();
            eprintln!(
                "t={} boss_active {} house0 {} npcs {t:?}",
                frame / 60,
                s.runtime.variables.counter(custom_hash("boss_active")),
                s.world.physics_damage[&h0].health
            );
        }
    }
    assert!(s.world.physics_damage[&h0].health < s.world.physics_damage[&h0].maximum);
}
