//! Character skeleton pose and equipment attach points on retail content
//! (skips without it): research/character-casting.md.
use magicka_runtime::{
    animation_controller::{animation_value, AnimationController, AnimationSet, PoseSample},
    character_pose::character_orientation,
    play_content::{load_character, load_rig},
};
use magicka_sim::character_physics::Orientation;
use std::path::{Path, PathBuf};

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir().then_some(root)
}

fn translation(m: &[f32; 16]) -> [f32; 3] {
    [m[12], m[13], m[14]]
}

#[test]
fn wizard_rig_pose_matches_reference_sampling_and_holds_the_staff() {
    let Some(root) = content() else {
        eprintln!("SKIP: retail content absent");
        return;
    };
    let template = load_character(&root, "Wizard").unwrap();
    let rig = load_rig(&root, &template)
        .unwrap()
        .expect("wizard has a skeleton");
    // Equipment: blade (Slash_Medium) on rightattach, staff on leftattach.
    assert_eq!(rig.attachments.len(), 2);
    assert_eq!(rig.attachments[0].weapon_class, 8);
    assert_eq!(rig.spell_source, Some(1));
    assert_ne!(
        rig.attachments[1].attach0,
        magicka_content::skinned::IDENTITY
    );

    let skeleton = &rig.skeleton;
    let identity = magicka_content::skinned::IDENTITY;
    // Without a clip every bone keeps its bind pose: skinned ≈ identity.
    for m in skeleton.skinned(None, &identity) {
        for (k, v) in m.iter().enumerate() {
            let want = if k % 5 == 0 { 1.0 } else { 0.0 };
            assert!((v - want).abs() < 2e-3, "bind skin {m:?}");
        }
    }
    // A non-looping, non-fading sample equals content's independent
    // reference implementation of the same XNAnimation arithmetic.
    let durations = skeleton.clip_durations();
    let set = AnimationSet::with_weapons(&template.animation_sets, &[8, 18], durations.clone());
    let idle = set
        .lookup(animation_value("idle").unwrap())
        .unwrap()
        .clip
        .clone();
    let time = durations[&idle] * 0.37;
    let ours = skeleton.skinned(
        Some(PoseSample {
            clip: &idle,
            duration: durations[&idle],
            time,
            looping: false,
            crossfade: None,
            crossfade_amount: 0.0,
        }),
        &identity,
    );
    let reference = magicka_content::skinned::skin_matrices(
        &skeleton.skeleton,
        &reference_clip(&root, &template, &idle),
        time,
        &identity,
    );
    for (a, b) in ours.iter().zip(&reference) {
        for k in 0..16 {
            assert!((a[k] - b[k]).abs() < 1e-4, "{a:?} vs {b:?}");
        }
    }

    // The staff hangs on LeftAttach and its orb (attach0, 1.005 up the
    // shaft) is the CastSource. Standing at the origin facing -Z, the
    // cast_force_spray pose points the orb ahead of the caster at chest
    // height.
    let height_offset = -0.5 * template.length - template.radius;
    let parent = character_orientation(
        &rig.static_transform,
        &Orientation::IDENTITY,
        [0.0, -height_offset, 0.0],
        height_offset,
    );
    let bind_absolute = skeleton.skeleton.bind_absolute();
    let left_attach = skeleton.bone("leftattach").unwrap();
    for animation in ["idle", "move_walk", "cast_force_spray"] {
        let mut controller = AnimationController::default();
        controller.start(&set, animation_value(animation).unwrap());
        controller.update(0.3);
        let skinned = skeleton.skinned_for(&controller, &parent);
        let staff = &rig.attachments[1];
        let item = translation(&staff.item_transform(&skinned));
        let orb = translation(&staff.attach_absolute(&skinned));
        let b = translation(&bind_absolute[left_attach]);
        let m = &skinned[left_attach];
        let bone = [0, 1, 2].map(|c| b[0] * m[c] + b[1] * m[4 + c] + b[2] * m[8 + c] + m[12 + c]);
        let distance = |a: [f32; 3], b: [f32; 3]| {
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        };
        eprintln!("{animation}: hand {bone:?} orb {orb:?}");
        assert!(
            distance(item, bone) < 0.02,
            "{animation}: item off the bone"
        );
        assert!(
            (distance(orb, item) - 1.005).abs() < 0.01,
            "{animation}: shaft length"
        );
        if animation == "cast_force_spray" {
            assert!(
                orb[2] < -0.6 && orb[1] > 0.7 && orb[1] < 1.6,
                "spray orb {orb:?}"
            );
        }
    }
}

fn reference_clip(
    root: &Path,
    template: &magicka_content::character::CharacterTemplate,
    clip: &str,
) -> magicka_content::assets::AnimationClip {
    // Same file, decoded through content's resolver.
    let relative = Path::new("Data/Characters")
        .join(template.skeleton_model.as_ref().unwrap().replace('\\', "/"))
        .with_extension("xnb");
    let path = magicka_runtime::play_content::resolve(root, &relative).unwrap();
    let bytes = std::fs::read(path).unwrap();
    let xnb = magicka_xnb::xnb::parse(&bytes).unwrap();
    let parsed = magicka_content::parse_primary(&xnb).unwrap().unwrap();
    let magicka_content::Content::Xna(object) = parsed.content else {
        panic!()
    };
    let magicka_content::xna::XnaObject::Asset(asset) = *object else {
        panic!()
    };
    let magicka_content::assets::Asset::SkinnedModel(model) = *asset else {
        panic!()
    };
    magicka_content::skinned::resolve(&model, parsed.shared)
        .unwrap()
        .clips
        .into_iter()
        .find(|c| c.name == clip)
        .unwrap()
}
