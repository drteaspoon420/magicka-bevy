use magicka_sim::{
    avatar_cast_input::*,
    avatar_input::AvatarInput,
    damage::CastType as C,
    spells::{Elements, Spell},
};
fn ready() -> CastInputContext {
    CastInputContext {
        networked: true,
        staff_equipped: true,
        cast_enabled: true,
        queue_nonempty: true,
        ..Default::default()
    }
}

#[test]
fn completed_weapon_and_self_buttons_clear_only_after_cast_and_spell_end() {
    for button in [C::Weapon, C::SelfCast, C::Force, C::Area, C::Blast] {
        for cast_type in [C::Weapon, C::SelfCast, C::Force, C::Area, C::Blast] {
            for current_spell in [false, true] {
                let mut s = CastInput {
                    pressed: button,
                    cast_type,
                    charge_unlocked: true,
                };
                let mut events = Vec::new();
                s.clear_completed_button(current_spell, true, &mut events);
                let clears = matches!(button, C::Weapon | C::SelfCast)
                    && cast_type == C::Blast
                    && !current_spell;
                assert_eq!(s.pressed, if clears { C::Blast } else { button });
                assert_eq!(s.cast_type, cast_type);
                assert!(s.charge_unlocked);
                assert_eq!(
                    events,
                    if clears {
                        vec![CastInputEvent::SendCast(C::Blast)]
                    } else {
                        vec![]
                    }
                );
            }
        }
    }
}
#[test]
fn force_and_area_require_staff_permission_payload_and_idle_buttons() {
    for area in [false, true] {
        for blocked in 0..6 {
            let mut s = CastInput::default();
            let mut c = ready();
            match blocked {
                0 => c.staff_equipped = false,
                1 => c.cast_enabled = false,
                2 => c.queue_nonempty = false,
                3 => s.pressed = C::Weapon,
                4 => s.cast_type = C::SelfCast,
                _ => {}
            }
            let before = s;
            let mut events = vec![];
            if area {
                s.area_pressed(c, &mut events);
            } else {
                s.force_pressed(c, &mut events);
            }
            if blocked < 5 {
                assert_eq!(s, before);
                assert!(events.is_empty());
            } else {
                let kind = if area { C::Area } else { C::Force };
                assert_eq!(s.pressed, kind);
                assert_eq!(s.cast_type, kind);
                assert_eq!(events, [CastInputEvent::SendCast(kind)]);
            }
        }
    }
    let mut s = CastInput::default();
    s.force_pressed(
        CastInputContext {
            queue_nonempty: false,
            push_enabled: true,
            ..ready()
        },
        &mut vec![],
    );
    assert_eq!(s.cast_type, C::Force);
}
#[test]
fn force_release_unlocks_even_without_a_matching_press_and_does_not_clear_cast_type() {
    let mut s = CastInput::default();
    let mut e = vec![];
    s.area_pressed(ready(), &mut e);
    e.clear();
    s.force_released(true, &mut e);
    assert!(s.charge_unlocked);
    assert!(e.is_empty());
    assert_eq!(s.pressed, C::Area);
    s.charge_unlocked = false;
    s.area_released(true, &mut e);
    assert!(!s.charge_unlocked);
    assert_eq!(s.cast_type, C::Area);
    assert_eq!(s.pressed, C::Blast);
    assert_eq!(e, [CastInputEvent::SendCast(C::Blast)]);
}
#[test]
fn disabled_self_cast_does_not_fall_back_to_item_special() {
    let mut s = CastInput::default();
    let mut e = vec![];
    s.special(
        CastInputContext {
            cast_enabled: false,
            item_special: true,
            ..ready()
        },
        |_, _| panic!(),
        &mut e,
    );
    assert!(e.is_empty());
    assert_eq!(s, CastInput::default());
}
#[test]
fn immediate_self_cast_precedes_network_and_observes_callback_changes() {
    for flag in 0..4 {
        let mut c = ready();
        match flag {
            0 => c.frozen = true,
            1 => c.flying = true,
            2 => c.pushed = true,
            _ => c.knocked_down = true,
        }
        let mut s = CastInput::default();
        let mut e = vec![];
        s.special(
            c,
            |state, events| {
                assert!(events.is_empty());
                assert_eq!(state.cast_type, C::SelfCast);
                state.special_released();
            },
            &mut e,
        );
        assert_eq!(e, [CastInputEvent::SendCast(C::Blast)]);
        assert_eq!(s.cast_type, C::SelfCast);
    }
}
#[test]
fn limited_input_allows_existing_spell_and_item_message_precedes_execution() {
    let mut c = CastInputContext {
        input_limited: true,
        item_special: true,
        source_item_index: 1,
        networked: true,
        ..Default::default()
    };
    let mut s = CastInput::default();
    let mut e = vec![];
    s.special(c, |_, _| panic!(), &mut e);
    assert!(e.is_empty());
    c.current_spell = true;
    s.special(c, |_, _| panic!(), &mut e);
    assert_eq!(
        e,
        [
            CastInputEvent::SendItemSpecial(1),
            CastInputEvent::ExecuteItemSpecial(1)
        ]
    );
    e.clear();
    c.charging = true;
    s.special(c, |_, _| panic!(), &mut e);
    assert!(e.is_empty());
}
#[test]
fn weapon_release_waits_for_cast_end_and_always_clears_gun_followup() {
    let mut s = CastInput {
        cast_type: C::Weapon,
        pressed: C::Weapon,
        ..Default::default()
    };
    let mut e = vec![];
    s.attack_released(true, &mut e);
    assert_eq!(s.pressed, C::Weapon);
    assert_eq!(e, [CastInputEvent::ClearNextAttackAnimation]);
    s.cast_type = C::Blast;
    s.attack_released(false, &mut e);
    assert_eq!(s.pressed, C::Blast);
}
#[test]
fn avatar_combination_resets_chants_and_preserves_other_input_state() {
    let mut a = AvatarInput {
        queue: vec![Spell {
            element: Elements::FIRE,
            ..Default::default()
        }],
        chanted_spells: 3,
        chanting_magick: true,
        ..Default::default()
    };
    let mut e = vec![];
    let spell = a.combine_avatar_spell(&mut e);
    assert_eq!(spell.element, Elements::FIRE);
    assert_eq!(a.prepared, spell);
    assert_eq!(a.chanted_spells, 0);
    assert!(a.queue.is_empty());
    assert!(a.chanting_magick);
    assert_eq!(e, [CastInputEvent::ClearIcons]);
}
