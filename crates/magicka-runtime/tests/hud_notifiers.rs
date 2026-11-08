//! Avatar.InternalPickUp's obtained text box (research/hud-notifiers.md):
//! the pickup string over the picker for 2 s through the dialog host's
//! additional text boxes. Requires the private retail content.
use magicka_runtime::play_session::PlaySession;
use magicka_runtime::scene_loading::custom_hash;
use std::path::PathBuf;

fn content() -> Option<PathBuf> {
    let p =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    p.is_dir().then_some(p)
}
fn session() -> Option<PlaySession> {
    let content = content()?;
    let mut s = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.headless_dialogs = Some([1280., 720.]);
    Some(s)
}
fn notices(s: &PlaySession) -> Vec<(Option<i32>, f32, bool, bool, Option<String>)> {
    s.world
        .dialog_text
        .as_ref()
        .unwrap()
        .additional
        .iter()
        .map(|w| {
            (
                w.state.owner,
                w.state.ttl,
                w.state.force_on_screen,
                w.state.visible(),
                w.state.text.clone(),
            )
        })
        .collect()
}

#[test]
fn book_pickup_shows_the_magick_pickup_string_over_the_picker_for_two_seconds() {
    let Some(mut s) = session() else {
        return;
    };
    let mut t = magicka_content::skinned::IDENTITY;
    t[12..15].copy_from_slice(&s.body().position);
    let h = s.spawn_magick_book(3, t).unwrap();
    s.step(1. / 60.);
    assert!(notices(&s).is_empty());
    assert_eq!(s.pickup_book(h).unwrap(), 3);
    let strings = &s.world.dialog_text.as_ref().unwrap().strings;
    let name = strings.get_string(custom_hash("#magick_haste")).unwrap();
    assert!(!name.starts_with("NOT FOUND"), "{name}");
    let template = strings.get_string(custom_hash("#magick_pick_up")).unwrap();
    assert!(template.contains("#1;"), "{template}");
    let expected = template.replace("#1;", &format!("[c=1,1,1]{name}[/c]"));
    let n = notices(&s);
    assert_eq!(n.len(), 1);
    let (owner, ttl, force, visible, text) = n[0].clone();
    assert_eq!(owner, Some(custom_hash("player1")));
    assert_eq!(ttl, 2.);
    assert!(force);
    assert!(visible);
    assert_eq!(text.as_deref(), Some(expected.as_str()));
    // Typing precedes the TTL: the box is still up after 2 s of play and
    // closes (hidden, shrunk and dropped) within a few seconds more.
    for _ in 0..120 {
        s.step(1. / 60.);
    }
    assert_eq!(notices(&s).len(), 1);
    assert!(notices(&s)[0].3);
    let mut closed_at = None;
    for frame in 120..1200 {
        s.step(1. / 60.);
        if notices(&s).is_empty() {
            closed_at = Some(frame);
            break;
        }
    }
    let closed_at = closed_at.expect("obtained notice never closed");
    assert!(
        closed_at > 120 && closed_at < 600,
        "closed at frame {closed_at}"
    );
    assert!(
        s.world.dialog_failure.is_none(),
        "{:?}",
        s.world.dialog_failure
    );
}

#[test]
fn repeated_pickups_reuse_one_obtained_box_per_player() {
    let Some(mut s) = session() else {
        return;
    };
    let mut t = magicka_content::skinned::IDENTITY;
    t[12..15].copy_from_slice(&s.body().position);
    let h = s.spawn_magick_book(3, t).unwrap();
    s.step(1. / 60.);
    s.pickup_book(h).unwrap();
    for _ in 0..30 {
        s.step(1. / 60.);
    }
    let scale = notices(&s).len();
    assert_eq!(scale, 1);
    let grown = s.world.dialog_text.as_ref().unwrap().additional[0]
        .state
        .scale;
    assert!(grown > 0.);
    let second = s.spawn_magick_book(5, t).unwrap();
    s.step(1. / 60.);
    s.pickup_book(second).unwrap();
    let n = notices(&s);
    assert_eq!(n.len(), 1, "Player.ObtainedTextBox is one box");
    let strings = &s.world.dialog_text.as_ref().unwrap().strings;
    let name = strings.get_string(custom_hash("#magick_teleport")).unwrap();
    assert!(n[0].4.as_deref().unwrap().contains(&name));
    assert_eq!(n[0].1, 2.);
    // Initialize keeps a still-growing scale instead of restarting it.
    let widget = &s.world.dialog_text.as_ref().unwrap().additional[0];
    assert!(
        widget.state.scale >= grown,
        "{} < {grown}",
        widget.state.scale
    );
    assert!(widget.state.grow);
}

#[test]
fn item_pickup_shows_the_item_pickup_string_with_its_display_name() {
    let Some(mut s) = session() else {
        return;
    };
    let mut t = magicka_content::skinned::IDENTITY;
    t[12..15].copy_from_slice(&s.body().position);
    let item = s.spawn_item_pickup("weapon_kris", t).unwrap();
    s.step(1. / 60.);
    let display_name = s.world.item_pickups[&item]
        .item
        .as_ref()
        .unwrap()
        .definition
        .display_name
        .clone();
    let outcome = s.pickup_item(item).unwrap();
    assert_eq!(
        outcome,
        magicka_runtime::pickups::EquipmentExchange::Swapped
    );
    let strings = &s.world.dialog_text.as_ref().unwrap().strings;
    let template = strings.get_string(custom_hash("#item_pick_up")).unwrap();
    let expected = if display_name.is_empty() {
        template
    } else {
        let name = strings.get_string(custom_hash(&display_name)).unwrap();
        template.replace("#1;", &format!("[c=1,1,1]{name}[/c]"))
    };
    let n = notices(&s);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].0, Some(custom_hash("player1")));
    assert_eq!(n[0].1, 2.);
    assert_eq!(n[0].4.as_deref(), Some(expected.as_str()));
}
