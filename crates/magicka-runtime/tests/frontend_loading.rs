use magicka_runtime::frontend_loading::{uses_opening_intro, LoadingGate, Phase};

#[test]
fn gameplay_waits_for_both_assets_and_final_narration() {
    for (assets, cue) in [(false, false), (true, false), (false, true)] {
        let mut gate = LoadingGate::default();
        gate.update(0.25, true, true);
        assert_eq!(gate.phase, Phase::FadeIn);
        assert_eq!(gate.alpha, 0.5);
        assert!(gate.blocks_play());
        gate.update(0.25, true, true);
        assert_eq!(gate.phase, Phase::Waiting);
        gate.update(60.0, assets, cue);
        assert_eq!(gate.phase, Phase::Waiting);
        assert!(gate.blocks_play());
        gate.update(0.0, true, true);
        assert_eq!(gate.phase, Phase::FadeOut);
        gate.update(0.25, true, true);
        assert_eq!(gate.alpha, 0.5);
        assert!(gate.blocks_play());
        gate.update(0.25, true, true);
        assert_eq!(gate.phase, Phase::Complete);
        assert_eq!(gate.alpha, 0.0);
        assert!(!gate.blocks_play());
    }
}

#[test]
fn opening_route_depends_on_campaign_chapter_and_hot_join() {
    assert!(uses_opening_intro(true, Some(0), false));
    assert!(!uses_opening_intro(false, Some(0), false));
    assert!(!uses_opening_intro(true, Some(1), false));
    assert!(!uses_opening_intro(true, None, false));
    assert!(!uses_opening_intro(true, Some(0), true));
}
