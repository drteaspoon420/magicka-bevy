//! Authored map contracts, independent of retail gameplay balance or rendering.
use magicka_runtime::{authored_arena, navigation, play_session::PlaySession};
use std::path::PathBuf;
fn scenarios() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scenarios")
}
#[test]
fn all_lanes_camps_and_objective_approaches_are_connected() {
    let bytes = std::fs::read(scenarios().join("confluence.arena")).unwrap();
    let level = authored_arena::decode(&bytes).unwrap();
    let nav = navigation::nav_mesh(&level.nav_mesh).unwrap();
    let locate = |name: &str| {
        let l = level.locators.iter().find(|l| l.name == name).unwrap();
        [l.transform[12], 0., l.transform[14]]
    };
    // Both teams have four reserved spawn slots, even though transport currently caps at four total.
    for team in ["blue", "red"] {
        for slot in 0..4 {
            let start = locate(&format!("{team}_spawn{slot}"));
            let (ok, _) = nav.find_path(start, [0., 0., 0.], 0, &|_| false).unwrap();
            assert!(ok, "{team} spawn {slot} isolated");
        }
    }
    for lane in ["top", "mid", "bot"] {
        for i in 0..2 {
            let a = locate(&format!("lane_{lane}_{i}"));
            let b = locate(&format!("lane_{lane}_{}", i + 1));
            let (ok, path) = nav.find_path(a, b, 0, &|_| false).unwrap();
            assert!(ok && !path.is_empty(), "{lane} segment {i} disconnected");
        }
        for i in 0..4 {
            locate(&format!("tower_{lane}_{i}"));
        }
    }
    for i in 0..6 {
        let (ok, _) = nav
            .find_path([0.; 3], locate(&format!("camp_{i}")), 0, &|_| false)
            .unwrap();
        assert!(ok, "camp {i} isolated");
    }
    for i in 0..2 {
        let mut point = locate(&format!("objective_{i}"));
        point[0] += 5.; // Shrine itself has a collider; its approach must remain reachable.
        let (ok, _) = nav.find_path([0.; 3], point, 0, &|_| false).unwrap();
        assert!(ok, "objective {i} isolated");
    }
    let visual = authored_arena::presentation(&bytes).unwrap();
    assert!(visual.scenery.len() > 100);
    assert!(visual.surfaces.len() > 25);
    // Every tree has an authoritative solid trunk, with the same authored placement.
    for tree in visual
        .scenery
        .iter()
        .filter(|s| s.asset.ends_with("ash.glb") || s.asset.ends_with("spruce.glb"))
    {
        assert!(level.collision_layers[1].vertices.chunks(8).any(|b| {
            ((b[0][0] + b[6][0]) * 0.5 - tree.position[0]).abs() < 0.001
                && ((b[0][2] + b[6][2]) * 0.5 - tree.position[2]).abs() < 0.001
        }));
    }
}
#[test]
fn invalid_visuals_fail_at_runtime_load() {
    let text = std::fs::read_to_string(scenarios().join("confluence.arena")).unwrap();
    for broken in [
        text.replace("gltf/Custom", "../Custom"),
        text.replace("yaw=\"0\"", "yaw=\"NaN\""),
        text.replace("tile=\"6\"", "tile=\"0\""),
    ] {
        assert!(authored_arena::decode(broken.as_bytes()).is_err());
    }
}
#[test]
fn map_loads_in_live_session_without_scenario_scripts() {
    let content =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !content.is_dir() {
        return;
    }
    let map = scenarios().join("confluence.lvl").canonicalize().unwrap();
    let mut s = PlaySession::load(&content, map.to_str().unwrap(), "Wizard").unwrap();
    for _ in 0..120 {
        s.step(1. / 60.);
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    assert_eq!(s.scene_name(), "confluence");
    assert!(s.body().position[1] > 0. && s.body().position[1] < 3.);
    assert!(s.checkpoint.is_none());
    assert!(s.world.npcs.is_empty());
}

#[test]
fn fords_wade_deep_channel_drowns_and_ice_supports_crossing() {
    use magicka_runtime::level_collision::{SceneCollision, StaticLevelCollision};
    use magicka_runtime::water_collision::WaterCollision;
    use magicka_sim::character_physics::{BodyEvent, CharacterBody, PhysicsWorld};
    let bytes = std::fs::read(scenarios().join("confluence.arena")).unwrap();
    let level = authored_arena::decode(&bytes).unwrap();
    assert_eq!(level.liquids.len(), 1);
    let water = &level.liquids[0];
    assert!(water.freezable && water.has_collision && !water.auto_freeze);
    for (point, frozen, should_drown) in [
        ([0., 1.05, 0.], false, false),
        ([-40., 1.05, -40.], false, true),
        ([-40., 1.05, -40.], true, false),
    ] {
        let mut liquid =
            WaterCollision::from_geometry(&water.collision_geometry().unwrap(), true, false)
                .unwrap();
        if frozen {
            liquid.freeze.freeze_all(1.);
        }
        let scene = SceneCollision {
            static_skin: StaticLevelCollision::from_layers(&level.collision_layers).unwrap(),
            water: vec![liquid],
            force_fields: vec![],
            animated_parts: vec![],
        };
        let mut physics = PhysicsWorld::new(-50.);
        let mut body = CharacterBody::new(0.75, 0.5, 70., 5.);
        body.position = point;
        physics.bodies.push(body);
        let mut drowned = false;
        let mut wet = false;
        for _ in 0..90 {
            let events = physics.integrate(1. / 60., &scene, &|_, _| None);
            drowned |= events
                .iter()
                .any(|(_, e)| matches!(e, BodyEvent::Drown { .. }));
            wet |= physics.bodies[0].is_in_water();
        }
        assert_eq!(drowned, should_drown, "point={point:?} frozen={frozen}");
        assert_eq!(physics.bodies[0].is_on_ice(), frozen);
        if !frozen {
            assert!(wet);
        }
    }
    let nav = navigation::nav_mesh(&level.nav_mesh).unwrap();
    let from = [-43., 0., -17.];
    let to = [-17., 0., -43.];
    let (ok, dry) = nav
        .find_path(from, to, magicka_sim::navmesh::movement::WATER, &|_| false)
        .unwrap();
    assert!(ok);
    let (ok, ice) = nav
        .find_path(from, to, magicka_sim::navmesh::movement::WATER, &|_| true)
        .unwrap();
    assert!(
        ok && ice.len() < dry.len(),
        "ice should unlock the short river route"
    );
}

#[test]
fn highground_has_real_collision_and_camera_confinement() {
    use magicka_content::level::TriMesh;
    use magicka_runtime::{camera::MagickCamera, level_collision::StaticLevelCollision};
    use magicka_sim::spray_pulse::Segment;
    let bytes = std::fs::read(scenarios().join("confluence.arena")).unwrap();
    let level = authored_arena::decode(&bytes).unwrap();
    let mut layers = vec![TriMesh::default(); 10];
    layers[0] = level.collision_layers[0].clone();
    let floor = StaticLevelCollision::from_layers(&layers).unwrap();
    let top = floor
        .segment_intersect(
            Segment {
                origin: [-30., 10., 12.],
                delta: [0., -20., 0.],
            },
            false,
        )
        .unwrap();
    assert!((top.hit.position[1] - 4.).abs() < 0.001);
    let low = Segment {
        origin: [-54., 1., 12.],
        delta: [48., 0., 0.],
    };
    assert!(
        floor.segment_intersect(low, false).is_some(),
        "raised terrain blocks a horizontal low ray"
    );
    assert!(floor
        .segment_intersect(
            Segment {
                origin: [-54., 6., 12.],
                ..low
            },
            false
        )
        .is_none());
    let mesh = level.camera_mesh.unwrap();
    assert!(!mesh.triangles.is_empty());
    let mut camera = MagickCamera::new(Some(
        mesh.triangles
            .iter()
            .map(|t| t.map(|i| mesh.vertices[i as usize]))
            .collect(),
    ));
    camera.set_position([500., 0., 0.], true);
    assert!(camera.ground_position()[0] <= 74.01);
    camera.set_position([-30., 4., 12.], true);
    assert!((camera.ground_position()[1] - 4.).abs() < 0.001);
    for name in ["camera_blue_jungle", "camera_red_jungle", "camera_river"] {
        assert!(level.trigger_areas.iter().any(|a| a.name == name));
    }
}
