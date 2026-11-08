use magicka_runtime::{play_session::PlaySession, scene_loading::custom_hash};
use magicka_sim::{
    character_impact::result as R,
    damage::{attack as A, Damage},
    spells::Elements as E,
};
use std::path::Path;

#[test]
fn arc_damage_hits_props_with_falloff_callbacks_and_query_exclusions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let mut transform = magicka_content::skinned::IDENTITY;
    let p = s.body().position;
    let origin = [p[0], p[1] + 20., p[2]];
    transform[12..15].copy_from_slice(&[origin[0], origin[1], origin[2] + 3.]);
    let prop = s.spawn_physics_prop("dungeon_wall", transform).unwrap();
    let radius = s
        .world
        .entities
        .iter()
        .find(|e| e.handle == prop)
        .unwrap()
        .radius;
    let state = s.world.physics_damage.get_mut(&prop).unwrap();
    state
        .resistances
        .fill(magicka_sim::resistance::Resistance::IDENTITY);
    state.on_damage = custom_hash("arc_prop_damage");
    state.on_death = custom_hash("arc_prop_death");
    let before = state.health;
    let damage = Damage {
        attack: A::DAMAGE,
        element: E::EARTH.0,
        amount: 10.,
        magnitude: 1.,
    };
    let owner = s.avatar().handle;
    assert!(s.world.arc_damage(
        Some(owner),
        None,
        origin,
        [0., 0., 1.],
        6.,
        std::f32::consts::FRAC_PI_4,
        &[damage],
        0.
    ));
    // DamageablePhysicsEntity truncates the scaled damage to an integer.
    let expected = before - (10. * (1. - (3. - radius) / 6.)) as i32 as f32;
    assert_eq!(s.world.physics_damage[&prop].health, expected);
    assert!(s
        .world
        .pending_triggers
        .contains(&(custom_hash("arc_prop_damage"), None)));
    let after = s.world.physics_damage[&prop].health;
    for (ignore, origin, direction, range) in [
        (Some(prop), origin, [0., 0., 1.], 6.),
        (None, origin, [0., 0., -1.], 6.),
        (
            None,
            [origin[0], origin[1] + 100., origin[2]],
            [0., 0., 1.],
            1.,
        ),
    ] {
        s.world.arc_damage(
            Some(owner),
            ignore,
            origin,
            direction,
            range,
            std::f32::consts::FRAC_PI_4,
            &[damage],
            0.,
        );
        assert_eq!(s.world.physics_damage[&prop].health, after);
    }
    let lethal = Damage {
        amount: before * 100.,
        ..damage
    };
    s.world.arc_damage(
        Some(owner),
        None,
        origin,
        [0., 0., 1.],
        6.,
        std::f32::consts::FRAC_PI_4,
        &[lethal],
        0.,
    );
    assert!(s
        .world
        .pending_triggers
        .contains(&(custom_hash("arc_prop_death"), None)));
    assert!(
        s.world
            .entities
            .iter()
            .find(|e| e.handle == prop)
            .unwrap()
            .dead
    );
}

