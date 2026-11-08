use magicka_runtime::main_menu::{hit_test, tiles, Direction::*, MenuCursor, TileId::*};

#[test]
fn controller_attachment_does_not_activate_or_move_on_first_press() {
    let mut cursor = MenuCursor::default();
    assert_eq!(cursor.select(), None);
    assert_eq!(cursor.selected, Some(Store));
    assert_eq!(cursor.select(), Some(Store));
    cursor.pointer(Some(Online));
    cursor.navigate(Down);
    assert_eq!(cursor.selected, Some(Store));
    cursor.navigate(Down);
    assert_eq!(cursor.selected, Some(Adventure));
}

#[test]
fn controller_traverses_tiles_and_bottom_row_using_authored_links() {
    let mut cursor = MenuCursor::default();
    cursor.select();
    for (direction, expected) in [
        (Down, Adventure),
        (Right, Dlc),
        (Down, Online),
        (Down, Leaderboards),
        (Right, Options),
        (Up, Online),
        (Left, Versus),
        (Down, Facebook),
        (Right, Twitter),
        (Left, Facebook),
        (Left, Exit),
        (Up, Challenge),
        (Up, Adventure),
        (Up, Store),
    ] {
        cursor.navigate(direction);
        assert_eq!(cursor.selected, Some(expected));
    }
    for direction in [Left, Right, Up] {
        cursor.navigate(direction);
        assert_eq!(cursor.selected, Some(Store), "missing links hold focus");
    }
}

#[test]
fn back_attaches_before_requesting_quit_and_mouse_detaches() {
    let mut cursor = MenuCursor::default();
    assert!(!cursor.back());
    assert!(cursor.back());
    cursor.pointer(None);
    assert!(!cursor.attached);
    assert_eq!(cursor.selected, None);
    assert!(!cursor.back());
    assert_eq!(cursor.selected, Some(Store));
}

#[test]
fn overlapping_cards_pick_in_reverse_creation_order_with_exclusive_upper_edges() {
    let page = tiles([700.0, 177.0]);
    assert_eq!(hit_test(&page, [400.0, 600.0]), Some(Versus));
    assert_eq!(hit_test(&page, [650.0, 600.0]), Some(Online));
    assert_eq!(hit_test(&page, [200.0, 600.0]), Some(Challenge));
    assert_eq!(hit_test(&page, [96.0, 912.0]), Some(Exit));
    assert_eq!(hit_test(&page, [416.0, 912.0]), None);
    assert_eq!(hit_test(&page, [1040.0, 500.0]), None);
}

#[test]
fn tile_size_comes_from_the_store_asset_and_label_positions_stay_on_page() {
    let page = tiles([809.0, 177.0]);
    assert_eq!(page.len(), 11);
    let store = page.iter().find(|t| t.id == Store).unwrap();
    assert_eq!(store.source, [0.0, 0.0, 809.0, 177.0]);
    assert_eq!(hit_test(&page, [974.0, 230.0]), Some(Store));
    assert_eq!(hit_test(&page, [975.0, 230.0]), None);
    let online = page.iter().find(|t| t.id == Online).unwrap();
    assert_eq!(online.label_offset, [20.0, -40.0]);
}
