use magicka_runtime::{
    avatar::{PlayerInput, ELEMENT_KEYS},
    network_world::{world_presentation, RemoteWorld},
    play_session::{LiveBolt, PlaySession},
    spell_lights::spray_light,
};
use magicka_sim::{
    lightning::BoltVisual,
    spells::{Elements as E, Spell},
};
const DT: f32 = 1.0 / 60.0;

fn session() -> Option<PlaySession> {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    Some(s)
}
fn frame(s: &mut PlaySession, input: PlayerInput) {
    s.handle_input(input, 16.0 / 9.0);
    s.step(DT);
}
fn conjure(s: &mut PlaySession, element: E) {
    let mut elements = [false; 8];
    elements[ELEMENT_KEYS.iter().position(|&e| e == element).unwrap()] = true;
    frame(
        s,
        PlayerInput {
            elements,
            ..Default::default()
        },
    );
    for _ in 0..12 {
        frame(s, PlayerInput::default());
    }
}
fn cast(s: &mut PlaySession, element: E) {
    conjure(s, element);
    for _ in 0..60 {
        frame(
            s,
            PlayerInput {
                cast_down: true,
                ..Default::default()
            },
        );
    }
}
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}

#[test]
fn spray_palette_uses_native_magnitudes_and_excludes_unlit_elements() {
    let spell = Spell::default_for_list(&[E::FIRE, E::FIRE]);
    let light = spray_light(&spell, 8.0, [1., 2., 3.], [0., 0., -1.]).unwrap();
    for (a, b) in light.diffuse.into_iter().zip([5.4, 1.8, 1.2]) {
        close(a, b);
    }
    assert_eq!(light.position, [1., 2., -1.]);
    assert_eq!(light.radius, 10.0);
    for i in 0..3 {
        close(light.ambient[i], light.diffuse[i] * 0.25);
    }
    for e in [E::WATER, E::STEAM, E::POISON] {
        assert!(spray_light(&Spell::default_for(e), 8., [0.; 3], [0., 0., -1.]).is_none());
    }
}

