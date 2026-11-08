//! Level.NoItems and hot-join equipment state. Hardware co-op report
//! 2026-11-03: a pad joining Wizard Castle's opening room arrived with a
//! sword and staff while the party was unarmed.
use magicka_runtime::play_session::PlaySession;
use magicka_runtime::scene_world::campaign_live::CampaignRequest;
use std::path::PathBuf;

fn content() -> Option<PathBuf> {
    let root = magicka_content::retail_content_dir();
    root.is_dir().then_some(root)
}
fn items(s: &PlaySession, slot: usize) -> Vec<(i32, String)> {
    let mut v: Vec<_> = s.world.players[slot]
        .as_ref()
        .unwrap()
        .items
        .iter()
        .map(|(k, i)| (*k, i.definition.name.to_ascii_lowercase()))
        .collect();
    v.sort();
    v
}
fn unarmed() -> Vec<(i32, String)> {
    vec![(0, "weapon_unarmed".into()), (1, "weapon_unarmed".into())]
}

#[test]
fn level_start_reads_no_items() {
    let Some(root) = content() else { return };
    let levels = root.join("Levels");
    assert!(
        magicka_runtime::play_content::read_level(&levels.join("Tsar/Tsar_WizardCastle.lvl"))
            .unwrap()
            .no_items
    );
    assert!(
        magicka_runtime::play_content::read_level(&levels.join("Wizard_Castle.lvl"))
            .unwrap()
            .no_items
    );
    assert!(
        !magicka_runtime::play_content::read_level(&levels.join("Havindr.lvl"))
            .unwrap()
            .no_items
    );
}

/// Level.GoToScene (no current scene, NoItems): weapon_unarmed in both slots.
/// Tsar Wizard Castle has no unequip section, so only NoItems disarms it.
#[test]
fn no_items_level_start_disarms_before_scene_items() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Tsar/Tsar_WizardCastle.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.step(1.0 / 60.0);
    assert_eq!(items(&s, 0), unarmed());
    // Ordinary levels keep the robe's starting equipment.
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.step(1.0 / 60.0);
    assert_ne!(items(&s, 0), unarmed());
}

#[test]
fn hot_joiner_takes_the_partys_denied_slots_not_robe_defaults() {
    let Some(root) = content() else { return };
    let mut s = PlaySession::load(&root, "Wizard_Castle.lvl", "Wizard").unwrap();
    for _ in 0..60 * 20 {
        s.step(1.0 / 60.0);
    }
    // wc_s1 opening: beginUnequipWeaponsSection and NoItems.
    assert_eq!(items(&s, 0), unarmed());
    s.join_player_near_party(1, "Wizard").unwrap();
    s.step(1.0 / 60.0);
    assert_eq!(items(&s, 1), unarmed());

    // Once the section ends the party is armed again, and so is a joiner.
    s.world
        .campaign_requests
        .push(CampaignRequest::UnequipWeaponsSection {
            begin: false,
            animation: 0,
        });
    s.step(1.0 / 60.0);
    let armed = items(&s, 0);
    assert_ne!(armed, unarmed());
    assert_eq!(items(&s, 1), armed);
    s.join_player_near_party(2, "Wizard").unwrap();
    assert_eq!(items(&s, 2), armed);
}
