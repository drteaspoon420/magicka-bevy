//! PlayState defeat timing (research/end-game.md).
use magicka_runtime::end_game::{EndGame, EndGameCondition};

#[test]
fn defeat_fades_text_and_restarts_after_six_seconds() {
    let mut e = EndGame::default();
    let dt = 0.01;
    let mut t = 0.0f32;
    let mut peak = 0.0f32;
    let mut restart_at = None;
    while t < 10.0 {
        e.update(dt, true);
        peak = peak.max(e.text_alpha);
        if e.restart && restart_at.is_none() {
            restart_at = Some(t);
        }
        t += dt;
    }
    assert_eq!(e.condition, EndGameCondition::Defeat);
    assert!((peak - 1.0).abs() < 1e-3);
    let at = restart_at.expect("restart");
    // 3 s countdown, then fade in (≤1 s), hold to −2, fade out to 0 by −3.
    assert!((at - 6.0).abs() < 0.05, "{at}");
}

#[test]
fn a_living_avatar_cancels_a_pending_defeat() {
    let mut e = EndGame::default();
    for _ in 0..100 {
        e.update(0.01, true);
    }
    assert_eq!(e.condition, EndGameCondition::Defeat);
    e.update(0.01, false);
    assert_eq!(e.condition, EndGameCondition::None);
    assert_eq!(e.timer, 3.0);
    assert!(!e.restart);
}

#[test]
fn level_complete_fades_then_advances_the_campaign() {
    use magicka_runtime::end_game::*;
    let mut e = EndGame::default();
    e.begin(EndGameCondition::LevelComplete, true, false, 0.0);
    assert!(e.input_limited);
    assert_eq!(e.text(), Some(LEVEL_COMPLETE_TEXT));
    let mut t = 0.0f32;
    while !e.level_complete {
        e.update(1.0 / 60.0, false);
        t += 1.0 / 60.0;
        assert!(t < 10.0);
        assert!(!e.restart);
    }
    // Alpha rises from 0 s, falls from 2 s, and the end runs after 3 s once faded.
    assert!((t - 3.0).abs() < 0.05, "{t}");
    assert!(!e.input_limited, "SubMenuEndGame unlimits input");
    assert_eq!(
        next_campaign_level("havindr.lvl"),
        Some(("Battlefield.lvl", false))
    );
    assert_eq!(
        next_campaign_level("EndOfWorld2.lvl"),
        Some(("Wizard_Castle.lvl", true))
    );
    assert_eq!(next_campaign_level("chs_vietnam.lvl"), None);
}

#[test]
fn phony_level_complete_clears_itself() {
    use magicka_runtime::end_game::*;
    let mut e = EndGame::default();
    e.begin(EndGameCondition::LevelComplete, true, true, 0.0);
    assert!(e.freezes_entities());
    for _ in 0..400 {
        e.update(1.0 / 60.0, false);
    }
    assert_eq!(e.condition, EndGameCondition::None);
    assert!(!e.level_complete && !e.restart && !e.input_limited && !e.phony);
}
