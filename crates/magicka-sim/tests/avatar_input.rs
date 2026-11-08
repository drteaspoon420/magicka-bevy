use magicka_sim::{
    avatar_input::*,
    damage::{attack, Damage},
    spells::{Direction as D, Elements as E, MagickTree, Spell},
    status::Status,
};
fn ctx() -> ConjureContext {
    ConjureContext {
        can_chant: true,
        ..Default::default()
    }
}
fn press(a: &mut AvatarInput, d: D, c: ConjureContext) {
    a.direction(
        d,
        c,
        &MagickTree::default(),
        (|_| true, |_| true),
        |_, _| panic!("unexpected damage"),
        &mut vec![],
    )
    .unwrap();
}
#[test]
fn conjuring_combines_queue_counts_growth_and_prepares_spell() {
    let mut a = AvatarInput::default();
    for d in [D::Left, D::Up, D::Right, D::Up] {
        press(&mut a, d, ctx());
    }
    assert_eq!(a.queue.len(), 1);
    assert_eq!(a.queue[0].element, E::ICE);
    assert_eq!(a.chanted_spells, 1);
    let s = a.combine_spell();
    assert_eq!(s.element, E::ICE);
    assert!(a.queue.is_empty());
    assert_eq!(a.prepared, s);
}
#[test]
fn wet_lightning_delivers_five_halved_slots_before_early_return() {
    let mut a = AvatarInput::default();
    let c = ConjureContext {
        status_mask: Status::Wet as u16,
        networked: true,
        can_chant: false,
        ..ctx()
    };
    press(&mut a, D::Left, c);
    a.chanting_magick = true;
    let mut events = vec![];
    let mut captured = None;
    a.direction(
        D::Down,
        c,
        &MagickTree::default(),
        (|_| true, |_| true),
        |damage, prior| {
            assert_eq!(
                prior,
                &[
                    ConjureEvent::SendConjure(E::LIGHTNING),
                    ConjureEvent::WetLightningTip
                ]
            );
            captured = Some(damage);
        },
        &mut events,
    )
    .unwrap();
    let damage = captured.unwrap();
    assert_eq!(damage[0].amount, 250.0);
    assert_eq!(damage[0].magnitude, 0.5);
    assert_eq!(damage[4], Damage::default());
    assert!(a.queue.is_empty());
    assert!(a.directions.is_empty());
    assert_eq!(a.chanted_spells, 0);
    assert_eq!(a.chant_direction, D::Down);
    assert!(a.chanting_magick);
}
#[test]
fn greased_fire_uses_lightning_table_including_two_status_entries() {
    let mut a = AvatarInput::default();
    let c = ConjureContext {
        status_mask: Status::Greased as u16,
        ..ctx()
    };
    press(&mut a, D::Right, c);
    let mut calls = 0;
    a.direction(
        D::Down,
        c,
        &MagickTree::default(),
        (|_| true, |_| true),
        |d, _| {
            calls += 1;
            assert_eq!(d[0].amount, 0.0);
            assert_eq!(d[0].attack, attack::DAMAGE);
            assert_eq!(d[1].amount, 60.0);
            assert_eq!(d[1].magnitude, 0.5);
            assert_eq!(d[1].attack, attack::STATUS);
            assert_eq!(d[4].amount, 60.0);
            assert_eq!(d[4].magnitude, 0.5);
        },
        &mut vec![],
    )
    .unwrap();
    assert_eq!(calls, 1);
    assert!(a.queue.is_empty());
}
#[test]
fn passive_allows_wet_lightning_and_guards_do_not_append_input() {
    let mut a = AvatarInput::default();
    let c = ConjureContext {
        status_mask: Status::Wet as u16,
        wet_lightning_passive: true,
        ..ctx()
    };
    press(&mut a, D::Left, c);
    press(&mut a, D::Down, c);
    assert_eq!(a.queue[0].element, E::LIGHTNING);
    for c in [
        ConjureContext {
            in_event: true,
            ..ctx()
        },
        ConjureContext {
            polymorphed: true,
            ..ctx()
        },
        ConjureContext {
            charging: true,
            ..ctx()
        },
    ] {
        press(&mut a, D::Up, c);
        assert!(a.directions.is_empty());
    }
}
#[test]
fn disabled_element_clears_input_without_queue_sound_or_damage() {
    let mut a = AvatarInput::default();
    press(&mut a, D::Right, ctx());
    let mut events = vec![];
    a.direction(
        D::Down,
        ctx(),
        &MagickTree::default(),
        (|_| false, |_| true),
        |_, _| panic!(),
        &mut events,
    )
    .unwrap();
    assert!(a.queue.is_empty());
    assert!(a.directions.is_empty());
    assert!(events.is_empty());
}
#[test]
fn magick_unlocked_predicate_is_respected_and_rejection_still_sounds() {
    let mut a = AvatarInput {
        queue: vec![Spell::default_for(E::EARTH); 5],
        ..Default::default()
    };
    let mut events = vec![];
    press(&mut a, D::Right, ctx());
    a.direction(
        D::Down,
        ctx(),
        &MagickTree::default(),
        (|_| true, |_| true),
        |_, _| panic!(),
        &mut events,
    )
    .unwrap();
    assert_eq!(a.queue.len(), 5);
    assert_eq!(a.chanted_spells, 0);
    assert!(events.contains(&ConjureEvent::ElementSound(E::FIRE)));
    let mut a = AvatarInput {
        queue: vec![Spell::default_for(E::ARCANE), Spell::default_for(E::SHIELD)],
        ..Default::default()
    };
    a.direction(
        D::Up,
        ctx(),
        &MagickTree::default(),
        (|_| true, |_| false),
        |_, _| panic!(),
        &mut vec![],
    )
    .unwrap();
    assert!(!a.chanting_magick);
}
