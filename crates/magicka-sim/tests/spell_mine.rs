use magicka_sim::damage::{attack, Damage};
use magicka_sim::spell_mine::*;
use magicka_sim::spells::{Elements as E, Spell};

const CLIP: f32 = 2.167; // Models/Effects/mine "Take 001" duration.

fn arcane_mine_spell() -> Spell {
    Spell::default_for(E(E::SHIELD.0 | E::ARCANE.0))
}

fn mine_at(hit: [f32; 3]) -> SpellMine {
    SpellMine::initialize(
        7,
        hit,
        [0.0, 0.0, -1.0],
        1.0,
        0.0,
        [1.0, 0.0, 0.0],
        0.5,
        2.0,
        arcane_mine_spell(),
        3.0,
        CLIP,
        0.0,
    )
    .unwrap()
}

fn finish_clip(m: &mut SpellMine) {
    while !m.clip_finished {
        m.update(0.1, Some(m.position));
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn mine_branch_requires_arcane_or_life_without_physical_elements() {
    let s = |m: u32| is_mine_spell(&Spell::default_for(E(m)));
    assert!(s(E::SHIELD.0 | E::ARCANE.0));
    assert!(s(E::SHIELD.0 | E::LIFE.0 | E::FIRE.0));
    assert!(!s(E::SHIELD.0));
    assert!(!s(E::SHIELD.0 | E::FIRE.0));
    assert!(!s(E::SHIELD.0 | E::ARCANE.0 | E::EARTH.0));
    assert!(!s(E::ARCANE.0));
}

#[test]
fn cast_placements_split_a_four_metre_ring_into_twelve_arcs() {
    let step = std::f32::consts::TAU / 12.0;
    let chord = 2.0 * 4.0 * (step * 0.5).sin();
    let area = cast_placements(true, [0.0; 3], [0.0, 0.0, -1.0]);
    for p in &area {
        assert!(close(p.distance, chord));
        assert!(close(p.range, 4.0 * std::f32::consts::PI - chord));
        // (n + k)/k from the CIL, not the Force formula.
        assert!(close(p.scale, (4.0 * std::f32::consts::PI + 12.0) / 12.0));
        let r = (p.probe_origin[0].powi(2) + p.probe_origin[2].powi(2)).sqrt();
        assert!(close(r, 4.0));
    }
    assert!(close(area[0].next_yaw, step) && close(area[1].next_yaw, -step));
    let force = cast_placements(false, [0.0; 3], [0.0, 0.0, -1.0]);
    assert!(close(
        force[0].scale,
        1.0 + (4.0 * std::f32::consts::PI - 12.0) / 12.0
    ));
    assert!(close(force[0].range, std::f32::consts::PI));
    // Force rings are centred 1 m behind the caster: placements lie 4 m from (0,0,1).
    for p in &force {
        let d = [p.probe_origin[0], p.probe_origin[2] - 1.0];
        assert!(close((d[0] * d[0] + d[1] * d[1]).sqrt(), 4.0));
        assert!(p.probe_origin[2] < 0.0);
    }
}

/// Follow every chain over flat ground: each probe hits 0.75 + 1.5/2 below.
fn chain(placement: MinePlacement) -> Vec<[f32; 3]> {
    let hit = |p: [f32; 3]| [p[0], 0.0, p[2]];
    let mut positions = Vec::new();
    let mut mine = SpellMine::initialize(
        1,
        hit(placement.probe_origin),
        placement.direction,
        placement.scale,
        placement.range,
        placement.next_direction,
        placement.next_yaw,
        placement.distance,
        arcane_mine_spell(),
        0.0,
        CLIP,
        0.0,
    )
    .unwrap();
    loop {
        positions.push(mine.position);
        let mut next = None;
        for _ in 0..10 {
            let u = mine.update(1.0 / 60.0, Some([0.0; 3]));
            next = next.or(u.spawn_next);
        }
        let Some(n) = next else { break };
        assert!(close(n.probe_drop, 1.5));
        mine = SpellMine::initialize(
            1,
            hit(n.probe_origin),
            n.direction,
            n.scale,
            n.range,
            n.next_direction,
            n.next_yaw,
            n.distance,
            arcane_mine_spell(),
            0.0,
            CLIP,
            0.0,
        )
        .unwrap();
    }
    positions
}

#[test]
fn area_chains_close_the_ring_and_force_chains_cover_the_front_arc() {
    let area = cast_placements(true, [0.0; 3], [0.0, 0.0, -1.0]);
    let all: Vec<_> = area.iter().flat_map(|&p| chain(p)).collect();
    assert_eq!(all.len(), 12);
    for p in &all {
        assert!(close(p[1], 0.1));
        assert!((p[0].hypot(p[2]) - 4.0).abs() < 1e-3);
    }
    let force = cast_placements(false, [0.0; 3], [0.0, 0.0, -1.0]);
    let front: Vec<_> = force.iter().flat_map(|&p| chain(p)).collect();
    assert_eq!(front.len(), 4);
    assert!(front.iter().all(|p| p[2] < 0.0));
}

#[test]
fn mines_arm_after_the_clip_and_detonate_half_a_second_after_contact() {
    let other = Toucher::Character {
        owner: false,
        dead: false,
        ethereal: false,
        levitating: false,
    };
    let mut m = mine_at([0.0; 3]);
    assert_eq!(m.on_collision(other), ContactResult::Ignored);
    m.update(0.7, None);
    assert!(!m.clip_finished, "2.1 < 2.167 clip seconds");
    m.update(0.05, None);
    assert!(m.clip_finished);
    let ethereal = Toucher::Character {
        owner: false,
        dead: false,
        ethereal: true,
        levitating: false,
    };
    assert_eq!(m.on_collision(ethereal), ContactResult::Ignored);
    assert_eq!(m.on_collision(other), ContactResult::Armed { first: true });
    assert!(close(m.detonation_timer, 0.5));
    assert_eq!(m.on_collision(other), ContactResult::Ignored);
    assert_eq!(m.update(0.25, None).detonations, 0);
    assert_eq!(m.update(0.25, None).detonations, 1);
    assert!(m.dead && m.removable());
}

#[test]
fn owner_contact_keeps_a_fresh_mine_unarmed_until_the_owner_leaves() {
    let owner = Toucher::Character {
        owner: true,
        dead: false,
        ethereal: false,
        levitating: false,
    };
    let mut m = mine_at([0.0; 3]);
    m.clip_finished = true;
    assert_eq!(m.on_collision(owner), ContactResult::OwnerHeld);
    m.update(0.1, None);
    assert!(!m.activated);
    m.detonation_timer = 0.2; // fuse frozen while the owner stands on it
    assert_eq!(m.on_collision(owner), ContactResult::Ignored);
    m.owner_collision = true;
    m.update(0.1, None);
    assert!(close(m.detonation_timer, 0.2));
    m.detonation_timer = 0.0;
    m.update(0.1, None); // owner gone
    assert!(m.activated);
    assert_eq!(m.on_collision(owner), ContactResult::Armed { first: true });
}

#[test]
fn damage_and_distance_detonations() {
    let mut m = mine_at([0.0; 3]);
    let zero = [Damage::default(); 5];
    assert!(!m.internal_damage(&zero));
    let mut pushed = zero;
    pushed[0] = Damage {
        attack: attack::PUSHED,
        element: E::WATER.0,
        amount: 50.0,
        magnitude: 1.0,
    };
    assert!(!m.internal_damage(&pushed));
    let mut status = zero;
    status[2] = Damage {
        attack: attack::STATUS,
        element: E::FIRE.0,
        amount: -3.0,
        magnitude: 1.0,
    };
    assert!(m.internal_damage(&status));
    assert!(close(m.detonation_timer, 0.1));
    // Timer detonation runs even before the clip finishes.
    assert_eq!(m.update(0.1, None).detonations, 1);
    // Owner beyond 30 m: Detonate repeats each update until removable.
    let mut far = mine_at([0.0; 3]);
    let away = Some([30.1, 0.1, 0.0]);
    assert_eq!(far.update(0.1, away).detonations, 1);
    assert_eq!(far.update(0.1, away).detonations, 1);
    assert!(!far.removable());
    let mut near = mine_at([0.0; 3]);
    assert_eq!(near.update(0.1, Some([29.9, 0.1, 0.0])).detonations, 0);
}

#[test]
fn area_mines_carry_force_damage() {
    let spell = arcane_mine_spell();
    let mine = mine_at([0.0; 3]);
    assert_eq!(mine.damage, mine_damage(&spell).unwrap());
    assert!(mine
        .damage
        .iter()
        .any(|d| d.element == E::ARCANE.0 && d.amount > 0.0));
}

#[test]
fn pool_wraps_and_reinitializes_live_mines_in_place() {
    let mut pool = MinePool::new();
    for i in 0..POOL_SIZE {
        assert_eq!(pool.add(mine_at([i as f32, 0.0, 0.0])), i);
    }
    let slot = pool.add(mine_at([99.0, 0.0, 0.0]));
    assert_eq!(slot, 0);
    assert_eq!(pool.order.len(), POOL_SIZE);
    assert_eq!(pool.order[0], 0);
    assert!(close(pool.get(0).unwrap().position[0], 99.0));
    let m = pool.get_mut(3).unwrap();
    m.kill();
    pool.remove_dead();
    assert_eq!(pool.order.len(), POOL_SIZE, "dead but clip unfinished");
    let m = pool.get_mut(3).unwrap();
    finish_clip(m);
    pool.remove_dead();
    assert_eq!(pool.order.len(), POOL_SIZE - 1);
    assert!(pool.get(3).is_none());
}
