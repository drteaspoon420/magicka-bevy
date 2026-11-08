use magicka_runtime::hud::{npc_bars, queue_quads, smooth_health, HealthBar};
use magicka_sim::spells::{Elements as E, Spell};

fn bar() -> HealthBar {
    HealthBar {
        normalized: 0.5,
        width: 0.5,
        height: 1.25,
        damage_age: 0.,
        fade: false,
        color: [1., 0., 0., 1.],
        offset: [0.; 2],
    }
}

#[test]
fn frame_stays_full_width_while_fill_tracks_health_and_flashes() {
    let q = bar().quads([100., 200.]);
    assert_eq!(q[0].bounds, [82.5, 216.5, 84.5, 226.5]);
    assert_eq!(q[1].bounds, [116.5, 216.5, 118.5, 226.5]);
    assert_eq!(q[2].bounds, [84.5, 216.5, 116.5, 226.5]);
    assert_eq!(q[3].bounds, [84.5, 216.5, 100.5, 226.5]);
    assert_eq!(q[0].color, [1.; 4]);
    assert_eq!(q[3].color, [2.2, 1.2, 1.2, 1.]);
    assert_eq!(q[3].atlas, [2., 81., 6., 87.]);
}

#[test]
fn health_clamps_width_caps_and_npc_fade_boundary() {
    let q = HealthBar {
        normalized: 2.,
        width: 3.,
        damage_age: 2.5,
        fade: true,
        ..bar()
    }
    .quads([0.; 2]);
    assert_eq!(q[3].bounds, [-47.5, 16.5, 48.5, 26.5]);
    assert_eq!(q[3].color, [1., 0., 0., 0.5]);
    assert!(HealthBar {
        damage_age: 3.,
        fade: true,
        ..bar()
    }
    .quads([0.; 2])
    .is_empty());
    assert!(!HealthBar {
        damage_age: 99.,
        ..bar()
    }
    .quads([0.; 2])
    .is_empty());
}

#[test]
fn stacked_npc_bars_keep_full_lower_bars_and_poison_only_the_last() {
    let b = npc_bars(0.25, 4, 1., 0., true);
    assert_eq!(
        b.iter().map(|b| b.normalized).collect::<Vec<_>>(),
        vec![1., 1., 1., 0.25]
    );
    assert_eq!(b[0].color, [1., 0., 0., 1.]);
    assert_eq!(b[1].color, [0.6, 0.6, 0.6, 1.]);
    assert_eq!(b[3].color, [0., 1., 0., 1.]);
    assert_eq!(b[3].offset, [0., 18.]);
    assert_eq!(b[0].quads([0.; 2])[3].color, [1., 0., 0., 1.]);
    assert_eq!(npc_bars(1., 3, 1., 1., false)[2].color, [2., 1.5, 1., 1.]);
    // Huge template stacks are bounded by the original submission capacity.
    let capped = npc_bars(0.25, i32::MAX, 1., 0., true);
    assert_eq!(capped.len(), 512);
    assert_eq!(capped[511].normalized, 1.0);
}

#[test]
fn displayed_health_initialization_and_damage_smoothing() {
    assert_eq!(smooth_health(0., 100., 100., 0.), 0.);
    assert!((smooth_health(0., 100., 100., 1.) - 0.998).abs() < 1e-6);
    assert!((smooth_health(1., 50., 100., 1.) - 0.501).abs() < 1e-6);
    let mut value = 0.;
    for _ in 0..60 {
        value = smooth_health(value, 100., 100., 1. / 60.);
    }
    assert!((value - 0.998).abs() < 1e-6);
}

#[test]
fn queue_atlas_slots_include_compound_elements_and_mirrored_brackets() {
    assert!(queue_quads(&[], [0.; 2]).is_empty());
    let queue: Vec<_> = [E::WATER, E::LIFE, E::SHIELD, E::ICE, E::POISON]
        .map(Spell::default_for)
        .into();
    let q = queue_quads(&queue, [100., 200.]);
    assert_eq!(q.len(), 7);
    assert_eq!(q[0].atlas, [248., 8., 256., 24.]);
    assert_eq!(q[1].atlas, [256., 8., 248., 24.]);
    assert_eq!(q[2].bounds, [37.5, 231.5, 62.5, 256.5]);
    assert_eq!(q[2].atlas, [50., 156., 100., 205.]);
    assert_eq!(q[5].atlas, [150., 205., 200., 254.]);
    assert_eq!(q[6].atlas, [0., 254., 50., 303.]);
}

#[test]
fn shield_bars_use_native_anchors_capacity_width_and_fading_tint() {
    use magicka_runtime::hud::shield_bar;
    use magicka_sim::shield_entity::{ShieldEntity, ShieldKind};
    for (kind, anchor, offset) in [
        (ShieldKind::Sphere, [1., 2., 5.], 12.),
        (ShieldKind::Disc, [2.2, 2., 2.37952], 16.),
        (ShieldKind::Wall, [-1.88, 2., 6.84], 24.),
    ] {
        let mut shield = ShieldEntity::initialize(
            None,
            [1., 2., 3.],
            2.,
            [0.6, 0., -0.8],
            kind,
            1250.,
            [2., 1.5, 1.],
            &mut [],
        );
        shield.tint[3] = 0.4;
        let (position, bar) = shield_bar(&shield);
        for i in 0..3 {
            assert!((position[i] - anchor[i]).abs() < 1e-5);
        }
        assert_eq!(bar.offset, [0., offset]);
        assert_eq!(bar.normalized, 0.25);
        let q = bar.quads([100., 200.]);
        assert_eq!(q[3].color, [2., 1.5, 1., 0.4]);
        assert_eq!(q[3].bounds, [52.5, 216.5 + offset, 76.5, 224.5 + offset]);
    }
}

