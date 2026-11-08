use magicka_sim::{
    damage::{attack, Damage},
    railgun::{RailEntityKind, RailStart, RailWorld},
    spells::Elements as E,
};

fn session() -> crate::play_session::PlaySession {
    let root = magicka_content::retail_content_dir();
    let mut s = crate::play_session::PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s
}

fn start(s: &mut crate::play_session::PlaySession, p: [f32; 3]) -> usize {
    let mut damages = [Damage::default(); 5];
    damages[0] = Damage {
        attack: attack::DAMAGE,
        element: E::ARCANE.0 | E::FIRE.0,
        amount: 100.,
        magnitude: 1.,
    };
    let id = s.world.rails.start(RailStart {
        owner: s.avatar().handle,
        owner_position: p,
        owner_direction: [1., 0., 0.],
        color: [1.; 3],
        damages,
        element: E::ARCANE | E::FIRE,
        timestamp: 12.,
    });
    s.world.rails.steer(id, p, [1., 0., 0.]);
    id
}

#[test]
fn beam_hits_egg_and_delivers_quarter_second_composite_payload() {
    let mut s = session();
    let owner = s.avatar().handle;
    s.world.summon_elemental_from(owner).unwrap();
    let slot = &mut s.world.elemental_eggs.as_mut().unwrap().slots[0];
    slot.actor.initialize(
        [4., 100. - slot.actor.geometry.radius, 0.],
        [0., 0., 1.],
        None,
    );
    let handle = slot.handle;
    let id = start(&mut s, [0., 100., 0.]);
    for _ in 0..40 {
        s.world.update_rails(1. / 60.);
    }
    let rail = s.world.rails.rail(id).unwrap();
    assert!((rail.length - 4.).abs() < 0.01, "{}", rail.length);
    let egg = &s.world.elemental_eggs.as_ref().unwrap().slots[0];
    for (cold, expected) in [(24., E::FIRE), (26., E::COLD)] {
        let mut state = egg.actor.state.clone();
        state.internal_damage(Damage {
            element: E::COLD.0,
            amount: cold,
            magnitude: 1.,
            ..Default::default()
        });
        state.update(0., 0., false, |_| 0);
        assert_eq!(state.spawn_element, expected);
    }
    s.world
        .with_rail_world(|world| {
            assert!(world
                .rail_entities([0., 100., 0.], 10.)
                .iter()
                .any(|e| e.handle == handle));
        })
        .unwrap();
}

#[test]
fn connected_portal_continues_beam_and_disconnect_kills_child() {
    let mut s = session();
    s.world.place_portal([4., 100., 0.]);
    s.world.place_portal([4., 100., 10.]);
    s.world.portals.portal.update_connection(1.);
    assert!(s.world.portals.portal.connected);
    let owner = s.avatar().handle;
    s.world.summon_elemental_from(owner).unwrap();
    let slot = &mut s.world.elemental_eggs.as_mut().unwrap().slots[0];
    slot.actor.initialize(
        [10., 100.25 - slot.actor.geometry.radius, 10.],
        [0., 0., 1.],
        None,
    );
    let id = start(&mut s, [0., 100.25, 0.]);
    for _ in 0..40 {
        s.world.update_rails(1. / 60.);
    }
    let root = s.world.rails.rail(id).unwrap();
    assert!((root.length - 4.).abs() < 0.001);
    let child_id = root.child.expect("portal continuation");
    let child = s.world.rails.rail(child_id).unwrap();
    assert_eq!(child.position, [3.5, 100.25, 10.]);
    assert_eq!(child.direction, [1., 0., 0.]);
    assert_eq!(child.depth, 1);
    assert_eq!(child.out_portal, Some(s.world.portals.handles.unwrap()[0]));
    assert_eq!(child.timestamp, 12.);
    assert_eq!(child.damages, root.damages);
    assert!(child.child.is_none(), "exit portal must be ignored");
    assert!(child.length > 5., "child should grow through exit portal");
    let mut egg = s.world.elemental_eggs.as_ref().unwrap().slots[0]
        .actor
        .state
        .clone();
    egg.update(0., 0., false, |_| 0);
    assert_eq!(
        egg.spawn_element,
        E::FIRE,
        "child beam must deliver damage at exit"
    );
    s.world.portals.portal.kill();
    s.world.portals.portal.update_connection(1.);
    for _ in 0..3 {
        s.world.update_rails(1. / 60.);
    }
    assert!(s.world.rails.rail(child_id).is_none_or(|r| r.dead));
    s.world
        .with_rail_world(|world| {
            assert!(!world
                .rail_entities([4., 100., 0.], 20.)
                .iter()
                .any(|e| e.kind == RailEntityKind::Portal));
        })
        .unwrap();
}

