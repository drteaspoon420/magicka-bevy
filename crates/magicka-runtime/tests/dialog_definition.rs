use magicka_runtime::{dialog_definition::*, scene_loading::custom_hash};
#[test]
fn defaults_nested_text_entities_sound_and_enum_aliases_decode() {
    let definitions = DialogDefinition::read_collection(&[br#"<Dialogs><Dialog id="Intro" iconText="USE"><interact><message character="OWNER"><text sound="Additional/Greeting" anim="idlelong0">#HELLO&amp;<b>WORLD</b>,#NAME</text></message></interact></Dialog></Dialogs>"#]).unwrap();
    let dialog = &definitions[0];
    assert_eq!(dialog.id, custom_hash("intro"));
    assert_eq!(dialog.icon, 2);
    assert_eq!(dialog.repeat_from, 0);
    let interaction = &dialog.interactions[0];
    assert!(!interaction.lock_game);
    assert!(interaction.focus_owner);
    assert_eq!(interaction.repeat, 1);
    let message = &interaction.messages[0];
    assert_eq!(message.owner, custom_hash("owner"));
    assert_eq!(message.width, 600);
    assert!(message.force_on_screen && message.facing);
    let text = &message.texts[0];
    assert_eq!(text.localization, custom_hash("#hello&world"));
    assert_eq!(text.arguments, Some(vec![custom_hash("#name")]));
    assert_eq!(
        (text.sound_bank, text.sound, text.animation),
        (512, custom_hash("greeting"), 2)
    );
    assert_eq!(text.ttl, -1.0);
    assert_eq!(dialog.cursor().interactions.len(), 1);
}
#[test]
fn message_try_parse_fallbacks_and_text_unknown_attributes_match_original() {
    let defs = DialogDefinition::read_collection(&[br#"<Dialogs><dialog><interact count="0"><message width="bad" facing="bad" forceOnScreen="bad"><arbitrary ignored="bad" anim="65535" ttl="0.25" sound="cue"> #KEY </arbitrary></message></interact></dialog></Dialogs>"#]).unwrap();
    let message = &defs[0].interactions[0].messages[0];
    assert_eq!(message.width, 600);
    assert!(!message.facing && !message.force_on_screen);
    assert_eq!(message.texts[0].animation, 65535);
    assert_eq!(message.texts[0].ttl, 0.25);
    assert_eq!(message.texts[0].sound_bank, 32);
    assert_eq!(message.texts[0].localization, custom_hash(" #key "));
}
#[test]
fn duplicate_dialogs_unknown_structure_and_invalid_sound_are_errors() {
    let bytes = br#"<Dialogs><Dialog id="same"/></Dialogs>"#;
    assert!(matches!(
        DialogDefinition::read_collection(&[bytes, bytes]),
        Err(DialogReadError::DuplicateDialog(_))
    ));
    assert!(
        DialogDefinition::read_collection(&[br#"<Dialogs><dialog wrong="x"/></Dialogs>"#]).is_err()
    );
    assert!(DialogDefinition::read_collection(&[
        br#"<Dialogs><dialog><unknown/></dialog></Dialogs>"#
    ])
    .is_err());
    assert!(matches!(DialogDefinition::read_collection(&[br#"<Dialogs><dialog><interact><message><text sound="a/b/c"/></message></interact></dialog></Dialogs>"#]), Err(DialogReadError::InvalidSound)));
    assert!(DialogDefinition::read_collection(&[br#"<Dialogs><dialog><interact><message><text anim="-1"/></message></interact></dialog></Dialogs>"#]).is_err());
}
