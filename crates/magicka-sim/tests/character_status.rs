use magicka_sim::character_status::{
    CharacterStatuses, CueState, StatusContext, StatusEvent as Event, StatusResult as R,
};
use magicka_sim::resistance::Resistance;
use magicka_sim::spells::Elements as E;
use magicka_sim::status::{Status as S, StatusEffect as Effect};

fn context(base: &[Resistance; 11]) -> StatusContext<'_> {
    StatusContext {
        immortal: false,
        ethereal: false,
        can_have_status: true,
        avatar: false,
        current_spell: false,
        base,
        equipment: None,
        buffs: &[],
        cues: CueState::default(),
    }
}
fn add(
    state: &mut CharacterStatuses,
    kind: S,
    dps: f32,
    magnitude: f32,
    ctx: &StatusContext<'_>,
) -> R {
    state
        .add(
            Effect::new(kind, dps, magnitude, false),
            ctx,
            &mut Vec::new(),
        )
        .unwrap()
}
fn seed(state: &mut CharacterStatuses, kind: S, dps: f32, magnitude: f32) {
    state
        .set_effect(Effect::new(kind, dps, magnitude, false))
        .unwrap();
    state.current_mask |= kind as u16;
}

#[test]
fn wet_then_cold_freezes_at_one_and_interrupts_cast() {
    let base = [Resistance::IDENTITY; 11];
    let mut ctx = context(&base);
    ctx.current_spell = true;
    let mut state = CharacterStatuses::default();
    assert_eq!(add(&mut state, S::Wet, 0.0, 2.0, &ctx), R::Added);
    let mut events = vec![];
    assert_eq!(
        state.add(Effect::new(S::Cold, 25.0, 5.0, false), &ctx, &mut events),
        Ok(R::Removed)
    );
    assert_eq!(state.effect(S::Frozen).unwrap().magnitude, 1.0);
    assert_eq!(state.current_mask, S::Frozen as u16);
    assert_eq!(
        events,
        [Event::StopCurrentSpell, Event::PlayStatusCue(S::Frozen)]
    );
    assert_eq!(add(&mut state, S::Cold, 25.0, 0.1, &ctx), R::Removed);
    assert_eq!(state.effect(S::Frozen).unwrap().magnitude, 1.0);
}

#[test]
fn ice_status_immunity_prevents_freeze_but_not_cold() {
    let mut base = [Resistance::IDENTITY; 11];
    base[E::ICE.index() as usize].status_resistance = true;
    let ctx = context(&base);
    let mut state = CharacterStatuses::default();
    add(&mut state, S::Wet, 0.0, 1.0, &ctx);
    assert_eq!(add(&mut state, S::Cold, 25.0, 2.0, &ctx), R::Added);
    assert!(state.has(S::Cold));
    assert!(state.has(S::Wet));
    assert!(!state.has(S::Frozen));
}

#[test]
fn fire_cancels_cold_and_wet_even_when_fire_is_resisted() {
    let mut base = [Resistance::IDENTITY; 11];
    base[E::FIRE.index() as usize].status_resistance = true;
    let mut ctx = context(&base);
    ctx.cues = CueState {
        exists: S::Wet as u16,
        playing: S::Wet as u16,
    };
    let mut state = CharacterStatuses::default();
    state.dry_timer = -1.0;
    seed(&mut state, S::Wet, 0.0, 1.0);
    seed(&mut state, S::Cold, 25.0, 1.0);
    let mut events = vec![];
    assert_eq!(
        state.add(Effect::new(S::Burning, 60.0, 1.0, false), &ctx, &mut events),
        Ok(R::Removed)
    );
    assert_eq!(state.current_mask, 0);
    assert_eq!(state.dry_timer, 0.9);
    assert_eq!(
        events,
        [
            Event::DryingEffect,
            Event::StopCue(S::Wet),
            Event::PlaySteamCue
        ]
    );
}

#[test]
fn fire_prefers_wet_over_frozen_then_thaws_by_incoming_magnitude() {
    let base = [Resistance::IDENTITY; 11];
    let ctx = context(&base);
    let mut state = CharacterStatuses::default();
    seed(&mut state, S::Wet, 0.0, 1.0);
    seed(&mut state, S::Frozen, 0.0, 2.0);
    add(&mut state, S::Burning, 60.0, 0.5, &ctx);
    assert_eq!(state.effect(S::Frozen).unwrap().magnitude, 2.0);
    add(&mut state, S::Burning, 60.0, 0.5, &ctx);
    assert_eq!(state.effect(S::Frozen).unwrap().magnitude, 1.5);
}