#[test]
fn portal_loop_stops_at_native_depth_limit() {
    let mut s = session();
    s.world.place_portal([4., 100., 0.]);
    s.world.place_portal([8., 100., 0.]);
    s.world.portals.portal.update_connection(1.);
    let id = start(&mut s, [6., 100., 0.]);
    for _ in 0..80 {
        s.world.update_rails(1. / 60.);
    }
    let mut rail = s.world.rails.rail(id).unwrap();
    for depth in 0..=magicka_sim::railgun::MAX_RAY_DEPTH {
        assert_eq!(rail.depth, depth);
        assert_eq!(rail.direction, [1., 0., 0.]);
        if depth < magicka_sim::railgun::MAX_RAY_DEPTH {
            rail = s
                .world
                .rails
                .rail(rail.child.expect("loop continuation"))
                .unwrap();
        } else {
            assert!(rail.child.is_none());
        }
    }
}

fn barrier(s: &mut crate::play_session::PlaySession, element: E) -> usize {
    use magicka_sim::barrier::{Barrier, BarrierClips, BarrierInit};
    let hit_list = s.world.barriers.new_hit_list();
    let (slot, stale) = s.world.barriers.take();
    let (mut b, _, _) = Barrier::initialize(
        BarrierInit {
            owner: Some(s.avatar().handle),
            hit_list,
            hit: [4., 100., 0.],
            direction: [0., 0., -1.],
            scale: 1.,
            range: 0.,
            next_direction: [0., 0., -1.],
            next_yaw: 0.,
            distance: 2.,
            timestamp: 0.,
        },
        magicka_sim::spells::Spell::default_for(E::SHIELD | element),
        [Damage::default(); 5],
        &BarrierClips {
            ice_appear: 0.5,
            ice_geyser: 1.,
            ice_tesla: 0.333,
            ice_earth: 0.333,
            earth_appear: 0.5,
            earth_volcano: 0.5,
        },
        stale,
    )
    .unwrap();
    // Isolate beam delivery from ground placement and the normal decay update.
    b.position = [4., 100., 0.];
    s.world.barriers.add(slot, b);
    slot
}

#[test]
fn beam_hits_solid_barrier_without_quartering_its_native_health_subtraction() {
    use super::{NetworkRole, BARRIER_KEY};
    for client in [false, true] {
        let mut s = session();
        let slot = barrier(&mut s, E::EARTH);
        let before = s.world.barriers.get(slot).unwrap().hit_points;
        let owner = s.avatar().handle;
        s.world.summon_elemental_from(owner).unwrap();
        let egg = &mut s.world.elemental_eggs.as_mut().unwrap().slots[0].actor;
        egg.initialize([8., 100.2 - egg.geometry.radius, 0.], [0., 0., 1.], None);
        if client {
            s.world.role = NetworkRole::Client;
        }
        start(&mut s, [0., 100.2, 0.]);
        for _ in 0..40 {
            s.world.update_rails(1. / 60.);
        }
        let lost = before - s.world.barriers.get(slot).unwrap().hit_points;
        if client {
            assert_eq!(lost, 0.);
        } else {
            assert!(lost >= 100.);
            assert_eq!(
                lost % 100.,
                0.,
                "barrier subtracts Amount, not Amount*Magnitude"
            );
        }
        let mut egg = s.world.elemental_eggs.as_ref().unwrap().slots[0]
            .actor
            .state
            .clone();
        egg.update(0., 0., false, |_| 0);
        assert_eq!(
            egg.spawn_element,
            E::NONE,
            "solid wall blocks target behind it"
        );
        s.world
            .with_rail_world(|world| {
                assert!(world
                    .rail_entities([0., 100., 0.], 10.)
                    .iter()
                    .any(|e| e.handle == BARRIER_KEY | slot as u16));
            })
            .unwrap();
    }
}

