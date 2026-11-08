//! Native interactions reach mod `OnInteract` callbacks, identified by the
//! same name hash `Levels.Hash` gives scripts.
use magicka_runtime::scene_loading::custom_hash;
use magicka_runtime::{mod_runtime::ModRuntime, play_session::PlaySession};
use std::path::PathBuf;

#[test]
fn interacting_with_the_weapon_rack_calls_on_interact() {
    let dir = std::env::temp_dir().join(format!("magicka-mod-interact-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let write = |p: &str, t: &str| {
        let p = dir.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    };
    write(
        "mods/watch/mod.kv",
        r#""Mod" { "Id" "watch" "Version" "1.0.0" "ApiVersion" "1" "ServerScripts" "1" }"#,
    );
    write(
        "mods/watch/scripts/vscripts/watch/init.lua",
        r#"
        function OnInteract(e)
            NetTables.Set("watch", "rack", e.interactable == Levels.Hash("POI_WeaponRack"))
            NetTables.Set("watch", "by_player", e.entity ~= nil)
        end
        "#,
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut s = PlaySession::load(
        &root.join("../../../../target/Magicka/Content"),
        "Wizard_Castle.lvl",
        "Wizard",
    )
    .unwrap();
    let mut mods = ModRuntime::load(None, &dir.join("mods"), Some(&["watch".into()]), 1).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    mods.attach(&mut s);
    s.change_scene_named("wc_s3", "start").unwrap();
    for _ in 0..10 {
        mods.step(&mut s, 1.0 / 60.0);
        s.step(1.0 / 60.0);
    }
    let id = custom_hash("poi_weaponrack");
    let (rack, _) = s
        .world
        .interactable_locator(id)
        .expect("weapon rack locator");
    s.world.flags.interactables_disabled.remove(&id);
    s.world.flags.cutscene = false;
    let body = s.avatar().body;
    let o = s.world.physics.bodies[body].orientation;
    let y = s.body().position[1];
    s.world.physics.bodies[body].move_to([rack[0], y, rack[2]], o);
    s.step(1.0 / 60.0);
    assert!(s.interact(), "the rack is in reach");
    mods.step(&mut s, 1.0 / 60.0);
    let t = &mods.scripts.net_tables()["watch"];
    assert_eq!(
        (t["rack"].clone(), t["by_player"].clone()),
        (true.into(), true.into())
    );
    assert!(s.interactions.is_empty(), "drained by the mod step");
}
