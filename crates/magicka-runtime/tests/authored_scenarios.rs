use magicka_runtime::{
    authored_arena, level_collision::StaticLevelCollision, play_content, play_session::PlaySession,
};
use magicka_sim::spray_pulse::Segment;
use std::path::PathBuf;
fn scenarios() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scenarios")
}
fn content() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
}
#[test]
fn authored_geometry_drives_collision_navigation_and_locator_edits() {
    let path = scenarios().join("lab.arena");
    let bytes = std::fs::read_to_string(path).unwrap();
    let a = authored_arena::decode(bytes.as_bytes()).unwrap();
    assert_eq!(a.collision_layers.len(), 10);
    assert!(a.model.is_none());
    let c = StaticLevelCollision::from_layers(&a.collision_layers).unwrap();
    let ray = Segment {
        origin: [-5., 2., -5.],
        delta: [10., 0., 0.],
    };
    assert!(c.segment_intersect(ray, false).is_some());
    let nav = magicka_runtime::navigation::nav_mesh(&a.nav_mesh).unwrap();
    let (success, path) = nav
        .find_path([-5., 0., -5.], [5., 0., -5.], 0, &|_| false)
        .unwrap();
    assert!(success);
    assert!(path.len() > 2);
    let changed = bytes
        .replace("0,2,-5", "8,2,-5")
        .replace("-7,1,5", "-10,1,5");
    let b = authored_arena::decode(changed.as_bytes()).unwrap();
    let c = StaticLevelCollision::from_layers(&b.collision_layers).unwrap();
    assert!(c.segment_intersect(ray, false).is_none());
    assert_ne!(a.nav_mesh, b.nav_mesh);
    assert_ne!(a.collision_layers[1], b.collision_layers[1]);
    assert_eq!(
        b.locators
            .iter()
            .find(|l| l.name == "start0")
            .unwrap()
            .transform[12],
        -10.
    );
    assert!(
        authored_arena::decode(bytes.replace("version=\"1\"", "version=\"2\"").as_bytes()).is_err()
    );
    assert!(authored_arena::decode(bytes.replace("cell=\"1\"", "cell=\"0\"").as_bytes()).is_err());
    assert!(authored_arena::decode(
        bytes
            .replace("size=\"2,4,10\"", "size=\"0,4,10\"")
            .as_bytes()
    )
    .is_err());
}
#[test]
fn custom_scenes_load_scripts_entities_transition_and_checkpoint() {
    let content = content();
    if !content.is_dir() {
        return;
    }
    let level = scenarios().join("feature-lab.lvl").canonicalize().unwrap();
    let mut s = PlaySession::load(&content, level.to_str().unwrap(), "Wizard").unwrap();
    for _ in 0..60 {
        s.step(1. / 60.);
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    assert_eq!(s.scene_name(), "lab");
    assert!(s.body().position[1] > 0.);
    assert!(s.body().position[1] < 3.);
    assert!(s
        .world
        .entities
        .iter()
        .any(|e| e.added && !e.removed && e.template_name.as_deref() == Some("bathball")));
    assert_eq!(s.world.book_pickups.len(), 1);
    assert_eq!(s.world.item_pickups.len(), 1);
    assert!(!s.world.npcs.is_empty());
    assert!(s.checkpoint.is_some());
    assert!(s.pickup_physics.is_some());
    let names = s.level_scene_names();
    assert_eq!(names, vec!["lab".to_string(), "corridor".to_string()]);
    // Exercise the authored portal's existing script host, then scene teardown.
    let index = s.avatar().body;
    s.world.physics.bodies[index].position = [-7., 1.5, 12.];
    for _ in 0..20 {
        s.step(1. / 60.);
    }
    assert_eq!(s.scene_name(), "corridor");
    assert!(s.world.book_pickups.is_empty());
    assert!(s.pickup_physics.is_none());
    assert!(s.checkpoint.is_some());
    assert!(s.restart_at_checkpoint().unwrap());
    assert_eq!(s.scene_name(), "corridor");
    let second = scenarios().join("corridor.lvl").canonicalize().unwrap();
    let other = play_content::load_start_scene(&content, second.to_str().unwrap()).unwrap();
    assert_eq!(other.scene.name, "corridor");
    let files = play_content::authored_level_files(&scenarios()).unwrap();
    assert!(files.iter().any(|p| p.ends_with("feature-lab.lvl")));
    assert!(files.iter().any(|p| p.ends_with("corridor.lvl")));
}

#[test]
fn source_edits_reach_new_live_sessions_without_retail_modification() {
    let content = content();
    if !content.is_dir() {
        return;
    }
    let root = std::env::temp_dir().join(format!("magicka-authored-edit-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    for name in [
        "feature-lab.lvl",
        "lab.xml",
        "lab.arena",
        "corridor.xml",
        "corridor.arena",
    ] {
        std::fs::copy(scenarios().join(name), root.join(name)).unwrap();
    }
    let level = root.join("feature-lab.lvl");
    let first = PlaySession::load(&content, level.to_str().unwrap(), "Wizard").unwrap();
    assert_eq!(first.body().position[0], -7.);
    let arena = root.join("lab.arena");
    let text = std::fs::read_to_string(&arena).unwrap();
    std::fs::write(
        &arena,
        text.replace("0,2,-5", "8,2,-5")
            .replace("-7,1,5", "-10,1,5"),
    )
    .unwrap();
    let second = PlaySession::load(&content, level.to_str().unwrap(), "Wizard").unwrap();
    assert_eq!(second.body().position[0], -10.);
    assert_ne!(
        first.world.level.collision_layers[1],
        second.world.level.collision_layers[1]
    );
    assert_ne!(first.world.level.nav_mesh, second.world.level.nav_mesh);
    std::fs::remove_dir_all(root).unwrap();
}