#[test]
fn beam_skips_elemental_barriers_and_uses_body_centered_solid_query() {
    use super::BARRIER_KEY;
    let mut s = session();
    let elemental = barrier(&mut s, E::FIRE);
    let solid = barrier(&mut s, E::EARTH);
    s.world
        .with_rail_world(|world| {
            let entries = world.rail_entities([0., 100., 0.], 10.);
            assert!(!entries
                .iter()
                .any(|e| e.handle == BARRIER_KEY | elemental as u16));
            let seg = magicka_sim::spray_pulse::Segment {
                origin: [0., 101.45, 0.],
                delta: [8., 0., 0.],
            };
            let handle = BARRIER_KEY | solid as u16;
            // Wider beam reaches the body-centred axis, above the offset skin.
            let point = world.rail_segment(handle, seg, 0.25).unwrap();
            assert!((point[1] - 101.3).abs() < 0.001, "{point:?}");
            assert!(world.rail_segment(handle, seg, 0.).is_none());
        })
        .unwrap();
}

#[test]
fn live_elementals_use_generated_beam_entries_and_template_resistances() {
    use magicka_sim::{
        damage::{calculate_damage, CastType, SpellType},
        spells::Spell,
    };
    // One quarter-second tick, from each decoded template and the native
    // CalculateDamage / ApplyResistances contracts. Columns are Arcane,
    // Arcane+Fire, Arcane+Cold, Arcane+Poison and Life. Negative loss heals.
    for (template, expected) in [
        ("Elemental_Arcane", [-56.25, -17.8125, -15.625, -56.25, 90.]),
        (
            "Elemental_Fire",
            [14.0625, -17.8125, 171.875, 14.0625, -11.25],
        ),
        (
            "Elemental_Cold",
            [14.0625, 89.0625, -9.375, 14.0625, -11.25],
        ),
        ("Elemental_Steam", [14.0625, 0., 68.75, 14.0625, -11.25]),
        (
            "Elemental_Lightning",
            [14.0625, 4.453125, 3.90625, 14.0625, -11.25],
        ),
        (
            "Elemental_Poison",
            [14.0625, 4.453125, 3.90625, 14.0625, 225.],
        ),
        ("Elemental_Life", [168.75, 53.4375, 46.875, 168.75, -90.]),
        (
            "Elemental_Water",
            [14.0625, 17.8125, 78.125, 14.0625, -11.25],
        ),
    ] {
        let mut s = session();
        let target = s
            .world
            .summon_npc(
                None,
                template,
                [4., 100., 0.],
                [0., 0., -1.],
                magicka_sim::summons::SummonKind::Normal,
                crate::spawn_action::SpawnOrder {
                    order: 0,
                    react_to: 0,
                    reaction: 0,
                    priority_target: 0,
                    priority_ability: 0,
                    trigger: 0,
                    events: &[],
                },
            )
            .expect("retail elemental");
        let npc = s.world.npcs.get_mut(&target).unwrap();
        assert!(!npc.character.definition.can_have_status);
        if template == "Elemental_Fire" {
            let resist = &npc.character.definition.resistances;
            assert_eq!(resist[E::FIRE.index() as usize].multiplier, -1.);
            assert_eq!(resist[E::ARCANE.index() as usize].multiplier, 0.25);
            assert_eq!(resist[E::COLD.index() as usize].multiplier, 5.);
            assert_eq!(resist[E::COLD.index() as usize].modifier, 300.);
            assert_eq!(resist[E::POISON.index() as usize].multiplier, 0.);
        }
        // Expected health losses for one quarter-second tick. Fire inverts the
        // entire instant Arcane|Fire entry; Cold adds 300 then multiplies by 5.
        // Poison is a separate status entry, so its immunity does not cancel Arcane.
        for (elements, expected_loss) in [
            E::ARCANE,
            E::ARCANE | E::FIRE,
            E::ARCANE | E::COLD,
            E::ARCANE | E::POISON,
            E::LIFE,
        ]
        .into_iter()
        .zip(expected)
        {
            let npc = s.world.npcs.get_mut(&target).unwrap();
            let before = npc.character.combat.health.max_hit_points * 0.5;
            npc.character.combat.health.hit_points = before;
            let body = &mut s.world.physics.bodies[npc.body];
            body.set_position([4., 100., 0.]);
            let mut damages = [Damage::default(); 5];
            for (slot, damage) in damages.iter_mut().zip(
                calculate_damage(
                    &Spell::default_for(elements),
                    SpellType::Beam,
                    CastType::Force,
                )
                .unwrap(),
            ) {
                *slot = damage;
            }
            let owner = s.avatar().handle;
            let origin = [0., 100., 0.2];
            let id = s.world.rails.start(RailStart {
                owner,
                owner_position: origin,
                owner_direction: [1., 0., 0.],
                color: [1.; 3],
                damages,
                element: elements,
                timestamp: 0.,
            });
            s.world.rails.steer(id, origin, [1., 0., 0.]);
            s.world.update_rails(0.25);
            s.world.update_rails(0.25);
            let npc = &s.world.npcs[&target];
            let lost = before - npc.character.combat.health.hit_points;
            assert!(
                (lost - expected_loss).abs() < 0.001,
                "{template} {elements:?}: lost {lost}, expected {expected_loss}"
            );
            assert_eq!(npc.character.combat.statuses.current_mask, 0);
            s.world.rails.kill(id);
            s.world.update_rails(0.25);
        }
    }
}

