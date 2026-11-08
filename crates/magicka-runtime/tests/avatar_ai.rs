//! Avatar event playback against a scripted character (no physics):
//! movement integrates `Movement × 5 m/s`, facing snaps to DesiredDirection.
use magicka_runtime::ai_event::{
    AiEvent, AnimationEvent, FaceEvent, LoopEvent, LoopType, MoveEvent,
};
use magicka_runtime::avatar_ai::{AiState, AvatarAi, AvatarAiHost};
use magicka_sim::navmesh::{movement, PathNode};

#[derive(Default)]
struct Mock {
    position: [f32; 3],
    direction: [f32; 3],
    movement: [f32; 3],
    triggers: Vec<i32>,
    animations: Vec<(u16, f32)>,
    current_animation: u16,
    finished: bool,
    paths: usize,
    target_checks: usize,
    target_available: bool,
    area_point: Option<[f32; 3]>,
}
impl AvatarAiHost for Mock {
    fn position(&self) -> [f32; 3] {
        self.position
    }
    fn direction(&self) -> [f32; 3] {
        self.direction
    }
    fn touching_ground(&self) -> bool {
        true
    }
    fn set_movement(&mut self, m: [f32; 3]) {
        self.movement = m;
    }
    fn set_desired_direction(&mut self, d: [f32; 3]) {
        self.direction = d;
    }
    fn find_path(&mut self, start: [f32; 3], end: [f32; 3]) -> Vec<PathNode> {
        self.paths += 1;
        [start, end]
            .map(|position| PathNode {
                position,
                properties: movement::DEFAULT,
            })
            .to_vec()
    }
    fn execute_trigger(&mut self, t: i32) {
        self.triggers.push(t);
    }
    fn go_to_animation(&mut self, a: u16, blend: f32) {
        self.animations.push((a, blend));
        self.current_animation = a;
        self.finished = false;
    }
    fn set_special_idle(&mut self, _: u16) {}
    fn current_animation(&self) -> u16 {
        self.current_animation
    }
    fn animation_finished(&self) -> bool {
        self.finished
    }
    fn entity_position(&self, _: i32) -> Option<[f32; 3]> {
        None
    }
    fn locator(&self, id: i32) -> Option<[f32; 16]> {
        (id == 77).then(|| {
            let mut m = [0.0; 16];
            m[0] = 1.0;
            m[5] = 1.0;
            m[10] = 1.0;
            m[15] = 1.0;
            m[12] = -3.0;
            m
        })
    }
    fn kill(&mut self, _: bool) {}
    fn jump(&mut self, _: [f32; 3], _: f32) {}
    fn closest_area_point(&self, _: i32, _: [f32; 3]) -> Option<[f32; 3]> {
        self.area_point
    }
    fn choose_and_add_target(&mut self) -> bool {
        self.target_checks += 1;
        self.target_available
    }
}

#[test]
fn defend_acquires_targets_after_returning_to_its_area_or_locator() {
    use magicka_runtime::avatar_ai::ORDER_DEFEND;
    for (target_area, area_point, destination) in [
        (77, None, [-3., 0., 0.]),
        (12, Some([2., 40., 0.]), [2., 40., 0.]),
    ] {
        let mut ai = AvatarAi::default();
        ai.agent = true;
        ai.order = ORDER_DEFEND;
        ai.target_area = target_area;
        let mut host = Mock {
            target_available: true,
            area_point,
            ..Default::default()
        };
        ai.agent_update(&mut host, DT);
        assert_eq!(ai.state(), AiState::Move);
        assert_eq!(ai.waypoint, destination);
        assert_eq!(host.target_checks, 0);
        ai.pop_to_idle(&mut host);
        // Only horizontal distance matters; a different elevation must not
        // prevent a ledge defender from acquiring a target.
        host.position = [destination[0], -50., destination[2]];
        ai.agent_update(&mut host, DT);
        assert_eq!(ai.state(), AiState::Attack);
        assert_eq!(host.target_checks, 1);
    }
}

#[test]
fn defend_without_a_target_area_attacks_but_idle_does_not() {
    use magicka_runtime::avatar_ai::ORDER_DEFEND;
    let mut ai = AvatarAi::default();
    ai.agent = true;
    ai.order = 1;
    let mut host = Mock {
        target_available: true,
        ..Default::default()
    };
    ai.agent_update(&mut host, DT);
    assert_eq!(ai.state(), AiState::Idle);
    assert_eq!(host.target_checks, 0);
    ai.order = ORDER_DEFEND;
    ai.agent_update(&mut host, DT);
    assert_eq!(ai.state(), AiState::Attack);
    assert_eq!(host.paths, 0);
}

const DT: f32 = 1.0 / 60.0;

fn step(ai: &mut AvatarAi, mock: &mut Mock) {
    ai.update(mock, DT);
    let l = (mock.movement[0].powi(2) + mock.movement[2].powi(2)).sqrt();
    let scale = if l > 1.0 { 5.0 / l } else { 5.0 };
    for i in [0, 2] {
        mock.position[i] += mock.movement[i] * scale * DT;
    }
}

fn mv(x: f32, z: f32, trigger: i32, delay: f32) -> AiEvent {
    AiEvent::Move(MoveEvent {
        waypoint: [x, 0.0, z],
        speed: 1.0,
        trigger,
        delay,
        ..Default::default()
    })
}

