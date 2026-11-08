use magicka_runtime::{
    entity_command::CharacterSelector,
    play_session::PlaySession,
    scene_loading::custom_hash,
    scene_script::{ActionDefinition, SceneScriptLayout},
};

#[test]
fn castle_opening_damage_and_quick_execute_use_public_player_damage() {
    let root = magicka_content::retail_content_dir();
    if !root.is_dir() {
        return;
    }
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    s.change_scene_named("wc_s3", "start").unwrap();
    let initial = s.avatar().character.combat.health.hit_points;
    for _ in 0..15 {
        s.step(1. / 60.);
    }
    assert!(s.script_error.is_none(), "{:?}", s.script_error);
    assert_eq!(
        s.avatar().character.combat.health.hit_points,
        initial - 140.
    );
    let id = custom_hash("player1");
    let index = s
        .runtime
        .layout()
        .actions
        .iter()
        .position(|a| {
            matches!(a,
                ActionDefinition::Damage(a) if a.target == CharacterSelector::Id(id)
            )
        })
        .unwrap();
    // The intro's AI event and cutscene make the avatar invulnerable. QuickExecute must
    // still use public damage rules, rather than subtracting health directly.
    s.runtime.quick_execute(index, &mut s.world).unwrap();
    assert_eq!(
        s.avatar().character.combat.health.hit_points,
        initial - 140.
    );
    let body = s.avatar().body;
    s.world.players[0]
        .as_mut()
        .unwrap()
        .set_events(&mut s.world.physics.bodies[body], None);
    s.world.players[0].as_mut().unwrap().sync_immortality(false);
    s.runtime.quick_execute(index, &mut s.world).unwrap();
    assert_eq!(
        s.avatar().character.combat.health.hit_points,
        initial - 280.
    );
}

#[test]
fn damage_children_preserve_order_defaults_and_combined_flags() {
    let layout = SceneScriptLayout::read(br#"<Scene><trigger id="t"><then>
        <damage id="player1" source="origin" ignoreShields="false" delay=".1">
          <damage attackProperty="damage,armourPiercing" element="fire,arcane" magnitude="2" amount="3"/>
          <unrelated/><damage amount="4"/>
        </damage>
    </then></trigger></Scene>"#, false).unwrap();
    let ActionDefinition::Damage(a) = &layout.actions[0] else {
        panic!("Damage action")
    };
    assert_eq!(a.damages.len(), 2);
    assert_eq!(a.damages[0].attack, 17);
    assert_eq!(a.damages[0].element, 0x28);
    assert_eq!(a.damages[0].magnitude, 2.);
    assert_eq!(a.damages[1].attack, 0);
    assert_eq!(a.damages[1].magnitude, 0.);
    assert!(!a.ignore_shields);
}
