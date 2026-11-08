use magicka_runtime::{
    entity_command::EntityCommandHost, give_order::GiveOrderHost, play_session::PlaySession,
    scene_loading::custom_hash,
};

#[test]
fn respawned_named_actor_supersedes_old_corpse_and_removal_does_not_restore_it() {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let id = custom_hash("#named_actor");
    let old = s.spawn_npc("goblin_warrior", "idle", 5.0).unwrap();
    s.world
        .entities
        .iter_mut()
        .find(|e| e.handle == old)
        .unwrap()
        .unique_id = id;
    s.world.kill(old).unwrap();
    let new = s.spawn_npc("goblin_warrior", "idle", 6.0).unwrap();
    s.world
        .entities
        .iter_mut()
        .find(|e| e.handle == new)
        .unwrap()
        .unique_id = id;
    assert_eq!(s.world.npc_by_id(id).unwrap(), Some(new));
    s.world.terminate(old).unwrap();
    assert_eq!(
        s.world.npc_by_id(id).unwrap(),
        Some(new),
        "removing old instance must preserve its successor's binding"
    );
    s.world.terminate(new).unwrap();
    assert_eq!(
        s.world.npc_by_id(id).unwrap(),
        None,
        "removal must not reveal the superseded actor"
    );
}
