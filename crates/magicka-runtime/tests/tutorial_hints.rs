//! Opening tutorial dismissal must run the retail continuation, not just hide UI.
use magicka_runtime::{avatar::PlayerInput, play_session::PlaySession};
use std::path::Path;

#[test]
fn tutorial_actions_gate_conjuring_cast_modes_and_empty_queue_push() {
    use magicka_runtime::toggle_action::ToggleAction;
    use magicka_sim::{damage::CastType, spells::Elements as E};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.disable_scripts();
    // Level.NoItems leaves the Wizard unarmed; casting needs the staff
    // (the opening's weapon rack, endUnequipWeaponsSection).
    {
        use magicka_runtime::scene_world::campaign_live::CampaignRequest;
        s.world
            .campaign_requests
            .push(CampaignRequest::UnequipWeaponsSection {
                begin: true,
                animation: 0,
            });
        s.world
            .campaign_requests
            .push(CampaignRequest::UnequipWeaponsSection {
                begin: false,
                animation: 0,
            });
        s.step(1.0 / 60.0);
    }
    let toggle = |s: &mut PlaySession, tag: &str, attrs: &[(&str, &str)]| {
        ToggleAction::read(tag, attrs.iter().copied())
            .unwrap()
            .execute(&mut s.world)
            .unwrap();
    };
    let input = |s: &mut PlaySession, input: PlayerInput| {
        s.handle_input(input, 16.0 / 9.0);
    };
    toggle(&mut s, "disableElement", &[("element", "all")]);
    let mut fire = PlayerInput::default();
    fire.elements[7] = true;
    input(&mut s, fire);
    assert!(s.avatar().conjure.queue.is_empty());
    toggle(&mut s, "enableElement", &[("element", "fire")]);
    // Enabling a held key does not invent a new key-down edge.
    input(&mut s, fire);
    assert!(s.avatar().conjure.queue.is_empty());
    input(&mut s, PlayerInput::default());
    for _ in 0..9 {
        s.step(1.0 / 60.0);
        input(&mut s, PlayerInput::default());
    }
    input(&mut s, fire);
    assert_eq!(s.avatar().conjure.queue[0].element, E::FIRE);
    input(&mut s, PlayerInput::default());
    let queue = s.avatar().conjure.queue.clone();
    for (name, kind, shift, self_cast) in [
        ("force", CastType::Force, false, false),
        ("area", CastType::Area, true, false),
        ("self", CastType::SelfCast, false, true),
    ] {
        s.world.players[0].as_mut().unwrap().conjure.queue = queue.clone();
        toggle(&mut s, "disableCastType", &[("type", name)]);
        let pressed = PlayerInput {
            cast_down: !self_cast,
            cast_self_down: self_cast,
            shift_down: shift,
            ..Default::default()
        };
        input(&mut s, pressed);
        assert_eq!(
            s.avatar().cast.cast_type,
            CastType::Blast,
            "disabled {name}"
        );
        input(&mut s, PlayerInput::default());
        toggle(&mut s, "enableCastType", &[("type", name)]);
        input(&mut s, pressed);
        assert_eq!(s.avatar().cast.cast_type, kind, "enabled {name}");
        let avatar = s.world.players[0].as_mut().unwrap();
        avatar.cast = Default::default();
        input(&mut s, PlayerInput::default());
    }
    s.world.players[0].as_mut().unwrap().conjure.queue.clear();
    toggle(&mut s, "disablePush", &[]);
    let force = PlayerInput {
        cast_down: true,
        ..Default::default()
    };
    input(&mut s, force);
    assert_eq!(s.avatar().cast.cast_type, CastType::Blast);
    input(&mut s, PlayerInput::default());
    toggle(&mut s, "enablePush", &[]);
    input(&mut s, force);
    assert_eq!(s.avatar().cast.cast_type, CastType::Force);
}

#[test]
fn castle_script_coverage_report() {
    use magicka_runtime::scene_script::{ActionDefinition, ConditionDefinition, SceneScriptLayout};
    use std::collections::BTreeMap;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../target/Magicka/Content/Levels/WizardCastle");
    if !root.is_dir() {
        return;
    }
    for scene in ["wc_s1", "wc_s2", "wc_s3", "wc_s4", "wc_s5"] {
        let bytes = std::fs::read(root.join(format!("{scene}.xml"))).unwrap();
        let layout = SceneScriptLayout::read(&bytes, false).unwrap();
        let mut gaps = BTreeMap::<String, usize>::new();
        for action in &layout.actions {
            if let ActionDefinition::Unsupported(node) = action {
                *gaps.entry(format!("action:{}", node.name)).or_default() += 1;
                assert!(!["enablepush", "disablepush", "sethint", "setdialoghint"]
                    .contains(&node.name.to_ascii_lowercase().as_str()));
            }
        }
        for condition in &layout.conditions {
            if let ConditionDefinition::Unsupported(node) = condition {
                *gaps.entry(format!("condition:{}", node.name)).or_default() += 1;
            }
        }
        eprintln!(
            "{scene}: {} triggers, {} actions, {} conditions; unresolved {gaps:?}",
            layout.triggers.len(),
            layout.actions.len(),
            layout.conditions.len()
        );
    }
}

