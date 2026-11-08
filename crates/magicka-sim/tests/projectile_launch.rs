use magicka_sim::{
    damage::CastType,
    impact_motion::Vector,
    projectile_launch::{
        ForceProjectile, LaunchError, LaunchSource, ProjectileLaunch, ProjectileLaunchHost,
    },
    spell_cast::EffectOwner,
    spells::{Elements as E, Spell},
    spray_pulse::Segment,
};
use std::collections::VecDeque;
#[derive(Default)]
struct Host {
    rng: VecDeque<f64>,
    draws: usize,
    spawned: Vec<ProjectileLaunch>,
    block: bool,
    blocked: Vec<(E, Vector)>,
    radii: Vec<f32>,
}
impl ProjectileLaunchHost for Host {
    fn shields(&mut self) -> Vec<u16> {
        vec![7, 8]
    }
    fn shield_intersection(&mut self, _: u16, _: Segment, r: f32) -> Option<Vector> {
        self.radii.push(r);
        self.block.then_some([1., 0., 0.])
    }
    fn blocked_effect(&mut self, e: E, _: Vector, d: Vector) {
        self.blocked.push((e, d));
    }
    fn next_double(&mut self) -> f64 {
        self.draws += 1;
        self.rng.pop_front().expect("unexpected RNG draw")
    }
    fn spawn(&mut self, l: ProjectileLaunch) {
        self.spawned.push(l);
    }
}
fn source() -> LaunchSource {
    LaunchSource {
        owner_position: [0.; 3],
        owner_direction: [0., 0., -1.],
        cast_position: [1., 0., 0.],
        may_spawn: true,
    }
}
fn owner() -> EffectOwner {
    EffectOwner {
        avatar: true,
        ..Default::default()
    }
}

#[test]
fn self_earth_falls_from_above_without_shield_query_or_random_draw() {
    let mut power = 0.0;
    let mut projectile =
        ForceProjectile::cast_self(Spell::default_for_list(&[E::EARTH]), &mut power, true, 9.0)
            .unwrap();
    let mut host = Host {
        block: true,
        ..Default::default()
    };
    projectile.update(0.0, owner(), source(), &mut host);
    assert_eq!(projectile.lifecycle.cast_type, CastType::SelfCast);
    assert_eq!(projectile.charge, 0.5);
    assert_eq!(host.spawned.len(), 1);
    assert_eq!(host.spawned[0].position, [1.0, 8.0, 0.0]);
    assert_eq!(host.spawned[0].velocity, [0.025, -25.0, 0.0]);
    assert_eq!(host.draws, 0);
    assert!(host.radii.is_empty());
}

