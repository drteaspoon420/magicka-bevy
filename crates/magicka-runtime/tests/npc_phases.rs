//! Multi-phase NPC templates on retail content: a Death condition's
//! RemoveEvent terminates the character with iKillItems: true and a
//! SpawnEvent brings in the next phase (research/npc-phases.md).
use magicka_runtime::{
    avatar::PlayerInput,
    play_content::load_start_scene,
    play_session::PlaySession,
    scene_script::{ActionDefinition, SceneScriptLayout},
    spawn_action::SpawnState,
};
use std::path::{Path, PathBuf};

const LEVEL: &str = "Havindr.lvl";

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir().then_some(root)
}

fn frame(s: &mut PlaySession) {
    s.step(1.0 / 60.0);
    s.handle_input(PlayerInput::default(), 16.0 / 9.0);
    s.events.clear();
}

fn spawn_in_front(s: &mut PlaySession, root: &Path, template: &str, distance: f32) -> u16 {
    let area = load_start_scene(root, LEVEL).unwrap().start.area.unwrap();
    let xml = format!(
        r#"<Scene><trigger id="t"><then><spawn type="{template}" area="{area}1" order="idle"/></then></trigger></Scene>"#
    );
    let ActionDefinition::Spawn(action) = SceneScriptLayout::read(xml.as_bytes(), false)
        .unwrap()
        .actions
        .remove(0)
    else {
        panic!("not a spawn action");
    };
    let mut state = SpawnState::default();
    action.initialize(&mut state, &mut s.world).unwrap();
    action.execute(&mut state, &mut s.world).unwrap();
    let handle = state.spawned[0].handle.unwrap();
    let (position, forward) = (s.body().position, s.body().direction());
    let npc = &s.world.npcs[&handle];
    let body = &mut s.world.physics.bodies[npc.body];
    let target = [
        position[0] + forward[0] * distance,
        body.position[1].max(position[1]),
        position[2] + forward[2] * distance,
    ];
    let orientation = body.orientation;
    body.move_to(target, orientation);
    handle
}

fn cleavers(s: &PlaySession) -> usize {
    s.world
        .item_pickups
        .values()
        .filter_map(|p| p.item.as_ref())
        .filter(|i| {
            i.definition
                .name
                .eq_ignore_ascii_case("weapon_goblin_captain_cleaver")
        })
        .count()
}

/// Goblin_warrior_captain's Death/OverKill conditions run Remove, then
/// Spawn goblin_warrior_captain_noshield. At 0 HP DeadState.OnEnter copies
/// the pickable cleaver to the ground before Die runs Remove, and the
/// no-shield phase drops its own cleaver at its death: two cleavers, the
/// original's double drop (confirmed by the user 2026-11-02).
#[test]
fn the_captain_drops_a_cleaver_at_each_phase_death() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut s = PlaySession::load(&root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s);
    }
    let captain = spawn_in_front(&mut s, &root, "Goblin_warrior_captain", 6.0);
    for _ in 0..30 {
        frame(&mut s);
    }
    assert!(!s.world.npcs[&captain].weapons.is_empty(), "armed captain");
    let before = cleavers(&s);
    s.world
        .npcs
        .get_mut(&captain)
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 0.0;
    for _ in 0..600 {
        frame(&mut s);
    }
    assert_eq!(
        cleavers(&s),
        before + 1,
        "the replaced first phase drops a cleaver copy"
    );
    let phase2 = s
        .world
        .entities
        .iter()
        .filter(|e| e.added && !e.removed && !e.dead)
        .find(|e| {
            e.template_name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case("goblin_warrior_captain_noshield"))
        })
        .expect("the no-shield phase spawns")
        .handle;
    assert!(
        !s.world.npcs[&phase2].weapons.is_empty(),
        "the second phase holds its own weapons"
    );
    // The final phase drops its own cleaver as well.
    s.world
        .npcs
        .get_mut(&phase2)
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 0.0;
    for _ in 0..600 {
        frame(&mut s);
    }
    assert_eq!(
        cleavers(&s),
        before + 2,
        "the final phase drops a second cleaver"
    );
}
