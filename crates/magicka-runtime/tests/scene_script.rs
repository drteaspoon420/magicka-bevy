use magicka_runtime::{scene_loading::custom_hash, scene_script::*};
#[test]
fn groups_share_common_actions_and_looped_content_is_selected_at_load_time() {
    let xml=br#"<Scene><trigger id="ROOT" repeat="true"><if><counter name="x" value="1"/></if><if/>
    <then><counter name="common"/><random><playAnimation name="door"/></random><random><timer name="clock"/></random>
    <looped><counter name="loop_common"/><random><stopAnimation name="door"/></random></looped></then></trigger></Scene>"#;
    let layout = SceneScriptLayout::read(xml, false).unwrap();
    assert_eq!(layout.triggers[0].id, custom_hash("root"));
    assert_eq!(layout.triggers[0].repeat, 0.0);
    assert_eq!(layout.triggers[0].conditions, [vec![0], vec![]]);
    assert_eq!(layout.triggers[0].actions, [vec![1, 0], vec![2, 0]]);
    assert_eq!(layout.actions.len(), 3);
    let layout = SceneScriptLayout::read(xml, true).unwrap();
    assert_eq!(
        layout.triggers[0].actions,
        [vec![1, 0, 3], vec![2, 0, 3], vec![4, 0, 3]]
    );
}
#[test]
fn unsupported_behaviors_remain_visible_and_known_malformed_attributes_fail() {
    let layout=SceneScriptLayout::read(br#"<Scene><trigger id="start"><if><unrecovered type="player"/></if><then><unrecoveredAction type="wizard"><arg/></unrecoveredAction></then></trigger><onInteract id="use"/></Scene>"#,false).unwrap();
    assert!(matches!(
        layout.conditions[0],
        ConditionDefinition::Unsupported(_)
    ));
    let ActionDefinition::Unsupported(node) = &layout.actions[0] else {
        panic!()
    };
    assert_eq!(node.children.len(), 1);
    assert_eq!(layout.deferred_interactables.len(), 1);
    assert!(matches!(
        SceneScriptLayout::read(
            br#"<Scene><trigger><then><counter value="bad"/></then></trigger></Scene>"#,
            false
        ),
        Err(LayoutError::Action(_))
    ));
}
#[test]
fn duplicate_ids_invalid_trigger_properties_and_incomplete_xml_are_errors() {
    assert!(matches!(
        SceneScriptLayout::read(
            br#"<Scene><trigger id="A"><then/></trigger><trigger id="a"><then/></trigger></Scene>"#,
            false
        ),
        Err(LayoutError::DuplicateTrigger(_))
    ));
    assert!(matches!(
        SceneScriptLayout::read(
            br#"<Scene><trigger unknown="x"><then/></trigger></Scene>"#,
            false
        ),
        Err(LayoutError::InvalidTriggerAttribute(_))
    ));
    assert!(SceneScriptLayout::read(b"<Scene><trigger>", false).is_err());
    assert!(SceneScriptLayout::read(b"<Level/>", false).is_err());
    assert!(SceneScriptLayout::read(br#"<!DOCTYPE Scene><Scene/>"#, false).is_err());
}
#[test]
fn comments_are_ignored_and_last_then_replaces_action_groups() {
    let layout=SceneScriptLayout::read(br#"<Scene><trigger repeat="false" autorun="false"><if><!--ignored--></if><then><counter/></then><then><!--ignored--><timer/></then></trigger></Scene>"#,false).unwrap();
    assert!(!layout.triggers[0].autorun);
    assert_eq!(layout.triggers[0].repeat, -1.0);
    assert_eq!(layout.triggers[0].actions, [vec![1]]);
    assert_eq!(layout.actions.len(), 2);
}

#[test]
fn deferred_content_preserves_references_and_only_one_looped_wrapper_is_expanded() {
    let layout = SceneScriptLayout::read(
        br#"<Scene><trigger><then><looped><looped><counter/></looped></looped>
        <unknown value="a&amp;b">before&amp;after<![CDATA[<raw>]]><child/></unknown>
        </then></trigger></Scene>"#,
        true,
    )
    .unwrap();
    let ActionDefinition::Unsupported(looped) = &layout.actions[0] else {
        panic!("nested Looped must remain an unresolved action")
    };
    assert_eq!(looped.name, "looped");
    assert_eq!(looped.children[0].name, "counter");
    let ActionDefinition::Unsupported(unknown) = &layout.actions[1] else {
        panic!("unknown action must retain its content")
    };
    assert_eq!(unknown.attributes[0].1, "a&b");
    assert_eq!(unknown.raw_text, "before&amp;after<raw>");
    assert_eq!(unknown.children[0].name, "child");
}