#[test]
fn prop_solid_flag_vetoes_response_without_skipping_missile_callback() {
    use magicka_runtime::missiles::LiveMissile;
    use magicka_sim::missile_events::{condition, EventCondition, MissileConditions, MissileEvent};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    for solid in [false, true] {
        let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        s.disable_scripts();
        for _ in 0..60 {
            s.step(1.0 / 60.0);
        }
        let mut transform = magicka_content::skinned::IDENTITY;
        transform[12..15].copy_from_slice(&s.body().position);
        transform[13] += 20.0;
        let prop = s.spawn_physics_prop("dungeon_wall", transform).unwrap();
        let state = s.world.physics_damage.get_mut(&prop).unwrap();
        // Authored variants of a decoded box isolate the native solid veto.
        // A resisted hit produces Deflected, which the missile accepts.
        state.solid = solid;
        state.resistances[E::EARTH.0.trailing_zeros() as usize].multiplier = 0.0;
        let bounds = state.world_box(&transform).unwrap();
        let outward = bounds.axes[0];
        let missile_half = 0.1;
        let extent = bounds.side_lengths[0] * 0.5
            + missile_half * outward.iter().map(|v| v.abs()).sum::<f32>();
        let position = std::array::from_fn(|i| bounds.center[i] + outward[i] * (extent + 0.01));
        let mut conditions = MissileConditions::default();
        conditions.slots[0].condition = EventCondition {
            flags: condition::HIT,
            ..Default::default()
        };
        conditions.slots[0].events = vec![MissileEvent::Damage {
            damage: Damage {
                attack: A::DAMAGE,
                element: E::EARTH.0,
                amount: 10.0,
                magnitude: 1.0,
            },
            velocity_based: false,
        }];
        s.world.missiles.push(LiveMissile::spawn_custom(
            950,
            s.avatar().handle,
            position,
            outward.map(|v| -v * 10.0),
            missile_half,
            false,
            conditions,
        ));
        s.step(1.0 / 60.0);
        let missile = s.world.missiles.iter().find(|m| m.body.id == 950).unwrap();
        assert!(
            missile.state.hits.contains(prop),
            "even a non-solid prop receives the missile callback"
        );
        assert!(
            missile.state.has_collided,
            "PhysicsObjectBody marks the contact"
        );
        let velocity = (0..3)
            .map(|i| missile.body.velocity[i] * outward[i])
            .sum::<f32>();
        if solid {
            assert!(velocity > 0.0, "solid prop response: {velocity}");
        } else {
            assert!(velocity < 0.0, "non-solid prop veto: {velocity}");
        }
    }
}

#[test]
fn missile_blast_damages_nearby_props_and_excludes_distant_props() {
    use magicka_runtime::{avatar::PlayerInput, missiles::LiveMissile};
    use magicka_sim::missile_events::{condition, EventCondition, MissileConditions, MissileEvent};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        s.step(1.0 / 60.0);
        s.handle_input(PlayerInput::default(), 16.0 / 9.0);
    }
    let position = s.body().position;
    let direction = s.body().direction();
    let mut transform = magicka_content::skinned::IDENTITY;
    for i in 0..3 {
        transform[12 + i] = position[i] + direction[i] * 3.0;
    }
    let nearby = s.spawn_physics_prop("rope_mechanism", transform).unwrap();
    let blast = [transform[12], transform[13] + 1.0, transform[14]];
    for i in 0..3 {
        transform[12 + i] = position[i] + direction[i] * 15.0;
    }
    let distant = s.spawn_physics_prop("rope_mechanism", transform).unwrap();
    let initial = s.world.physics_damage[&distant].health;
    let mut damages = [Damage::default(); 5];
    damages[0] = Damage {
        attack: A::DAMAGE,
        element: E::EARTH.0,
        amount: 1000.0,
        magnitude: 1.0,
    };
    let mut conditions = MissileConditions::default();
    conditions.slots[0].condition = EventCondition {
        flags: condition::TIMER,
        time: 0.1,
        ..Default::default()
    };
    conditions.slots[0].events = vec![
        MissileEvent::Blast {
            radius: 5.0,
            damages,
        },
        MissileEvent::Remove { bounces: 0 },
    ];
    // Exercise the normal missile-event/full-blast host with an authored
    // timed recipe, without calling a direct prop damage method.
    let missile = LiveMissile::spawn_custom(
        900,
        s.avatar().handle,
        blast,
        [0.0; 3],
        0.05,
        false,
        conditions,
    );
    s.world.missiles.push(missile);
    for _ in 0..60 {
        s.step(1.0 / 60.0);
        s.handle_input(PlayerInput::default(), 16.0 / 9.0);
    }
    assert!(
        s.world.physics_damage[&nearby].health <= 0.0,
        "blast did not damage its nearby prop"
    );
    assert_eq!(s.world.physics_damage[&distant].health, initial);
}