#[test]
fn spray_light_steers_from_previous_position_and_fades_when_owner_disappears() {
    use magicka_runtime::{avatar::AvatarEvent, spell_lights::SprayLights, spray_decals::Frame};
    use magicka_sim::character_physics::Orientation;
    let mut lights = SprayLights::default();
    lights.event(
        7,
        &AvatarEvent::SprayStarted {
            spell: Spell::default_for(E::FIRE),
            range: 8.0,
            position: [0.0; 3],
            direction: [0.0, 0.0, -1.0],
        },
        Orientation::IDENTITY,
    );
    lights.update(0.5);
    lights.event(
        7,
        &AvatarEvent::SprayDecalUpdate(Frame {
            dt: 0.125,
            scale: 0.01,
            direction: [1.0, 0.0, 0.0],
            position: [0.0; 3],
            cast_position: [0.0; 3],
        }),
        Orientation::from_forward_up([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    );
    let halfway = lights.lights().next().unwrap().position;
    close(halfway[0], 8.0_f32.sqrt());
    close(halfway[1], 0.0);
    close(halfway[2], -8.0_f32.sqrt());
    // A quarter-second CastUpdate reaches the body's new heading exactly.
    // Clipped particle scale must not shorten the spell light's half-range.
    lights.event(
        7,
        &AvatarEvent::SprayDecalUpdate(Frame {
            dt: 0.25,
            scale: 0.01,
            direction: [1.0, 0.0, 0.0],
            position: [1.0, 2.0, 3.0],
            cast_position: [1.0, 2.0, 3.0],
        }),
        Orientation::from_forward_up([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    );
    let light = lights.lights().next().unwrap();
    for (actual, expected) in light.position.into_iter().zip([5.0, 2.0, 3.0]) {
        close(actual, expected);
    }
    close(light.draw_intensity(), 1.0);
    lights.retain_owners(&Default::default());
    lights.update(0.25);
    close(lights.lights().next().unwrap().draw_intensity(), 0.5);
    lights.update(0.25);
    assert_eq!(lights.lights().count(), 0);
}

#[test]
fn real_fire_and_cold_sprays_light_the_scene_and_fade_after_release() {
    for element in [E::FIRE, E::COLD] {
        let Some(mut s) = session() else {
            return;
        };
        cast(&mut s, element);
        let lights: Vec<_> = s.spray_lights.lights().collect();
        assert_eq!(lights.len(), 1, "{element:?}");
        let light = lights[0];
        close(light.draw_intensity(), 1.0);
        assert!(light.radius > 5.0);
        if element == E::FIRE {
            assert!(light.diffuse[0] > light.diffuse[2]);
        } else {
            assert!(light.diffuse[2] > light.diffuse[0]);
        }
        assert_eq!(light.ambient, light.diffuse.map(|c| c * 0.25));
        frame(&mut s, PlayerInput::default());
        assert!(
            s.spray_lights.lights().count() > 0,
            "release must fade, not pop"
        );
        for _ in 0..60 {
            frame(&mut s, PlayerInput::default());
        }
        assert_eq!(s.spray_lights.lights().count(), 0);
        // Repeated casts must not leak an old light.
        cast(&mut s, element);
        assert_eq!(s.spray_lights.lights().count(), 1);
    }
}

#[test]
fn real_beams_and_lightning_expose_capsule_lights() {
    for element in [E::ARCANE, E::LIGHTNING] {
        let Some(mut s) = session() else {
            return;
        };
        cast(&mut s, element);
        let lights = s.spell_capsule_lights();
        assert!(!lights.is_empty(), "{element:?}");
        if element == E::ARCANE {
            assert_eq!(lights.len(), s.beams.len());
            for (light, beam) in lights.iter().zip(&s.beams) {
                assert_eq!((light.start, light.end), beam.light);
                assert_eq!(light.radius, 5.0);
                assert_eq!(light.intensity, 0.6);
            }
        } else {
            assert_eq!(lights.len(), s.bolts.len());
            for (light, bolt) in lights.iter().zip(&s.bolts) {
                assert_eq!((light.start, light.end), (bolt.visual.from, bolt.visual.to));
                close(light.intensity, bolt.ttl.sqrt() * 1.5);
                close(light.radius, 3.0 + 2.0 * bolt.visual.scale);
            }
        }
        for _ in 0..90 {
            frame(&mut s, PlayerInput::default());
        }
        assert!(
            s.spell_capsule_lights().is_empty(),
            "{element:?} must stop illuminating after expiry"
        );
    }
}

#[test]
fn point_ambient_and_capsule_geometry_survive_network_mirroring() {
    let Some(mut host) = session() else {
        return;
    };
    let Some(mut client) = session() else {
        return;
    };
    let mut point = spray_light(
        &Spell::default_for(E::COLD),
        8.,
        [1., 2., 3.],
        [0., 0., -1.],
    )
    .unwrap();
    point.update(0.5);
    let ambient = point.ambient;
    host.dynamic_lights.push(point);
    host.bolts.push(LiveBolt {
        serial: 123,
        ttl: 0.2,
        visual: BoltVisual {
            from: [1., 2., 3.],
            to: [5., 2., 3.],
            direction: [1., 0., 0.],
            color: [0.75, 0.5, 1.],
            scale: 1.,
            ttl: 0.25,
        },
    });
    // A previous local light must not double the host's authoritative snapshot.
    client.status_lights.sync(
        [magicka_runtime::spell_lights::StatusLightOwner {
            handle: 999,
            position: [0.; 3],
            kind: magicka_runtime::spell_lights::StatusLightKind::Character {
                burning: true,
                steamed: false,
            },
        }],
        1.0,
        &mut magicka_runtime::particles::DotNetRandom::new(1),
    );
    assert!(client.spell_point_lights().count() > 0);
    let state = world_presentation(&host);
    assert!(state.valid());
    let wire = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    RemoteWorld::default()
        .apply(&wire, &mut client, DT, true)
        .unwrap();
    assert_eq!(client.dynamic_lights.len(), 1);
    assert_eq!(
        client.spell_point_lights().count(),
        host.spell_point_lights().count()
    );
    assert_eq!(client.dynamic_lights[0].ambient, ambient);
    assert_eq!(client.spell_capsule_lights(), host.spell_capsule_lights());
    let mut bad = state.clone();
    bad.spells.lights[0].ambient[0] = f32::NAN;
    assert!(!bad.valid());
    let mut bad = state;
    bad.spells
        .lights
        .iter_mut()
        .find(|l| l.capsule_end.is_some())
        .unwrap()
        .capsule_end = Some([f32::INFINITY, 0., 0.]);
    assert!(!bad.valid());
}

#[test]
fn burning_from_self_fire_lights_the_owner_and_water_eventually_extinguishes_it() {
    let Some(mut s) = session() else {
        return;
    };
    conjure(&mut s, E::FIRE);
    frame(
        &mut s,
        PlayerInput {
            cast_self_down: true,
            ..Default::default()
        },
    );
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    let lights: Vec<_> = s.status_lights.lights().collect();
    assert_eq!(lights.len(), 1);
    let light = lights[0];
    assert_eq!(light.position, s.body().position);
    assert_eq!(light.diffuse, [1.0, 0.4, 0.0]);
    assert_eq!(light.ambient, [0.4, 0.2, 0.0]);
    assert!(light.draw_intensity() > 0.5);
    conjure(&mut s, E::WATER);
    frame(
        &mut s,
        PlayerInput {
            cast_self_down: true,
            ..Default::default()
        },
    );
    for _ in 0..180 {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!(s.status_lights.lights().count(), 0);
}

#[test]
fn status_lights_follow_owners_preserve_steam_priority_and_fade_when_removed() {
    use magicka_runtime::spell_lights::{StatusLightKind, StatusLightOwner, StatusLights};
    let mut lights = StatusLights::default();
    let mut random = magicka_runtime::particles::DotNetRandom::new(42);
    let owner = |burning, steamed| StatusLightOwner {
        handle: 1,
        position: [3., 4., 5.],
        kind: StatusLightKind::Character { burning, steamed },
    };
    lights.sync([owner(true, true)], DT, &mut random);
    assert_eq!(
        lights.lights().count(),
        0,
        "Steam prevents a new fire light"
    );
    lights.sync([owner(true, false)], DT, &mut random);
    lights.sync([owner(false, true)], DT, &mut random);
    assert_eq!(
        lights.lights().count(),
        1,
        "Steam preserves an existing light"
    );
    for _ in 0..90 {
        lights.sync([], DT, &mut random);
    }
    assert_eq!(lights.lights().count(), 0);
    for _ in 0..60 {
        lights.sync(
            [StatusLightOwner {
                handle: 2,
                position: [6., 7., 8.],
                kind: StatusLightKind::Physics {
                    burning: true,
                    magnitude: 4.0,
                },
            }],
            DT,
            &mut random,
        );
    }
    let light = lights.lights().next().unwrap();
    assert_eq!(light.position, [6., 7., 8.]);
    assert_eq!(light.ambient, [0.2; 3]);
    assert!(light.draw_intensity() > 4.5 && light.draw_intensity() < 5.5);
}
