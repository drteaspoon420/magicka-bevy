use magicka_sim::size_magicks::{CastResult, SizeKind, SizeMagicks};

#[test]
fn capsule_resize_preserves_feet_and_orientation_and_zeroes_velocity() {
    use magicka_sim::character_physics::{CharacterBody, Orientation};
    let mut body = CharacterBody::new(0.75, 0.5, 70.0, 5.0);
    body.move_to([3.0, 5.0, 7.0], Orientation::IDENTITY);
    let feet = body.feet();
    for (length, radius) in [(1.0, 1.5), (0.25, 0.375), (0.5, 0.75)] {
        body.velocity = [1.0, 2.0, 3.0];
        body.set_capsule_form(length, radius);
        assert_eq!(body.feet(), feet);
        assert_eq!(body.velocity, [0.0; 3]);
        assert_eq!(body.orientation, Orientation::IDENTITY);
        assert_eq!((body.length, body.radius), (length, radius));
    }
    assert_eq!(body.position, [3.0, 5.0, 7.0]);
}

fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}

#[test]
fn native_ripple_changes_size_speed_and_template_mass_together() {
    for (kind, scale, speed, mass) in [
        (SizeKind::Grow, 1.1875, 0.90625, 220.625),
        (SizeKind::Shrink, 0.90625, 1.09375, 72.5),
    ] {
        let mut state = SizeMagicks::default();
        assert_eq!(state.cast(kind), CastResult::Started);
        let first = state.update(0.0625, 100.0, 80.0).unwrap();
        near(first.scale, scale);
        near(first.speed_multiplier, speed);
        near(first.mass, mass);
        // Three sixteenths reaches the negative ripple: not linear growth.
        let next = state.update(0.125, 100.0, 80.0).unwrap();
        match kind {
            SizeKind::Grow => near(next.scale, 1.0625),
            SizeKind::Shrink => near(next.scale, 0.96875),
        }
        let full = state.update(0.8125, 100.0, 80.0).unwrap();
        match kind {
            SizeKind::Grow => {
                near(full.scale, 2.0);
                near(full.speed_multiplier, 0.5);
                near(full.mass, 830.0);
            }
            SizeKind::Shrink => {
                near(full.scale, 0.5);
                near(full.speed_multiplier, 1.5);
                near(full.mass, 40.0);
            }
        }
        assert_eq!(state.update(0.0, 100.0, 80.0), Some(full));
    }
}

#[test]
fn repeat_cast_refreshes_without_restarting_animation() {
    for kind in [SizeKind::Grow, SizeKind::Shrink] {
        let mut state = SizeMagicks::default();
        state.cast(kind);
        state.update(0.25, 100.0, 80.0);
        let animation = state.active().unwrap().animation_time;
        assert_eq!(state.cast(kind), CastResult::Refreshed);
        let active = state.active().unwrap();
        assert_eq!(active.ttl, kind.duration());
        assert_eq!(active.animation_time, animation);
    }
}

#[test]
fn opposite_cast_cancels_and_cannot_replace_an_effect_until_fade_finishes() {
    for (old, new) in [
        (SizeKind::Grow, SizeKind::Shrink),
        (SizeKind::Shrink, SizeKind::Grow),
    ] {
        let mut state = SizeMagicks::default();
        state.cast(old);
        state.update(1.0, 100.0, 80.0);
        assert_eq!(state.cast(new), CastResult::CancelledOpposite);
        assert_eq!(state.active().unwrap().kind, old);
        assert_eq!(state.active().unwrap().ttl, 0.0);
        assert!(!state.update(0.5, 100.0, 80.0).unwrap().removed);
        assert_eq!(state.cast(new), CastResult::CancelledOpposite);
        let last = state.update(0.5, 100.0, 80.0).unwrap();
        assert_eq!(last.kind, old); // Still execute the old auras on this frame.
        assert!(last.removed);
        assert_eq!(
            (last.scale, last.speed_multiplier, last.mass),
            (1.0, 1.0, 80.0)
        );
        assert_eq!(state.active(), None);
        assert_eq!(state.cast(new), CastResult::Started);
    }
}

#[test]
fn recasting_same_kind_during_cancelled_fade_revives_it() {
    let mut state = SizeMagicks::default();
    state.cast(SizeKind::Grow);
    state.update(1.0, 100.0, 80.0);
    state.cast(SizeKind::Shrink);
    state.update(0.5, 100.0, 80.0);
    assert_eq!(state.cast(SizeKind::Grow), CastResult::Refreshed);
    near(state.update(0.5, 100.0, 80.0).unwrap().scale, 2.0);
}

#[test]
fn death_fades_size_but_does_not_expire_the_positive_timer() {
    let mut state = SizeMagicks::default();
    state.cast(SizeKind::Shrink);
    state.update(1.0, 100.0, 80.0);
    let faded = state.update(1.0, 0.0, 80.0).unwrap();
    assert_eq!(faded.scale, 1.0);
    assert!(!faded.removed);
    assert_eq!(state.active().unwrap().ttl, 8.0);
    // Restoring health while TTL is positive permits the animation to return.
    near(state.update(1.0, 100.0, 80.0).unwrap().scale, 0.5);
    assert!(state.update(7.0, 0.0, 80.0).unwrap().removed);
}

#[test]
fn expiry_decrements_ttl_before_animation_and_returns_the_final_aura_frame() {
    for kind in [SizeKind::Grow, SizeKind::Shrink] {
        let mut state = SizeMagicks::default();
        state.cast(kind);
        state.update(kind.duration() - 0.5, 100.0, 80.0);
        let fading = state.update(0.5, 100.0, 80.0).unwrap();
        assert!(!fading.removed);
        assert_eq!(state.active().unwrap().animation_time, 0.5);
        assert!(state.update(0.5, 100.0, 80.0).unwrap().removed);
        assert_eq!(state.update(0.5, 100.0, 80.0), None);
    }
}

#[test]
fn clearing_requires_one_restoration_and_discards_the_animation() {
    let mut state = SizeMagicks::default();
    assert!(!state.clear());
    state.cast(SizeKind::Grow);
    state.update(1.0, 100.0, 80.0);
    assert!(state.clear());
    assert!(!state.clear());
    assert_eq!(state.update(1.0, 100.0, 80.0), None);
    assert_eq!(state.cast(SizeKind::Shrink), CastResult::Started);
}
