use magicka_runtime::frontend_intro::{
    illustration_uv, IntroEvent as Event, IntroPlayback, PAGE_SECONDS,
};

#[test]
fn dark_opening_starts_narration_before_the_tome_lights_up() {
    let (mut intro, events) = IntroPlayback::enter();
    assert_eq!(events, [Event::StartMusic, Event::Light([0.0, 0.0, 1.5])]);
    assert!(intro.update(2.0, false).is_empty());
    assert_eq!(
        intro.update(1.0, false),
        [Event::PlayNarration(0), Event::ShowOpeningSubtitles]
    );
    assert!(intro.update(8.0, false).is_empty());
    assert_eq!(intro.index, None, "chapter waits for the tome to open");
    assert_eq!(intro.update(0.0, true), [Event::Light([1.0, 0.2, 0.5])]);
    assert_eq!(intro.index, Some(0));
    assert_eq!(intro.remaining, 17.0);
    assert!(intro.update(0.0, true).is_empty(), "first cue is retained");
}

fn chapter() -> IntroPlayback {
    let (mut intro, _) = IntroPlayback::enter();
    intro.update(3.0, true);
    intro.update(5.0, true);
    intro
}

#[test]
fn pages_riffle_then_swap_on_draw_and_play_the_next_cue_on_the_next_update() {
    let mut intro = chapter();
    assert_eq!(
        intro.update(20.0, true),
        [Event::HideSubtitles, Event::PreparePage(1), Event::Riffle]
    );
    assert_eq!(intro.remaining, 6.0, "overshoot is discarded");
    assert_eq!(intro.texture_index, 0);
    assert!(intro.swap_pending);
    let page = intro.draw_page();
    assert_eq!(page.index, 1);
    assert_eq!(page.old_index, 0);
    assert!(page.swap_text);
    assert!(!intro.draw_page().swap_text, "swap is consumed once");
    assert_eq!(intro.update(0.0, true), [Event::PlayNarration(1)]);
    assert!(intro.update(0.0, true).is_empty());
}

#[test]
fn all_six_illustrations_end_in_a_loading_tip_after_the_transition() {
    let mut intro = chapter();
    for index in 0..6 {
        if index > 0 {
            intro.update(0.0, true);
        }
        let next = index + 1;
        assert_eq!(
            intro.update(PAGE_SECONDS[index], true),
            [
                Event::HideSubtitles,
                Event::PreparePage(next),
                Event::Riffle
            ]
        );
        assert_eq!(intro.draw_page().index, next);
        assert_eq!(intro.transition_end(), None);
    }
    assert_eq!(intro.update(0.0, true), [Event::PlayNarration(6)]);
    assert_eq!(
        intro.update(9.0, true),
        [Event::HideSubtitles, Event::FadeToBlack { seconds: 2.0 }]
    );
    assert_eq!(intro.index, Some(7));
    assert_eq!(intro.texture_index, 6, "last narration has no illustration");
    assert_eq!(
        intro.transition_end(),
        Some(Event::LoadFirstLevel {
            loading_tip: Some(7)
        })
    );
    assert_eq!(intro.transition_end(), None);
    assert!(intro.update(100.0, true).is_empty());
}

#[test]
fn skip_has_release_edge_and_open_tome_gates_and_omits_loading_narration() {
    let mut intro = chapter();
    assert!(intro.mouse_action(true, false, false).is_empty());
    assert!(intro.mouse_action(true, true, false).is_empty());
    assert!(intro.mouse_action(false, false, true).is_empty());
    assert_eq!(
        intro.mouse_action(true, false, true),
        [Event::StopNarration]
    );
    assert_eq!(
        intro.update(0.0, true),
        [Event::FadeToBlack { seconds: 2.0 }]
    );
    assert!(intro.update(0.0, true).is_empty());
    assert_eq!(
        intro.transition_end(),
        Some(Event::LoadFirstLevel { loading_tip: None })
    );
    assert_eq!(
        IntroPlayback::exit_events(),
        [Event::Light([1.0, 0.2, 6.0]), Event::HideSubtitles]
    );
}

#[test]
fn skip_during_the_final_fade_still_omits_the_loading_tip() {
    let mut intro = chapter();
    for seconds in PAGE_SECONDS.iter().take(7) {
        intro.update(0.0, true);
        intro.update(*seconds, true);
        intro.draw_page();
    }
    intro.skip(true);
    assert_eq!(
        intro.update(0.0, true),
        [Event::FadeToBlack { seconds: 2.0 }]
    );
    assert_eq!(
        intro.transition_end(),
        Some(Event::LoadFirstLevel { loading_tip: None })
    );
}

#[test]
fn illustration_motion_uses_double_smoothstep_and_original_uv_formulas() {
    let mut intro = chapter();
    intro.update(17.0, true);
    assert_eq!(intro.draw_page().motion, 0.0);
    intro.update(3.0, true);
    assert_eq!(intro.draw_page().motion, 0.5);
    intro.update(2.5, true);
    assert_eq!(intro.draw_page().motion, 1.0);
    assert_eq!(illustration_uv(3, 0.5, [1000.0; 2]), ([0.4, 0.0], [1.0; 2]));
    assert_eq!(
        illustration_uv(4, 0.5, [1000.0, 512.0]),
        ([0.125, 0.0625], [1.5; 2])
    );
    assert_eq!(
        illustration_uv(5, 0.5, [1000.0; 2]),
        ([0.14, 0.11], [1.05; 2])
    );
    assert_eq!(illustration_uv(6, 0.5, [1000.0; 2]), ([0.0; 2], [1.0; 2]));
}
