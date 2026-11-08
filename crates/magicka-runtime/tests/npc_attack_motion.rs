use magicka_content::character::{AbilityKind, ActionKind};
use magicka_runtime::{
    animation_controller::animation_value, play_content::load_character, play_session::PlaySession,
};

#[test]
fn authored_melee_charge_moves_the_npc_body() {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    let mut paths: Vec<_> = std::fs::read_dir(root.join("Data/Characters"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xnb")))
        .collect();
    paths.sort();
    let mut charge: Option<(f32, String, String)> = None;
    for path in paths {
        let name = path.file_stem().unwrap().to_str().unwrap();
        let template = load_character(&root, name).unwrap();
        for ability in &template.abilities {
            if !matches!(ability.kind, AbilityKind::Melee { .. }) {
                continue;
            }
            for animation in &ability.animation_keys {
                for action in template
                    .animation_sets
                    .iter()
                    .flatten()
                    .filter(|a| a.animation.eq_ignore_ascii_case(animation))
                    .flat_map(|a| &a.actions)
                {
                    if let ActionKind::Move { velocity } = action.kind {
                        let horizontal = velocity[0] * velocity[0] + velocity[2] * velocity[2];
                        if horizontal > charge.as_ref().map_or(0.0, |c| c.0) {
                            charge = Some((horizontal, name.to_string(), animation.clone()));
                        }
                    }
                }
            }
        }
    }
    let (_, name, animation) = charge.expect("retail melee animation with authored movement");
    eprintln!("authored melee charge: {name}/{animation}");
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    let h = s.spawn_npc(&name, "idle", 5.0).unwrap();
    let body = s.world.npcs[&h].body;
    let initial = s.world.physics.bodies[body].position;
    s.world
        .npcs
        .get_mut(&h)
        .unwrap()
        .motion
        .attack(animation_value(&animation).unwrap(), false);
    let mut displacement: f32 = 0.0;
    for _ in 0..180 {
        s.step(1.0 / 60.0);
        let p = s.world.physics.bodies[body].position;
        displacement = displacement.max((p[0] - initial[0]).hypot(p[2] - initial[2]));
    }
    assert!(
        displacement > s.world.physics.bodies[body].radius,
        "{name}/{animation} did not charge: displacement={displacement}"
    );
}
