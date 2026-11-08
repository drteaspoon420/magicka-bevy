use magicka_content::skinned::{self, Bone, Skeleton, IDENTITY};
use magicka_runtime::{
    animation_effects::bone_orientation, avatar::PlayerInput, play_session::PlaySession,
    scene_loading::custom_hash,
};
use std::path::Path;

#[test]
fn effect_bone_cancels_inverse_bind_and_reverses_forward() {
    let mut inverse_bind = IDENTITY;
    inverse_bind[12..15].copy_from_slice(&[-2.0, -3.0, -4.0]);
    let skeleton = Skeleton {
        bones: vec![Bone {
            name: "Root".into(),
            parent: None,
            translation: [2.0, 3.0, 4.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
            inverse_bind,
        }],
    };
    let mut absolute = IDENTITY;
    absolute[12..15].copy_from_slice(&[5.0, 6.0, 7.0]);
    let skin = skinned::mul(&inverse_bind, &absolute);
    let m = bone_orientation(&skeleton, &[skin], "ROOT").unwrap();
    assert_eq!(&m[12..15], &[5.0, 6.0, 7.0]);
    assert_eq!([-m[8], -m[9], -m[10]], [0.0, 0.0, 1.0]);
    assert!(bone_orientation(&skeleton, &[skin], "missing").is_none());
    assert!(bone_orientation(&skeleton, &[], "root").is_none());
}

#[test]
fn real_charge_actions_start_build_follow_loop_and_stop_on_release() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return;
    }
    let dt = 1.0 / 60.0;
    // Empty Force/Area push and Earth/Ice Force projectiles share native clips.
    for (area, element) in [
        (false, None),
        (true, None),
        (false, Some(6)),
        (false, Some(1)),
    ] {
        let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        s.disable_scripts();
        for _ in 0..60 {
            s.step(dt);
            s.handle_input(PlayerInput::default(), 16.0 / 9.0);
        }
        if let Some(element) = element {
            let mut input = PlayerInput::default();
            if element == 1 {
                // Cold + Water forms Ice (R + Q).
                input.elements[0] = true;
                s.handle_input(input, 16.0 / 9.0);
                s.step(dt);
                s.handle_input(PlayerInput::default(), 16.0 / 9.0);
                input = PlayerInput::default();
                input.elements[3] = true;
            } else {
                input.elements[element] = true;
            }
            s.handle_input(input, 16.0 / 9.0);
            s.step(dt);
            s.handle_input(PlayerInput::default(), 16.0 / 9.0);
        }
        let prefix = if area {
            "cast_area_charge"
        } else {
            "cast_force_charge"
        };
        let build = custom_hash(&format!("{prefix}_build"));
        let maximum = custom_hash(&format!("{prefix}_max"));
        assert!(
            s.effects.library.effects.contains_key(&build),
            "missing definition"
        );

        let mut saw_build = false;
        let mut saw_max = false;
        for _ in 0..240 {
            s.handle_input(
                PlayerInput {
                    cast_down: true,
                    shift_down: area,
                    ..Default::default()
                },
                16.0 / 9.0,
            );
            s.step(dt);
            saw_build |= s.effects.active_effects_by_hash(build) == 1;
            saw_max |= s.effects.active_effects_by_hash(maximum) == 1;
            assert!(s.effects.active_effects_by_hash(build) <= 1);
            assert!(s.effects.active_effects_by_hash(maximum) <= 1);
        }
        assert!(
            saw_build,
            "missing build particles: area={area}, element={element:?}"
        );
        assert!(
            saw_max,
            "missing loop particles: area={area}, element={element:?}"
        );
        let expected = match element {
            Some(6) => magicka_sim::spells::Elements::EARTH,
            Some(1) => magicka_sim::spells::Elements::ICE,
            None => magicka_sim::spells::Elements::NONE,
            _ => unreachable!(),
        };
        assert_eq!(s.avatar().spell.element, expected);
        assert_eq!(s.effects.active_effects_by_hash(build), 0);
        assert_eq!(s.effects.active_effects_by_hash(maximum), 1);
        // Sprite 139 is specified by both original maximum-charge effects.
        // Require charging sparks around this player, not ambient map particles.
        let position = s.body().position;
        assert!(s.effects.particles.active().iter().any(|p| {
            p.sprite == 139
                && p.hsv_colorize == 50
                && p.position
                    .iter()
                    .zip(position)
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f32>()
                    < 9.0
        }));
        s.handle_input(PlayerInput::default(), 16.0 / 9.0);
        for _ in 0..60 {
            s.step(dt);
            s.handle_input(PlayerInput::default(), 16.0 / 9.0);
        }
        assert_eq!(s.effects.active_effects_by_hash(build), 0);
        assert_eq!(s.effects.active_effects_by_hash(maximum), 0);
    }
}