#[test]
fn self_shield_bar_stacks_uses_recipe_color_and_flashes_on_damage() {
    use magicka_runtime::hud::self_shield_bar;
    use magicka_sim::character_health::{SelfShield, SelfShieldType};
    assert!(self_shield_bar(&SelfShield::default(), 0.5, 1).is_none());
    let shield = SelfShield {
        kind: SelfShieldType::Shield,
        spell: Spell::default_for(E::SHIELD),
        health: 250.,
        max_health: 500.,
        time_since_damage: 0.,
        ..Default::default()
    };
    let bar = self_shield_bar(&shield, 0.5, 3).unwrap();
    assert_eq!(bar.normalized, 0.5);
    assert_eq!(bar.offset, [0., 18.]);
    assert_eq!(bar.color, [2., 1.5, 1., 1.]);
    let q = bar.quads([100., 200.]);
    assert_eq!(q[3].bounds, [84.5, 234.5, 100.5, 242.5]);
    assert_eq!(q[3].color, [3.2, 2.7, 2.2, 1.]);
    assert!(!bar.fade);
}
#[test]
fn aura_ttl_bars_fill_by_remaining_ttl_without_flash_or_fade() {
    use magicka_runtime::hud::{aura_bar, aura_bars, SHIELD_COLOR};
    assert_eq!(SHIELD_COLOR, [2., 1.5, 1.]);
    // Not self-casted auras and expired auras submit nothing.
    assert!(aura_bar(5., 10., false, 0.5, 1).is_none());
    assert!(aura_bar(0., 10., true, 0.5, 1).is_none());
    let bar = aura_bar(5., 10., true, 0.5, 1).unwrap();
    assert_eq!(bar.normalized, 0.5);
    assert_eq!(bar.width, 0.5);
    assert_eq!(bar.height, 1.);
    assert_eq!(bar.damage_age, f32::MAX);
    assert!(!bar.fade);
    assert_eq!(bar.color, [2., 1.5, 1., 1.]);
    assert_eq!(bar.offset, [0., 6.]);
    let q = bar.quads([100., 200.]);
    // Same anchor rule as the self-shield bar: X +.5 - 32w, Y +16.5 + 6*bars.
    assert_eq!(q[2].bounds, [84.5, 222.5, 116.5, 230.5]);
    assert_eq!(q[3].bounds, [84.5, 222.5, 100.5, 230.5]);
    // float.MaxValue damage age: no flash, full alpha.
    assert_eq!(q[3].color, [2., 1.5, 1., 1.]);
    assert_eq!(q[0].color, [1., 1., 1., 1.]);
    // Three stacked NPC bars push the aura bar to Y +18; list order is kept.
    let bars = aura_bars([(1., 4., true), (2., 2., false), (3., 3., true)], 1., 3);
    assert_eq!(bars.len(), 2);
    assert_eq!(bars[0].normalized, 0.25);
    assert_eq!(bars[1].normalized, 1.);
    assert!(bars.iter().all(|b| b.offset == [0., 18.]));
}
#[test]
fn pickup_text_replaces_the_name_reference_only_when_a_name_exists() {
    use magicka_runtime::hud::pickup_text;
    assert_eq!(
        pickup_text("You found #1;!", Some("Haste")),
        "You found [c=1,1,1]Haste[/c]!"
    );
    assert_eq!(pickup_text("You found #1;!", None), "You found #1;!");
}
#[test]
fn notifier_intensity_dims_the_key_while_boosting_and_recovers_toward_one() {
    use magicka_runtime::hud::notifier_intensity;
    let dt = 1. / 60.;
    let k = 0.001_f64.powf(f64::from(dt)) as f32;
    // SetButtonIntensity(.5) then Lerp(1, .5, .001^dt).
    let dimmed = notifier_intensity(1., true, 1., dt);
    assert_eq!(dimmed, 1. + (0.5 - 1.) * k);
    assert!(dimmed > 0.55 && dimmed < 0.56, "{dimmed}");
    // A hidden button keeps the set intensity without lerping.
    assert_eq!(notifier_intensity(1., true, 0., dt), 0.5);
    // Without a boost the previous intensity relaxes toward one.
    let relaxed = notifier_intensity(dimmed, false, 1., dt);
    assert!(relaxed > dimmed && relaxed < 1.);
    assert_eq!(notifier_intensity(0.7, false, 0., dt), 0.7);
    let mut i = 0.5;
    for _ in 0..600 {
        i = notifier_intensity(i, false, 1., dt);
    }
    assert!((i - 1.).abs() < 1e-3);
}
