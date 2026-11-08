use magicka_runtime::pickups::*;

fn item(handle: u16, x: f32, y: f32, z: f32) -> PickupCandidate {
    PickupCandidate {
        handle,
        position: [x, y, z],
        radius: 0.0,
        pickable: true,
        dead: false,
    }
}
#[test]
fn targeting_is_three_dimensional_then_checks_horizontal_facing() {
    let p = [0.0; 3];
    let f = [0.0, 0.0, 1.0];
    // A high item is outside the query even directly above the player.
    assert_eq!(
        find_pickup([item(1, 0.0, 3.0, 0.0)], p, f, 0.75, false, None),
        None
    );
    assert_eq!(
        find_pickup([item(1, f32::NAN, 0.0, 0.0)], p, f, 0.75, false, None),
        None
    );
    // The query adds the entity radius and accepts its exact boundary.
    let mut a = item(1, 0.0, 3.0, 0.0);
    a.radius = 0.5;
    assert_eq!(find_pickup([a], p, f, 0.75, true, None), Some(1));
    assert_eq!(
        find_pickup([item(1, 0.0, 0.0, -2.0)], p, f, 0.75, true, None),
        None
    );
    assert_eq!(
        find_pickup([item(1, 0.0, 0.0, -0.75)], p, f, 0.75, true, None),
        Some(1)
    );
    assert_eq!(
        find_pickup([item(1, 2.0, 0.0, 0.0)], p, f, 0.75, true, None),
        None
    );
    assert_eq!(
        find_pickup([item(1, 2.0, 0.0, 0.0)], p, f, 0.75, false, None),
        Some(1)
    );
}
#[test]
fn target_matching_and_host_order_are_preserved() {
    let candidates = [item(10, 0.0, 0.0, 2.0), item(20, 0.0, 0.0, 1.0)];
    assert_eq!(
        find_pickup(candidates, [0.0; 3], [0.0, 0.0, 1.0], 0.75, false, None),
        Some(10)
    );
    assert_eq!(
        find_pickup(candidates, [0.0; 3], [0.0, 0.0, 1.0], 0.75, false, Some(20)),
        Some(20)
    );
    let mut candidates = candidates;
    candidates[0].dead = true;
    candidates[1].pickable = false;
    assert_eq!(
        find_pickup(candidates, [0.0; 3], [0.0, 0.0, 1.0], 0.75, false, None),
        None
    );
}
#[test]
fn equipment_swap_preserves_item_cooldowns_and_obeys_bound_equipment() {
    use magicka_runtime::{
        avatar::AvatarItem,
        play_content::{avatar_items, load_character},
    };

    let content = magicka_content::retail_content_dir();
    if !content.is_dir() {
        return;
    }
    let template = load_character(&content, "Wizard").unwrap();
    let mut items = avatar_items(&content, &template);
    let (_, mut held) = items.remove(items.iter().position(|(s, _)| *s == 0).unwrap());
    let (_, mut ground) = items.remove(items.iter().position(|(s, _)| *s == 1).unwrap());
    assert_eq!(equipment_slot(&held), 0);
    assert_eq!(equipment_slot(&ground), 1);
    let names = (held.definition.name.clone(), ground.definition.name.clone());
    held.definition.bound = false;
    held.cooldown = 1.25;
    held.special_cooldown = 2.5;
    ground.cooldown = 3.0;
    ground.special_cooldown = 4.0;
    assert_eq!(
        exchange_equipment(&mut held, &mut ground),
        EquipmentExchange::Swapped
    );
    assert_eq!(held.definition.name, names.1);
    assert_eq!(ground.definition.name, names.0);
    assert_eq!((held.cooldown, held.special_cooldown), (3.0, 4.0));
    assert_eq!((ground.cooldown, ground.special_cooldown), (1.25, 2.5));
    held.definition.bound = true;
    let previous = held.definition.clone();
    let mut another = AvatarItem::new(ground.definition.clone());
    assert_eq!(
        exchange_equipment(&mut held, &mut another),
        EquipmentExchange::Bound
    );
    assert_eq!(held.definition, previous);
    assert_eq!(held.cooldown, 3.0);
}
