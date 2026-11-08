use magicka_runtime::{
    entity_command::EntityCommandHost,
    play_session::PlaySession,
    scene_script::ActionDefinition,
    spawn_elemental::{SpawnElementalAction, SpawnElementalHost, SpawnElementalState},
};

#[derive(Default)]
struct Host {
    client: bool,
    calls: Vec<Option<f32>>,
    dead: Vec<u16>,
}
impl SpawnElementalHost for Host {
    type Error = ();
    fn is_client(&self) -> bool {
        self.client
    }
    fn spawn_elemental(
        &mut self,
        _: &SpawnElementalAction,
        proximity: Option<f32>,
    ) -> Result<u16, ()> {
        self.calls.push(proximity);
        Ok(self.calls.len() as u16)
    }
    fn elemental_dead(&self, handle: u16) -> bool {
        self.dead.contains(&handle)
    }
}

#[test]
fn replay_replaces_only_survivors_and_leaves_cached_proximity_unassigned() {
    let action =
        SpawnElementalAction::read([("nr", "3"), ("proximity", "10"), ("snapToNavMesh", "true")])
            .unwrap();
    let mut state = SpawnElementalState::default();
    let mut host = Host::default();
    action.execute(&mut state, &mut host).unwrap();
    assert_eq!(state.handles, [1, 2, 3]);
    assert_eq!(host.calls, [Some(10.); 3]);
    host.dead.push(2);
    SpawnElementalAction::track(&mut state, &host);
    action.quick_execute(&mut state, &mut host).unwrap();
    assert_eq!(state.handles, [4, 5]);
    assert_eq!(&host.calls[3..], [None, None]);
    host.client = true;
    action.execute(&mut state, &mut host).unwrap();
    action.quick_execute(&mut state, &mut host).unwrap();
    assert_eq!(host.calls.len(), 5);
    assert_eq!(state.handles, [4, 5]);
}

fn session() -> Option<PlaySession> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir()
        .then(|| PlaySession::load(&root, "Niflheim.lvl", "Wizard").unwrap())
}

#[test]
fn niflheim_cutscene_spawns_45_eggs_and_nearby_eggs_hatch_into_enemies() {
    let Some(mut s) = session() else { return };
    s.step(1. / 60.);
    assert!(s.run_trigger("Cutscene over", true).unwrap());
    s.step(1. / 60.);
    let cache = s
        .world
        .elemental_eggs
        .as_ref()
        .expect("script creates egg cache");
    let eggs: Vec<_> = cache.slots.iter().filter(|e| e.actor.registered).collect();
    assert_eq!(eggs.len(), 45);
    assert!(eggs
        .iter()
        .all(|e| e.actor.state.proximity == 10. && e.actor.state.summoner.is_none()));
    let handle = eggs[0].handle;
    let position = eggs[0].actor.position;
    s.disable_scripts();
    let player = s.avatar().body;
    s.world.physics.bodies[player].position = position;
    for _ in 0..90 {
        s.step(1. / 60.);
    }
    assert!(
        !s.world
            .elemental_eggs
            .as_ref()
            .unwrap()
            .slots
            .iter()
            .find(|e| e.handle == handle)
            .unwrap()
            .actor
            .registered
    );
    let enemies: Vec<_> = s
        .world
        .npcs
        .values()
        .filter(|n| n.definition.id.starts_with("elemental_"))
        .collect();
    assert!(!enemies.is_empty(), "proximity hatches campaign enemies");
    assert!(
        enemies.iter().all(|n| n.summon.is_none()),
        "script eggs are ownerless"
    );
}

#[test]
fn killed_script_egg_is_pruned_before_checkpoint_replay() {
    let Some(mut s) = session() else { return };
    s.step(1. / 60.);
    assert!(s.run_trigger("Cutscene over", true).unwrap());
    s.step(1. / 60.);
    let handle = s
        .world
        .elemental_eggs
        .as_ref()
        .unwrap()
        .slots
        .iter()
        .find(|e| e.actor.registered)
        .unwrap()
        .handle;
    EntityCommandHost::kill(&mut s.world, handle).unwrap();
    s.step(1. / 60.);
    let actions: Vec<_> = s
        .runtime
        .layout()
        .actions
        .iter()
        .enumerate()
        .filter_map(|(i, a)| matches!(a, ActionDefinition::SpawnElemental(_)).then_some(i))
        .collect();
    assert_eq!(actions.len(), 3);
    s.step(1. / 60.); // SpawnElemental.Update prunes the removed handle.
    s.disable_scripts();
    for e in &mut s.world.elemental_eggs.as_mut().unwrap().slots {
        e.actor.deinitialize(true);
    }
    for index in actions {
        s.runtime.quick_execute(index, &mut s.world).unwrap();
    }
    assert_eq!(
        s.world
            .elemental_eggs
            .as_ref()
            .unwrap()
            .slots
            .iter()
            .filter(|e| e.actor.registered)
            .count(),
        44
    );
}

#[test]
fn script_identity_moves_the_live_egg_and_overkill_removes_without_hatching() {
    let Some(mut s) = session() else { return };
    s.disable_scripts();
    let mut action = s
        .runtime
        .layout()
        .actions
        .iter()
        .find_map(|a| {
            if let ActionDefinition::SpawnElemental(a) = a {
                Some(a.clone())
            } else {
                None
            }
        })
        .unwrap();
    action.nr = 1;
    action.unique_id = magicka_runtime::scene_loading::custom_hash("test_egg");
    let mut state = SpawnElementalState::default();
    action.execute(&mut state, &mut s.world).unwrap();
    let entity = EntityCommandHost::entity_by_id(&mut s.world, action.unique_id)
        .unwrap()
        .unwrap();
    assert_eq!(entity.handle, state.handles[0]);
    assert!(entity.character.is_none());
    let mut orientation = magicka_content::skinned::IDENTITY;
    orientation[12..15].copy_from_slice(&[1., 2., 3.]);
    EntityCommandHost::move_body(&mut s.world, entity.handle, [1., 2., 3.], orientation).unwrap();
    let egg = s
        .world
        .elemental_eggs
        .as_ref()
        .unwrap()
        .slots
        .iter()
        .find(|e| e.handle == entity.handle)
        .unwrap();
    assert_eq!(egg.actor.position, [1., 2., 3.]);
    let npcs_before = s.world.npcs.len();
    s.world.overkill(entity.handle);
    s.step(1. / 60.);
    assert_eq!(s.world.npcs.len(), npcs_before);
    assert!(
        EntityCommandHost::entity_by_id(&mut s.world, action.unique_id)
            .unwrap()
            .is_none()
    );
}
