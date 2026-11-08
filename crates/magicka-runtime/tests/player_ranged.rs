use magicka_runtime::{avatar::PlayerInput, play_session::PlaySession};
use std::collections::BTreeSet;
const DT: f32 = 1.0 / 60.0;

fn session(weapon: &str, pickup: bool) -> Option<PlaySession> {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    s.world.camera.lock_input = false;
    if pickup {
        let mut transform = magicka_content::skinned::IDENTITY;
        transform[12..15].copy_from_slice(&s.body().position);
        let h = s.spawn_item_pickup(weapon, transform).unwrap();
        s.pickup_item(h).unwrap();
    } else {
        s.debug_player_loadout(
            "Wizard",
            &s.avatar()
                .items
                .iter()
                .find(|(slot, _)| *slot == 1)
                .unwrap()
                .1
                .definition
                .name
                .clone(),
            weapon,
        )
        .unwrap();
    }
    for _ in 0..180 {
        s.step(DT);
    }
    Some(s)
}

#[test]
fn player_crossbow_fires_through_input_and_pickups() {
    for (name, pickup) in [("weapon_xbow", true)] {
        let Some(mut s) = session(name, pickup) else {
            return;
        };
        let held = &s
            .avatar()
            .items
            .iter()
            .find(|(slot, _)| *slot == 0)
            .unwrap()
            .1;
        let ranged = held
            .melee
            .ranged
            .clone()
            .expect("player projectile metadata retained");
        let cooldown = held.definition.cooldown_time;
        let me = s.avatar().handle;
        let mut ids = BTreeSet::new();
        let mut first_spawn = None;
        let mut saw_cooldown = false;
        for tick in 0..240 {
            s.handle_input(
                PlayerInput {
                    shift_down: true,
                    move_down: true,
                    ..Default::default()
                },
                16.0 / 9.0,
            );
            s.step(DT);
            saw_cooldown |= s.avatar().items[0].1.cooldown > 0.;
            for m in s.world.missiles.iter().filter(|m| m.body.owner == me) {
                if ids.insert(m.body.id) {
                    first_spawn.get_or_insert(tick);
                    assert_eq!(m.body.item_model, ranged.model);
                    assert!((m.body.radius - ranged.radius).abs() < 1e-5);
                    assert!(m.body.velocity.iter().all(|v| v.is_finite()));
                    assert!(m.body.homing.is_none(), "player shots are not NPC-targeted");
                }
            }
        }
        eprintln!(
            "{name}: shots={}, first={first_spawn:?}, cooldown={cooldown}, model={:?}",
            ids.len(),
            ranged.model
        );
        assert!(
            !ids.is_empty(),
            "{name}: SpawnMissile must execute from the attack animation"
        );
        assert!(ids.len() >= 2, "holding fire must repeat");
        assert!(
            cooldown == 0. || saw_cooldown,
            "firing consumes the item cooldown"
        );
        assert!(
            ids.len() as f32 <= 4.0 / cooldown.max(DT) + 2.,
            "item cooldown bounds firing rate"
        );
    }
}

