//! NPC Wander/Panic orders, stumbling, flocking and danger avoidance
//! (research/npc-agents.md, 2026-10-30): AIStateWander's walk/rest clock
//! and steering, AIStatePanic's entry/exit rules, FollowGroup's separation
//! and cohesion terms, the Entity.GetDanger catalogue and the idle-state
//! danger escape through AIStateMove.
use magicka_content::character::AnimationClipAction;
use magicka_runtime::{
    ai_event::{AiEvent, MoveEvent},
    animation_controller::AnimationSet,
    avatar_ai::{AiState, GroupLeader, ORDER_ATTACK, ORDER_PANIC, ORDER_WANDER},
    character_motion::{ActorFrame, CharacterState, MOVE_PANIC, MOVE_STUMBLE},
    grip::GRIP_RIDE,
    npc::{avoidance, AgentWeights, Neighbour, NpcActorTemplate, NpcCharacter},
    CharacterDefinition,
};
use magicka_sim::{
    agent_steering::{
        drift_angle, elect_leader, follow_group, group_members, is_panicing, panic_state_movement,
        panic_steer, stumble_state_movement, wander_steer, wander_vector, wrap_angle, DangerSource,
        GroupCandidate, GroupWeights, WanderClock, WanderInputs, LEADER_MAX_AGE,
        MISSILE_DEFAULT_DANGER, PANIC_SPEED, STUMBLE_SPEED, WANDER_PAUSE_BASE, WANDER_PAUSE_RANDOM,
        WANDER_REST_BASE, WANDER_REST_RANDOM,
    },
    character_physics::CharacterBody,
    navmesh::{NavMesh, NavTriangle},
    resistance::Resistance,
};
use std::{collections::HashMap, sync::Arc};

type Vector = [f32; 3];

fn clip(animation: &str, clip: &str, looping: bool) -> AnimationClipAction {
    AnimationClipAction {
        animation: animation.into(),
        clip: clip.into(),
        speed: 1.0,
        blend_time: 0.0,
        looping,
        actions: Vec::new(),
    }
}

fn template() -> NpcActorTemplate {
    let durations = HashMap::from([("idle".to_string(), 2.0)]);
    let animations = Arc::new(AnimationSet::new(&[clip("idle", "idle", true)], durations));
    NpcActorTemplate {
        summon_element_cue: None,
        definition: CharacterDefinition {
            id: "test".into(),
            faction: 1,
            ethereal: false,
            looks_ethereal: false,
            can_have_status: true,
            resistances: [Resistance::IDENTITY; 11],
            max_hit_points: 100.0,
            health_bars: 1,
            event_conditions: Arc::new(Vec::new()),
            hit_tolerance: 0.0,
            knockdown_tolerance: 0.0,
            max_panic: 0.75,
            fearless: false,
            uncharmable: false,
            nonslippery: false,
            length: 1.0,
            radius: 0.4,
            mass: 80.0,
            speed: 4.0,
            turn_speed: 10.0,
            regeneration: 0,
            bleed_rate: 0.0,
            stun_time: 0.0,
            gib_count: 0,
            gibs: Vec::new(),
            blood: 5,
            attached_sounds: Vec::new(),
            zap_modifier: 1.0,
            break_free_strength: 1.0,
            undying: false,
            undie_time: 0.0,
            undie_hit_points: 0.0,
            model_tints: Vec::new(),
            score_value: 0,
            display_id: String::new(),
        },
        animations,
        move_abilities: 0,
        weights: AgentWeights {
            alert_radius: 10.0,
            friendly_avoidance: 1.0,
            enemy_avoidance: 1.0,
            sight_avoidance: 2.0,
            danger_avoidance: 1.0,
            group: GroupWeights {
                chase: 0.0,
                separation: 1.0,
                cohesion: 0.5,
                alignment: 1.0,
                wander: 0.5,
            },
        },
        abilities: Arc::new(Vec::new()),
        target_weights: Default::default(),
        rig: None,
        weapons: Vec::new(),
        held_effects: Vec::new(),
    }
}

fn length(v: Vector) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}
fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn frame<'a>(
    body: &'a mut CharacterBody,
    triggers: &'a mut Vec<(i32, Option<u16>)>,
    nav: Option<&'a NavMesh>,
    avoidance: &'a dyn Fn(Vector) -> Vector,
) -> ActorFrame<'a> {
    ActorFrame {
        body,
        nav,
        move_abilities: 0,
        triggers,
        sender: 7,
        entity_position: &|_| None,
        locator: &|_| None,
        closest_area_point: &|_, _| None,
        avoidance,
        in_cutscene: false,
        play_time: 0.0,
    }
}

fn grounded_body() -> CharacterBody {
    let mut body = CharacterBody::new(0.4, 1.0, 80.0, 4.0);
    body.enable();
    body
}

/// One AIManager-style step: the agent runs with the accumulated time, then
/// accumulates this frame's delta.
fn step(npc: &mut NpcCharacter, body: &mut CharacterBody, dt: f32) {
    let mut triggers = Vec::new();
    let zero = |_: Vector| [0.0; 3];
    npc.agent_update(frame(body, &mut triggers, None, &zero), 1, &[], None);
    npc.time_since_update += dt;
}

