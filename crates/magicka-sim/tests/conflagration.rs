use magicka_sim::{
    conflagration::{owns_damage, Conflagration, Target, World, INSTANT_DAMAGE, STATUS_DAMAGE},
    damage::Damage,
    impact_motion::Vector,
    spray_pulse::Segment,
};

#[derive(Default)]
struct Scene {
    targets: Vec<Target>,
    blocked: bool,
    queries: Vec<(Vector, f32)>,
    probes: Vec<Segment>,
    damage: Vec<(u16, Damage, Option<u16>, f64, Vector)>,
    moves: Vec<(Vector, Vector)>,
}
impl World for Scene {
    fn targets(&mut self, center: Vector, radius: f32) -> Vec<Target> {
        self.queries.push((center, radius));
        self.targets.clone()
    }
    fn shielded(&mut self, segment: Segment) -> bool {
        self.probes.push(segment);
        self.blocked
    }
    fn damage(
        &mut self,
        handle: u16,
        damage: Damage,
        owner: Option<u16>,
        timestamp: f64,
        source: Vector,
    ) {
        self.damage.push((handle, damage, owner, timestamp, source));
    }
    fn move_effect(&mut self, position: Vector, direction: Vector) {
        self.moves.push((position, direction));
    }
}
fn target(handle: u16, position: Vector) -> Target {
    Target {
        handle,
        position,
        radius: 0.5,
        ethereal_character: false,
    }
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.00001, "{a} != {b}");
}

#[test]
fn caster_and_directed_overloads_keep_different_origins() {
    let mut sweep = Conflagration::from_caster(Some(3), [10.0, 2.0, 4.0], [0.0, 0.0, -1.0], 7.0);
    assert_eq!(sweep.start, [10.0, 2.0, 4.0]);
    assert_eq!(sweep.position, [10.0, 2.0, 1.0]);
    sweep.update(0.5, false, &mut Scene::default());
    assert_eq!(sweep.center(), [10.0, 2.0, -4.0]);
    assert_eq!(sweep.visual_axes()[2], [0.0, 0.0, 1.5]);
    near(sweep.distortion(), 0.84375);
    let directed = Conflagration::directed([10.0, 2.0, 4.0], [0.0, 0.0, -1.0], 7.0);
    assert_eq!(directed.position, directed.start);
    assert_eq!(directed.owner, None);
}

#[test]
fn rounded_box_corners_and_rotation_do_not_use_visual_stretch() {
    let mut sweep = Conflagration::directed([0.0; 3], [1.0, 0.0, 0.0], 0.0);
    // Facing +X: the wide local X axis lies along world Z.
    assert!(sweep.touches([1.5, 0.0, 0.0], 0.5));
    assert!(!sweep.touches([1.5001, 0.0, 0.0], 0.5));
    assert!(sweep.touches([1.3, 1.4, 0.0], 0.50001));
    assert!(!sweep.touches([1.4, 1.4, 0.0], 0.5));
    assert!(sweep.touches([0.0, 0.0, 6.0], 0.5));
    sweep.update(0.5, false, &mut Scene::default());
    assert!(sweep.touches([6.5, 0.0, 0.0], 0.5));
    assert!(!sweep.touches([6.6, 0.0, 0.0], 0.5));
}

#[test]
fn each_target_gets_two_ordered_damage_calls_once_with_fixed_source() {
    let mut sweep = Conflagration::from_caster(Some(1), [0.0; 3], [0.0, 0.0, -1.0], 9.25);
    let mut scene = Scene {
        targets: vec![target(1, [0.0, 0.0, -3.0]), target(2, [0.0, 0.0, -3.0])],
        ..Default::default()
    };
    sweep.update(0.1, true, &mut scene);
    sweep.update(0.1, true, &mut scene);
    assert_eq!(
        scene.damage,
        vec![
            (2, INSTANT_DAMAGE, Some(1), 9.25, [0.0, 0.0, -3.0]),
            (2, STATUS_DAMAGE, Some(1), 9.25, [0.0, 0.0, -3.0])
        ]
    );
    assert_eq!(scene.probes.len(), 1);
    assert_eq!(scene.probes[0].origin, [0.0; 3]);
    assert_eq!(scene.probes[0].delta, [0.0, 0.0, -3.0]);
    assert_eq!(scene.moves.len(), 2);
    assert_eq!(scene.queries[0].1, 10.0);
}

#[test]
fn shield_and_ethereal_rejections_can_be_retried() {
    let mut sweep = Conflagration::directed([0.0; 3], [0.0, 0.0, -1.0], 0.0);
    let mut scene = Scene {
        targets: vec![target(2, [0.0; 3])],
        blocked: true,
        ..Default::default()
    };
    sweep.update(0.01, true, &mut scene);
    assert!(scene.damage.is_empty());
    scene.blocked = false;
    scene.targets[0].ethereal_character = true;
    sweep.update(0.01, true, &mut scene);
    assert!(scene.damage.is_empty());
    scene.targets[0].ethereal_character = false;
    sweep.update(0.01, true, &mut scene);
    assert_eq!(scene.damage.len(), 2);
}

#[test]
fn last_update_still_hits_and_ownership_only_suppresses_damage() {
    let mut sweep = Conflagration::directed([0.0; 3], [0.0, 0.0, -1.0], 0.0);
    let mut scene = Scene {
        targets: vec![target(2, [0.0, 0.0, -20.0])],
        ..Default::default()
    };
    sweep.update(0.75, false, &mut scene);
    assert!(scene.queries.is_empty());
    assert_eq!(scene.moves.len(), 1);
    sweep.update(0.25, true, &mut scene);
    assert!(sweep.is_dead());
    assert_eq!(scene.damage.len(), 2);
    near(sweep.distortion(), 0.0);
    for client in [false, true] {
        assert!(owns_damage(client, true, false));
        assert!(!owns_damage(client, true, true));
        assert_eq!(owns_damage(client, false, false), !client);
    }
}