#[test]
fn beam_hits_mine_skin_and_arms_delayed_detonation() {
    use super::MINE_KEY;
    use magicka_sim::{spell_mine::SpellMine, spells::Spell, spray_pulse::Segment};
    let mut s = session();
    let owner = s.avatar().handle;
    let owner_body = s.avatar().body;
    s.world.physics.bodies[owner_body].set_position([0., 100.1, 0.]);
    let mine = SpellMine::initialize(
        owner,
        [4., 100., 0.],
        [0., 0., -1.],
        1.,
        0.,
        [0., 0., -1.],
        0.,
        2.,
        Spell::default_for(E::SHIELD | E::ARCANE),
        0.,
        1.,
        0.,
    )
    .unwrap();
    let slot = s.world.mines.add(mine);
    let handle = MINE_KEY | slot as u16;
    s.world
        .with_rail_world(|world| {
            let segment = Segment {
                origin: [0., 100.1, 0.],
                delta: [8., 0., 0.],
            };
            let p = world.rail_segment(handle, segment, 0.25).unwrap();
            assert!((p[0] - 3.1).abs() < 0.0001, "{p:?}");
            assert!(
                world
                    .rail_segment(
                        handle,
                        Segment {
                            origin: [0., 100.1, 0.95],
                            ..segment
                        },
                        0.25
                    )
                    .is_none(),
                "mine skin ignores beam radius"
            );
        })
        .unwrap();
    start(&mut s, [0., 100.1, 0.]);
    s.world.update_rails(0.25);
    assert_eq!(s.world.mines.get(slot).unwrap().detonation_timer, 0.);
    s.world.update_rails(0.25);
    assert_eq!(s.world.mines.get(slot).unwrap().detonation_timer, 0.1);
    s.world.update_mines(0.11);
    assert!(s.world.mines.get(slot).is_none_or(|m| m.dead));
    assert!(s
        .world
        .mine_effects
        .iter()
        .any(|(name, _, _)| *name == "mine_explosion"));
}

#[test]
fn beam_killing_runed_barrier_routes_its_detonation_event() {
    let mut s = session();
    let slot = barrier(&mut s, E::EARTH | E::ARCANE);
    s.world.barriers.get_mut(slot).unwrap().hit_points = 50.;
    start(&mut s, [0., 100.2, 0.]);
    s.world.update_rails(0.25);
    s.world.update_rails(0.25);
    assert!(s.world.barriers.get(slot).unwrap().is_dead());
    assert!(s.world.barrier_visuals.iter().any(|v| matches!(
        v,
        super::BarrierVisual::OneShot {
            name: "mine_explosion",
            ..
        }
    )));
}

fn spawn_actor(s: &mut crate::play_session::PlaySession, template: &str) -> u16 {
    s.world
        .summon_npc(
            None,
            template,
            [4., 100., 0.],
            [0., 0., -1.],
            magicka_sim::summons::SummonKind::Normal,
            crate::spawn_action::SpawnOrder {
                order: 0,
                react_to: 0,
                reaction: 0,
                priority_target: 0,
                priority_ability: 0,
                trigger: 0,
                events: &[],
            },
        )
        .expect("native character template")
}

