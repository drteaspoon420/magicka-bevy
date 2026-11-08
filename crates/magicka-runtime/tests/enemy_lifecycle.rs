use magicka_runtime::{avatar::PlayerInput, play_session::PlaySession};
use magicka_sim::{
    damage::{attack, Damage},
    spells::Elements,
};
use std::path::Path;
fn session() -> Option<PlaySession> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    Some(s)
}
fn frame(s: &mut PlaySession) {
    s.step(1. / 60.);
    s.handle_input(PlayerInput::default(), 16. / 9.);
}
fn hit(s: &mut PlaySession, h: u16, amount: f32, element: Elements) {
    let p = s.world.physics.bodies[s.world.npcs[&h].body].position;
    s.world.arc_damage(
        None,
        None,
        p,
        [0., 0., -1.],
        0.1,
        std::f32::consts::PI,
        &[Damage {
            attack: attack::DAMAGE,
            element: element.0,
            amount,
            magnitude: 1.,
        }],
        s.world.play_time,
    );
}
#[test]
fn warrior_shield_loss_spawns_one_live_unshielded_warrior() {
    let Some(mut s) = session() else {
        return;
    };
    let h = s.spawn_npc("Goblin_warrior", "idle", 5.).unwrap();
    magicka_runtime::entity_command::EntityCommandHost::kill(&mut s.world, h).unwrap();
    for _ in 0..180 {
        frame(&mut s);
    }
    assert!(
        s.world
            .entities
            .iter()
            .find(|e| e.handle == h)
            .unwrap()
            .removed
    );
    let children: Vec<_> = s
        .world
        .npcs
        .values()
        .filter(|n| n.definition.id == "goblin_warrior_noshield")
        .collect();
    assert_eq!(children.len(), 1, "{:?}", s.world.log);
    assert!(!children[0].dead());
    assert!(s.world.agents.contains(&children[0].handle));
    assert!(s.world.physics.bodies[children[0].body].active);
    let replacement = children[0].handle;
    let mut expected = children[0].character.combat.health.hit_points;
    // The live replacement must use its own template: Fire halves the entire
    // mixed entry, while Lightning multiplies it by 1.5 after shield loss.
    for (element, lost) in [
        (Elements::ARCANE | Elements::FIRE, 10.0),
        (Elements::ARCANE | Elements::LIGHTNING, 30.0),
    ] {
        direct_hit(
            &mut s,
            replacement,
            Damage {
                attack: attack::DAMAGE,
                element: element.0,
                amount: 20.0,
                magnitude: 1.0,
            },
            -1.0,
        );
        expected -= lost;
        assert_eq!(
            s.world.npcs[&replacement]
                .character
                .combat
                .health
                .hit_points,
            expected,
            "replacement damage for {element:?}"
        );
    }
}
#[test]
fn bomber_one_fire_damage_overkills_and_spawns_four_live_bombs() {
    let Some(mut s) = session() else {
        return;
    };
    let h = s.spawn_npc("Goblin_bomber", "idle", 5.).unwrap();
    hit(&mut s, h, 1., Elements::FIRE);
    assert!(s.world.npcs[&h].character.combat.overkilled);
    frame(&mut s);
    assert_eq!(s.world.missiles.len(), 4, "{:?}", s.world.log);
    assert!(s
        .world
        .missiles
        .iter()
        .all(|m| m.body.item_model.as_deref() == Some("Models/Items_NPC/goblin_bomb_0")));
    for _ in 0..300 {
        frame(&mut s);
    }
    assert!(s.world.missiles.is_empty(), "fuse must remove the bombs");
}
#[test]
fn shaman_death_emits_particles_then_removes_actor() {
    let Some(mut s) = session() else {
        return;
    };
    let h = s.spawn_npc("Goblin_shaman", "idle", 5.).unwrap();
    magicka_runtime::entity_command::EntityCommandHost::kill(&mut s.world, h).unwrap();
    let mut names = Vec::new();
    for _ in 0..180 {
        s.world.update_npcs(1. / 60.);
        let Some(npc) = s.world.npcs.get_mut(&h) else {
            break;
        };
        for event in npc.animation_events.drain(..) {
            if let magicka_runtime::avatar::AvatarEvent::AnimationEffect {
                effect,
                first: true,
                ..
            } = event
            {
                names.push(effect);
            }
        }
    }
    assert!(
        names.iter().any(|n| n == "goblin_shaman_particles"),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n == "goblin_shaman_explode"),
        "{names:?}"
    );
    assert!(
        s.world
            .entities
            .iter()
            .find(|e| e.handle == h)
            .unwrap()
            .removed
    );
}
#[test]
fn warrior_closes_distance_and_lands_a_melee_hit() {
    let Some(mut s) = session() else {
        return;
    };
    let h = s.spawn_npc("Goblin_warrior", "attack", 7.).unwrap();
    let initial = s.avatar().character.combat.health.hit_points;
    // The lone campaign avatar's revival fairy restores full health once the
    // warrior kills it (revival-fairy.md), so track the lowest health seen.
    let mut lowest = initial;
    for _ in 0..900 {
        frame(&mut s);
        lowest = lowest.min(s.avatar().character.combat.health.hit_points);
    }
    assert!(
        lowest < initial,
        "warrior stopped without hitting: {:?} player {:?}",
        s.world.physics.bodies[s.world.npcs[&h].body].position,
        s.body().position
    );
}

