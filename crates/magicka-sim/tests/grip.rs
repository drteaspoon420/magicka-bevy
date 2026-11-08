use magicka_sim::grip::*;
fn world() -> Lifecycle {
    Lifecycle {
        actors: (1..=4)
            .map(|h| {
                (
                    h,
                    Actor {
                        allow_move: true,
                        allow_rotate: true,
                        ..Default::default()
                    },
                )
            })
            .collect(),
    }
}
fn action() -> ActionFacts {
    ActionFacts {
        npc: true,
        owner_position: [0., 0., 0.],
        owner_radius: 1.,
        victim_position: [1., 500., 0.],
        victim_radius: 1.,
        grip_attach_position: None,
        action_radius: 0.01,
        victim_dead: false,
        victim_drowning: false,
        victim_solid_shield: false,
        scene_blocked: false,
        shield_blocked: false,
    }
}
fn facts() -> AcquisitionFacts {
    AcquisitionFacts {
        victim_mass: 500.,
        victim_ride_animation: true,
        ..Default::default()
    }
}
fn request(kind: Kind) -> Request {
    Request {
        kind,
        grip_joint: 7,
        target_joint: 9,
        tolerance: 10,
        finish_on_grip: true,
    }
}
#[test]
fn acquisition_selects_joint_and_freezes_the_native_bodies_for_all_modes() {
    for kind in [Kind::Pickup, Kind::Ride, Kind::Hold] {
        let mut w = world();
        w.actors.get_mut(&1).unwrap().damage_accumulation = 100.;
        w.actors.get_mut(&1).unwrap().break_free_count = 4;
        w.actors.get_mut(&2).unwrap().break_free_count = 8;
        let result = w.attempt(1, 2, request(kind), action(), facts());
        assert!(result.acquired && result.enter_gripping && result.release_victim_entanglement);
        assert_eq!(w.actors[&1].victim, Some(2));
        assert_eq!(w.actors[&2].gripper, Some(1));
        assert_eq!(w.actors[&1].joint, if kind == Kind::Pickup { 7 } else { 9 });
        assert!(w.actors[&1].immovable);
        assert_eq!(w.actors[&2].immovable, kind == Kind::Pickup);
        assert_eq!(w.actors[&1].damage_accumulation, 0.);
        assert_eq!(w.actors[&1].break_free_count, 0);
        assert_eq!(w.actors[&2].break_free_count, 0);
    }
}
#[test]
fn action_reach_is_strict_horizontal_and_uses_the_optional_attach() {
    let w = world();
    let mut a = action();
    assert!(
        w.action_allowed(1, 2, a),
        "vertical separation is ignored; no attach uses owner radius"
    );
    a.victim_position[0] = 2.;
    assert!(!w.action_allowed(1, 2, a), "equal reach is rejected");
    a.grip_attach_position = Some([2., 0., 0.]);
    assert!(w.action_allowed(1, 2, a));
    for flag in 0..6 {
        let mut blocked = a;
        match flag {
            0 => blocked.npc = false,
            1 => blocked.victim_dead = true,
            2 => blocked.victim_drowning = true,
            3 => blocked.victim_solid_shield = true,
            4 => blocked.scene_blocked = true,
            _ => blocked.shield_blocked = true,
        }
        assert!(!w.action_allowed(1, 2, blocked));
    }
}
#[test]
fn finish_on_grip_survives_second_stage_rejection_and_client_suppression() {
    for flag in 0..7 {
        let mut w = world();
        let mut f = facts();
        match flag {
            0 => f.victim_dead = true,
            1 => f.victim_immortal = true,
            2 => f.victim_ethereal = true,
            3 => f.victim_polymorphed = true,
            4 => f.victim_mass = 501.,
            5 => f.owner_knocked_down = true,
            _ => f.client = true,
        }
        let before = w.actors.clone();
        let result = w.attempt(1, 2, request(Kind::Pickup), action(), f);
        assert!(result.action_passed && result.enter_gripping && !result.acquired);
        assert_eq!(w.actors, before);
    }
    let mut w = world();
    let mut f = facts();
    f.victim_ride_animation = false;
    assert!(!w.attempt(1, 2, request(Kind::Ride), action(), f).acquired);
    assert!(w.attempt(1, 2, request(Kind::Hold), action(), f).acquired);
}
#[test]
fn acquiring_a_gripping_victim_releases_its_old_victim_first() {
    let mut w = world();
    assert!(
        w.attempt(2, 3, request(Kind::Pickup), action(), facts())
            .acquired
    );
    let f = AcquisitionFacts {
        victim_state: ReleaseState::GripAttack,
        ..facts()
    };
    let result = w.attempt(1, 2, request(Kind::Hold), action(), f);
    assert_eq!(
        result.released,
        vec![Release {
            owner: 2,
            victim: 3,
            victim_may_move: false
        }]
    );
    assert_eq!(w.actors[&2].victim, None);
    assert_eq!(w.actors[&2].gripper, Some(1));
    assert_eq!(w.actors[&3].gripper, None);
    assert!(!w.actors[&3].immovable);
    assert!(
        !w.actors[&2].immovable,
        "Hold leaves the former gripper body free after its own release"
    );
    assert!(w.actors[&1].immovable);
}
#[test]
fn release_uses_grippers_state_and_preserves_stale_joint_type_and_tolerance() {
    for state in [
        ReleaseState::Busy,
        ReleaseState::Dead,
        ReleaseState::Drop,
        ReleaseState::Land,
        ReleaseState::Idle,
        ReleaseState::Jump,
        ReleaseState::Move,
        ReleaseState::Entangled,
        ReleaseState::GripAttack,
        ReleaseState::Resurrection,
        ReleaseState::Other,
    ] {
        let mut w = world();
        w.attempt(1, 2, request(Kind::Pickup), action(), facts());
        w.actors.get_mut(&1).unwrap().break_free_count = 2;
        w.actors.get_mut(&2).unwrap().break_free_count = 3;
        let released = w.release(1, state).unwrap();
        assert_eq!(released.victim_may_move, state == ReleaseState::Other);
        assert_eq!(w.actors[&2].allow_move, state == ReleaseState::Other);
        assert!(w.actors[&2].allow_rotate);
        assert_eq!(w.actors[&2].break_free_count, 0);
        assert_eq!(w.actors[&1].break_free_count, 2);
        assert_eq!(w.actors[&1].joint, 7);
        assert_eq!(w.actors[&1].tolerance, 10.);
        assert!(!w.actors[&1].immovable && !w.actors[&2].immovable);
        assert_eq!(
            released.ground_probe([1., 2., 3.]),
            ([1., 5., 3.], [0., -6., 0.])
        );
        assert_eq!(released.landing([1., 0., 3.], 0.5, 1.), [1., 1.1, 3.]);
    }
}
#[test]
fn break_free_and_damage_thresholds_retain_native_comparisons() {
    let mut w = world();
    let mut r = request(Kind::Pickup);
    r.tolerance = -1;
    w.attempt(1, 2, r, action(), facts());
    assert_eq!(w.actors[&1].tolerance, 3.);
    w.break_free(2, 2);
    assert!(!w.should_release(1, false, false, 100.));
    w.break_free(2, 1);
    assert!(w.should_release(1, false, false, 100.));
    w.actors.get_mut(&2).unwrap().break_free_count = 0;
    w.actors.get_mut(&1).unwrap().damage_accumulation = 100.;
    assert!(!w.should_release(1, false, false, 100.));
    w.actors.get_mut(&1).unwrap().damage_accumulation = 101.;
    assert!(w.should_release(1, false, false, 100.));
    assert!(w.should_release(1, true, false, f32::NAN));
    assert!(w.should_release(1, false, true, f32::NAN));
    w.actors.get_mut(&1).unwrap().damage_accumulation = 50.;
    assert!(!w.damage_grip_useful(1, 100.));
    assert_eq!(w.damage_grip_score(1, 100.), 0.);
    w.actors.get_mut(&1).unwrap().damage_accumulation = 25.;
    assert!(w.damage_grip_useful(1, 100.));
    assert_eq!(w.damage_grip_score(1, 100.), 1.);
    w.release(1, ReleaseState::Other);
    assert!(w.should_release(1, false, false, 100.));
    assert!(!w.damage_grip_useful(1, 100.));
    assert_eq!(w.damage_grip_score(1, 100.), f32::MIN);
}

