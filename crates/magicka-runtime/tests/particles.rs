//! PolygonHead particle effects (research/particles.md): .NET Random port,
//! keyed values, emitters, ParticleSystem integration and EffectManager
//! slots, plus every retail effect file (skips without content).
use magicka_content::skinned::IDENTITY;
use magicka_runtime::particles::*;
use std::path::PathBuf;

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir().then_some(root)
}

#[test]
fn dotnet_random_zero_seed_sequence() {
    // Documented .NET Framework outputs of new Random(0).Next().
    let mut r = DotNetRandom::new(0);
    assert_eq!(r.next_int(), 1_559_595_546);
    assert_eq!(r.next_int(), 1_755_192_844);
    let mut r = DotNetRandom::new(0);
    let d = r.next_double();
    assert!((d - 1_559_595_546.0 / f64::from(i32::MAX)).abs() < 1e-15);
}

#[test]
fn keyed_values_blend_hold_and_default() {
    let keys = [
        KeyedValue {
            time: 0.5,
            value: 2.0,
        },
        KeyedValue {
            time: 1.5,
            value: 4.0,
        },
    ];
    assert_eq!(keyed_value(None, 3.0), 1.0);
    assert_eq!(keyed_value(Some(&keys[..1]), 9.0), 2.0);
    assert_eq!(keyed_value(Some(&keys), 0.0), 2.0); // before the first key
    assert_eq!(keyed_value(Some(&keys), 1.0), 3.0);
    assert_eq!(keyed_value(Some(&keys), 2.0), 4.0); // after the last key
    assert_eq!(keyed_value(Some(&[]), 2.0), 0.0);
}

fn library(xml: &str) -> (EffectLibrary, i32) {
    let mut library = EffectLibrary::default();
    let hash = library
        .add_xml("Test", xml.as_bytes(), &mut || RandomCursor(0))
        .unwrap();
    (library, hash)
}

#[test]
fn continuous_emitter_spawns_rate_times_time_and_integrates() {
    let xml = r#"<Effect type="Single" duration="2" keyFramesPerSecond="10" version="2">
      <ContinuousEmitter>
        <BlendMode Value="Additive"/><SpreadType Value="Cone"/>
        <ParticlesPerSecond value="30"/><Particle value="3"/>
        <LifeTimeMin value="0.5"/><LifeTimeMax value="0.5"/><LifeTimeDistribution value="1"/>
        <SizeStartMin value="1"/><SizeStartMax value="1"/><SizeEndMin value="3"/><SizeEndMax value="3"/>
        <VelocityMin value="0"/><VelocityMax value="0"/><Gravity value="-10"/><Drag value="1"/>
        <SpreadConeAngle value="0"/><RotationY value="0"/><HueMin value="0"/><HueMax value="0"/>
        <RotationMin value="0"/><RotationMax value="0"/><RotationSpeedMin value="0"/>
        <RotationSpeedMax value="0"/><SpreadArcHorizontalAngle value="0"/>
        <SpreadArcVerticalMin value="0"/><SpreadArcVerticalMax value="0"/>
        <PositionX value="0"/><PositionY value="0"/><PositionZ value="0"/>
        <PositionXOffset value="0"/><PositionYOffset value="0"/><PositionZOffset value="0"/>
      </ContinuousEmitter>
    </Effect>"#;
    let (library, hash) = library(xml);
    let mut fx = EffectSystem::new(library);
    let mut m = IDENTITY;
    m[12..15].copy_from_slice(&[1.0, 2.0, 3.0]);
    let r = fx.start(hash, m).unwrap();
    assert_eq!(r.id, 0);
    // 0.1 s at 30/s: floor(30·0.1) − floor(0) = 3 particles in one step.
    fx.update(0.1);
    let spawned = fx.particles.active();
    assert_eq!(spawned.len(), 3);
    for p in spawned {
        assert_eq!(p.position, [1.0, 2.0, 3.0]);
        assert_eq!((p.sprite, p.radius, p.normalized_lifetime), (3, 1.0, 0.0));
    }
    // Integrate 0.25 s: half of the 0.5 s life; gravity then drag^dt (1).
    fx.update_particles(0.25);
    let p = fx.particles.active()[0];
    assert!((p.normalized_lifetime - 0.5).abs() < 1e-6);
    assert!((p.radius - 2.0).abs() < 1e-6); // start·0.5 + end·0.5
    assert!((p.velocity[1] + 2.5).abs() < 1e-6);
    assert!((p.position[1] - (2.0 - 0.625)).abs() < 1e-6);
    // Expiry frees every slot and shrinks the active range.
    fx.update_particles(0.3);
    assert!(fx.particles.active().is_empty());
    // A Single effect stops once its timeline passes the duration.
    for _ in 0..25 {
        fx.update(0.1);
    }
    assert!(!fx.is_active(&r));
    assert_eq!(fx.active_effects(), 0);
}