fn direct_hit(s: &mut PlaySession, h: u16, damage: Damage, side: f32) {
    let npc = s.world.npcs.get_mut(&h).unwrap();
    let body = &mut s.world.physics.bodies[npc.body];
    let attack_position = std::array::from_fn(|i| body.position[i] + body.direction()[i] * side);
    magicka_runtime::avatar::deliver_to_body(
        &mut npc.character,
        body,
        &[magicka_sim::resistance::Resistance::IDENTITY; 11],
        false,
        h,
        magicka_sim::damage_delivery::DamagePayload::Single(damage),
        None,
        attack_position,
        s.world.play_time,
    );
}

/// Shield-stage destruction releases gib index 0 (the shield), without a
/// whole-body explosion. Both native Death and OverKill recipes use this.
#[test]
fn warrior_shield_debris_has_no_body_gibs_or_gore_burst() {
    for amount in [100.0, 1000.0] {
        let Some(mut s) = session() else { return };
        let h = s.spawn_npc("Goblin_warrior", "idle", 5.).unwrap();
        assert!(s.world.npcs[&h].definition.gibs[0]
            .model
            .ends_with("goblin_shield_0"));
        direct_hit(
            &mut s,
            h,
            Damage {
                attack: attack::DAMAGE,
                element: Elements::ARCANE.0,
                amount,
                magnitude: 1.0,
            },
            1.0,
        );
        // Advance the death state. Inspect the lifecycle's requests before
        // presentation drains them, stopping at the authored replacement.
        for _ in 0..120 {
            s.world.update_npcs(1. / 60.);
            if !s.world.gibs.live.is_empty() {
                break;
            }
        }
        assert_eq!(
            s.world.gibs.live.len(),
            1,
            "damage {amount}: {:?}",
            s.world.log
        );
        assert!(s.world.gibs.live[0].model.ends_with("goblin_shield_0"));
        assert!(
            s.world
                .gib_visuals
                .iter()
                .all(|v| { matches!(v, magicka_runtime::gibs::GibVisual::Trail { .. }) }),
            "shield loss must not emit a character gore burst: {:?}",
            s.world.gib_visuals
        );
        assert_eq!(
            s.world
                .npcs
                .values()
                .filter(|n| { n.definition.id == "goblin_warrior_noshield" && !n.dead() })
                .count(),
            1
        );
        let replacement = s
            .world
            .npcs
            .values()
            .find(|n| n.definition.id == "goblin_warrior_noshield")
            .unwrap();
        assert_eq!(
            replacement.character.combat.health.hit_points, 900.0,
            "even overkill of the shield stage spawns the new actor at full health"
        );
        for _ in 0..180 {
            frame(&mut s);
        }
        assert_eq!(s.world.gibs.live.len(), 1, "no late body-gib explosion");
    }
}

/// Retail resistances belong to the shield stage's character template. The
/// equipped shield has no resistance entries and the template has no Block
/// ability: these are element/amount rules, not a facing-dependent shield.
#[test]
fn warrior_shield_stage_applies_the_authored_element_resistances() {
    let cases = [
        (Elements::FIRE, 100.0, attack::DAMAGE, 100.0),
        (Elements::LIGHTNING, 100.0, attack::DAMAGE, 100.0),
        (Elements::POISON, 100.0, attack::DAMAGE, 100.0),
        (Elements::EARTH, 500.0, attack::DAMAGE, 100.0),
        (Elements::EARTH, 600.0, attack::DAMAGE, 50.0),
        (Elements::ARCANE, 20.0, attack::DAMAGE, 80.0),
        (Elements::ICE, 20.0, attack::DAMAGE, 80.0),
        (
            Elements::ARCANE | Elements::FIRE,
            20.0,
            attack::DAMAGE,
            100.0,
        ),
        (
            Elements::EARTH,
            50.0,
            attack::DAMAGE | attack::ARMOUR_PIERCING,
            50.0,
        ),
    ];
    let Some(mut s) = session() else { return };
    for side in [-1.0, 1.0] {
        for (element, amount, flags, expected) in cases {
            let h = s.spawn_npc("Goblin_warrior", "idle", 5.).unwrap();
            direct_hit(
                &mut s,
                h,
                Damage {
                    attack: flags,
                    element: element.0,
                    amount,
                    magnitude: 1.0,
                },
                side,
            );
            for _ in 0..60 {
                s.world.update_npcs(1. / 60.);
            }
            assert_eq!(
                s.world.npcs[&h].character.combat.health.hit_points, expected,
                "element {element:?}, amount {amount}, flags {flags}, side {side}"
            );
            magicka_runtime::entity_command::EntityCommandHost::terminate(&mut s.world, h).unwrap();
        }
    }
}
