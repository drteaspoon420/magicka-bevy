//! LightningSpell / LightningBolt rules (research/lightning.md).
use magicka_sim::{
    damage::Damage,
    lightning::{
        cast_bolt, spell_color, BoltVisual, LightningScene, LightningSource, LightningSpell,
        LightningTarget, LightningWorld,
    },
    spell_cast::EffectOwner,
    spells::{Elements as E, Spell},
    spray_pulse::{HitList, Segment},
};

#[derive(Default)]
struct World {
    targets: Vec<LightningTarget>,
    damaged: Vec<u16>,
    bolts: Vec<BoltVisual>,
}
impl LightningScene for World {
    fn lightning_targets(&mut self, center: [f32; 3], range: f32) -> Vec<LightningTarget> {
        self.targets
            .iter()
            .filter(|t| {
                let d: f32 = (0..3).map(|i| (t.position[i] - center[i]).powi(2)).sum();
                d.sqrt() <= range
            })
            .copied()
            .collect()
    }
    fn lightning_shielded(&mut self, _: Segment) -> bool {
        false
    }
    fn lightning_scene(&mut self, _: Segment) -> Option<f32> {
        None
    }
    fn lightning_damage(&mut self, target: u16, _: u16, _: [Damage; 5], _: f64, _: [f32; 3]) {
        self.damaged.push(target);
    }
}
impl LightningWorld for World {
    fn lightning_bolt(&mut self, bolt: BoltVisual) {
        self.bolts.push(bolt);
    }
    fn next_double(&mut self) -> f64 {
        0.5
    }
}

fn target(handle: u16, x: f32, z: f32) -> LightningTarget {
    LightningTarget {
        handle,
        position: [x, 0.0, z],
        dead: false,
        eligible: true,
        frozen: false,
        ends_chain: false,
    }
}

#[test]
fn force_parameters_follow_cast_force() {
    let s = Spell::default_for_list(&[E::LIGHTNING, E::LIGHTNING]);
    let l = LightningSpell::cast_force(s, false, 0.0).unwrap();
    assert_eq!(l.to_cast, 8);
    assert_eq!(l.range, 4.0 + 2.0 / 5.0 * 8.0 * 0.5);
    assert_eq!(l.time_between, 0.125);
    assert!(l.damages[0].magnitude > 0.0);
    let a = LightningSpell::cast_area(s, 0.0).unwrap();
    assert_eq!((a.to_cast, a.all_around), (32, true));
    assert_eq!(spell_color(&s), [0.75, 0.5, 1.0]);
}

#[test]
fn bolt_chains_through_targets_with_full_range() {
    // Forward is +Z. 1 at 3 m ahead, 2 another 3 m beyond, 3 behind the caster.
    let mut w = World {
        targets: vec![
            target(1, 0.0, 3.0),
            target(2, 0.5, 6.0),
            target(3, 0.0, -2.0),
        ],
        ..Default::default()
    };
    let mut hits = HitList::default();
    hits.add(99);
    cast_bolt(
        &mut w,
        99,
        0.0,
        [0.0; 3],
        [0.0, 0.0, 1.0],
        &mut hits,
        [1.0; 3],
        1.0,
        4.0,
        [Damage::default(); 5],
    );
    // 1, then 2 (chaining keeps the full 4 m range), then 3 is behind the
    // hop direction (> 90°): the chain ends with a cut bolt and the list clears.
    assert_eq!(w.damaged, vec![1, 2]);
    assert_eq!(w.bolts.len(), 3);
    assert_eq!(w.bolts[2].from, [0.5, 0.0, 6.0]);
    assert!(!hits.contains(1));
}

#[test]
fn closer_but_wider_target_does_not_replace_a_straighter_one() {
    // The original needs both a smaller angle and a smaller distance.
    let mut w = World {
        targets: vec![target(1, 0.0, 3.0), target(2, 1.5, 1.5)],
        ..Default::default()
    };
    let mut hits = HitList::default();
    cast_bolt(
        &mut w,
        99,
        0.0,
        [0.0; 3],
        [0.0, 0.0, 1.0],
        &mut hits,
        [1.0; 3],
        1.0,
        0.0 + 3.5,
        [Damage::default(); 5],
    );
    assert_eq!(w.damaged[0], 1);
}