#[test]
fn melee_prop_target_flag_and_swept_box_distance() {
    use magicka_runtime::npc_combat::{MeleeGeometry, MeleeTarget};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let me = s.world.combatants()[0];
    let mut weapon = s
        .avatar()
        .items
        .iter()
        .find(|(slot, _)| *slot == 0)
        .unwrap()
        .1
        .melee
        .clone();
    let bounds = magicka_sim::box_intersection::OrientedBox {
        center: [0.0; 3],
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        side_lengths: [2.0; 3],
    };
    let candidate = MeleeTarget {
        handle: 100,
        query: magicka_sim::entity_query::QueryEntity {
            position: [0.0; 3],
            radius: 3.0f32.sqrt(),
            dead: false,
            shield: false,
            damageable: true,
        },
        geometry: MeleeGeometry::Prop(bounds),
    };
    // The weapon's current endpoint misses the box, but its sweep crosses it.
    weapon.last_attach = [-2.0, 0.0, 0.0];
    weapon.attach = [2.0, 0.0, 0.0];
    assert!(
        weapon.execute_targets(me, None, 3, &[candidate]).is_empty(),
        "friendly/enemy flags do not include props"
    );
    assert!(
        !weapon.execute_targets(me, None, 4, &[candidate]).is_empty(),
        "non-character flag includes swept box hits"
    );
    weapon.clear_hitlist();
    weapon.last_attach = [2.0, 10.0, 0.0];
    weapon.attach = [-2.0, 10.0, 0.0];
    assert!(
        weapon.execute_targets(me, None, 4, &[candidate]).is_empty(),
        "a distant box cannot be hit"
    );
}

