use magicka_sim::barrier::*;
use magicka_sim::character_impact::result;
use magicka_sim::damage::{attack, Damage};
use magicka_sim::spells::{Elements as E, Spell};
use std::f32::consts::{PI, TAU};

const CLIPS: BarrierClips = BarrierClips {
    ice_appear: 0.5,
    ice_geyser: 1.0,
    ice_tesla: 0.333,
    ice_earth: 0.333,
    earth_appear: 0.5,
    earth_volcano: 0.5,
};

fn spell(mask: u32) -> Spell {
    Spell::default_for(E(mask | E::SHIELD.0))
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

fn init(spell: Spell, hit: [f32; 3]) -> (Barrier, BarrierVisuals, [Damage; 5]) {
    let cast = cast_placements(false, [0.0; 3], [0.0, 0.0, -1.0], &spell).unwrap();
    Barrier::initialize(
        BarrierInit {
            owner: Some(1),
            hit_list: 0,
            hit,
            direction: [0.0, 0.0, -1.0],
            scale: 1.0,
            range: 0.0,
            next_direction: [1.0, 0.0, 0.0],
            next_yaw: 0.3,
            distance: 2.0,
            timestamp: 0.0,
        },
        spell,
        cast.damage,
        &CLIPS,
        Stale::default(),
    )
    .unwrap()
}

#[test]
fn barrier_branch_excludes_pure_shield_and_mines() {
    let b = |m: u32| is_barrier_spell(&spell(m));
    assert!(b(E::EARTH.0) && b(E::FIRE.0) && b(E::ICE.0) && b(E::EARTH.0 | E::ARCANE.0));
    assert!(!b(0) && !b(E::ARCANE.0) && !b(E::LIFE.0 | E::FIRE.0));
    assert!(!is_barrier_spell(&Spell::default_for(E::EARTH)));
}

#[test]
fn hardness_overlay_uses_each_elements_magnitude_and_native_clamp() {
    for (magnitude, expected) in [
        (0.0, 0.0),
        (1.0, 0.0),
        (2.0, 0.333333),
        (3.0, 0.666666),
        (4.0, 0.9999989),
        (5.0, 1.0),
    ] {
        let mut s = spell(E::EARTH.0 | E::ICE.0);
        s.mag[0] = magnitude;
        s.mag[8] = 2.0;
        let (b, _, _) = init(s, [0.0; 3]);
        assert_eq!(b.overlay_alpha(false), expected);
        assert_eq!(b.overlay_alpha(true), 0.333333);
        let mut s = spell(E::EARTH.0 | E::ICE.0);
        s.mag[0] = 2.0;
        s.mag[8] = magnitude;
        let (b, _, _) = init(s, [0.0; 3]);
        assert_eq!(b.overlay_alpha(true), expected);
        assert_eq!(b.overlay_alpha(false), 0.333333);
    }
}

#[test]
fn force_placements_follow_the_recovered_ring() {
    let earth = cast_placements(false, [0.0; 3], [0.0, 0.0, -1.0], &spell(E::EARTH.0)).unwrap();
    let step = TAU / 10.0; // 4·2π / 2.5 = 10.05 → 10 arcs
    let [a, b] = earth.placements;
    assert!(close(a.distance, 8.0 * (step * 0.5).sin()));
    assert!(close(a.range, PI) && close(b.range, PI));
    assert!(close(a.scale, 1.0 + (8.0 * PI / 2.5 - 10.0) / 10.0));
    // Pure Earth: centred 2 m behind; only the first probe is lifted.
    assert!(close(a.probe_origin[1], 1.0) && close(b.probe_origin[1], 0.0));
    let d = (a.probe_origin[0].powi(2) + (a.probe_origin[2] - 2.0).powi(2)).sqrt();
    assert!(close(d, 4.0));
    assert!(earth
        .damage
        .iter()
        .all(|d| d.magnitude == 0.0 || close(d.magnitude, 0.25)));
    // Fire has a status element: centred on the caster, 16 arcs.
    let fire = cast_placements(false, [0.0; 3], [0.0, 0.0, -1.0], &spell(E::FIRE.0)).unwrap();
    let f = fire.placements[0];
    assert!(close(f.next_yaw, TAU / 16.0));
    assert!(close(f.probe_origin[0].hypot(f.probe_origin[2]), 4.0));
}

#[test]
fn area_rings_scale_with_shield_and_keep_full_damage() {
    let mut s = spell(E::EARTH.0);
    s.mag[7] = 2.0; // Shield magnitude
    let area = cast_placements(true, [0.0; 3], [0.0, 0.0, -1.0], &s).unwrap();
    let p = area.placements[0];
    let c = 5.0 * TAU;
    assert!(close(p.range, (c - p.distance) * 0.5));
    assert!(close(p.probe_origin[0].hypot(p.probe_origin[2]), 5.0));
    let force = cast_placements(false, [0.0; 3], [0.0, 0.0, -1.0], &s).unwrap();
    for (a, f) in area.damage.iter().zip(&force.damage) {
        if a.magnitude != 0.0 {
            assert!(f.magnitude < a.magnitude);
        }
    }
}

fn chain_count(area: bool, mask: u32) -> usize {
    let s = spell(mask);
    let cast = cast_placements(area, [0.0; 3], [0.0, 0.0, -1.0], &s).unwrap();
    let mut count = 0;
    for p in cast.placements {
        let mut next = Some((
            p.probe_origin,
            p.direction,
            p.range,
            p.next_direction,
            p.next_yaw,
        ));
        while let Some((origin, direction, range, nd, yaw)) = next.take() {
            count += 1;
            let (mut b, _, _) = Barrier::initialize(
                BarrierInit {
                    owner: Some(1),
                    hit_list: 0,
                    hit: [origin[0], 0.0, origin[2]],
                    direction,
                    scale: p.scale,
                    range,
                    next_direction: nd,
                    next_yaw: yaw,
                    distance: p.distance,
                    timestamp: 0.0,
                },
                s,
                cast.damage,
                &CLIPS,
                Stale::default(),
            )
            .unwrap();
            for _ in 0..6 {
                for e in b.update(1.0 / 60.0) {
                    if let BarrierEvent::SpawnNext(n) = e {
                        assert!(close(n.probe_drop, 1.5));
                        next = Some((
                            n.probe_origin,
                            n.direction,
                            n.range,
                            n.next_direction,
                            n.next_yaw,
                        ));
                    }
                }
            }
        }
    }
    count
}

#[test]
fn chains_fill_force_arcs_and_close_area_rings() {
    assert_eq!(chain_count(false, E::EARTH.0), 4);
    assert_eq!(chain_count(false, E::FIRE.0), 6);
    assert_eq!(chain_count(true, E::EARTH.0), 10);
    assert_eq!(chain_count(true, E::FIRE.0), 16);
}

#[test]
fn initialize_sets_kind_health_armour_and_visuals() {
    let (earth, v, initial) = init(spell(E::EARTH.0), [0.0; 3]);
    assert_eq!(earth.kind, BarrierKind::Solid);
    assert!(close(earth.hit_points, 1000.0) && earth.armour == 0);
    assert!(close(earth.position[1], 1.3));
    assert_eq!(earth.earth_model, Some(EarthModel::Plain));
    assert_eq!(v.spawn_effects, vec!["barrier_earth_spawn"]);
    assert_eq!(v.cue, Some("spell_earth_barrier"));
    assert!(
        initial.iter().any(|d| d.amount > 0.0),
        "Projectile/Area impact"
    );
    let (ice, v, _) = init(spell(E::ICE.0), [0.0; 3]);
    assert_eq!(ice.armour, 50);
    assert!(close(ice.ice_barrier_ttl, 1.0));
    assert_eq!(ice.ice_model, Some(IceModel::Plain));
    assert_eq!(v.cue, Some("spell_ice_barrier"));
    let (volcano, v, _) = init(spell(E::EARTH.0 | E::FIRE.0), [0.0; 3]);
    assert_eq!(volcano.earth_model, Some(EarthModel::Volcano));
    assert!(close(volcano.effect_ttl, 5.0));
    assert_eq!(v.effect, Some("barrier_geyser_fire"));
    let (fire, v, initial) = init(spell(E::FIRE.0), [0.0; 3]);
    assert_eq!(fire.kind, BarrierKind::Elemental);
    assert!(close(fire.hit_points, 200.0) && close(fire.radius, 0.75));
    assert_eq!(v.effect, Some("barrier_fire1_b"));
    assert_eq!(v.cue, Some("spell_fire_barrier"));
    assert!(initial.iter().all(|d| d.amount == 0.0));
    let (_, v, _) = init(spell(E::STEAM.0 | E::LIGHTNING.0), [0.0; 3]);
    assert_eq!(
        v.effect,
        Some("barrier_steam1_b"),
        "Steam wins with Lightning"
    );
    // The loops start at −1, so Water (magnitude 0) wins for Shield+Poison.
    let (_, v, _) = init(spell(E::POISON.0), [0.0; 3]);
    assert_eq!(v.effect, Some("barrier_water1_b"));
}

#[test]
fn elemental_walls_decay_and_vanish_at_once() {
    let (mut fire, _, _) = init(spell(E::FIRE.0), [0.0; 3]);
    fire.update(0.01);
    assert!(close(fire.hit_points, 190.0), "first tick is immediate");
    let mut t = 0.01f32;
    while !fire.removable() {
        fire.update(0.01);
        t += 0.01;
    }
    assert!((t - 3.81).abs() < 0.05, "200 HP at 50/s, {t}");
}

#[test]
fn solid_walls_play_the_reverse_clip_and_arcane_walls_detonate() {
    let (mut earth, _, _) = init(spell(E::EARTH.0), [0.0; 3]);
    earth.update(0.3);
    assert!(
        !earth.earth_clip.playing,
        "appear clip 0.5 s at ClipSpeed 2"
    );
    let events = earth.kill();
    assert_eq!(
        events,
        vec![BarrierEvent::Death {
            ice: false,
            earth: true
        }]
    );
    assert!(!earth.removable());
    earth.update(0.2);
    assert!(!earth.removable());
    earth.update(0.06);
    assert!(earth.removable());
    let (mut runes, _, _) = init(spell(E::EARTH.0 | E::ARCANE.0), [0.0; 3]);
    assert_eq!(runes.runes, Some(E::ARCANE));
    assert_eq!(runes.kill(), vec![BarrierEvent::Detonate]);
    assert!(runes.removable());
    let (mut ice, _, _) = init(spell(E::ICE.0), [0.0; 3]);
    let mut died = false;
    for _ in 0..80 {
        died |= ice
            .update(1.0 / 60.0)
            .iter()
            .any(|e| matches!(e, BarrierEvent::Death { ice: true, .. }));
    }
    assert!(
        died && ice.is_dead(),
        "ice spikes last Ice magnitude seconds"
    );
}

fn hit(element: E, amount: f32) -> [Damage; 5] {
    let mut d = [Damage::default(); 5];
    d[0] = Damage {
        attack: attack::DAMAGE,
        element: element.0,
        amount,
        magnitude: 1.0,
    };
    d
}

#[test]
fn damage_rules_for_solid_and_elemental_walls() {
    let (mut ice, _, _) = init(spell(E::ICE.0), [0.0; 3]);
    let (r, _) = ice.internal_damage(&hit(E::EARTH, 40.0), true);
    assert_eq!(r & result::DEFLECTED, result::DEFLECTED, "armour 50");
    assert!(close(ice.hit_points, 500.0), "deflected: no loss");
    let hp = ice.hit_points;
    ice.internal_damage(&hit(E::FIRE, 40.0), true);
    assert!(close(ice.hit_points, hp - 40.0), "fire ignores ice armour");
    let (mut cold, _, _) = init(spell(E::EARTH.0 | E::COLD.0), [0.0; 3]);
    assert!(cold.effect_ttl > 0.0);
    let hp = cold.hit_points;
    cold.internal_damage(&hit(E::FIRE, 100.0), true);
    assert!(cold.effect_ttl == 0.0 && cold.hit_points == hp, "quenched");
    let (mut fire, _, _) = init(spell(E::FIRE.0), [0.0; 3]);
    fire.internal_damage(&hit(E::WATER, 7.0), true);
    assert!(close(fire.hit_points, 200.0 - 35.0));
    fire.internal_damage(&hit(E::EARTH, 7.0), true);
    assert!(close(fire.hit_points, 165.0), "not opposed");
}

#[test]
fn pool_reuses_the_last_freed_instance_with_its_stale_effect_ttl() {
    let mut pool = BarrierPool::new();
    let list = pool.new_hit_list();
    let (slot, stale) = pool.take();
    assert_eq!(stale, Stale::default());
    let (mut volcano, _, _) = init(spell(E::EARTH.0 | E::FIRE.0), [0.0; 3]);
    volcano.hit_list = list;
    volcano.update(1.0);
    volcano.earth_clip.playing = false;
    volcano.hit_points = 0.0;
    pool.add(slot, volcano);
    assert!(pool.ages_hit_list(slot));
    assert_eq!(pool.remove_dead().len(), 1);
    let (again, stale) = pool.take();
    assert_eq!(again, slot);
    assert!(close(stale.effect_ttl, 4.0));
    let list = pool.new_hit_list();
    let cast = cast_placements(false, [0.0; 3], [0.0, 0.0, -1.0], &spell(E::EARTH.0)).unwrap();
    let (earth, _, _) = Barrier::initialize(
        BarrierInit {
            owner: Some(1),
            hit_list: list,
            hit: [0.0; 3],
            direction: [0.0, 0.0, -1.0],
            scale: 1.0,
            range: 0.0,
            next_direction: [1.0, 0.0, 0.0],
            next_yaw: 0.0,
            distance: 1.0,
            timestamp: 0.0,
        },
        spell(E::EARTH.0),
        cast.damage,
        &CLIPS,
        stale,
    )
    .unwrap();
    let hl = magicka_sim::spray_pulse::HitList::default();
    assert!(
        earth.contact_damages(false, &hl, 9),
        "stale TTL arms a pure Earth wall"
    );
    assert!(earth.physical_contact(0) && !earth.physical_contact(1));
}

#[test]
fn arcane_and_life_solid_walls_draw_tinted_spinning_runes() {
    let (mut b, _, _) = init(spell(E::EARTH.0 | E::ARCANE.0), [1.0, 2.0, 3.0]);
    let (rune, color) = b.rune().unwrap();
    assert_eq!(rune, Rune::Arcane);
    // Earth and Shield are left out of the tint: pure Arcane colour.
    assert_eq!(color, [2.0, 0.4, 0.6]);
    b.update(1.0);
    let (yaw, at) = b.rune_placement();
    assert!(close(yaw, -0.25));
    assert!(
        close(at[1], b.position[1] - 1.3) && at[0] == b.position[0],
        "{at:?} {:?}",
        b.position
    );
    let (b, _, _) = init(spell(E::ICE.0 | E::ARCANE.0 | E::LIFE.0), [0.0; 3]);
    assert_eq!(b.rune().unwrap().0, Rune::Life);
    let (b, _, _) = init(spell(E::EARTH.0), [0.0; 3]);
    assert!(b.rune().is_none());
}

#[test]
fn segment_query_keeps_scaled_world_axis_and_body_origin() {
    let (mut b, _, _) = init(spell(E::EARTH.0), [0.; 3]);
    b.position = [4., 100., 0.];
    b.direction = [0., 0., -2.];
    b.scale = 2.;
    b.radius = 2.5;
    // MakeOrientationMatrix keeps Up length 2, then Scale(2) makes the
    // capsule Backward length 4. Axis starts at body Y - 0.05 and ends
    // at Y + 0.35, rather than at the offset collision-skin position.
    let segment = magicka_sim::spray_pulse::Segment {
        origin: [0., 103., 0.],
        delta: [8., 0., 0.],
    };
    let point = b.segment_intersect(segment, 0.25).unwrap();
    assert!(close(point[0], 4.));
    assert!(close(point[1], 102.85));
    assert!(b.segment_intersect(segment, 0.).is_none());
}