#[test]
fn goblin_warrior_beams_respect_native_composite_masks() {
    use magicka_sim::{
        damage::{calculate_damage, CastType, SpellType},
        spells::Spell,
    };
    for (elements, expected_loss) in [
        (E::ARCANE, 56.25),
        (E::ARCANE | E::FIRE, 0.),
        (E::ARCANE | E::LIGHTNING, 0.),
        (E::ARCANE | E::COLD, 62.5),
        (E::ARCANE | E::POISON, 56.25),
    ] {
        let mut s = session();
        let target = spawn_actor(&mut s, "Goblin_warrior");
        let before = s.world.npcs[&target].character.combat.health.hit_points;
        let base = &s.world.npcs[&target].character.definition.resistances;
        assert_eq!(base[E::FIRE.index() as usize].multiplier, 0.);
        assert_eq!(base[E::LIGHTNING.index() as usize].multiplier, 0.);
        assert_eq!(base[E::EARTH.index() as usize].modifier, -500.);
        let mut damages = [Damage::default(); 5];
        for (slot, d) in damages.iter_mut().zip(
            calculate_damage(
                &Spell::default_for(elements),
                SpellType::Beam,
                CastType::Force,
            )
            .unwrap(),
        ) {
            *slot = d;
        }
        let owner = s.avatar().handle;
        let origin = [0., 100., 0.2];
        let id = s.world.rails.start(RailStart {
            owner,
            owner_position: origin,
            owner_direction: [1., 0., 0.],
            color: [1.; 3],
            damages,
            element: elements,
            timestamp: 0.,
        });
        s.world.rails.steer(id, origin, [1., 0., 0.]);
        s.world.update_rails(0.25);
        s.world.update_rails(0.25);
        let lost = before - s.world.npcs[&target].character.combat.health.hit_points;
        assert!(
            (lost - expected_loss).abs() < 0.001,
            "{elements:?}: lost {lost}, expected {expected_loss}"
        );
    }
}

#[test]
fn goblin_warrior_mixed_earth_missiles_keep_physical_damage() {
    use magicka_sim::{
        missile_events::{condition, EventArgs, MissileEvent},
        projectile_launch::ProjectileLaunch,
        spells::Spell,
    };
    for extra in [E::NONE, E::FIRE, E::LIGHTNING, E::FIRE | E::LIGHTNING] {
        let mut s = session();
        let target = spawn_actor(&mut s, "Goblin_warrior");
        let before = s.world.npcs[&target].character.combat.health.hit_points;
        let owner = s.avatar().handle;
        let mut spell = Spell::default_for(E::EARTH | extra);
        spell.mag[E::EARTH.index() as usize] = 2.;
        let launch = ProjectileLaunch {
            position: [4., 100., 0.2],
            velocity: [50., 0., 0.],
            spell,
            splash: 3.,
            total_projectiles: 1,
        };
        let mut missile = crate::missiles::LiveMissile::spawn(1, owner, true, &launch).unwrap();
        let MissileEvent::Damage { damage, .. } = &missile.state.conditions.slots[1].events[0]
        else {
            panic!("physical hit must come first")
        };
        assert_eq!(damage.element, E::EARTH.0);
        assert_eq!(damage.amount, 600.);
        let mut visuals = Vec::new();
        s.world
            .with_rail_world(|world| {
                let mut host = crate::missiles::MissileHost {
                    body: &mut missile.body,
                    world: &mut world.hit,
                    visuals: &mut visuals,
                    target: Some(target),
                    entity_handle: None,
                };
                let (executed, _) = missile.state.conditions.slots[1].execute(
                    &mut EventArgs {
                        flags: condition::HIT,
                        ..Default::default()
                    },
                    &mut host,
                );
                assert!(executed);
            })
            .unwrap();
        let lost = before - s.world.npcs[&target].character.combat.health.hit_points;
        // Native Earth armour: (600 - 500) * 0.5 at normalized speed one.
        assert!((lost - 50.).abs() < 0.001, "{extra:?}: lost {lost}");
    }
}

fn equip_item(s: &mut crate::play_session::PlaySession, slot: i32, name: &str) {
    let root = magicka_content::retail_content_dir();
    let bytes = std::fs::read(root.join(format!("Data/items/Wizard/{name}.xnb"))).unwrap();
    let xnb = magicka_xnb::xnb::parse(&bytes).unwrap();
    let magicka_content::Content::Item(item) = magicka_content::parse_primary(&xnb)
        .unwrap()
        .unwrap()
        .content
    else {
        panic!("native item")
    };
    let p = s.world.players[0].as_mut().unwrap();
    p.items.retain(|(index, _)| *index != slot);
    p.items.push((slot, crate::avatar::AvatarItem::new(*item)));
    p.character.avatar_in_cutscene = false;
    p.character.immortal_time = 0.;
}

