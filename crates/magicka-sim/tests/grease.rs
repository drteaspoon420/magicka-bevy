use magicka_sim::grease::*;
use magicka_sim::spells::Elements as E;

#[test]
fn fire_ignites_cold_and_water_cool_and_burning_triples_decay() {
    let mut f = GreaseField::default();
    assert_eq!(f.damage(E::FIRE, 0.5), FireChange::None);
    assert_eq!(f.damage(E::FIRE | E::ARCANE, 0.5), FireChange::Ignited);
    assert!(f.burning && f.temperature == 1.0);
    assert_eq!(f.damage(E::EARTH, 5.0), FireChange::None, "not opposite");
    assert_eq!(f.damage(E::COLD, 0.6), FireChange::None);
    assert!((f.temperature - 0.4).abs() < 1e-6);
    assert_eq!(f.damage(E::WATER, 1.0), FireChange::Extinguished);
    assert!(!f.burning && f.temperature == 0.0);
    // Unlit: 1 s costs 1 s of TTL; burning: 3 s, and it reheats by dt/2.
    let mut g = GreaseField::default();
    assert!(!g.update(1.0));
    assert_eq!(g.ttl, 59.0);
    g.burn(1.0);
    assert!(!g.update(1.0));
    assert_eq!(g.ttl, 56.0);
    assert_eq!(g.danger(), 10.0);
    assert!(g.update(19.0), "burnt out: TTL < 0");
}

#[test]
fn burn_ignition_sets_the_given_temperature() {
    let mut f = GreaseField::default();
    assert_eq!(f.burn(0.4), FireChange::None);
    assert_eq!(f.burn(0.7), FireChange::Ignited);
    assert!(f.burning && (f.temperature - 0.7).abs() < 1e-6);
    assert_eq!(GreaseField::default().danger(), 0.5);
}

#[test]
fn spray_sweeps_ninety_degrees_and_spawns_six_steps() {
    use std::f32::consts::FRAC_PI_4;
    // The greasing arc starts at -63° (step 6); fields use whole steps 5..0,
    // from -45° to +45°.
    assert!((spray_yaw(spray_step(SPRAY_TTL)) + 1.4 * FRAC_PI_4).abs() < 1e-5);
    assert!((spray_yaw(5.0) + FRAC_PI_4).abs() < 1e-6);
    assert!((spray_yaw(0.0) - FRAC_PI_4).abs() < 1e-6);
    // 60 fps over the 0.5 s spray: steps 5, 4, 3, 2, 1, 0 each spawn once.
    let (mut ttl, mut steps) = (SPRAY_TTL, vec![]);
    while ttl > 0.0 {
        let before = ttl;
        ttl -= 1.0 / 60.0;
        steps.extend(spray_spawn_step(before, ttl));
    }
    assert_eq!(steps, [5.0, 4.0, 3.0, 2.0, 1.0, 0.0]);
    assert_eq!(splash_yaw(0), 0.0);
}

#[test]
fn the_pool_recycles_the_oldest_of_seventy_two_fields() {
    let mut pool = GreasePool::default();
    let first = pool.spawn(Some(1), [0.0; 3], [0.0, 1.0, 0.0], 0.0);
    for i in 1..POOL_SIZE {
        pool.spawn(Some(1), [i as f32, 0.0, 0.0], [0.0, 1.0, 0.0], 0.0);
    }
    assert_eq!(pool.len(), POOL_SIZE);
    let serial = pool.get(first).unwrap().serial;
    // The 73rd takes the first field's slot while it is still alive.
    let again = pool.spawn(Some(2), [9.0; 3], [0.0, 1.0, 0.0], 1.0);
    assert_eq!(again, first);
    let f = pool.get(first).unwrap();
    assert_eq!((f.owner, f.position), (Some(2), [9.0; 3]));
    assert_ne!(f.serial, serial);
    pool.remove(first);
    assert_eq!(pool.len(), POOL_SIZE - 1);
}