#[test]
fn wander_clock_walks_then_rests_with_native_timing() {
    let mut clock = WanderClock::default();
    clock.enter(0.5);
    assert_eq!(clock.pause, WANDER_PAUSE_BASE + 0.5 * WANDER_PAUSE_RANDOM);
    assert!(!clock.paused && clock.timer == 0.0);
    // Walking at the wander speed until the pause elapses.
    let mut walked = 0.0;
    let dt = 0.1;
    loop {
        let speed = clock.tick(dt, 0.5, || 0.2);
        if clock.paused {
            assert_eq!(speed, 0.0);
            break;
        }
        assert_eq!(speed, 0.5);
        walked += dt;
        assert!(walked < 16.0, "never rested");
    }
    assert!((14.9..=15.1).contains(&walked), "walked {walked}");
    // The rest lasts 2 + 3·random seconds, then walking resumes.
    assert!((clock.timer - (WANDER_REST_BASE + 0.2 * WANDER_REST_RANDOM)).abs() < 1e-5);
    let mut rested = 0.0;
    while clock.paused {
        assert_eq!(clock.tick(dt, 0.5, || 0.9), 0.0);
        rested += dt;
    }
    assert!((2.5..=2.7).contains(&rested), "rested {rested}");
    assert_eq!(clock.tick(dt, 0.5, || 0.9), 0.5);
}

#[test]
fn wander_steer_drifts_the_heading_and_reverses_at_the_area_edge() {
    assert!((wrap_angle(std::f32::consts::PI * 3.0) - std::f32::consts::PI).abs() < 1e-5);
    assert!((wrap_angle(-std::f32::consts::PI * 1.5) - std::f32::consts::PI / 2.0).abs() < 1e-5);
    // (random − 0.5) · 60 · dt: a full 1.0 draw over 1/60 s turns +0.5 rad.
    assert!((drift_angle(0.0, 1.0, 60.0, 1.0 / 60.0) - 0.5).abs() < 1e-5);
    let inputs = WanderInputs {
        position: [0.0, 0.0, 0.0],
        direction: [0.0, 0.0, -1.0],
        radius: 0.4,
        dt: 1.0 / 60.0,
    };
    // No area, the navmesh returns the point itself: Direction · 4 plus the
    // wander unit vector, normalized.
    let (angle, movement) = wander_steer(0.0, inputs, 0.5, |p| p, |_| None);
    assert_eq!(angle, 0.0);
    let w = wander_vector(0.0);
    let expected = [w[0], 0.0, -4.0 + w[2]];
    let l = length(expected);
    for i in 0..3 {
        assert!((movement[i] - expected[i] / l).abs() < 1e-5, "{movement:?}");
    }
    assert!((length(movement) - 1.0).abs() < 1e-5);
    // Inside the target area (closest point within the radius): the angle
    // flips by π and the heading uses Direction · 1.5.
    let (angle, movement) = wander_steer(0.0, inputs, 0.5, |p| p, |_| Some([0.1, 0.0, 0.0]));
    assert!((angle.abs() - std::f32::consts::PI).abs() < 1e-5, "{angle}");
    let w = wander_vector(angle);
    let expected = [w[0], 0.0, -1.5 + w[2]];
    let l = length(expected);
    for i in 0..3 {
        assert!((movement[i] - expected[i] / l).abs() < 1e-5, "{movement:?}");
    }
    // Far outside the area: the heading is the closest area point.
    let (_, movement) = wander_steer(0.0, inputs, 0.5, |p| p, |_| Some([6.0, 2.0, 8.0]));
    assert!((movement[0] - 0.6).abs() < 1e-5 && (movement[2] - 0.8).abs() < 1e-5);
    assert_eq!(movement[1], 0.0);
}

#[test]
fn wander_order_walks_rests_and_resumes_through_the_agent() {
    let t = template();
    let mut body = grounded_body();
    let mut npc = NpcCharacter::new(7, 0, &t);
    npc.ai.wander_speed = 0.5;
    npc.set_order(ORDER_WANDER, 0, 0, 0, None);
    let dt = 1.0 / 60.0;
    // Idle pushes AIStateWander and ends with Movement = 0.
    step(&mut npc, &mut body, dt);
    assert_eq!(npc.ai.state(), AiState::Wander);
    assert_eq!(body.movement(), [0.0; 3]);
    let pause = npc.ai.wander.pause;
    assert!((10.0..20.0).contains(&pause), "pause {pause}");
    let mut headings = Vec::new();
    let mut walking_frames = 0;
    let mut rest_started = None;
    let mut rest_frames = 0;
    let mut resumed = None;
    let mut was_paused = false;
    for f in 0..(25.0 * 60.0) as usize {
        step(&mut npc, &mut body, dt);
        assert_eq!(npc.ai.state(), AiState::Wander);
        let m = body.movement();
        assert_eq!(m[1], 0.0);
        let paused = npc.ai.wander.paused;
        if paused {
            if rest_started.is_none() {
                rest_started = Some(f);
            }
            if resumed.is_none() {
                assert_eq!(m, [0.0; 3], "resting frame {f}");
                rest_frames += 1;
            }
        } else if was_paused {
            // The update that ends the rest still moves at 0 (num stays 0
            // after WanderPaused clears); walking resumes next update.
            assert_eq!(m, [0.0; 3], "resume frame {f}");
            if resumed.is_none() {
                resumed = Some(f);
            }
        } else {
            assert!((length(m) - 0.5).abs() < 1e-4, "frame {f}: {m:?}");
            walking_frames += 1;
            headings.push([m[0] / 0.5, m[2] / 0.5]);
        }
        was_paused = paused;
    }
    // The walk lasted WanderPause seconds (within one agent update).
    let rest_at = rest_started.expect("rested") as f32 * dt;
    assert!(
        (rest_at - pause).abs() <= 2.0 * dt,
        "rest at {rest_at}, pause {pause}"
    );
    let rest = rest_frames as f32 * dt;
    assert!((2.0..5.0 + 2.0 * dt).contains(&rest), "rest {rest}");
    assert!(resumed.is_some() && walking_frames > 60);
    // The heading drifts: not every walking frame points the same way.
    let first = headings[0];
    assert!(headings
        .iter()
        .any(|h| (h[0] - first[0]).abs() > 0.05 || (h[1] - first[1]).abs() > 0.05));
    // A different order pops the state; the next update stands idle.
    npc.set_order(ORDER_ATTACK, 0, 0, 0, None);
    assert_eq!(npc.ai.state(), AiState::Idle);
    step(&mut npc, &mut body, dt);
    assert_eq!(npc.ai.state(), AiState::Idle);
    assert_eq!(body.movement(), [0.0; 3]);
}