#[test]
fn pulse_fires_once_when_the_timeline_crosses_its_trigger() {
    let xml = r#"<Effect type="Looping" duration="1" version="2">
      <PulseEmitter><NrOfParticles value="5"/><TriggerTime value="0.25"/>
        <LifeTimeMin value="1"/><LifeTimeMax value="1"/><LifeTimeDistribution value="1"/>
      </PulseEmitter>
    </Effect>"#;
    let (library, hash) = library(xml);
    let mut fx = EffectSystem::new(library);
    fx.start(hash, IDENTITY).unwrap();
    let mut counts = vec![];
    for _ in 0..13 {
        fx.update(0.1);
        counts.push(fx.particles.active().len());
    }
    // Fires when the timeline first passes 0.25 (step 3). Ten f32 steps of
    // 0.1 leave 0.99999994 < duration, so the loop wraps one step later and
    // the pulse fires again on step 13 (timeline ≈ 0.3).
    assert_eq!(counts[1], 0);
    assert_eq!(counts[2], 5);
    assert_eq!(counts[11], 5);
    assert_eq!(counts[12], 10);
}

#[test]
fn particle_provenance_survives_stopped_effect_and_slot_reuse() {
    let (library, hash) = library(
        r#"<Effect type="Infinite" duration="1" version="2">
      <PulseEmitter><NrOfParticles value="1"/><TriggerTime value="0.05"/>
        <LifeTimeMin value="1"/><LifeTimeMax value="1"/><LifeTimeDistribution value="1"/>
      </PulseEmitter>
    </Effect>"#,
    );
    let mut fx = EffectSystem::new(library);
    let mut old = fx.start(hash, IDENTITY).unwrap();
    let other = fx.start(hash, IDENTITY).unwrap();
    fx.update(0.1);
    let original = old;
    assert_eq!(
        fx.particles.active_sources(),
        &[Some(other), Some(original)]
    );
    fx.stop(&mut old);
    let replacement = fx.start(hash, IDENTITY).unwrap();
    assert_eq!(replacement.id, original.id);
    assert_ne!(replacement.hash, original.hash);
    fx.update(0.1);
    assert_eq!(
        fx.particles.active_sources(),
        &[Some(other), Some(original), Some(replacement)]
    );
    // Metadata must not alias the still-flying old particles to the reused
    // slot or to another instance of the same definition.
    assert_eq!(fx.particles.active().len(), 3);
}

#[test]
fn effect_slots_reuse_the_lowest_free_index_and_reject_stale_refs() {
    let (library, hash) = library(r#"<Effect type="Infinite" duration="1"></Effect>"#);
    let mut fx = EffectSystem::new(library);
    let mut a = fx.start(hash, IDENTITY).unwrap();
    let b = fx.start(hash, IDENTITY).unwrap();
    assert_eq!((a.id, b.id), (0, 1));
    let stale = a;
    fx.stop(&mut a);
    assert_eq!(a, EffectRef::default());
    assert!(!fx.is_active(&stale));
    let c = fx.start(hash, IDENTITY).unwrap();
    assert_eq!(c.id, 0);
    let mut stale = stale;
    assert!(!fx.update_orientation(&mut stale, IDENTITY));
    assert_eq!(stale.id, -1);
    assert!(fx.start(custom_unknown(), IDENTITY).is_none());
}

fn custom_unknown() -> i32 {
    magicka_runtime::scene_loading::custom_hash("no such effect")
}

#[test]
fn retail_effects_all_load_and_spray_fire_emits() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let library = EffectLibrary::load_directory(&root.join("Effects")).unwrap();
    let (mut continuous, mut pulse) = (0, 0);
    for e in &library.emitters {
        match e {
            Emitter::Continuous(_) => continuous += 1,
            Emitter::Pulse(_) => pulse += 1,
        }
    }
    eprintln!(
        "{} effects, {continuous} continuous and {pulse} pulse emitters",
        library.effects.len()
    );
    assert_eq!(library.effects.len(), 904);
    let fire = magicka_runtime::scene_loading::custom_hash("spray_fire1");
    let definition = &library.effects[&fire];
    assert_eq!(definition.kind, VisualEffectType::Infinite);
    let mut fx = EffectSystem::new(library);
    let mut r = fx
        .start_at(fire, [0.0, 1.0, 0.0], [0.0, 0.0, -1.0])
        .unwrap();
    for _ in 0..60 {
        fx.update(1.0 / 60.0);
        fx.update_particles(1.0 / 60.0);
    }
    let n = fx.particles.active().len();
    eprintln!("spray_fire1 after 1 s: {n} particle slots");
    assert!(n > 60, "{n}");
    // Particles fly forward (−Z) from the cast point.
    let mean_z: f32 = fx
        .particles
        .active()
        .iter()
        .map(|p| p.position[2])
        .sum::<f32>()
        / n as f32;
    assert!(mean_z < -1.0, "{mean_z}");
    fx.stop(&mut r);
    // The longest spray_fire1 particle lives 1.8 s.
    for _ in 0..120 {
        fx.update(1.0 / 60.0);
        fx.update_particles(1.0 / 60.0);
    }
    assert!(fx.particles.active().is_empty());
}