#[test]
fn chained_moves_walk_the_route_then_release_control() {
    let mut ai = AvatarAi::default();
    let mut mock = Mock::default();
    assert!(!ai.set_events(Some(vec![mv(4.0, 0.0, 11, 0.0), mv(4.0, 3.0, 12, 0.0)])));
    assert!(ai.controls_avatar());
    // First frame: delay 0 is not < 0, so it only counts down; nothing moves.
    step(&mut ai, &mut mock);
    assert_eq!(mock.movement, [0.0; 3]);
    let mut frames = 0;
    while ai.controls_avatar() && frames < 600 {
        let before = ai.state();
        step(&mut ai, &mut mock);
        frames += 1;
        // The push frame returns before setting Movement (stale value kept).
        if before == AiState::Move && ai.state() == AiState::Move {
            // AIStateMove: 0.9 × speed along the path.
            let l = (mock.movement[0].powi(2) + mock.movement[2].powi(2)).sqrt();
            assert!((l - 0.9).abs() < 1e-4, "{l}");
        }
    }
    assert!(!ai.controls_avatar(), "route never finished");
    // Original quirk: a zero-delay Move fires its trigger on completion and
    // again next frame (delay 0 is not < 0, so the countdown branch runs and
    // re-executes the previous event's trigger). The last event escapes this
    // because Avatar.Update stops once all events are consumed.
    assert_eq!(mock.triggers, [11, 11, 12]);
    assert!((mock.position[0] - 4.0).abs() < 0.5 && (mock.position[2] - 3.0).abs() < 0.5);
    // Idle zeroes Movement once nothing chains.
    step(&mut ai, &mut mock);
    assert_eq!(mock.movement, [0.0; 3]);
    // Re-planned at most once per second of walking per leg.
    assert!(mock.paths >= 2, "{}", mock.paths);
}

#[test]
fn delayed_move_trigger_fires_when_delay_expires() {
    let mut ai = AvatarAi::default();
    let mut mock = Mock::default();
    ai.set_events(Some(vec![mv(0.2, 0.0, 21, 0.5)]));
    for _ in 0..3 {
        step(&mut ai, &mut mock);
    }
    // Already at the waypoint: event completes, trigger waits for the delay.
    assert!(!ai.controls_avatar());
    assert!(mock.triggers.is_empty());
    for _ in 0..40 {
        step(&mut ai, &mut mock);
    }
    assert_eq!(mock.triggers, [21]);
}

#[test]
fn animation_event_waits_for_the_clip_then_fires_its_trigger() {
    let mut ai = AvatarAi::default();
    let mut mock = Mock::default();
    ai.set_events(Some(vec![
        AiEvent::Animation(AnimationEvent {
            animation: 200,
            blend_time: 0.3,
            trigger: 31,
            ..Default::default()
        }),
        mv(2.0, 0.0, 0, 0.0),
    ]));
    for _ in 0..10 {
        step(&mut ai, &mut mock);
    }
    assert_eq!(mock.animations, [(200, 0.3)]);
    assert!(ai.event_delay.is_nan());
    assert!(mock.triggers.is_empty());
    assert_eq!(mock.movement, [0.0; 3]);
    mock.finished = true;
    step(&mut ai, &mut mock);
    assert_eq!(mock.triggers, [31]);
    assert_eq!(ai.event_delay, -1.0);
    for _ in 0..120 {
        step(&mut ai, &mut mock);
    }
    assert!(!ai.controls_avatar());
    assert!((mock.position[0] - 2.0).abs() < 0.5);
}

#[test]
fn face_event_turns_toward_a_locator_before_advancing() {
    let mut ai = AvatarAi::default();
    let mut mock = Mock {
        direction: [0.0, 0.0, -1.0],
        ..Default::default()
    };
    ai.set_events(Some(vec![AiEvent::Face(FaceEvent {
        target_id: 77,
        ..Default::default()
    })]));
    step(&mut ai, &mut mock);
    step(&mut ai, &mut mock);
    assert_eq!(mock.direction, [-1.0, 0.0, 0.0]);
    assert!(!ai.controls_avatar());
}

#[test]
fn reverse_loop_mirrors_all_but_the_loop_event() {
    let mut ai = AvatarAi::default();
    let mut mock = Mock::default();
    let events = vec![
        mv(0.0, 0.0, 1, 0.0),
        mv(0.1, 0.0, 2, 0.0),
        mv(0.2, 0.0, 3, 0.0),
        mv(0.3, 0.0, 4, 0.0),
        AiEvent::Loop(LoopEvent {
            kind: LoopType::Reverse,
            ..Default::default()
        }),
    ];
    ai.set_events(Some(events));
    for _ in 0..40 {
        step(&mut ai, &mut mock);
        if ai.current_event == 0 && mock.triggers.len() >= 4 {
            break;
        }
    }
    let order: Vec<i32> = ai
        .events()
        .unwrap()
        .iter()
        .map(|e| match e {
            AiEvent::Move(m) => m.trigger,
            _ => 0,
        })
        .collect();
    // Original quirk: swaps only i < (n-2)/2 (integer division) with n-2-i,
    // so four moves + loop make one swap: [4,2,3,1,loop].
    assert_eq!(order, [4, 2, 3, 1, 0]);
}

#[test]
fn empty_events_request_idle_state() {
    let mut ai = AvatarAi::default();
    assert!(ai.set_events(None));
    assert!(ai.set_events(Some(vec![])));
    assert!(!ai.controls_avatar());
}
