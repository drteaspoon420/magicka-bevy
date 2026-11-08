use magicka_sim::{spawn_slime::*, spells::Elements as E};

#[test]
fn wizard_default_and_parent_batches_follow_native_switch() {
    for overkill in [false, true] {
        let wizard = batch("wizard", E::ARCANE, overkill, || {
            panic!("default has no element draw")
        })
        .unwrap();
        assert_eq!(
            wizard,
            [Slime {
                size: Size::Small,
                element: E::POISON
            }]
        );
        assert_eq!(wizard[0].template(), "dungeons_slimecube_poison_small");
        for parent in [
            "dungeons_slimecube_poison_medium",
            "dungeons_slimecube_medium",
            "dungeons_slimecube_poison",
            "dungeons_acolyte_a",
        ] {
            assert_eq!(
                batch(parent, E::NONE, overkill, || unreachable!()).unwrap(),
                vec![
                    Slime {
                        size: Size::Small,
                        element: E::POISON
                    };
                    3
                ]
            );
        }
        let arcane = batch(
            "dungeons_slimecube_arcane",
            E::NONE,
            overkill,
            || unreachable!(),
        )
        .unwrap();
        assert_eq!(
            arcane,
            vec![
                Slime {
                    size: Size::Small,
                    element: E::ARCANE
                };
                3
            ]
        );
        let acolyte = batch("dungeons_acolyte_c", E::NONE, overkill, || unreachable!()).unwrap();
        assert_eq!(
            acolyte.iter().map(|s| s.size).collect::<Vec<_>>(),
            [Size::Medium, Size::Medium, Size::Small, Size::Small]
        );
    }
    let normal = batch(
        "dungeons_slimecube_poison_large",
        E::NONE,
        false,
        || unreachable!(),
    )
    .unwrap();
    assert_eq!(
        normal,
        vec![
            Slime {
                size: Size::Medium,
                element: E::POISON
            };
            3
        ]
    );
    let overkill = batch(
        "dungeons_slimecube_poison_large",
        E::NONE,
        true,
        || unreachable!(),
    )
    .unwrap();
    assert_eq!(
        overkill,
        vec![
            Slime {
                size: Size::Small,
                element: E::POISON
            };
            8
        ]
    );
}

#[test]
fn element_is_rejection_sampled_once_for_the_whole_batch() {
    let mut indices = [0, 1, 10].into_iter();
    let mut calls = 0;
    let plan = batch("dungeons_acolyte_c", E::POISON, false, || {
        calls += 1;
        indices.next().unwrap()
    })
    .unwrap();
    assert_eq!(calls, 3);
    assert!(plan.iter().all(|s| s.element == E::POISON));
    assert!(batch("dungeons_acolyte_a", E(0x8000), false, || unreachable!()).is_none());
}

#[test]
fn template_mapping_preserves_unused_large_fallback() {
    for element in E::BY_INDEX {
        let arcane = [
            E::EARTH,
            E::WATER,
            E::COLD,
            E::FIRE,
            E::LIGHTNING,
            E::ARCANE,
        ]
        .contains(&element);
        let small = Slime {
            size: Size::Small,
            element,
        }
        .template();
        let medium = Slime {
            size: Size::Medium,
            element,
        }
        .template();
        assert_eq!(
            small,
            if arcane {
                "dungeons_slimecube_arcane_small"
            } else {
                "dungeons_slimecube_poison_small"
            }
        );
        assert_eq!(
            medium,
            if arcane {
                "dungeons_slimecube_arcane"
            } else {
                "dungeons_slimecube_poison"
            }
        );
        assert_eq!(
            Slime {
                size: Size::Large,
                element
            }
            .template(),
            "dungeons_slimecube_poison_small"
        );
    }
    assert_eq!(
        Slime {
            size: Size::Medium,
            element: E::NONE
        }
        .template(),
        "dungeons_slimecube_poison_small"
    );
}

#[test]
fn placement_is_a_disc_at_parent_height_and_spawn_state_has_three_exclusions() {
    let origin = [4.0, 7.0, 9.0];
    assert_eq!(position(origin, 0.0, 0.9), origin);
    let p = position(origin, 0.25, 0.25);
    assert!((p[0] - 4.0).abs() < 1e-6);
    assert_eq!(p[1], 7.0);
    assert!((p[2] - 11.5).abs() < 1e-6);
    for animation in [0, 1, 5] {
        assert!(!needs_resurrection(animation));
    }
    assert!(needs_resurrection(170));
}