#[test]
fn wander_order_pops_while_airborne() {
    let t = template();
    let mut body = CharacterBody::new(0.4, 1.0, 80.0, 4.0);
    let mut npc = NpcCharacter::new(7, 0, &t);
    npc.set_order(ORDER_WANDER, 0, 0, 0, None);
    step(&mut npc, &mut body, 1.0 / 60.0);
    assert_eq!(npc.ai.state(), AiState::Wander);
    // Not touching the ground: AIStateWander pops at once; idle re-pushes it.
    body.sync_external_motion(body.position, body.orientation, [0.0; 3], false);
    assert!(!body.is_touching_ground());
    step(&mut npc, &mut body, 1.0 / 60.0);
    assert_eq!(npc.ai.state(), AiState::Idle);
    assert_eq!(body.movement(), [0.0; 3]);
}

#[test]
fn panic_order_runs_while_panicking_and_ends_when_the_panic_drops() {
    let t = template();
    let mut body = grounded_body();
    let mut npc = NpcCharacter::new(7, 0, &t);
    // GiveOrder Panic: SetOrder then Character.OrderPanic (mPanic = 10).
    npc.set_order(ORDER_PANIC, 0, 0, 0, None);
    assert!(!npc.is_panicing());
    npc.order_panic();
    assert!(npc.is_panicing());
    assert!(is_panicing(10.0, 0.75) && !is_panicing(0.75, 0.75));
    let dt = 1.0 / 60.0;
    step(&mut npc, &mut body, dt);
    assert_eq!(npc.ai.state(), AiState::Panic);
    assert_eq!(body.movement(), [0.0; 3]);
    let entry = npc.ai.wander_angle;
    assert!(entry > -std::f32::consts::PI && entry <= std::f32::consts::PI);
    let direction = body.direction();
    let mut last = entry;
    for f in 0..120 {
        step(&mut npc, &mut body, dt);
        assert_eq!(npc.ai.state(), AiState::Panic);
        let m = body.movement();
        // Direction · 1.25 plus a unit wander vector: forward-biased, unit
        // length, flat.
        assert!((length(m) - 1.0).abs() < 1e-4, "frame {f}: {m:?}");
        assert!(dot(m, direction) > 0.0, "frame {f}: {m:?}");
        assert_eq!(m[1], 0.0);
        // The angle drifts by at most 0.5 rad per 1/60 s.
        let d = (npc.ai.wander_angle - last).abs();
        assert!(d <= 0.5 + 1e-4 || d >= std::f32::consts::PI * 2.0 - 0.5 - 1e-4);
        last = npc.ai.wander_angle;
    }
    // The status update clears mPanic (burning light stopped): AIStatePanic
    // pops with Movement = 0. The order is still Panic, so idle re-pushes
    // it and the pair alternate without moving.
    npc.character.status_frame.panic = 0.0;
    assert!(!npc.is_panicing());
    step(&mut npc, &mut body, dt);
    assert_eq!(npc.ai.state(), AiState::Idle);
    assert_eq!(body.movement(), [0.0; 3]);
    for _ in 0..6 {
        step(&mut npc, &mut body, dt);
        assert_eq!(npc.ai.state(), AiState::Panic);
        assert_eq!(body.movement(), [0.0; 3]);
        step(&mut npc, &mut body, dt);
        assert_eq!(npc.ai.state(), AiState::Idle);
        assert_eq!(body.movement(), [0.0; 3]);
    }
}

#[test]
fn panic_order_waits_for_the_status_when_the_template_resists_it() {
    // MaxPanic 1.0 (goblin warrior): OrderPanic's 10 still exceeds it, but a
    // burning magnitude below MaxPanic yields no panic.
    assert!(is_panicing(10.0, 1.0));
    assert!(!is_panicing(0.0, 1.0));
    assert!(!is_panicing(1.0, 1.0));
    assert!(is_panicing(1.0 + 1e-3, 1.0));
}

#[test]
fn stumbling_keeps_the_panic_state_and_runs_backwards_until_released() {
    let t = template();
    let mut body = grounded_body();
    let mut npc = NpcCharacter::new(7, 0, &t);
    npc.set_order(ORDER_PANIC, 0, 0, 0, None);
    // A Ride grip (Grip action with GripType.Ride) makes the victim stumble;
    // PanicState.OnEnter sets RunBackward for a stumbling character.
    npc.character.grip.gripper = Some(9);
    npc.character.grip.grip_type = GRIP_RIDE;
    body.run_backwards = true;
    assert!(npc.is_stumbling() && !npc.is_panicing());
    let dt = 1.0 / 60.0;
    step(&mut npc, &mut body, dt);
    assert_eq!(npc.ai.state(), AiState::Panic);
    let direction = body.direction();
    for _ in 0..30 {
        step(&mut npc, &mut body, dt);
        assert_eq!(npc.ai.state(), AiState::Panic);
        let m = body.movement();
        assert!((length(m) - 1.0).abs() < 1e-4);
        assert!(dot(m, direction) < 0.0, "runs backwards: {m:?}");
    }
    // The gripper lets go: IsStumbling is false and the state pops.
    npc.character.grip.gripper = None;
    npc.character.grip.grip_type = 0;
    body.run_backwards = false;
    assert!(!npc.is_stumbling());
    step(&mut npc, &mut body, dt);
    assert_eq!(npc.ai.state(), AiState::Idle);
    assert_eq!(body.movement(), [0.0; 3]);
    // A Hold grip is not a stumble.
    npc.character.grip.gripper = Some(9);
    npc.character.grip.grip_type = 2;
    assert!(!npc.is_stumbling());
}