fn cursor(s: &PlaySession, position: [f32; 3]) -> [f32; 2] {
    let p = [position[0], s.body().position[1], position[2], 1.];
    let vp = magicka_runtime::camera::multiply(
        &s.world.camera.view(),
        &s.world.camera.projection(16. / 9.),
    );
    let clip: [f32; 4] = std::array::from_fn(|j| (0..4).map(|k| p[k] * vp[k * 4 + j]).sum());
    [clip[0] / clip[3], clip[1] / clip[3]]
}
#[test]
fn player_guns_fire_timed_clips_damage_targets_and_stop_on_release() {
    for weapon in [
        "weapon_pistol",
        "weapon_m16",
        "weapon_m60",
        "weapon_tommygun",
    ] {
        let Some(mut s) = session(weapon, true) else {
            return;
        };
        let target = s.spawn_npc("Orc_regular", "idle", 6.).unwrap();
        let body = s.world.npcs[&target].body;
        s.world.npcs.get_mut(&target).unwrap().combat.abilities = std::sync::Arc::new(Vec::new());
        s.world
            .npcs
            .get_mut(&target)
            .unwrap()
            .character
            .combat
            .health
            .hit_points = 100000.;
        s.world
            .npcs
            .get_mut(&target)
            .unwrap()
            .character
            .combat
            .health
            .max_hit_points = 100000.;
        s.world
            .npcs
            .get_mut(&target)
            .unwrap()
            .character
            .definition
            .max_hit_points = 100000.;
        let definition = &s
            .avatar()
            .items
            .iter()
            .find(|(slot, _)| *slot == 0)
            .unwrap()
            .1
            .definition;
        let sprites: Vec<_> = [&definition.tracer_sprite, &definition.non_tracer_sprite]
            .into_iter()
            .filter_map(|s| magicka_runtime::gun::sprite(s))
            .collect();
        eprintln!(
            "{weapon}: range={} accuracy={} conditions={:?}",
            definition.gun_range, definition.gun_accuracy, definition.gun_conditions
        );
        let mut shots = 0;
        let mut observed_tracer = false;
        let me = s.avatar().handle;
        for tick in 0..180 {
            let at = s.world.physics.bodies[body].position;
            let aim = cursor(&s, at);
            s.handle_input(
                PlayerInput {
                    cursor_ndc: Some(aim),
                    shift_down: true,
                    move_down: tick < 120,
                    ..Default::default()
                },
                16. / 9.,
            );
            // World entity update before session drains shot presentation requests.
            s.step(DT);
            shots = s
                .avatar()
                .items
                .iter()
                .find(|(slot, _)| *slot == 0)
                .unwrap()
                .1
                .gun
                .tracer_count;
            observed_tracer |= s
                .effects
                .particles
                .active()
                .iter()
                .any(|p| sprites.contains(&p.sprite));
            assert!(
                s.world.missiles.iter().all(|m| m.body.owner != me),
                "guns are hitscan, not crossbow missiles"
            );
        }
        let held = &s
            .avatar()
            .items
            .iter()
            .find(|(slot, _)| *slot == 0)
            .unwrap()
            .1;
        eprintln!(
            "{weapon}: gun rate={} clip={} shots={shots} health={} sprites={}/{}",
            held.definition.gun_rate,
            held.definition.gun_clip,
            s.world.npcs[&target].character.combat.health.hit_points,
            held.definition.tracer_sprite,
            held.definition.non_tracer_sprite
        );
        assert!(shots > 0, "authored Gunfire must fire");
        assert!(
            s.world.npcs[&target].character.combat.health.hit_points < 100000.,
            "gun Hit conditions must deal damage"
        );
        assert!(!held.gun.firing, "release must stop firing");
        assert!(s
            .effects
            .particles
            .active()
            .iter()
            .all(|p| p.position.iter().all(|v| v.is_finite())));
        assert!(
            sprites.is_empty() || observed_tracer,
            "authored gun tracers must reach the presentation particle system"
        );
    }
}

#[test]
fn gun_timer_and_stop_rules_match_native_branches() {
    use magicka_runtime::gun::GunState;
    let mut gun = GunState::default();
    gun.execute(3, 1.);
    assert!(gun.tick(DT, 10));
    assert_eq!(gun.clip, 2);
    assert!(!gun.tick(DT, 10));
    gun.stop(true, true, 16);
    assert!(gun.firing, "machinegun continues while attack is active");
    gun.stop(true, false, 16);
    assert!(!gun.firing);
    gun.execute(3, 1.);
    gun.stop(true, true, 14);
    assert!(!gun.firing);
    gun.execute(3, 1.);
    assert!(gun.tick(1., 10));
    assert_eq!(gun.clip, 2, "slow frames do not loop over missed shots");
}

#[test]
fn item_aligned_player_shots_use_the_held_item_transform() {
    let Some(mut s) = session("weapon_xbow", true) else {
        return;
    };
    let held = &s
        .avatar()
        .items
        .iter()
        .find(|(slot, _)| *slot == 0)
        .unwrap()
        .1;
    let frame = held.frame.unwrap();
    let velocity = [0., 0., -20.];
    let expected: [f32; 3] = std::array::from_fn(|i| -20. * frame.item[8 + i]);
    let me = s.avatar().handle;
    s.world.players[0]
        .as_mut()
        .unwrap()
        .missile_requests
        .push((0, Some(velocity), true));
    s.world.update_players(0.);
    let shot = s
        .world
        .missiles
        .iter()
        .find(|m| m.body.owner == me)
        .unwrap();
    for (actual, expected) in shot.body.velocity.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 0.001);
    }
}