#[test]
fn frozen_dead_and_ineligible_targets_are_skipped() {
    let mut frozen = target(1, 0.0, 2.0);
    frozen.frozen = true;
    let mut dead = target(2, 0.0, 2.5);
    dead.dead = true;
    let mut ethereal = target(3, 0.0, 3.0);
    ethereal.eligible = false;
    let mut w = World {
        targets: vec![frozen, dead, ethereal],
        ..Default::default()
    };
    let mut hits = HitList::default();
    cast_bolt(
        &mut w,
        99,
        0.0,
        [0.0; 3],
        [0.0, 0.0, 1.0],
        &mut hits,
        [1.0; 3],
        1.0,
        5.0,
        [Damage::default(); 5],
    );
    assert!(w.damaged.is_empty());
    assert_eq!(w.bolts.len(), 1);
    // next_double 0.5: no deviation; full range.
    assert_eq!(w.bolts[0].to, [0.0, 0.0, 5.0]);
}

#[test]
fn force_fires_eight_bolts_over_one_second_then_ends() {
    let s = Spell::default_for_list(&[E::LIGHTNING]);
    let mut l = LightningSpell::cast_force(s, false, 0.0).unwrap();
    let mut w = World {
        targets: vec![target(1, 0.0, 3.0)],
        ..Default::default()
    };
    let source = LightningSource {
        owner: 99,
        owner_direction: [0.0, 0.0, 1.0],
        cast_direction: [0.0, 0.0, 1.0],
        cast_position: [0.0; 3],
        weapon_direction: [0.0, 0.0, 1.0],
        weapon_position: [0.0; 3],
        character: true,
        wet: false,
        play_time: 0.0,
    };
    let owner = EffectOwner {
        avatar: true,
        ..Default::default()
    };
    let mut frames = 0;
    while l.update(1.0 / 60.0, owner, source, &mut w).0 {
        frames += 1;
        assert!(frames < 200);
    }
    assert_eq!(w.damaged.len(), 8);
    // Bolts at 0, 0.125, ..., 0.875 s; the spell ends on the next tick.
    assert!((52..=60).contains(&frames), "{frames}");
}

#[test]
fn a_wet_caster_fires_all_around_at_two_metres() {
    let s = Spell::default_for_list(&[E::LIGHTNING]);
    let mut l = LightningSpell::cast_force(s, false, 0.0).unwrap();
    let mut w = World::default();
    let source = LightningSource {
        owner: 99,
        owner_direction: [0.0, 0.0, 1.0],
        cast_direction: [0.0, 0.0, 1.0],
        cast_position: [0.0; 3],
        weapon_direction: [0.0, 0.0, 1.0],
        weapon_position: [0.0; 3],
        character: true,
        wet: true,
        play_time: 0.0,
    };
    l.update(0.01, EffectOwner::default(), source, &mut w);
    assert_eq!(l.range, 2.0);
    let b = w.bolts[0];
    let len: f32 = (0..3)
        .map(|i| (b.to[i] - b.from[i]).powi(2))
        .sum::<f32>()
        .sqrt();
    assert!((len - 2.0).abs() < 1e-5);
    // to_cast 7 → yaw 7π/4.
    assert!(
        b.direction[0] < -0.7 && b.direction[2] > 0.7,
        "{:?}",
        b.direction
    );
}

/// LightningSpell.CastWeapon: CastForce bolts, but from WeaponSource's
/// translation (and its Forward when cast from the staff).
#[test]
fn weapon_bolts_leave_from_the_weapon_source() {
    let s = Spell::default_for_list(&[E::LIGHTNING]);
    let mut l = LightningSpell::cast_weapon(s, true, 0.0).unwrap();
    assert_eq!(l.lifecycle.cast_type, magicka_sim::damage::CastType::Weapon);
    let mut w = World::default();
    let source = LightningSource {
        owner: 99,
        owner_direction: [0.0, 0.0, 1.0],
        cast_direction: [0.0, 0.0, 1.0],
        cast_position: [0.0; 3],
        weapon_direction: [1.0, 0.0, 0.0],
        weapon_position: [0.5, 1.0, 0.25],
        character: true,
        wet: false,
        play_time: 0.0,
    };
    l.update(0.01, EffectOwner::default(), source, &mut w);
    let b = w.bolts[0];
    assert_eq!(b.from, [0.5, 1.0, 0.25]);
    assert!(b.direction[0] > 0.99, "{:?}", b.direction);
    // A wet caster's all-around bolts return to CastSource.
    let mut l = LightningSpell::cast_weapon(s, true, 0.0).unwrap();
    let mut w = World::default();
    l.update(
        0.01,
        EffectOwner::default(),
        LightningSource {
            wet: true,
            ..source
        },
        &mut w,
    );
    assert_eq!(w.bolts[0].from, [0.0; 3]);
}