#[test]
fn panic_state_movement_blends_by_max_panic_and_stumbles_backwards() {
    let direction = [0.0, 0.0, -1.0];
    // Panic without an existing movement: the wander walk itself.
    let walk = panic_state_movement(0.0, direction, [0.0; 3], 0.75);
    let w = wander_vector(0.0);
    let e = [w[0], 0.0, -4.0 + w[2]];
    let l = length(e);
    for i in 0..3 {
        assert!((walk[i] - e[i] / l).abs() < 1e-5);
    }
    // With an agent movement: normalize(walk · p + movement · (1 − p)).
    let blended = panic_state_movement(0.0, direction, [1.0, 0.0, 0.0], 0.75);
    let e = [walk[0] * 0.75 + 0.25, 0.0, walk[2] * 0.75];
    let l = length(e);
    for i in 0..3 {
        assert!((blended[i] - e[i] / l).abs() < 1e-5, "{blended:?}");
    }
    // Stumbling runs against the facing direction at 0.6 speed.
    let stumble = stumble_state_movement(0.0, direction);
    assert!(dot(stumble, direction) < 0.0 && (length(stumble) - 1.0).abs() < 1e-5);
    assert_eq!(PANIC_SPEED, 0.8);
    assert_eq!(STUMBLE_SPEED, 0.6);
    // The agent-level panic steer runs backwards only while RunBackward.
    let (_, forward) = panic_steer(0.0, direction, false, 0.5, 0.0);
    let (_, backward) = panic_steer(0.0, direction, true, 0.5, 0.0);
    assert!(dot(forward, direction) > 0.0 && dot(backward, direction) < 0.0);
}

#[test]
fn flocking_separation_keeps_two_npcs_apart_and_cohesion_draws_them_in() {
    let weights = GroupWeights {
        chase: 0.0,
        separation: 1.0,
        cohesion: 0.0,
        alignment: 0.0,
        wander: 0.0,
    };
    let me = [0.0, 0.0, 0.0];
    let other = [0.0, 0.0, -1.0];
    let (_, out) = follow_group(
        me,
        [0.0, 0.0, -1.0],
        [1.0, 0.0, 0.0],
        &[other],
        &weights,
        0.0,
        1.0 / 60.0,
        |n| n - 1,
        0.5,
    );
    // delta · (separation / −|delta|): a unit push away from the neighbour.
    assert!(
        (out[2] - 1.0).abs() < 1e-5 && out[0].abs() < 1e-5,
        "{out:?}"
    );
    // Closer neighbours push harder; the sum is capped at unit length.
    let (_, near) = follow_group(
        me,
        [0.0, 0.0, -1.0],
        [1.0, 0.0, 0.0],
        &[[0.0, 0.0, -0.25]],
        &weights,
        0.0,
        1.0 / 60.0,
        |n| n - 1,
        0.5,
    );
    assert!((length(near) - 1.0).abs() < 1e-5 && near[2] > 0.0);
    // Cohesion alone moves toward the group, alignment alone copies the
    // leader's facing.
    let cohesion = GroupWeights {
        separation: 0.0,
        cohesion: 0.5,
        ..weights
    };
    let (_, toward) = follow_group(
        me,
        [0.0, 0.0, -1.0],
        [1.0, 0.0, 0.0],
        &[other],
        &cohesion,
        0.0,
        1.0 / 60.0,
        |n| n - 1,
        0.5,
    );
    assert!((toward[2] + 0.5).abs() < 1e-5, "{toward:?}");
    let alignment = GroupWeights {
        separation: 0.0,
        alignment: 0.25,
        ..weights
    };
    let (_, aligned) = follow_group(
        me,
        [0.0, 0.0, -1.0],
        [1.0, 0.0, 0.0],
        &[],
        &alignment,
        0.0,
        1.0 / 60.0,
        |_| 0,
        0.5,
    );
    assert!((aligned[0] - 0.25).abs() < 1e-5 && aligned[2].abs() < 1e-5);
    // Up to eight distinct members are sampled: a pick stream that repeats
    // still terminates with each member once.
    let group: Vec<Vector> = (0..10).map(|i| [i as f32 + 1.0, 0.0, 0.0]).collect();
    let mut draws = 0;
    let (_, many) = follow_group(
        me,
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0],
        &group,
        &weights,
        0.0,
        1.0 / 60.0,
        |n| {
            draws += 1;
            (draws / 2) % n
        },
        0.5,
    );
    assert!(draws >= 8 && many[0] < 0.0);
}

