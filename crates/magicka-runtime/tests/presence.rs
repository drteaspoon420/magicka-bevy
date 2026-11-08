use magicka_content::level::TriggerArea;
use magicka_runtime::{presence::*, scene_loading::custom_hash};
fn entity(id: i32, dead: bool, invisible: bool) -> PresenceEntity {
    PresenceEntity {
        handle: 0,
        unique_id: id,
        position: [1.0; 3],
        radius: 0.0,
        dead,
        ignored_avatar: false,
        physics: None,
        character: Some(PresenceCharacter {
            type_id: custom_hash("wizard"),
            original_faction: 0x101,
            faction: 1,
            undying: false,
            invisible,
        }),
    }
}
fn areas() -> PresenceAreas {
    PresenceAreas::from_content(&[TriggerArea {
        name: "Room".into(),
        position: [0.0; 3],
        sides: [2.0; 3],
        orientation: [0.0, 0.0, 0.0, 1.0],
    }])
    .unwrap()
}
fn met(
    tag: &str,
    attrs: &[(&str, &str)],
    areas: &PresenceAreas,
    entities: &[PresenceEntity],
) -> bool {
    PresenceCondition::read(tag, attrs.iter().copied())
        .unwrap()
        .is_met(PresenceFacts { areas, entities })
        .unwrap()
}
#[test]
fn cached_named_and_any_membership_preserves_dead_and_faction_quirks() {
    let mut areas = areas();
    let mut entities = [
        entity(1, false, false),
        entity(2, true, true),
        entity(3, true, false),
        entity(4, false, false),
        entity(5, false, false),
    ];
    entities[2].character.as_mut().unwrap().undying = true;
    entities[3].character = None;
    entities[4].ignored_avatar = true;
    areas.update_present(&entities);
    let named = areas.get(custom_hash("room")).unwrap();
    let any = areas.get(custom_hash("any")).unwrap();
    assert_eq!(named.entities, [0, 1, 2, 3]);
    assert_eq!(named.characters, [0, 1, 2]);
    assert_eq!(any.characters, [0, 2]);
    assert_eq!(named.type_count(custom_hash("any")), 2);
    assert_eq!(named.faction_count(0x101), 4);
    assert_eq!(any.faction_count(0x101), 2);
    assert!(met(
        "present",
        &[
            ("area", "room"),
            ("type", "wizard"),
            ("nr", "1"),
            ("includeInvisible", "false")
        ],
        &areas,
        &entities
    ));
    assert!(met(
        "present",
        &[
            ("area", "any"),
            ("type", "wizard"),
            ("nr", "2"),
            ("includeInvisible", "false")
        ],
        &areas,
        &entities
    ));
    // 'any' type count does not subtract invisible characters of another type.
    entities[0].character.as_mut().unwrap().invisible = true;
    assert!(met(
        "present",
        &[
            ("area", "room"),
            ("type", "any"),
            ("nr", "2"),
            ("includeInvisible", "false")
        ],
        &areas,
        &entities
    ));
    // Original faction cached twice per entity; current faction subtracts once.
    assert!(met(
        "factionPresent",
        &[
            ("area", "room"),
            ("factions", "EVIL, PLAYER0"),
            ("nr", "2"),
            ("includeInvisible", "false")
        ],
        &areas,
        &entities
    ));
}
#[test]
fn invisibility_and_dead_state_are_live_but_cached_counts_do_not_refresh() {
    let mut areas = areas();
    let mut entities = [entity(1, true, true)];
    areas.update_present(&entities);
    assert!(met(
        "present",
        &[
            ("area", "room"),
            ("type", "wizard"),
            ("nr", "-1"),
            ("includeInvisible", "false")
        ],
        &areas,
        &entities
    ));
    assert!(!met("idPresent", &[("id", "named")], &areas, &entities));
    entities[0].unique_id = custom_hash("named");
    entities[0].dead = false;
    assert!(met("idPresent", &[("id", "named")], &areas, &entities));
    assert_eq!(
        areas
            .get(custom_hash("room"))
            .unwrap()
            .type_count(custom_hash("wizard")),
        0
    );
}
#[test]
fn id_present_returns_first_match_and_dead_undying_is_still_absent() {
    let mut areas = areas();
    let mut entities = [
        entity(custom_hash("named"), true, false),
        entity(custom_hash("named"), false, false),
    ];
    entities[0].character.as_mut().unwrap().undying = true;
    areas.update_present(&entities);
    assert!(!met("idPresent", &[("id", "NAMED")], &areas, &entities));
    assert!(met(
        "idPresent",
        &[("id", "NAMED"), ("invert", "true")],
        &areas,
        &entities
    ));
}
#[test]
fn corner_origin_rotation_and_radius_boundary_select_members() {
    let mut areas = PresenceAreas::from_content(&[TriggerArea {
        name: "rotated".into(),
        position: [10.0, 0.0, 0.0],
        sides: [2.0; 3],
        orientation: [0.0, 0.0, 1.0, 0.0],
    }])
    .unwrap();
    let mut entities = [entity(1, false, false); 3];
    entities[0].position = [7.0, -1.0, 1.0];
    entities[0].radius = 1.0;
    entities[1].position = [7.0, -1.0, 1.0];
    entities[1].radius = 0.5;
    entities[2].position = [9.0, 1.0, 1.0];
    entities[2].radius = 0.5;
    areas.update_present(&entities);
    assert_eq!(areas.get(custom_hash("rotated")).unwrap().entities, [0]);
    assert_eq!(areas.get(custom_hash("any")).unwrap().entities, [0, 1, 2]);
}
#[test]
fn defaults_numeric_factions_unknown_compare_and_invalid_inputs() {
    let id = PresenceCondition::read("idPresent", []).unwrap();
    assert_eq!(id.area, custom_hash("any"));
    let present = PresenceCondition::read("present", []).unwrap();
    assert_eq!(present.area, 0);
    let mut areas = areas();
    let entities = [entity(1, false, false)];
    areas.update_present(&entities);
    assert!(met(
        "factionPresent",
        &[("area", "room"), ("factions", "257"), ("nr", "2")],
        &areas,
        &entities
    ));
    assert!(!met(
        "present",
        &[("area", "room"), ("compareMethod", "42")],
        &areas,
        &entities
    ));
    assert_eq!(
        present.is_met(PresenceFacts {
            areas: &areas,
            entities: &entities
        }),
        Err(PresenceError::MissingArea(0))
    );
    assert!(PresenceCondition::read("factionPresent", [("factions", "unknown")]).is_err());
    assert!(PresenceCondition::read("present", [("nr", "bad")]).is_err());
    assert!(PresenceAreas::from_content(&[TriggerArea {
        name: "ANY".into(),
        position: [0.0; 3],
        sides: [1.0; 3],
        orientation: [0.0, 0.0, 0.0, 1.0]
    }])
    .is_err());
}