#[test]
fn spell_input_reaches_castle_puzzle_props() {
    use magicka_runtime::{avatar::PlayerInput, spawn_phys::SpawnPhysHost};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    // Isolate combat targeting from the tutorial's permissions and triggers.
    // The prop templates and element/cast input are the actual gameplay path.
    for (name, keys, lethal) in [
        ("generator", &[4][..], false),
        ("generator", &[5, 4][..], false), // Arcane + Lightning beam.
        ("torch_mechanism", &[7][..], true),
        ("furnace_fire", &[0][..], true),
    ] {
        let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        s.disable_scripts();
        for _ in 0..60 {
            s.step(1.0 / 60.0);
            s.handle_input(PlayerInput::default(), 16.0 / 9.0);
        }
        let p = s.body().position;
        let d = s.body().direction();
        let mut transform = magicka_content::skinned::IDENTITY;
        for i in 0..3 {
            transform[12 + i] = p[i] + d[i] * 3.0;
        }
        let handle = s.spawn_physics_prop(name, transform).unwrap();
        let damaged = custom_hash("prop damaged");
        let killed = custom_hash("prop killed");
        s.world
            .set_damage_triggers(handle, killed, damaged)
            .unwrap();
        let layout = magicka_runtime::scene_script::SceneScriptLayout::read(br#"<Scene>
            <trigger id="prop damaged" autorun="false" repeat="0"><then><counter name="damage callbacks" action="inc" value="1"/></then></trigger>
            <trigger id="prop killed" autorun="false" repeat="false"><then><counter name="death callbacks" action="inc" value="1"/></then></trigger>
        </Scene>"#, false).unwrap();
        s.runtime = magicka_runtime::script_runtime::ScriptRuntime::new(layout, Default::default());
        s.script_error = None;
        let initial = s.world.physics_damage[&handle].health;
        let mut elements = [false; 8];
        for &key in keys {
            elements[key] = true;
        }
        s.handle_input(
            PlayerInput {
                elements,
                ..Default::default()
            },
            16.0 / 9.0,
        );
        for _ in 0..90 {
            s.step(1.0 / 60.0);
            s.handle_input(PlayerInput::default(), 16.0 / 9.0);
        }
        s.world.pending_triggers.clear();
        for i in 0..180 {
            s.step(1.0 / 60.0);
            s.handle_input(
                PlayerInput {
                    cast_down: i < 120,
                    ..Default::default()
                },
                16.0 / 9.0,
            );
        }
        let state = &s.world.physics_damage[&handle];
        assert!(
            state.health < initial,
            "{name}: {initial} -> {}",
            state.health
        );
        assert!(
            s.runtime.variables.counter(custom_hash("damage callbacks")) > 0,
            "{name}: missing damage callback"
        );
        if lethal {
            assert!(
                state.health <= 0.0,
                "{name}: not destroyed ({})",
                state.health
            );
            assert_eq!(
                s.runtime.variables.counter(custom_hash("death callbacks")),
                1,
                "{name}: death callback"
            );
        }
        let entity = s
            .world
            .entities
            .iter()
            .find(|e| e.handle == handle)
            .unwrap();
        assert_eq!(entity.hit_points, state.health);
        assert_eq!(entity.dead, state.health <= 0.0);
    }
}

#[test]
fn castle_generator_resistances_callbacks_and_respawn() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    for _ in 0..15 {
        s.step(1. / 60.);
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    let handle = s
        .world
        .entities
        .iter()
        .find(|e| e.unique_id == custom_hash("generator1"))
        .unwrap()
        .handle;
    let state = s.world.physics_damage.get(&handle).unwrap();
    assert_eq!(state.on_damage, custom_hash("generator_1"));
    let initial = state.health;
    s.world.pending_triggers.clear();
    let damage = Damage {
        attack: A::DAMAGE,
        element: E::LIGHTNING.0,
        amount: 100.,
        magnitude: 1.,
    };
    let result = s.world.damage_physics(handle, damage, 15).unwrap();
    assert_ne!(result & R::DAMAGED, 0);
    assert!(s.world.physics_damage[&handle].health < initial);
    assert_eq!(
        s.world.pending_triggers,
        vec![(custom_hash("generator_1"), None)]
    );
    // A new level load spawns fresh props. Re-entering the same GameScene
    // would keep its fired startoflevel trigger, as the native scene does.
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    for _ in 0..15 {
        s.step(1. / 60.);
    }
    let handle = s
        .world
        .entities
        .iter()
        .find(|e| e.unique_id == custom_hash("generator1"))
        .unwrap()
        .handle;
    assert_eq!(s.world.physics_damage[&handle].health, initial);
    assert_eq!(s.world.physics_damage[&handle].status_mask, 0);
}

#[test]
fn castle_rope_death_has_null_sender_and_dead_prop_ignores_further_hits() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    for _ in 0..15 {
        s.step(1. / 60.);
    }
    let handle = s
        .world
        .entities
        .iter()
        .find(|e| e.template_name.as_deref() == Some("rope_mechanism"))
        .unwrap()
        .handle;
    let damage = Damage {
        attack: A::DAMAGE,
        element: E::EARTH.0,
        amount: 10000.,
        magnitude: 1.,
    };
    s.world.pending_triggers.clear();
    let result = s.world.damage_physics(handle, damage, 15).unwrap();
    assert_ne!(result & R::KILLED, 0);
    assert_eq!(
        s.world.pending_triggers,
        vec![(custom_hash("ropecut"), None)]
    );
    assert_eq!(s.world.damage_physics(handle, damage, 15).unwrap(), 0);
    assert_eq!(s.world.pending_triggers.len(), 1);
}