#[test]
fn danger_sources_follow_the_get_danger_overrides() {
    assert_eq!(DangerSource::Character { spell_queue: 3 }.danger(), 3.0);
    assert_eq!(
        DangerSource::Avatar {
            charging: true,
            spell_queue: 0
        }
        .danger(),
        10.0
    );
    assert_eq!(
        DangerSource::Avatar {
            charging: false,
            spell_queue: 2
        }
        .danger(),
        2.0
    );
    assert_eq!(DangerSource::SpellMine { detonating: false }.danger(), 1.0);
    assert_eq!(
        DangerSource::SpellMine { detonating: true }.danger(),
        10000.0
    );
    assert_eq!(
        DangerSource::Missile {
            danger: MISSILE_DEFAULT_DANGER
        }
        .danger(),
        2.0
    );
    assert_eq!(DangerSource::Tornado { alpha: 0.5 }.danger(), 5.0);
    assert_eq!(
        DangerSource::Barrier {
            damage_magnitude: 7.0
        }
        .danger(),
        7.0
    );
    assert_eq!(
        DangerSource::Grease {
            burning: false,
            temperature: 3.0
        }
        .danger(),
        0.5
    );
    assert_eq!(
        DangerSource::Grease {
            burning: true,
            temperature: 3.0
        }
        .danger(),
        30.0
    );
    assert_eq!(DangerSource::Vortex { mass: 2.0 }.danger(), 40.0);
    assert_eq!(DangerSource::MagickDeath.danger(), 1000.0);
    assert_eq!(DangerSource::Harmless.danger(), 0.0);
}

/// A flat 40 m square navmesh around the origin (two triangles).
fn flat_navmesh() -> NavMesh {
    let vertices = vec![
        [-20.0, 0.0, -20.0],
        [20.0, 0.0, -20.0],
        [20.0, 0.0, 20.0],
        [-20.0, 0.0, 20.0],
    ];
    let triangles = vec![
        NavTriangle {
            vertices: [0, 1, 2],
            neighbours: [u16::MAX, u16::MAX, 1],
            costs: [28.28, 28.28, 28.28],
            properties: 0,
        },
        NavTriangle {
            vertices: [0, 2, 3],
            neighbours: [0, u16::MAX, u16::MAX],
            costs: [28.28, 28.28, 28.28],
            properties: 0,
        },
    ];
    NavMesh::new(vertices, triangles).unwrap()
}

#[test]
fn danger_avoidance_moves_an_idle_npc_out_of_the_danger_radius() {
    let t = template();
    let weights = t.weights;
    let nav = flat_navmesh();
    let mut body = grounded_body();
    body.position = [0.0, 0.0, 0.0];
    let mut npc = NpcCharacter::new(7, 0, &t);
    // Idle order with no events: nothing but the avoidance prelude acts.
    npc.set_order(1, 0, 0, 0, None);
    // An armed spell mine 3 m north: danger 1 is too weak (|avoidance| =
    // 0.75 < 20), a detonating one (10000) is not.
    let mine = |danger: f32| Neighbour {
        handle: 2,
        position: [0.0, 0.0, -3.0],
        direction: [0.0, 0.0, 1.0],
        radius: 0.5,
        faction: None,
        danger,
        invisible: false,
    };
    let weak = avoidance([0.0; 3], 1, &weights, &[mine(1.0)]);
    assert!((weak[2] - 0.75).abs() < 1e-5, "{weak:?}");
    let strong = avoidance([0.0; 3], 1, &weights, &[mine(10000.0)]);
    assert!((strong[2] - 7500.0).abs() < 1e-2, "{strong:?}");
    let dt = 1.0 / 60.0;
    let mut distance = 3.0;
    for f in 0..(6.0 * 60.0) as usize {
        let position = body.position;
        let d = [
            mine(0.0).position[0] - position[0],
            mine(0.0).position[1] - position[1],
            mine(0.0).position[2] - position[2],
        ];
        distance = length(d);
        // Agent.GetAvoidance sees entities within 12 m (+ their radius).
        let near: Vec<Neighbour> = if distance <= 12.5 {
            vec![mine(DangerSource::SpellMine { detonating: true }.danger())]
        } else {
            Vec::new()
        };
        let avoid = |p: Vector| avoidance(p, 1, &weights, &near);
        let mut triggers = Vec::new();
        npc.agent_update(
            frame(&mut body, &mut triggers, Some(&nav), &avoid),
            1,
            &[],
            None,
        );
        npc.time_since_update += dt;
        // Integrate the body's movement at the template speed.
        let m = body.movement();
        assert_eq!(m[1], 0.0);
        body.position = [
            body.position[0] + m[0] * 4.0 * dt,
            0.0,
            body.position[2] + m[2] * 4.0 * dt,
        ];
        if f == 1 {
            // The first update pushed AIStateMove toward position + avoidance;
            // the second moves away from the mine.
            assert!(m[2] > 0.5, "frame 1 movement {m:?}");
        }
    }
    assert!(
        distance > 12.5,
        "still within the danger radius: {distance}"
    );
    assert_eq!(npc.ai.state(), AiState::Idle);
    assert_eq!(body.movement(), [0.0; 3]);
}

/// The test template with the PanicState clips (move_panic 139 and
/// move_stumble 140, as 337 / 62 of the 338 retail templates define them).
fn panic_template() -> NpcActorTemplate {
    let mut t = template();
    let durations = HashMap::from([
        ("idle".to_string(), 2.0),
        ("panic".to_string(), 1.0),
        ("stumble".to_string(), 1.0),
    ]);
    t.animations = Arc::new(AnimationSet::new(
        &[
            clip("idle", "idle", true),
            clip("move_panic", "panic", true),
            clip("move_stumble", "stumble", true),
        ],
        durations,
    ));
    t
}

