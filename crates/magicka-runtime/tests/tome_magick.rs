//! IconRenderer.TomeMagick: the mouse-wheel magick hint under the player
//! (research/magick-hint.md). Retail tests skip when content is absent.
use magicka_runtime::{avatar::PlayerInput, play_session::PlaySession, tome_magick::*};
use magicka_sim::spells::MagickType as M;
use std::path::PathBuf;

fn only(set: &'static [M]) -> impl Fn(M) -> bool {
    move |t| set.contains(&t)
}

#[test]
fn wheel_cycles_through_allowed_magicks_and_wraps_through_none() {
    let allowed = only(&[M::Haste, M::Fear, M::Nullify]);
    let mut t = TomeMagick::default();
    let mut seen = vec![];
    for _ in 0..4 {
        t.next(&allowed);
        seen.push(t.displayed);
    }
    assert_eq!(seen, [M::Haste, M::Fear, M::Nullify, M::None]);
    seen.clear();
    for _ in 0..4 {
        t.prev(&allowed);
        seen.push(t.displayed);
    }
    assert_eq!(seen, [M::Nullify, M::Fear, M::Haste, M::None]);
}

#[test]
fn nothing_unlocked_keeps_none_without_restarting_the_fade() {
    let mut t = TomeMagick::default();
    t.update(1.0);
    assert_eq!(t.alpha, 1.0);
    t.next(|_| false);
    t.prev(|_| false);
    assert_eq!(t.displayed, M::None);
    t.update(0.1);
    assert_eq!(t.alpha, 1.0, "no change, no fade");
}

#[test]
fn custom_pool_cycles_over_the_native_sentinel_gap_and_has_hints_and_names() {
    let allowed = only(&[
        M::Amalgameddon,
        M::Grow,
        M::Polymorph,
        M::InflictPoisonDamage,
    ]);
    let mut t = TomeMagick::default();
    let mut seen = Vec::new();
    for _ in 0..5 {
        t.next(&allowed);
        seen.push(t.displayed);
    }
    assert_eq!(
        seen,
        [
            M::Amalgameddon,
            M::Grow,
            M::Polymorph,
            M::InflictPoisonDamage,
            M::None
        ]
    );
    seen.clear();
    for _ in 0..5 {
        t.prev(&allowed);
        seen.push(t.displayed);
    }
    assert_eq!(
        seen,
        [
            M::InflictPoisonDamage,
            M::Polymorph,
            M::Grow,
            M::Amalgameddon,
            M::None
        ]
    );
    t.set(M::Grow, &allowed);
    t.update(0.0);
    t.update(1.0);
    assert_eq!(t.elements().len(), 5);
    for i in 36..magicka_sim::spells::MAGICK_INDEX_COUNT {
        assert!(custom_name(M::try_from(i).unwrap()).is_some_and(|s| !s.is_empty()));
    }
}

#[test]
fn setter_walks_up_to_an_allowed_magick() {
    let mut t = TomeMagick::default();
    t.set(M::Grease, only(&[M::Fear]));
    assert_eq!(t.displayed, M::Fear);
    t.set(M::Amalgameddon, only(&[M::Fear]));
    assert_eq!(t.displayed, M::None, "past the last magick wraps to None");
}

#[test]
fn change_fades_out_switches_then_fades_in_at_four_per_second() {
    let mut t = TomeMagick::default();
    t.update(1.0);
    t.next(only(&[M::Fear]));
    assert_eq!((t.displayed, t.shown), (M::Fear, M::None));
    t.update(0.125);
    assert!((t.alpha - 0.5).abs() < 1e-6);
    t.update(0.125);
    assert_eq!((t.alpha, t.shown), (0.0, M::None));
    t.update(0.1);
    assert_eq!(
        (t.alpha, t.shown),
        (0.0, M::Fear),
        "switch frame keeps alpha"
    );
    assert_eq!(t.name_key().as_deref(), Some("#magick_fear"));
    t.update(0.125);
    assert!((t.alpha - 0.5).abs() < 1e-6);
}

#[test]
fn hint_icons_fill_the_slots_the_queue_leaves_free_at_half_alpha() {
    let mut t = TomeMagick::default();
    t.next(only(&[M::Fear]));
    t.update(0.0);
    t.update(1.0);
    assert_eq!(t.shown, M::Fear);
    let q = hint_quads(&t, 0, [100., 200.]);
    assert_eq!(q.len(), 3);
    // Cold, Arcane, Shield: element indices 2, 5, 7 in the 5-wide atlas.
    assert_eq!(q[0].bounds, [37.5, 231.5, 62.5, 256.5]);
    assert_eq!(q[0].atlas, [100., 156., 150., 205.]);
    assert_eq!(q[1].atlas, [0., 205., 50., 254.]);
    assert_eq!(q[2].atlas, [100., 205., 150., 254.]);
    assert!(q.iter().all(|q| q.color == [1., 1., 1., 0.5 * t.alpha]));
    let q = hint_quads(&t, 1, [100., 200.]);
    assert_eq!(q.len(), 2);
    assert_eq!(q[0].bounds[0], 75. - 12.5, "slot 1");
    assert!(hint_quads(&t, 5, [0.; 2]).is_empty());
    assert!(draws_brackets(&t, 0));
    assert!(!draws_brackets(&TomeMagick::default(), 0));
}

#[test]
fn every_magick_has_an_english_name() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let strings = magicka_runtime::play_content::english_strings(&root, &["Wizard"]);
    for i in 1..35u8 {
        let t = M::try_from(i).unwrap();
        let key = name_key(t).unwrap();
        let text = strings.get_string(magicka_runtime::scene_loading::custom_hash(&key));
        assert!(text.is_some_and(|s| !s.is_empty()), "{key}");
    }
}

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir().then_some(root)
}

/// Real input on Havindr: scroll down selects the first unlocked magick,
/// scroll up from None wraps to the last one.
#[test]
fn scroll_input_selects_unlocked_magicks_in_a_live_session() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let frame = |s: &mut PlaySession, input: PlayerInput| {
        s.step(1.0 / 60.0);
        s.handle_input(input, 16.0 / 9.0);
    };
    for _ in 0..30 {
        frame(&mut s, PlayerInput::default());
    }
    let next = PlayerInput {
        magick_next: true,
        ..Default::default()
    };
    let prev = PlayerInput {
        magick_prev: true,
        ..Default::default()
    };
    frame(&mut s, next);
    assert_eq!(s.tome[0].displayed, M::None, "nothing unlocked");
    s.unlocked_magicks[0] |= 1 << M::Haste as u8 | 1 << M::Fear as u8;
    frame(&mut s, next);
    assert_eq!(s.tome[0].displayed, M::Haste);
    frame(&mut s, next);
    assert_eq!(s.tome[0].displayed, M::Fear);
    frame(&mut s, next);
    assert_eq!(s.tome[0].displayed, M::None);
    frame(&mut s, prev);
    assert_eq!(s.tome[0].displayed, M::Fear);
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!((s.tome[0].shown, s.tome[0].alpha), (M::Fear, 1.0));
}