#[test]
fn grip_attack_queue_preloads_before_pickup_and_preserves_the_native_overwrite_gate() {
    let mut w = world();
    assert_eq!(
        w.queue_attack(1, 58, false, true, 7),
        AttackQueue {
            queued: true,
            send_network_message: true
        }
    );
    assert_eq!(
        w.actors[&1].victim, None,
        "PickUpCharacter can queue its second key before acquisition"
    );
    assert_eq!(
        w.queue_attack(1, 59, false, true, 7),
        AttackQueue::default()
    );
    assert_eq!(w.actors[&1].next_grip_attack_animation, 58);
    assert_eq!(
        w.queue_attack(1, 59, true, true, 59),
        AttackQueue {
            queued: true,
            send_network_message: false
        },
        "native compares ordinary NextAttackAnimation"
    );
    w.actors.get_mut(&1).unwrap().next_grip_attack_animation = 0;
    assert!(w.queue_attack(1, 60, false, false, 7).queued);
    w.attempt(1, 2, request(Kind::Pickup), action(), facts());
    assert_eq!(w.actors[&1].next_grip_attack_animation, 60);
    assert!(w.actors[&1].grip_attacking);
    assert!(!w.actors[&1].allow_attack_rotate);
}
#[test]
fn accumulated_damage_is_signed_and_only_counts_the_held_victim() {
    let mut w = world();
    w.attempt(1, 2, request(Kind::Pickup), action(), facts());
    w.accumulate_from(1, 3, 1000.);
    assert_eq!(w.actors[&1].damage_accumulation, 0.);
    w.accumulate_from(1, 2, 150.);
    assert!(w.should_release(1, false, false, 100.));
    w.accumulate_from(1, 2, -13000.);
    assert_eq!(w.actors[&1].damage_accumulation, -12850.);
    assert!(w.damage_grip_useful(1, 100.));
}