#[test]
fn ordered_panic_enters_panic_state_at_the_panic_speed_and_exits_when_the_panic_drops() {
    let t = panic_template();
    let mut body = grounded_body();
    let mut npc = NpcCharacter::new(7, 0, &t);
    let dt = 1.0 / 60.0;
    npc.update_motion(&mut body, false, dt);
    assert_eq!(npc.motion.state, CharacterState::Idle);
    // Character.OrderPanic: ChangeState(PanicState) and mPanic = 10. The
    // state switch lands at the next motion update (INFERRED timing).
    npc.order_panic();
    assert!(npc.is_panicing());
    npc.update_motion(&mut body, false, dt);
    assert_eq!(npc.motion.state, CharacterState::Panic);
    // PanicState.OnEnter: move_panic (blend 0.2), no RunBackward.
    assert_eq!(npc.motion.controller.current_animation, MOVE_PANIC);
    assert!(!body.run_backwards);
    let direction = body.direction();
    let mut last = npc.motion.wander_angle;
    for f in 0..60 {
        npc.update_motion(&mut body, false, dt);
        assert_eq!(npc.motion.state, CharacterState::Panic, "frame {f}");
        // PanicState.Update panic branch: SpeedMultiplier *= 0.8 and the
        // Direction · 4 + wander walk (the previous Movement blends in by
        // MaxPanic, so the result stays unit and forward-biased).
        assert!(
            (body.speed_multiplier() - PANIC_SPEED).abs() < 1e-6,
            "frame {f}: {}",
            body.speed_multiplier()
        );
        let m = body.movement();
        assert!((length(m) - 1.0).abs() < 1e-4, "frame {f}: {m:?}");
        assert!(dot(m, direction) > 0.0, "frame {f}: {m:?}");
        // Character.WanderAngle drifts by at most 0.5 rad per 1/60 s.
        let d = (npc.motion.wander_angle - last).abs();
        assert!(d <= 0.5 + 1e-4 || d >= std::f32::consts::PI * 2.0 - 0.5 - 1e-4);
        last = npc.motion.wander_angle;
    }
    // UpdateStatusEffects recomputed mPanic (0): neither panicking, feared
    // nor stumbling, so PanicState returns IdleState at full speed.
    npc.character.status_frame.panic = 0.0;
    npc.update_motion(&mut body, false, dt);
    assert_eq!(npc.motion.state, CharacterState::Idle);
    npc.update_motion(&mut body, false, dt);
    assert_eq!(npc.motion.state, CharacterState::Idle);
    assert!((body.speed_multiplier() - 1.0).abs() < 1e-6);
    // OrderPanic while already panicking: ChangeState ignores the current
    // state (no re-entry), mPanic is simply 10 again.
    npc.order_panic();
    npc.update_motion(&mut body, false, dt);
    assert_eq!(npc.motion.state, CharacterState::Panic);
    let before = npc.motion.controller.current_animation;
    npc.order_panic();
    npc.update_motion(&mut body, false, dt);
    assert_eq!(npc.motion.state, CharacterState::Panic);
    assert_eq!(npc.motion.controller.current_animation, before);
}

#[test]
fn stumbling_enters_panic_state_backwards_at_the_stumble_speed_until_released() {
    let t = panic_template();
    let mut body = grounded_body();
    let mut npc = NpcCharacter::new(7, 0, &t);
    let dt = 1.0 / 60.0;
    npc.update_motion(&mut body, false, dt);
    assert_eq!(npc.motion.state, CharacterState::Idle);
    // A Ride grip: IsStumbling without any panic value.
    npc.character.grip.gripper = Some(9);
    npc.character.grip.grip_type = GRIP_RIDE;
    assert!(npc.is_stumbling() && !npc.is_panicing());
    npc.update_motion(&mut body, false, dt);
    assert_eq!(npc.motion.state, CharacterState::Panic);
    // PanicState.OnEnter while stumbling: RunBackward and move_stumble.
    assert!(body.run_backwards);
    assert_eq!(npc.motion.controller.current_animation, MOVE_STUMBLE);
    let direction = body.direction();
    for f in 0..30 {
        npc.update_motion(&mut body, false, dt);
        assert_eq!(npc.motion.state, CharacterState::Panic, "frame {f}");
        // Stumble branch: SpeedMultiplier *= 0.6, Movement =
        // normalize(walk − Direction · 4): backwards.
        assert!((body.speed_multiplier() - STUMBLE_SPEED).abs() < 1e-6);
        let m = body.movement();
        assert!((length(m) - 1.0).abs() < 1e-4, "frame {f}: {m:?}");
        assert!(dot(m, direction) < 0.0, "frame {f}: {m:?}");
    }
    // The gripper releases: IdleState, and OnExit clears RunBackward.
    npc.character.grip.gripper = None;
    npc.character.grip.grip_type = 0;
    npc.update_motion(&mut body, false, dt);
    assert_eq!(npc.motion.state, CharacterState::Idle);
    assert!(!body.run_backwards);
}