#[test]
fn collision_population_and_global_count_append_without_deduplication() {
    let mut areas = areas();
    let entities = [entity(1, false, false)];
    areas.reset();
    areas.count_any(&entities);
    assert!(areas.get(custom_hash("room")).unwrap().entities.is_empty());
    for _ in 0..2 {
        areas.add_entity(custom_hash("room"), 0, &entities).unwrap();
    }
    assert_eq!(areas.get(custom_hash("room")).unwrap().entities, [0, 0]);
    assert_eq!(
        areas
            .get(custom_hash("room"))
            .unwrap()
            .type_count(custom_hash("wizard")),
        2
    );
    areas.count_any(&entities);
    assert_eq!(
        areas
            .get(custom_hash("any"))
            .unwrap()
            .type_count(custom_hash("wizard")),
        2
    );
    areas.reset();
    assert_eq!(
        areas
            .get(custom_hash("room"))
            .unwrap()
            .type_count(custom_hash("wizard")),
        0
    );
}

#[test]
fn physics_present_counts_physics_entities_and_damageable_subclasses() {
    let mut areas = areas();
    let mut entities = [
        entity(1, false, false),
        entity(2, false, false),
        entity(3, true, false),
        entity(4, false, false),
    ];
    for (e, physics) in entities
        .iter_mut()
        .zip([None, Some(false), Some(true), Some(true)])
    {
        if physics.is_some() {
            e.character = None;
        }
        e.physics = physics;
    }
    areas.update_present(&entities);
    // Dead entities stay in PresentEntities; the default area is "any" and
    // the default CompareMethod is EQUAL.
    assert!(met("PhysicsPresent", &[("nr", "3")], &areas, &entities));
    assert!(met(
        "physicspresent",
        &[("area", "room"), ("nr", "3")],
        &areas,
        &entities
    ));
    assert!(met(
        "PhysicsPresent",
        &[("area", "room"), ("onlyDamagable", "true"), ("nr", "2")],
        &areas,
        &entities
    ));
    assert!(met(
        "PhysicsPresent",
        &[("nr", "3"), ("comparemethod", "less"), ("invert", "true")],
        &areas,
        &entities
    ));
    assert!(!met(
        "PhysicsPresent",
        &[("nr", "2"), ("comparemethod", "LESS")],
        &areas,
        &entities
    ));
}