#[test]
fn castle_script_unlocks_reach_permissions_notices_and_next_scene() {
    use magicka_runtime::{scene_script::ActionDefinition, toggle_action::ToggleKind};
    use magicka_sim::spells::MagickType;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s2", "start").unwrap();
    s.headless_dialogs = Some([1280., 720.]);
    let expected = [MagickType::Revive, MagickType::Ctd, MagickType::Confuse]
        .into_iter()
        .fold(0u64, |bits, kind| bits | (1 << kind as u8));
    assert_eq!(s.magicks_allowed() & expected, 0);
    assert!(s.run_trigger("Get revive", true).unwrap());
    for _ in 0..(8 * 60) {
        s.step(1. / 60.);
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    assert_eq!(s.magicks_allowed() & expected, expected);
    assert!(s
        .unlocked_magicks
        .iter()
        .all(|bits| bits & expected == expected));
    let text = s.world.dialog_text.as_ref().unwrap();
    assert!(
        text.additional.iter().any(|widget| {
            widget
                .state
                .text
                .as_ref()
                .is_some_and(|text| !text.contains("???"))
        }),
        "localized unlock notice"
    );
    let index = s
        .runtime
        .layout()
        .actions
        .iter()
        .position(|action| {
            matches!(action,
                ActionDefinition::Toggle(a) if matches!(a.kind, ToggleKind::UnlockMagick { .. })
            )
        })
        .unwrap();
    s.runtime.quick_execute(index, &mut s.world).unwrap();
    s.step(1. / 60.);
    assert_eq!(s.magicks_allowed() & expected, expected);
    s.change_scene_named("wc_s3", "start").unwrap();
    assert_eq!(
        s.magicks_allowed() & expected,
        expected,
        "unlocks survive scene transition"
    );
}

#[test]
fn opening_hint_dismissal_unlocks_room_and_shows_interaction_hint() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    s.headless_dialogs = Some([1280.0, 720.0]);
    assert!(s.run_trigger("Intro hint", false).unwrap());
    s.step(1.0 / 60.0);
    assert!(s.world.tutorial.dialog.is_some(), "{:?}", s.script_error);
    assert!(!s.world.tutorial.dialog.as_ref().unwrap().text.is_empty());
    s.handle_input_with_dialog_select(PlayerInput::default(), 16.0 / 9.0, true);
    assert!(s.world.tutorial.dialog.is_some(), "one-second holdoff");
    // Advance hint time without running unrelated opening cutscene autoruns.
    s.world.tutorial.update(1.01);
    s.handle_input_with_dialog_select(PlayerInput::default(), 16.0 / 9.0, true);
    assert!(s.world.tutorial.dialog_done());
    assert!(
        !s.run_trigger("intro done", false).unwrap(),
        "callback already executed"
    );
    for _ in 0..90 {
        s.step(1.0 / 60.0);
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    let hint = s.world.tutorial.hint.as_ref().expect("interaction hint");
    assert_eq!(hint.animation, 1);
    assert!(!hint.text.is_empty());
}

/// User report: ending the tutorial gave Confuse but no Meteor Shower.
/// wc_s2's Get revive grants Revive, CtD and Confuse unconditionally and
/// runs Get DLC Magick after 10 s, which unlocks MeteorS only when
/// HasDLC WizardHat holds. The rewrite has no Steam; HasDLC is treated as
/// owned (user decision), so the authored chain must grant Meteor Shower.
#[test]
fn castle_end_of_tutorial_grants_meteor_shower() {
    use magicka_sim::spells::MagickType;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s2", "start").unwrap();
    s.headless_dialogs = Some([1280., 720.]);
    let meteor = 1u64 << MagickType::MeteorS as u8;
    assert_eq!(s.magicks_allowed() & meteor, 0);
    assert!(s.run_trigger("Get revive", true).unwrap());
    for _ in 0..(11 * 60) {
        s.step(1. / 60.);
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    assert_eq!(s.magicks_allowed() & meteor, meteor, "Meteor Shower unlock");
    assert!(s
        .unlocked_magicks
        .iter()
        .all(|bits| bits & meteor == meteor));
}