#[test]
fn leader_election_prefers_the_last_candidate_in_front_with_the_same_target() {
    let me = [0.0, 0.0, 0.0];
    let dir = [0.0, 0.0, -1.0];
    let waypoint = [1.0, 0.0, -5.0];
    let candidate = |handle, position, target, waypoint| GroupCandidate {
        handle,
        position,
        radius: 0.4,
        in_move: true,
        is_leader: true,
        target,
        waypoint,
    };
    // AlertRadius 15 → 5 m (+ radius) query.
    let ahead = candidate(1, [0.0, 0.0, -3.0], Some(5), [0.0; 3]);
    let behind = candidate(2, [0.0, 0.0, 3.0], Some(5), [0.0; 3]);
    let far = candidate(3, [0.0, 0.0, -9.0], Some(5), [0.0; 3]);
    let other_target = candidate(4, [1.0, 0.0, -2.0], Some(6), [0.0; 3]);
    let later = candidate(7, [-1.0, 0.0, -2.0], Some(5), [0.0; 3]);
    assert_eq!(
        elect_leader(me, dir, 15.0, Some(5), waypoint, &[ahead]),
        Some(1)
    );
    assert_eq!(
        elect_leader(me, dir, 15.0, Some(5), waypoint, &[behind]),
        None
    );
    assert_eq!(elect_leader(me, dir, 15.0, Some(5), waypoint, &[far]), None);
    assert_eq!(
        elect_leader(me, dir, 15.0, Some(5), waypoint, &[other_target]),
        None
    );
    // The loop keeps overwriting: the last qualifying candidate leads.
    assert_eq!(
        elect_leader(me, dir, 15.0, Some(5), waypoint, &[ahead, later, behind]),
        Some(7)
    );
    // Not in AIStateMove or already following: never a leader.
    let mut idle = ahead;
    idle.in_move = false;
    let mut follower = ahead;
    follower.is_leader = false;
    assert_eq!(
        elect_leader(me, dir, 15.0, Some(5), waypoint, &[idle]),
        None
    );
    assert_eq!(
        elect_leader(me, dir, 15.0, Some(5), waypoint, &[follower]),
        None
    );
    // Without a target the candidate's WayPoint must be within 1 m.
    let same_goal = candidate(8, [0.0, 0.0, -3.0], None, [1.5, 0.0, -5.0]);
    let other_goal = candidate(9, [0.0, 0.0, -3.0], None, [3.0, 0.0, -5.0]);
    assert_eq!(
        elect_leader(me, dir, 15.0, None, waypoint, &[same_goal]),
        Some(8)
    );
    assert_eq!(
        elect_leader(me, dir, 15.0, None, waypoint, &[other_goal]),
        None
    );
    // A target mismatch (None vs Some) never matches.
    assert_eq!(elect_leader(me, dir, 15.0, None, waypoint, &[ahead]), None);
    // OnExecute members: same target within the radius, no state check;
    // without a target the flattened waypoints must be within 1 m.
    let members = group_members(
        me,
        15.0,
        Some(5),
        waypoint,
        &[ahead, idle, far, other_target],
    );
    assert_eq!(members, vec![ahead.position, idle.position]);
    let raised = candidate(10, [0.0, 0.0, -3.0], None, [1.5, 7.0, -5.0]);
    let members = group_members(me, 15.0, None, waypoint, &[raised, other_goal]);
    assert_eq!(members, vec![raised.position]);
}

#[test]
fn a_follower_steers_by_follow_group_separation_and_pops_when_the_leader_stops() {
    let mut t = template();
    // Separation only, so the result is independent of the random draws.
    t.weights.group = GroupWeights {
        chase: 0.0,
        separation: 5.0,
        cohesion: 0.0,
        alignment: 0.0,
        wander: 0.0,
    };
    let dt = 1.0 / 60.0;
    let dir = grounded_body().direction();
    let scale = |v: Vector, s: f32| [v[0] * s, v[1] * s, v[2] * s];
    // Goal 2.5 m ahead (within the 3 m no-path limit of a navmesh-less leader).
    let waypoint = scale(dir, 2.5);
    let leader_position = scale(dir, 1.0);
    let walk_to = |npc: &mut NpcCharacter, body: &mut CharacterBody| {
        npc.set_order(
            1,
            0,
            0,
            0,
            Some(vec![AiEvent::Move(MoveEvent {
                waypoint,
                waypoint_id: 0,
                direction: [0.0; 3],
                fixed_direction: false,
                delay: -1.0,
                speed: 1.0,
                trigger: 0,
            })]),
        );
        for _ in 0..4 {
            if npc.ai.state() == AiState::Move {
                break;
            }
            step(npc, body, dt);
        }
        assert_eq!(npc.ai.state(), AiState::Move);
    };
    // Alone: a leader (no one to follow) walks the 0.9-scaled direction to
    // the waypoint.
    let mut body = grounded_body();
    let mut alone = NpcCharacter::new(7, 0, &t);
    walk_to(&mut alone, &mut body);
    assert!(alone.ai.is_leader());
    step(&mut alone, &mut body, dt);
    let m = body.movement();
    assert!((dot(m, dir) - 0.9).abs() < 1e-4, "{m:?}");

    // With a same-type warrior 1 m ahead already moving to the same
    // waypoint: AIStateMove.OnEnter elects it, no path is planned, and
    // OnExecute runs FollowGroup: separation alone pushes straight away
    // from the leader (|5 · unit| > 1, normalized).
    let mut body = grounded_body();
    let mut npc = NpcCharacter::new(8, 0, &t);
    let leader = GroupCandidate {
        handle: 1,
        position: leader_position,
        radius: 0.4,
        in_move: true,
        is_leader: true,
        target: None,
        waypoint,
    };
    let facts = GroupLeader {
        position: leader_position,
        direction: dir,
        in_move: true,
        goal: waypoint,
    };
    npc.group_candidates = vec![leader];
    npc.group_leader = Some((1, facts));
    walk_to(&mut npc, &mut body);
    assert_eq!(npc.ai.leader, Some(1));
    assert!(!npc.ai.is_leader());
    assert!(npc.ai.path.is_empty());
    step(&mut npc, &mut body, dt);
    assert_eq!(npc.ai.state(), AiState::Move);
    let m = body.movement();
    assert!((length(m) - 1.0).abs() < 1e-4, "{m:?}");
    assert!(
        (dot(m, dir) + 1.0).abs() < 1e-4,
        "separation away from the leader: {m:?}"
    );
    // Agent.LeaderAge grows every agent update; past 1 s the follower pops.
    let mut popped = false;
    for _ in 0..90 {
        step(&mut npc, &mut body, dt);
        if npc.ai.leader_age > LEADER_MAX_AGE + dt {
            popped = true;
            break;
        }
    }
    assert!(
        !popped,
        "LeaderAge was reset by a re-election after the pop"
    );
    // Each pop condition: back in AIStateMove (the idle Move event
    // re-pushes it and re-elects the leader), change the leader facts,
    // and the next update pops to AIStateIdle.
    let ensure_move = |npc: &mut NpcCharacter, body: &mut CharacterBody| {
        for _ in 0..3 {
            if npc.ai.state() == AiState::Move {
                break;
            }
            step(npc, body, dt);
        }
        assert_eq!(npc.ai.state(), AiState::Move);
        assert_eq!(npc.ai.leader, Some(1));
    };
    // The leader left AIStateMove.
    ensure_move(&mut npc, &mut body);
    npc.group_leader = Some((
        1,
        GroupLeader {
            in_move: false,
            ..facts
        },
    ));
    step(&mut npc, &mut body, dt);
    assert_eq!(npc.ai.state(), AiState::Idle);
    // The leader is within 0.5 m of its goal.
    npc.group_leader = Some((
        1,
        GroupLeader {
            position: waypoint,
            ..facts
        },
    ));
    ensure_move(&mut npc, &mut body);
    step(&mut npc, &mut body, dt);
    assert_eq!(npc.ai.state(), AiState::Idle);
    // The leader no longer exists.
    npc.group_leader = None;
    ensure_move(&mut npc, &mut body);
    step(&mut npc, &mut body, dt);
    assert_eq!(npc.ai.state(), AiState::Idle);
    // The leader is fine again: following resumes.
    npc.group_leader = Some((1, facts));
    ensure_move(&mut npc, &mut body);
    step(&mut npc, &mut body, dt);
    assert_eq!(npc.ai.state(), AiState::Move);
    assert!((dot(body.movement(), dir) + 1.0).abs() < 1e-4);
}

