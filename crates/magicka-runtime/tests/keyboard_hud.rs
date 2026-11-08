use magicka_runtime::keyboard_hud::{anchor, key_anchor, quads, KeyboardHud, KeyboardIcon};
use magicka_sim::spells::Elements as E;

#[test]
fn cooldown_crossing_defers_the_reset_bloom_to_the_next_update() {
    let mut hud = KeyboardHud::default();
    hud.cooldown(E::FIRE).unwrap();
    hud.update(0.125);
    let icon = hud.icons[3];
    assert_eq!(icon.cooldown, 0.);
    assert_eq!(icon.reset_timer, 0.125);
    assert_eq!(icon.saturation, 0.5);
    assert_eq!(icon.intensity, 1.);
    hud.update(0.125);
    assert_eq!(hud.icons[3].reset_timer, 0.);
    assert_eq!(hud.icons[3].intensity, 3.5);
    assert_eq!(hud.icons[3].saturation, 1.);
    hud.update(0.01);
    assert_eq!(hud.icons[3].intensity, 1.);
}

#[test]
fn disabled_state_freezes_timers_and_intensity_and_reset_only_enables() {
    let mut hud = KeyboardHud::default();
    hud.icons[0] = KeyboardIcon {
        enabled: false,
        cooldown: 0.1,
        reset_timer: 0.05,
        intensity: 7.,
        ..default_icon()
    };
    hud.update(1.);
    assert_eq!(hud.icons[0].saturation, 0.);
    assert_eq!(hud.icons[0].cooldown, 0.1);
    assert_eq!(hud.icons[0].reset_timer, 0.05);
    assert_eq!(hud.icons[0].intensity, 7.);
    hud.reset();
    assert!(hud.icons[0].enabled);
    assert_eq!(hud.icons[0].saturation, 0.);
    assert_eq!(hud.icons[0].cooldown, 0.1);
}
fn default_icon() -> KeyboardIcon {
    KeyboardIcon::default()
}

#[test]
fn retrigger_keeps_reset_timer_and_large_steps_do_not_catch_up_branches() {
    let mut hud = KeyboardHud::default();
    hud.icons[1].reset_timer = 0.1;
    hud.icons[1].intensity = 4.;
    hud.cooldown(E::WATER).unwrap();
    assert_eq!(hud.icons[1].reset_timer, 0.1);
    hud.update(2.);
    assert_eq!(hud.icons[1].cooldown, -1.875);
    assert_eq!(hud.icons[1].intensity, 4.);
    assert_eq!(hud.icons[1].reset_timer, 0.125);
    hud.update(2.);
    assert_eq!(hud.icons[1].intensity, 20.);
    hud.update(0.);
    assert_eq!(hud.icons[1].intensity, 1.);
    let previous = hud.icons;
    assert_eq!(hud.cooldown(E::STEAM), Err(E::STEAM));
    assert_eq!(hud.cooldown(E::NONE), Err(E::NONE));
    assert_eq!(hud.icons, previous);
}

#[test]
fn keyboard_layout_uses_the_original_stagger_and_second_atlas_row() {
    assert_eq!(anchor(1, 720.), [40., 580.]);
    assert_eq!(anchor(4, 720.), [65., 640.]);
    assert_eq!(anchor(6, 720.), [90., 580.]);
    assert_eq!(anchor(3, 720.), [215., 640.]);
    assert_eq!(anchor(3, 1080.), [215., 1000.]);
    assert_eq!(key_anchor(3, 720.), [217., 628.]);
    let q = quads(
        6,
        720.,
        KeyboardIcon {
            intensity: 2.,
            ..default_icon()
        },
    );
    assert_eq!(q[0].bounds, [85., 585., 135., 635.]);
    assert_eq!(q[0].atlas, [25., 0., 50., 25.]);
    assert_eq!(q[1].bounds, [90., 580., 140., 630.]);
    assert_eq!(q[1].atlas, [50., 206., 100., 256.]);
    assert_eq!(q[2].bounds, [78., 568., 103., 593.]);
    assert_eq!(q[2].atlas, [0., 0., 25., 25.]);
    assert_eq!(q[1].color, [2., 2., 2., 1.]);
}
