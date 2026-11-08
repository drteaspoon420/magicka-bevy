//! Wave / WaveEntity numerics (research/wave.md).
use magicka_sim::barrier::{Barrier, BarrierClips, BarrierEvent, BarrierInit, Stale};
use magicka_sim::damage::attack;
use magicka_sim::spells::Elements as E;
use magicka_sim::wave::{self, Contact, ContactTarget, FirstPillar, WaveEffect};

const CLIPS: BarrierClips = BarrierClips {
    ice_appear: 1.0,
    ice_geyser: 1.0,
    ice_tesla: 1.0,
    ice_earth: 1.0,
    earth_appear: 1.0,
    earth_volcano: 1.0,
};
/// rockpillar_animation's emerge0 (decoded from the retail model).
const PILLAR: f32 = 0.8166666;

fn pillar(at: [f32; 3], range: f32) -> Barrier {
    let first = FirstPillar::new([0.0; 3], [0.0, 0.0, -1.0], 0.5);
    let p = first.placement(at);
    let init = BarrierInit {
        owner: Some(7),
        hit_list: 0,
        hit: p.probe_origin,
        direction: p.direction,
        scale: p.scale,
        range,
        next_direction: p.next_direction,
        next_yaw: p.next_yaw,
        distance: p.distance,
        timestamp: 0.0,
    };
    let damage = wave::cast_damage().unwrap();
    let (b, cue) = Barrier::initialize_wave(
        init,
        wave::spell(),
        damage,
        &CLIPS,
        PILLAR,
        Stale::default(),
        1,
    );
    assert_eq!(cue, Some("spell_earth_barrier"));
    b
}

#[test]
fn first_pillar_geometry() {
    let first = FirstPillar::new([1.0, 0.0, 2.0], [0.0, 0.0, -1.0], 0.5);
    assert_eq!(first.probe_origin, [1.0, 2.0, 1.5]);
    let p = first.placement([1.0, 0.0, 1.5]);
    assert_eq!(p.probe_origin, [1.0, 0.0, -1.5]);
    // (Direction.Z, 0, −Direction.X): sideways.
    assert_eq!(p.direction, [-1.0, 0.0, 0.0]);
    assert_eq!(p.range, 15.0);
    assert_eq!(p.distance, 2.5);
    assert_eq!(p.next_direction, [0.0, 0.0, -1.5]);
}

#[test]
fn pillar_is_a_solid_earth_barrier_with_knockback_damage() {
    let b = pillar([0.0; 3], 15.0);
    assert!(b.solid());
    assert_eq!(b.radius, 1.25);
    assert_eq!(b.position[1], 1.3);
    assert_eq!(b.hit_points, 2000.0);
    assert_eq!(b.armour, 0);
    assert_eq!(b.next_ttl, 0.1);
    assert_eq!(b.next_range, 12.5);
    assert!(b.runes.is_none());
    assert!(!b.draw_ice);
    assert_eq!(b.earth_clip.duration, PILLAR);
    for (i, d) in b.damage.iter().enumerate() {
        assert_eq!(d.element, E::EARTH.0);
        assert_eq!(d.amount, 0.0);
        let want = if i == 0 {
            attack::KNOCKBACK | attack::DAMAGE
        } else {
            attack::KNOCKBACK
        };
        assert_eq!(d.attack, want);
    }
    assert_eq!(b.damage[0].magnitude, 1.0);
    // Steam is the largest W/C/F/L/S magnitude: TTL Steam · 5, no effect.
    assert!(b.added_effect);
    assert_eq!(b.effect_ttl, 10.0);
}

#[test]
fn chain_spawns_after_a_tenth_and_ends_after_six() {
    // Range 15 − 2.5 per pillar: pillars with 12.5 … 2.5 left spawn one more.
    let mut ranges = vec![15.0f32];
    let mut b = pillar([0.0; 3], 15.0);
    loop {
        let mut spawned = None;
        for _ in 0..10 {
            for e in b.update(1.0 / 60.0) {
                if let BarrierEvent::SpawnNext(next) = e {
                    spawned = Some(next);
                }
            }
            if spawned.is_some() {
                break;
            }
        }
        let Some(next) = spawned else { break };
        assert_eq!(next.probe_drop, 1.5);
        ranges.push(next.range);
        b = pillar([0.0; 3], next.range);
    }
    assert_eq!(ranges, [15.0, 12.5, 10.0, 7.5, 5.0, 2.5]);
}

#[test]
fn timer_kills_without_decay_and_kill_is_empty() {
    let mut b = pillar([0.0; 3], 0.0);
    assert!(b.kill().is_empty());
    assert_eq!(b.hit_points, 2000.0);
    let mut t = 0.0;
    while !b.removable() {
        b.update(1.0 / 60.0);
        t += 1.0 / 60.0;
        if t < PILLAR - 0.02 {
            assert_eq!(b.hit_points, 2000.0, "no self-decay");
        }
        assert!(t < 1.0);
    }
    assert!(t >= PILLAR);
    // ClipSpeed 2: the rise finished long before.
    assert!(!b.earth_clip.playing);
}

#[test]
fn contact_rules() {
    let grounded = ContactTarget {
        grounded: true,
        mass: 80.0,
        ..Default::default()
    };
    assert_eq!(wave::contact(grounded, false), Contact::Hit { lift: true });
    assert_eq!(wave::contact(grounded, true), Contact::None);
    let air = ContactTarget {
        grounded: false,
        ..grounded
    };
    assert_eq!(wave::contact(air, false), Contact::None);
    let ghost = ContactTarget {
        ethereal: true,
        ..grounded
    };
    assert_eq!(wave::contact(ghost, false), Contact::None);
    let heavy = ContactTarget {
        mass: 3000.0,
        ..grounded
    };
    assert_eq!(wave::contact(heavy, false), Contact::Hit { lift: false });
    let levitating = ContactTarget {
        levitating: true,
        ..grounded
    };
    assert_eq!(
        wave::contact(levitating, false),
        Contact::Hit { lift: false }
    );
}

#[test]
fn distortion_peaks_at_two_thirds_and_ends_after_a_second() {
    let mut w = WaveEffect::from_owner([0.0; 3], [1.0, 0.0, 0.0], 0.0);
    assert_eq!(w.position, [3.0, 0.0, 0.0]);
    let mut peak = (0.0f32, 0.0f32);
    while !w.is_dead() {
        w.update(1.0 / 60.0);
        let d = w.distortion();
        if d.strength > peak.0 {
            peak = (d.strength, d.time);
        }
    }
    assert!((peak.0 - 1.0).abs() < 1e-3, "{peak:?}");
    assert!((peak.1 - 2.0 / 3.0).abs() < 0.02);
    let end = w.distortion();
    // 60 steps of 1/60 leave TTL just above 0: one more (t ≈ 1.017) frame.
    assert!((end.center[0] - 23.0).abs() < 1.0, "{end:?}");
    // Positional Execute only points into +X/+Z.
    let p = WaveEffect::from_position([0.0; 3], [0.0, 0.5], 0.0);
    assert_eq!(p.direction, [0.0, 0.0, 1.0]);
}