#[test]
fn a_dangerous_missile_pushes_an_idle_npc_but_a_default_one_does_not() {
    let t = template();
    let weights = t.weights;
    let nav = flat_navmesh();
    let dt = 1.0 / 60.0;
    // MissileEntity.GetDanger = mDanger: 2 by default (Initialize), 30 for
    // the HolyGrenade, 10 for the GreaseLump, 0 for a Potion. AIStateIdle
    // only pushes AIStateMove past |avoidance| 20.
    let missile = |danger: f32| Neighbour {
        handle: u16::MAX,
        position: [0.0, 0.0, -2.0],
        direction: [0.0, 0.0, 1.0],
        radius: 0.15,
        faction: None,
        danger,
        invisible: false,
    };
    let default = avoidance(
        [0.0; 3],
        1,
        &weights,
        &[missile(
            DangerSource::Missile {
                danger: MISSILE_DEFAULT_DANGER,
            }
            .danger(),
        )],
    );
    assert!((length(default) - 2.0 * (1.0 - 2.0 / 12.0)).abs() < 1e-4);
    let grenade = avoidance([0.0; 3], 1, &weights, &[missile(30.0)]);
    assert!((length(grenade) - 30.0 * (1.0 - 2.0 / 12.0)).abs() < 1e-3);
    for (danger, expect_move) in [(MISSILE_DEFAULT_DANGER, false), (30.0, true)] {
        let mut body = grounded_body();
        body.position = [0.0; 3];
        let mut npc = NpcCharacter::new(7, 0, &t);
        npc.set_order(1, 0, 0, 0, None);
        let near = vec![missile(danger)];
        let avoid = |p: Vector| avoidance(p, 1, &weights, &near);
        let mut triggers = Vec::new();
        npc.agent_update(
            frame(&mut body, &mut triggers, Some(&nav), &avoid),
            1,
            &[],
            None,
        );
        npc.time_since_update += dt;
        assert_eq!(
            npc.ai.state() == AiState::Move,
            expect_move,
            "danger {danger}"
        );
        npc.agent_update(
            frame(&mut body, &mut triggers, Some(&nav), &avoid),
            1,
            &[],
            None,
        );
        let m = body.movement();
        if expect_move {
            assert!(m[2] > 0.5, "away from the missile: {m:?}");
        } else {
            assert_eq!(m, [0.0; 3]);
        }
    }
}

#[test]
fn retail_templates_define_the_panic_clips_and_group_weights() {
    use magicka_runtime::play_content::load_character;
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let has = |t: &magicka_content::character::CharacterTemplate, name: &str| {
        t.animation_sets
            .iter()
            .flatten()
            .any(|c| c.animation.eq_ignore_ascii_case(name))
    };
    // Survey (output/evidence/npc-orders/panic-clips.txt): 337 of 338
    // templates define move_panic (MountainKing does not), 62 move_stumble.
    for (name, panic, stumble) in [
        ("Goblin_warrior", true, false),
        ("Wizard", true, true),
        ("Soldier_swordsman", true, true),
        ("Cultist", true, true),
        ("MountainKing", false, false),
    ] {
        let t = load_character(&root, name).unwrap();
        assert_eq!(has(&t, "move_panic"), panic, "{name}");
        assert_eq!(has(&t, "move_stumble"), stumble, "{name}");
    }
    // Goblin warriors flock (Group* weights); wizards and cultists do not
    // (all zero) but avoid danger (2.5) where goblins ignore it (0).
    let goblin = load_character(&root, "Goblin_warrior").unwrap();
    assert_eq!(
        (
            goblin.group_separation,
            goblin.group_cohesion,
            goblin.group_alignment,
            goblin.group_wander,
            goblin.danger_avoidance,
            goblin.max_panic,
        ),
        (2.0, 0.6, 1.0, 1.0, 0.0, 1.0)
    );
    let wizard = load_character(&root, "Wizard").unwrap();
    assert_eq!(
        (
            wizard.group_separation,
            wizard.group_alignment,
            wizard.danger_avoidance,
            wizard.max_panic
        ),
        (0.0, 0.0, 2.5, 0.75)
    );
}