#[test]
fn cold_cancellation_retains_audio_cue_dependency() {
    let base = [Resistance::IDENTITY; 11];
    for playing in [false, true] {
        let mut ctx = context(&base);
        ctx.cues = CueState {
            exists: S::Burning as u16,
            playing: if playing { S::Burning as u16 } else { 0 },
        };
        let mut state = CharacterStatuses::default();
        seed(&mut state, S::Burning, 60.0, 1.0);
        assert_eq!(add(&mut state, S::Cold, 25.0, 1.0, &ctx), R::Removed);
        assert_eq!(state.effect(S::Burning).unwrap().dead(), playing);
        assert_eq!(state.dry_timer, if playing { 0.9 } else { 0.0 });
    }
}

#[test]
fn status_resistance_scales_fire_dps_but_other_status_magnitude() {
    let mut base = [Resistance::IDENTITY; 11];
    for e in [E::FIRE, E::POISON] {
        base[e.index() as usize].multiplier = 0.5;
        base[e.index() as usize].modifier = -10.0;
    }
    let equipment = [Resistance::IDENTITY; 11];
    let buffs = [Resistance {
        against: E::FIRE | E::POISON,
        multiplier: 0.5,
        modifier: 2.0,
        status_resistance: true,
    }];
    let mut ctx = context(&base);
    ctx.avatar = true;
    ctx.equipment = Some(&equipment);
    ctx.buffs = &buffs;
    let mut state = CharacterStatuses::default();
    add(&mut state, S::Burning, 60.0, 2.0, &ctx);
    let fire = state.effect(S::Burning).unwrap();
    assert_eq!((fire.dps, fire.magnitude), (13.0, 2.0));
    add(&mut state, S::Poisoned, 60.0, 2.0, &ctx);
    let poison = state.effect(S::Poisoned).unwrap();
    assert_eq!((poison.dps, poison.magnitude), (52.0, 0.5));
}

#[test]
fn cache_rebuild_excludes_higher_statuses_without_deleting_slots() {
    let base = [Resistance::IDENTITY; 11];
    let ctx = context(&base);
    let mut state = CharacterStatuses::default();
    assert_eq!(add(&mut state, S::Poisoned, 50.0, 1.0, &ctx), R::Added);
    assert!(!state.effect(S::Poisoned).unwrap().dead());
    assert!(!state.has(S::Poisoned));
    // Healing checks the cached mask, so this immediate heal does not cure
    // poison until another phase has rebuilt the full mask.
    add(&mut state, S::Healing, -80.0, 1.0, &ctx);
    assert!(!state.effect(S::Poisoned).unwrap().dead());
    state.current_mask |= S::Poisoned as u16;
    add(&mut state, S::Healing, -80.0, 1.0, &ctx);
    assert!(state.effect(S::Poisoned).unwrap().dead());
}

#[test]
fn resisted_healing_still_cures_and_grease_is_consumed_before_fire_resistance() {
    let mut base = [Resistance::IDENTITY; 11];
    base[E::LIFE.index() as usize].status_resistance = true;
    base[E::FIRE.index() as usize].multiplier = 0.5;
    let ctx = context(&base);
    let mut state = CharacterStatuses::default();
    seed(&mut state, S::Poisoned, 50.0, 1.0);
    seed(&mut state, S::Bleeding, 20.0, 1.0);
    assert_eq!(add(&mut state, S::Healing, -80.0, 1.0, &ctx), R::None);
    assert!(state.effect(S::Poisoned).unwrap().dead());
    assert!(state.effect(S::Bleeding).unwrap().dead());
    seed(&mut state, S::Greased, 1.0, 1.0);
    assert_eq!(add(&mut state, S::Burning, 60.0, 1.0, &ctx), R::Added);
    assert_eq!(state.effect(S::Burning).unwrap().dps, 120.0);
    assert!(state.effect(S::Greased).unwrap().dead());
}

#[test]
fn tiny_nonzero_dps_is_not_confused_with_machine_epsilon() {
    let base = [Resistance::IDENTITY; 11];
    let ctx = context(&base);
    let mut state = CharacterStatuses::default();
    assert_eq!(
        add(&mut state, S::Poisoned, f32::from_bits(1), 1.0, &ctx),
        R::Added
    );
    assert_eq!(add(&mut state, S::Greased, 0.0, 1.0, &ctx), R::None);
    assert_eq!(add(&mut state, S::Wet, 0.0, 1.0, &ctx), R::Added);
}

#[test]
fn guards_leave_all_state_and_cached_mask_untouched() {
    let base = [Resistance::IDENTITY; 11];
    let mut ctx = context(&base);
    let mut state = CharacterStatuses::default();
    seed(&mut state, S::Poisoned, 50.0, 1.0);
    for guard in 0..3 {
        ctx.immortal = guard == 0;
        ctx.ethereal = guard == 1;
        ctx.can_have_status = guard != 2;
        assert_eq!(add(&mut state, S::Healing, -80.0, 1.0, &ctx), R::None);
        assert!(state.has(S::Poisoned));
        assert!(!state.effect(S::Poisoned).unwrap().dead());
    }
}
