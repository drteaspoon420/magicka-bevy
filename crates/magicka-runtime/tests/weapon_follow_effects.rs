use magicka_runtime::{avatar::PlayerInput, play_session::PlaySession, scene_loading::custom_hash};
use std::path::Path;

const DT: f32 = 1.0 / 60.0;

#[test]
fn surt_swing_effects_stop_after_each_completed_attack() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    s.world.camera.lock_input = false;
    let mut transform = magicka_content::skinned::IDENTITY;
    transform[12..15].copy_from_slice(&s.body().position);
    let pickup = s
        .spawn_item_pickup("weapon_blade_of_surt", transform)
        .unwrap();
    s.pickup_item(pickup).unwrap();
    for _ in 0..180 {
        s.step(DT);
    }
    let weapon = &s
        .avatar()
        .items
        .iter()
        .find(|(slot, _)| *slot == 0)
        .unwrap()
        .1;
    let effects: Vec<_> = weapon
        .definition
        .melee_conditions
        .0
        .iter()
        .flat_map(|collection| collection.events.iter())
        .filter_map(|event| match event {
            magicka_content::common::Event::Effect {
                effect,
                follow: true,
                ..
            } => Some(custom_hash(&effect.to_ascii_lowercase())),
            _ => None,
        })
        .collect();
    assert!(
        !effects.is_empty(),
        "Surt must have authored followed swing effects"
    );
    for _ in 0..3 {
        let mut seen = false;
        for i in 0..240 {
            s.step(DT);
            s.handle_input(
                PlayerInput {
                    shift_down: i < 20,
                    move_down: i < 20,
                    ..Default::default()
                },
                16.0 / 9.0,
            );
            seen |= effects
                .iter()
                .any(|&hash| s.effects.active_effects_by_hash(hash) > 0);
        }
        assert!(seen, "attack must actually emit the authored swing effect");
        for &hash in &effects {
            assert_eq!(
                s.effects.active_effects_by_hash(hash),
                0,
                "followed emitter survived completed attack: {hash}"
            );
        }
    }
}