#[test]
fn live_staff_base_and_buff_resistances_apply_to_mixed_beam_payload() {
    use magicka_sim::{
        damage::{calculate_damage, CastType, SpellType},
        resistance::Resistance,
        spells::Spell,
    };
    let mut s = session();
    let attacker = spawn_actor(&mut s, "Goblin_miner");
    let target = s.avatar().handle;
    equip_item(&mut s, 1, "staff_of_fire");
    let mut damages = [Damage::default(); 5];
    for (slot, d) in damages.iter_mut().zip(
        calculate_damage(
            &Spell::default_for(E::ARCANE | E::FIRE),
            SpellType::Beam,
            CastType::Force,
        )
        .unwrap(),
    ) {
        *slot = Damage {
            magnitude: d.magnitude * 0.25,
            ..d
        };
    }
    for (base, buff, expected_loss) in [(1., 1., 35.625), (0.5, 0.5, 8.90625), (1., 0., 0.)] {
        let p = s.world.players[0].as_mut().unwrap();
        p.character.definition.resistances[E::FIRE.index() as usize].multiplier = base;
        p.character.buffs.list.clear();
        p.character.buffs.add(
            901,
            Resistance {
                against: E::FIRE,
                multiplier: buff,
                ..Resistance::IDENTITY
            },
            [1.; 3],
            true,
            false,
            [0.; 3],
            1.,
            &mut s.world.decals,
        );
        let before = p.character.combat.health.hit_points;
        s.world
            .with_rail_world(|world| {
                world.rail_damage(target, attacker, damages, 0., [0.; 3]);
            })
            .unwrap();
        let lost = before - s.avatar().character.combat.health.hit_points;
        assert!(
            (lost - expected_loss).abs() < 0.001,
            "lost {lost}, expected {expected_loss}"
        );
    }
    let view = s.world.combatants();
    assert_eq!(
        view.iter().find(|c| c.handle == target).unwrap().equipment[E::FIRE.index() as usize]
            .multiplier,
        0.5
    );
}

#[test]
fn staff_swap_is_immediate_for_damage_status_and_fall_routes() {
    use magicka_sim::{
        damage_delivery::DamagePayload,
        status::{Status, StatusEffect},
    };
    let mut s = session();
    let body = s.avatar().body;
    for (staff, expected) in [
        ("staff_vlads_gauntlet", 0.),
        ("staff_of_fire", 100.),
        ("staff_vlads_gauntlet", 0.),
    ] {
        equip_item(&mut s, 1, staff);
        let p = s.world.players[0].as_mut().unwrap();
        let before = p.character.combat.health.hit_points;
        p.receive_payload(
            &mut s.world.physics.bodies[body],
            DamagePayload::Single(Damage {
                attack: attack::DAMAGE,
                element: E::ARCANE.0,
                amount: 100.,
                magnitude: 1.,
            }),
            None,
            [0.; 3],
            0.,
        );
        assert_eq!(before - p.character.combat.health.hit_points, expected);
    }
    equip_item(&mut s, 1, "staff_of_fire");
    let p = s.world.players[0].as_mut().unwrap();
    p.add_status_effect(
        StatusEffect::new(Status::Burning, 100., 1., false),
        &mut vec![],
    )
    .unwrap();
    assert_eq!(
        p.character
            .combat
            .statuses
            .effect(Status::Burning)
            .unwrap()
            .dps,
        50.
    );
    equip_item(&mut s, 1, "staff_vlads_gauntlet");
    let p = s.world.players[0].as_mut().unwrap();
    let before = p.character.combat.health.hit_points;
    p.receive_payload(
        &mut s.world.physics.bodies[body],
        DamagePayload::Single(Damage {
            attack: attack::DAMAGE,
            element: E::LIFE.0,
            amount: -100.,
            magnitude: 1.,
        }),
        None,
        [0.; 3],
        0.,
    );
    assert_eq!(
        before - p.character.combat.health.hit_points,
        100.,
        "staff's negative Life multiplier inverts healing"
    );
    equip_item(&mut s, 1, "staff_gadget_dwarven_utility_stick");
    let p = s.world.players[0].as_mut().unwrap();
    let before = p.character.combat.health.hit_points;
    p.apply_fall_damage(&mut s.world.physics.bodies[body], 300., false, 0.);
    assert!((before - p.character.combat.health.hit_points - 90.).abs() < 0.001);
}
