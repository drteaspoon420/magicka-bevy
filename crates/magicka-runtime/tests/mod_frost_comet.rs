//! The example Frost Comet Storm mod, built only from world operations:
//! the sky darkens and Ice comets fall from Meteor Shower's height.
use magicka_definitions::DefinitionId;
use magicka_runtime::{mod_runtime::ModRuntime, play_session::PlaySession};
use std::path::PathBuf;

#[test]
fn frost_comet_storm_drops_ice_comets_from_the_sky() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut s = PlaySession::load(
        &magicka_content::retail_content_dir(),
        "Havindr.lvl",
        "Wizard",
    )
    .unwrap();
    s.disable_scripts();
    let mut mods = ModRuntime::load(
        None,
        &root.join("../../mods"),
        Some(&["example.frost_comet".into()]),
        1,
    )
    .unwrap();
    mods.attach(&mut s);
    let tick = |s: &mut PlaySession, m: &mut ModRuntime| {
        m.step(s, 1.0 / 60.0);
        s.step(1.0 / 60.0);
    };
    tick(&mut s, &mut mods);
    assert!(!s.world.scene_indoors);
    let handle = s.avatar().handle;
    let id = DefinitionId::parse("example.frost_comet:frost_comet_storm", "x").unwrap();
    mods.invoke_magick(&mut s, handle, &id, None).unwrap();
    let mut comets = 0;
    for _ in 0..120 {
        tick(&mut s, &mut mods);
        comets = comets.max(
            s.world
                .missiles
                .iter()
                .filter(|m| m.body.position[1] > 20.0)
                .count(),
        );
    }
    let errors: Vec<_> = s
        .world
        .log
        .iter()
        .filter(|l| l.contains("mod ") || l.contains("magick"))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(s.world.light_target_intensity, 0.4, "the sky darkened");
    assert!(comets >= 2, "comets in the air: {comets}");
}
