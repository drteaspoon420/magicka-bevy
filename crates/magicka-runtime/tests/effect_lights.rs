//! Effect `<Light>` (PointLightBatcher) and particle SpawnLights
//! (ParticleLightBatcher) light records (research/particles.md, 2026-10-30):
//! native timing of spawn/despawn transitions, particle-bound lights and
//! the 128/256 slot limits. Retail checks skip without content.
use magicka_content::skinned::IDENTITY;
use magicka_runtime::particle_lights::*;
use magicka_runtime::particles::*;
use std::path::PathBuf;

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir().then_some(root)
}

fn library(xml: &str) -> (EffectLibrary, i32) {
    let mut library = EffectLibrary::default();
    let hash = library
        .add_xml("Test", xml.as_bytes(), &mut || RandomCursor(0))
        .unwrap();
    (library, hash)
}

fn translation(x: f32, y: f32, z: f32) -> [f32; 16] {
    let mut m = IDENTITY;
    m[12] = x;
    m[13] = y;
    m[14] = z;
    m
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

const LINEAR_LIGHT: &str = r#"<Effect type="Single" duration="2" version="2">
  <Light>
    <PositionX value="0"/><PositionY value="0.3"/><PositionZ value="0"/>
    <DiffuseR value="2"/><DiffuseG value="1"/><DiffuseB value="2"/>
    <AmbientR value="1"/><AmbientG value="0.5"/><AmbientB value="1"/>
    <Specular value="0.8"/><Radius value="8"/>
    <VariationType value="None"/><VariationSpeed value="1"/><VariationAmount value="0.5"/>
    <SpawnTransitionType value="Linear"/><SpawnTransitionTime value="1"/>
    <DespawnTransitionType value="Linear"/><DespawnTransitionTime value="0.5"/>
  </Light>
</Effect>"#;

#[test]
fn effect_light_parses_every_child_and_defaults_to_zero() {
    let (lib, hash) = library(LINEAR_LIGHT);
    let light = lib.effects[&hash].light.as_ref().unwrap();
    assert_eq!(light.offset, [0.0, 0.3, 0.0]);
    assert_eq!(light.light.diffuse, [2.0, 1.0, 2.0]);
    assert_eq!(light.light.ambient, [1.0, 0.5, 1.0]);
    assert_eq!(light.light.specular, 0.8);
    assert_eq!(light.light.radius, 8.0);
    assert_eq!(light.light.variation_type, 0);
    assert_eq!(light.light.variation_speed, 1.0);
    assert_eq!(light.light.variation_amount, 0.5);
    assert_eq!(light.spawn_transition, LightTransition::Linear);
    assert_eq!(light.spawn_transition_time, 1.0);
    assert_eq!(light.despawn_transition, LightTransition::Linear);
    assert_eq!(light.despawn_transition_time, 0.5);

    // An empty <Light/> still enables the light with the zeroed struct.
    let (lib, hash) = library(r#"<Effect type="Infinite" duration="1"><Light/></Effect>"#);
    let light = lib.effects[&hash].light.as_ref().unwrap();
    assert_eq!(light.light, BatchedPointLight::default());
    assert_eq!(light.spawn_transition, LightTransition::None);

    // Enum.Parse rejects unknown names.
    let mut bad = EffectLibrary::default();
    let err = bad.add_xml(
        "Bad",
        br#"<Effect><Light><SpawnTransitionType value="Fade"/></Light></Effect>"#,
        &mut || RandomCursor(0),
    );
    assert!(matches!(err, Err(EffectError::Parse(_, _))), "{err:?}");
}

#[test]
fn effect_light_follows_the_transform_and_fades_with_its_transitions() {
    let (library, hash) = library(LINEAR_LIGHT);
    let mut fx = EffectSystem::new(library);
    let mut r = fx.start(hash, translation(1.0, 2.0, 3.0)).unwrap();
    let dt = 0.25;

    // Spawned by VisualEffect.Update with Intensity 0; the batcher's Update
    // later in the frame starts the linear 1 s ramp.
    fx.update(dt);
    assert_eq!(fx.effect_lights.active().len(), 1);
    assert_eq!(fx.effect_lights.active()[0].intensity, 0.0);
    assert_eq!(fx.draw_lights().count(), 0);
    fx.update_particles(dt);
    let l = fx.effect_lights.active()[0];
    assert_eq!(l.position, [1.0, 2.3, 3.0]);
    assert_eq!(l.radius, 8.0);
    assert_eq!(l.diffuse, [2.0, 1.0, 2.0]);
    assert_eq!(l.ambient, [1.0, 0.5, 1.0]);
    assert_eq!(l.specular, 0.8);
    assert!(close(l.intensity, 0.25), "{l:?}");
    assert_eq!(fx.draw_lights().count(), 1);

    // Moving the effect moves the light (SetLightPosition).
    assert!(fx.update_orientation(&mut r, translation(5.0, 0.0, 0.0)));
    fx.update(dt);
    fx.update_particles(dt);
    let l = fx.effect_lights.active()[0];
    assert_eq!(l.position, [5.0, 0.3, 0.0]);
    assert!(close(l.intensity, 0.5), "{l:?}");

    let mut intensities = vec![];
    for _ in 0..9 {
        fx.update(dt);
        fx.update_particles(dt);
        intensities.push(fx.effect_lights.active().first().map(|l| l.intensity));
    }
    // Ramp completes at 1 s (transition becomes None → 1), holds while the
    // effect runs, then the Single effect stops at timeline 2.25 and the
    // 0.5 s linear despawn runs: 0.5, 0.0, then freed.
    assert!(close(intensities[0].unwrap(), 0.75));
    for i in &intensities[1..6] {
        assert_eq!(*i, Some(1.0), "{intensities:?}");
    }
    assert!(close(intensities[6].unwrap(), 0.5), "{intensities:?}");
    assert_eq!(intensities[7], Some(0.0), "{intensities:?}");
    assert_eq!(intensities[8], None, "{intensities:?}");
    assert!(!fx.is_active(&r));
    assert_eq!(fx.effect_lights.free_slots(), MAX_EFFECT_LIGHTS);
    assert_eq!(fx.draw_lights().count(), 0);
}

#[test]
fn light_without_transitions_is_lit_after_one_update_and_freed_on_stop() {
    let (library, hash) = library(
        r#"<Effect type="Infinite" duration="1" version="2">
      <Light><Radius value="3"/><DiffuseR value="1"/></Light>
    </Effect>"#,
    );
    let mut fx = EffectSystem::new(library);
    let mut r = fx.start(hash, IDENTITY).unwrap();
    fx.update(1.0 / 60.0);
    fx.update_particles(1.0 / 60.0);
    let l = fx.effect_lights.active()[0];
    assert_eq!(
        (l.intensity, l.radius, l.diffuse),
        (1.0, 3.0, [1.0, 0.0, 0.0])
    );
    // Stop: DisableLight(None, 0) leaves TransitionTime 0 (still "active"),
    // the next batcher Update drives it negative and frees the slot.
    fx.stop(&mut r);
    assert_eq!(fx.effect_lights.active().len(), 1);
    fx.update_particles(1.0 / 60.0);
    assert_eq!(fx.effect_lights.active().len(), 0);
    assert_eq!(fx.effect_lights.free_slots(), MAX_EFFECT_LIGHTS);
    // A restarted instance gets a fresh light (the null reference is copied).
    fx.start(hash, IDENTITY).unwrap();
    fx.update(1.0 / 60.0);
    fx.update_particles(1.0 / 60.0);
    assert_eq!(fx.draw_lights().count(), 1);
}

#[test]
fn effect_light_variation_multiplies_the_transition_intensity() {
    let (library, hash) = library(
        r#"<Effect type="Infinite" duration="1" version="2">
      <Light><Radius value="3"/><VariationType value="Sine"/>
        <VariationSpeed value="2"/><VariationAmount value="0.5"/></Light>
    </Effect>"#,
    );
    let mut fx = EffectSystem::new(library);
    fx.start(hash, IDENTITY).unwrap();
    fx.update(0.5);
    fx.update_particles(0.5);
    // Transition None → 1, times Sine: 1 + sin(0.5 · 2) · 0.5.
    let expected = 1.0 + 1.0f32.sin() * 0.5;
    assert!(close(fx.effect_lights.active()[0].intensity, expected));
}

const PULSE_LIGHTS: &str = r#"<Effect type="Infinite" duration="1" version="2">
  <PulseEmitter><NrOfParticles value="4"/><TriggerTime value="0"/>
    <SpawnLights Value="True"/><LightSizeRelative Value="True"/>
    <LifeTimeMin value="1"/><LifeTimeMax value="1"/><LifeTimeDistribution value="1"/>
    <SizeStartMin value="0.5"/><SizeStartMax value="0.5"/><SizeStartDistribution value="1"/>
    <SizeEndMin value="1"/><SizeEndMax value="1"/><SizeEndDistribution value="1"/>
    <VelocityMin value="3"/><VelocityMax value="3"/><VelocityDistribution value="1"/>
    <Gravity value="-1"/><Drag value="0.5"/>
    <ColorR value="0.1"/><ColorG value="0.2"/><ColorB value="0.3"/>
    <LightRadius value="2"/><LightSpecular value="0.7"/>
    <LightDiffuseR value="1"/><LightDiffuseG value="0.5"/><LightDiffuseB value="0.25"/>
    <LightAmbientR value="0.1"/><LightAmbientG value="0.2"/><LightAmbientB value="0.3"/>
  </PulseEmitter>
</Effect>"#;

#[test]
fn particle_lights_ride_their_particles_and_fade_with_life() {
    let (library, hash) = library(PULSE_LIGHTS);
    let mut fx = EffectSystem::new(library);
    fx.start(hash, IDENTITY).unwrap();
    fx.update(0.1);
    assert_eq!(fx.particles.active().len(), 4);
    let lights = fx.particles.lights.active();
    assert_eq!(lights.len(), 4);
    for (l, p) in lights.iter().zip(fx.particles.active()) {
        assert_eq!(l.position, p.position);
        // LightSizeRelative: StartSize · LightRadius, Intensity 0 at spawn.
        assert_eq!(l.radius, 1.0);
        assert_eq!(l.intensity, 0.0);
        // Colour comes from the emitter's Light keys, not the particle.
        assert_eq!(l.diffuse, [1.0, 0.5, 0.25]);
        assert_eq!(l.ambient, [0.1, 0.2, 0.3]);
        assert_eq!(l.specular, 0.7);
    }
    fx.update_particles(0.1);
    let lights = fx.particles.lights.active();
    assert_eq!(lights.len(), 4);
    for (l, p) in lights.iter().zip(fx.particles.active()) {
        // Same gravity/drag/position integration as the particle.
        assert_eq!(l.position, p.position, "{l:?} {p:?}");
        // n = 0.9: radius = 1·0.9 + 2·0.1, intensity = sat(8·0.9·0.1).
        assert!(close(l.radius, 1.1), "{l:?}");
        assert!(close(l.intensity, 0.72), "{l:?}");
    }
    assert_eq!(fx.draw_lights().count(), 4);
    // Peak at mid-life, then the lights expire with their particles.
    let mut peak = 0.0f32;
    let mut expired = false;
    for _ in 0..12 {
        fx.update_particles(0.1);
        for l in fx.particles.lights.active() {
            peak = peak.max(l.intensity);
        }
        if fx.particles.lights.active().is_empty() {
            expired = true;
            break;
        }
    }
    // n = 0.5 gives 8·0.25 = 2, clamped to 1.
    assert_eq!(peak, 1.0);
    assert!(expired, "lights never expired");
    assert_eq!(fx.particles.lights.free_slots(), MAX_PARTICLE_LIGHTS);
    assert_eq!(fx.draw_lights().count(), 0);
}

#[test]
fn particle_light_batcher_caps_at_256_and_reuses_freed_slots() {
    let xml = PULSE_LIGHTS.replace(
        r#"<NrOfParticles value="4"/>"#,
        r#"<NrOfParticles value="300"/>"#,
    );
    let (library, hash) = library(&xml);
    let mut fx = EffectSystem::new(library);
    fx.start(hash, IDENTITY).unwrap();
    fx.update(0.1);
    assert_eq!(fx.particles.active().len(), 300);
    assert_eq!(fx.particles.lights.active().len(), MAX_PARTICLE_LIGHTS);
    assert_eq!(fx.particles.lights.free_slots(), 0);
    fx.update_particles(0.1);
    assert_eq!(fx.draw_lights().count(), MAX_PARTICLE_LIGHTS);
    for _ in 0..12 {
        fx.update_particles(0.1);
    }
    assert_eq!(fx.particles.lights.active().len(), 0);
    assert_eq!(fx.particles.lights.free_slots(), MAX_PARTICLE_LIGHTS);
}

#[test]
fn point_light_batcher_caps_at_128_and_late_effects_retry_each_update() {
    let (library, hash) = library(
        r#"<Effect type="Infinite" duration="1" version="2">
      <Light><Radius value="1"/><DiffuseR value="1"/></Light>
    </Effect>"#,
    );
    let mut fx = EffectSystem::new(library);
    let mut refs: Vec<_> = (0..130)
        .map(|_| fx.start(hash, IDENTITY).unwrap())
        .collect();
    fx.update(0.1);
    fx.update_particles(0.1);
    assert_eq!(fx.effect_lights.active().len(), MAX_EFFECT_LIGHTS);
    assert_eq!(fx.effect_lights.free_slots(), 0);
    assert_eq!(fx.draw_lights().count(), MAX_EFFECT_LIGHTS);
    // Stopping one effect frees its slot on the next batcher update; an
    // effect whose spawn failed holds the null reference, so its next
    // VisualEffect.Update spawns into the freed slot.
    // EffectManager.Update runs slots in descending order, so slots 129..2
    // took the 128 lights and slots 1 and 0 hold null references.
    fx.stop(&mut refs[129]);
    fx.update_particles(0.1);
    assert_eq!(fx.effect_lights.free_slots(), 1);
    fx.update(0.1);
    assert_eq!(fx.effect_lights.free_slots(), 0);
    fx.update_particles(0.1);
    assert_eq!(fx.draw_lights().count(), MAX_EFFECT_LIGHTS);
    assert_eq!(fx.active_effects(), 129);
}

#[test]
fn retail_asplosion_light_and_goblin_fuse_particle_lights() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let library = EffectLibrary::load_directory(&root.join("Effects")).unwrap();
    let with_light = library
        .effects
        .values()
        .filter(|e| e.light.is_some())
        .count();
    let spawn_lights = library
        .emitters
        .iter()
        .filter(|e| match e {
            Emitter::Continuous(c) => c.flags.spawn_lights,
            Emitter::Pulse(p) => p.flags.spawn_lights,
        })
        .count();
    eprintln!("{with_light} effects with <Light>, {spawn_lights} SpawnLights emitters");
    assert_eq!(with_light, 31);
    assert!(spawn_lights >= 34, "{spawn_lights}");

    let asplosion = magicka_runtime::scene_loading::custom_hash("lol_asplosion");
    let light = library.effects[&asplosion].light.as_ref().unwrap();
    assert_eq!(light.offset, [0.0, 0.3, 0.0]);
    assert_eq!(light.light.radius, 8.0);
    assert!(close(light.light.diffuse[1], 1.003922));
    assert_eq!(light.spawn_transition, LightTransition::Linear);
    assert_eq!(light.despawn_transition_time, 1.0);

    let fuse = magicka_runtime::scene_loading::custom_hash("goblin_bomb_fuse");
    let mut fx = EffectSystem::new(library);
    let mut asplosion_ref = fx.start(asplosion, translation(0.0, 1.0, 0.0)).unwrap();
    let mut fuse_ref = fx
        .start_at(fuse, [2.0, 1.0, 0.0], [0.0, 0.0, -1.0])
        .unwrap();
    let mut peak_particle_lights = 0;
    for _ in 0..60 {
        fx.update(1.0 / 60.0);
        fx.update_particles(1.0 / 60.0);
        peak_particle_lights = peak_particle_lights.max(fx.particles.lights.active().len());
        for l in fx.particles.lights.active() {
            // Goblin_Bomb_Fuse's SpawnLights emitter: black diffuse, warm ambient.
            assert_eq!(l.diffuse, [0.0, 0.0, 0.0]);
            assert!(close(l.ambient[0], 1.0) && close(l.ambient[1], 0.5019608));
            assert!(l.radius >= 0.0);
        }
    }
    assert!(peak_particle_lights > 0, "no particle lights spawned");
    assert!(peak_particle_lights <= MAX_PARTICLE_LIGHTS);
    // The asplosion light ramped to full over its 1 s linear spawn.
    let effect = fx.effect_lights.active();
    assert_eq!(effect.len(), 1);
    assert!(close(effect[0].intensity, 1.0), "{effect:?}");
    assert!(close(effect[0].position[1], 1.3), "{effect:?}");
    assert_eq!((effect[0].position[0], effect[0].position[2]), (0.0, 0.0));
    eprintln!(
        "{} draw lights after 1 s ({peak_particle_lights} peak particle lights)",
        fx.draw_lights().count()
    );
    fx.stop(&mut asplosion_ref);
    fx.stop(&mut fuse_ref);
    for _ in 0..90 {
        fx.update(1.0 / 60.0);
        fx.update_particles(1.0 / 60.0);
    }
    assert_eq!(fx.draw_lights().count(), 0);
    assert_eq!(fx.effect_lights.free_slots(), MAX_EFFECT_LIGHTS);
}