/// User report: destroyed props like the Earth lesson's wooden wall showed
/// no pieces. DamageablePhysicsEntity.SpawnGibs launches the template gibs
/// once from the entity position (speed = cached gib mass × 1.25). A
/// never-used native gib has body mass 1; the rewrite launches those with the
/// gib's own mass (10 → 12.5 m/s), a documented divergence so the tutorial
/// boards fly as they do with a used pool.
#[test]
fn castle_wooden_wall_death_spawns_its_gibs_once() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    for _ in 0..15 {
        s.step(1. / 60.);
    }
    let wall = s
        .world
        .entities
        .iter()
        .find(|e| e.unique_id == custom_hash("dungeonwall"))
        .unwrap();
    let (handle, position) = (wall.handle, wall.position);
    let before = s.world.gibs.live.len();
    let damage = Damage {
        attack: A::DAMAGE,
        element: E::EARTH.0,
        amount: 5000.,
        magnitude: 1.,
    };
    s.world.damage_physics(handle, damage, 15).unwrap();
    assert!(s.world.physics_damage[&handle].health <= 0.0);
    s.step(1. / 60.);
    s.step(1. / 60.);
    let gibs: Vec<_> = s.world.gibs.live[before..].to_vec();
    assert_eq!(gibs.len(), 8, "dungeon_wall decodes eight gib references");
    for g in &gibs {
        assert!(
            g.model.to_ascii_lowercase().contains("mines_door_gib"),
            "{}",
            g.model
        );
        assert_eq!(g.blood, magicka_runtime::gibs::BLOOD_NONE);
        let speed = g.velocity.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!(speed > 11.0 && speed < 13.5, "speed {speed}");
        assert!(g.velocity[1] >= 0.0);
        let d = (0..3)
            .map(|i| (g.position[i] - position[i]).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!(d < 1.0, "gib starts at the entity position");
    }
    for _ in 0..30 {
        s.step(1. / 60.);
    }
    assert_eq!(s.world.gibs.live.len(), before + 8, "gibs spawn once");
}

/// wc2_s1's authored kill starts all nine debris bodies in one frame.
/// Never-used native cache bodies have mass 1 (not each chunk's mass 100).
#[test]
fn castle2_tower_explosion_spawns_all_chunks_at_native_fresh_cache_speed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let template =
        magicka_runtime::play_content::load_physics_template(&root, "tower_explosion_gibs")
            .unwrap();
    let definitions = magicka_runtime::play_content::physics_gib_definitions(&root, &template);
    let mut pool = magicka_runtime::gibs::Gibs::new(1);
    pool.spawn_physics([0.0; 3], 0.3, false, "", &definitions, false);
    assert_eq!(pool.live.len(), 9);
    for g in &pool.live {
        let speed = g.velocity.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!(
            (speed - 1.25).abs() < 0.0001,
            "native never-used launch: {speed}"
        );
        assert_eq!(g.position, [0.0; 3]);
    }
    pool.update(21.0, [0.0; 3], None, &mut Vec::new());
    pool.spawn_physics([0.0; 3], 0.3, false, "", &definitions, false);
    for g in &pool.live {
        let speed = g.velocity.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!(
            (speed - 125.0).abs() < 0.001,
            "native used-pool mass is retained: {speed}"
        );
    }
    let mut s = PlaySession::load(&root, "Wizard_Castle2.lvl", "Wizard").unwrap();
    assert_eq!(s.scene_name(), "wc2_s1");
    assert_eq!(s.world.gibs.live.len(), 0);
    let mut spawn_frame = None;
    let mut snapshot = Vec::new();
    for frame in 0..240 {
        s.step(1. / 60.);
        if !s.world.gibs.live.is_empty() {
            spawn_frame = Some(frame);
            snapshot = s.world.gibs.live.clone();
            break;
        }
    }
    assert!(
        spawn_frame.is_some(),
        "Intro's kill must release the tower debris"
    );
    assert_eq!(snapshot.len(), 9, "all references spawn together");
    // Session.step already advanced gravity/collision once after spawning.
    let max_speed = 1.25 + magicka_sim::character_physics::GRAVITY[1].abs() / 60.0;
    for g in &snapshot {
        assert!(g.model.to_ascii_lowercase().contains("tower_debris"));
        assert_eq!(g.mass, 100.0);
        let speed = g.velocity.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!(
            speed <= max_speed + 0.0001,
            "first physical step must start from the native 1.25 launch: {speed}"
        );
    }
    for g in &snapshot {
        let separation = (0..3)
            .map(|i| (g.position[i] - snapshot[0].position[i]).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!(
            separation <= max_speed * 2.0 / 60.0 + 0.0001,
            "debris must start together: {separation}"
        );
    }
    for _ in 0..30 {
        s.step(1. / 60.);
    }
    assert_eq!(s.world.gibs.live.len(), 9, "no staggered later spawns");
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
}