#[test]
fn self_ice_early_shard_uses_ring_position_and_aims_at_owner() {
    let mut power = 1.0;
    let mut projectile =
        ForceProjectile::cast_self(Spell::default_for_list(&[E::ICE]), &mut power, false, 9.0)
            .unwrap();
    assert_eq!(power, 0.0);
    let mut host = Host {
        rng: [0.0, 0.0].into(),
        ..Default::default()
    };
    projectile.update(0.0, owner(), source(), &mut host);
    assert_eq!(host.spawned.len(), 1);
    assert_eq!(host.spawned[0].position, [7.0, 10.0, 0.0]);
    let velocity = host.spawned[0].velocity;
    assert!(velocity[0] < 0.0 && velocity[1] < 0.0);
    assert!((velocity[0] / velocity[1] - 0.7).abs() < 1e-6);
    assert_eq!(host.draws, 2);
    assert!((velocity.iter().map(|v| v * v).sum::<f32>().sqrt() - 137.5).abs() < 1e-4);
}
fn cast(list: &[E], mut power: f32) -> ForceProjectile {
    ForceProjectile::cast(Spell::default_for_list(list), &mut power, true, true, 9.).unwrap()
}
#[test]
fn charged_earth_launches_once_without_randomness_and_consumes_power() {
    let mut power = 1.;
    let mut p = ForceProjectile::cast(
        Spell::default_for_list(&[E::EARTH, E::EARTH]),
        &mut power,
        true,
        true,
        9.,
    )
    .unwrap();
    assert_eq!(power, 0.);
    assert_eq!(p.force, 82.5);
    assert_eq!(p.mass, 101.);
    assert_eq!(p.radius, 0.);
    let mut h = Host::default();
    let (keep, turn) = p.update(1. / 60., owner(), source(), &mut h);
    assert!(!keep);
    assert_eq!(turn, 0.);
    assert_eq!(h.draws, 0);
    assert_eq!(h.spawned.len(), 1);
    assert_eq!(h.spawned[0].velocity, [0., 0., -82.5]);
    assert_eq!(h.spawned[0].position, [1., 0., 0.]);
    assert_eq!(h.spawned[0].total_projectiles, 1);
    assert!(!p.lifecycle.active);
}
#[test]
fn ice_burst_accumulates_direction_within_update_and_final_shot_uses_base_speed() {
    let mut p = cast(&[E::ICE], 0.);
    let mut h = Host {
        rng: VecDeque::from([0.75, 0., 0.75, 0.]),
        ..Default::default()
    };
    assert!(!p.update(0.2, owner(), source(), &mut h).0);
    assert_eq!(h.draws, 4);
    assert_eq!(h.spawned.len(), 3);
    let final_v = h.spawned[2].velocity;
    assert!((final_v[0] + 25. * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
    assert!((final_v[2] + 25. * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
    assert_eq!(h.spawned[0].total_projectiles, 3);
}
#[test]
fn split_updates_restart_direction_and_full_charge_still_draws_speed_rng() {
    let mut p = cast(&[E::ICE], 1.);
    let interval = p.interval;
    let mut h = Host {
        rng: VecDeque::from([0., 0., 1., 1.]),
        ..Default::default()
    };
    assert!(p.update(0.001, owner(), source(), &mut h).0);
    assert!(p.update(interval, owner(), source(), &mut h).0);
    assert!(!p.update(interval, owner(), source(), &mut h).0);
    assert_eq!(h.spawned[0].velocity, [0., 0., -137.5]);
    assert_eq!(h.spawned[1].velocity, [0., 0., -220.]);
    assert_eq!(h.spawned[2].velocity, [0., 0., -110.]);
    assert_eq!(h.draws, 4);
}
#[test]
fn shield_block_consumes_shot_before_random_or_spawn() {
    for (element, shots, radius) in [(E::EARTH, 1, 0.17500001), (E::ICE, 3, 0.1)] {
        let mut p = cast(&[element], 1.);
        let mut h = Host {
            block: true,
            ..Default::default()
        };
        for shot in 0..shots {
            let dt = if shot == 0 { 0.001 } else { p.interval };
            assert_eq!(p.update(dt, owner(), source(), &mut h).0, shot + 1 < shots);
            assert_eq!(p.remaining, shots - shot - 1);
        }
        assert!(h.spawned.is_empty());
        assert_eq!(h.draws, 0);
        assert_eq!(h.blocked, vec![(element, [-1., -0., -0.]); shots as usize]);
        assert_eq!(h.radii, vec![radius; shots as usize]);
    }
}
#[test]
fn network_gate_still_consumes_burst_and_dead_owner_is_checked_after_launch() {
    let mut p = cast(&[E::ICE], 0.5);
    let mut h = Host {
        rng: VecDeque::from([0.5; 4]),
        ..Default::default()
    };
    let mut s = source();
    s.may_spawn = false;
    p.update(0.2, owner(), s, &mut h);
    assert_eq!(h.draws, 4);
    assert!(h.spawned.is_empty());
    assert!(!p.lifecycle.active);
    let mut p = cast(&[E::ICE], 1.);
    let mut h = Host {
        rng: VecDeque::from([0.5; 2]),
        ..Default::default()
    };
    let mut o = owner();
    o.dead = true;
    assert!(!p.update(0.001, o, source(), &mut h).0);
    assert_eq!(h.spawned.len(), 1);
    assert_eq!(p.cooldown, 0.);
}
#[test]
fn blast_classification_retains_magnitudes_after_mask_edit() {
    let mut s = Spell::default_for_list(&[E::EARTH, E::FIRE, E::FIRE]);
    assert!((s.blast_size() - (1. / 5_f64).sqrt() as f32).abs() < 1e-6);
    s.element = E::FIRE;
    assert!((s.blast_size() - (2. / 5_f64).sqrt() as f32).abs() < 1e-6);
    let mut s = Spell::default_for_list(&[E::EARTH, E::ICE]);
    s.mag[0] = 5.;
    s.element = E::ICE;
    assert_eq!(s.blast_size(), 1., "Earth magnitude survives mask removal");
    let p = ForceProjectile::cast(s, &mut 0., false, false, 0.).unwrap();
    assert_eq!(p.radius, 10.);
    assert_eq!(p.charge, 0.5);
    assert_eq!(p.force, 25.);
    s.mag[8] = f32::NAN;
    assert!(s.blast_size().is_nan());
}
#[test]
fn zero_fractional_count_and_invalid_spells_do_not_invent_a_projectile() {
    let mut s = Spell::default_for_list(&[E::ICE]);
    s.mag[8] = 0.1;
    let mut p = ForceProjectile::cast(s, &mut 0., true, false, 0.).unwrap();
    assert_eq!(p.total, 0);
    let mut h = Host::default();
    assert!(!p.update(0.01, owner(), source(), &mut h).0);
    assert!(h.spawned.is_empty());
    let mut power = 1.;
    assert_eq!(
        ForceProjectile::cast(Spell::default(), &mut power, true, false, 0.).unwrap_err(),
        LaunchError::InvalidPhysicalSpell
    );
    assert_eq!(
        power, 0.,
        "native resets power before invalid physical check"
    );
    assert_eq!(p.lifecycle.cast_type, CastType::Force);
}
