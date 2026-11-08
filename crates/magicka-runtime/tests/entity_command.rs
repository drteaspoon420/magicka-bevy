use magicka_runtime::{entity_command::*, scene_loading::custom_hash};

fn read(tag: &str, attrs: &[(&str, &str)]) -> EntityCommand {
    EntityCommand::read(tag, attrs.iter().copied()).unwrap()
}

#[derive(Default)]
struct Host {
    ops: Vec<String>,
    avatars: [Option<EntityFacts>; 4],
    entities: Vec<(i32, EntityFacts)>,
    hit: Option<[f32; 3]>,
    area: Vec<AreaCharacter>,
}
fn locator() -> [f32; 16] {
    let mut m = [0.0; 16];
    m[0] = 1.0;
    m[5] = 1.0;
    m[10] = -1.0; // Forward = (0,0,1)
    m[15] = 1.0;
    m[12..15].copy_from_slice(&[4.0, 2.0, 6.0]);
    m
}
impl EntityCommandHost for Host {
    fn set_dialog(&mut self, handle: u16, dialog: i32) -> Result<(), Self::Error> {
        self.ops.push(format!("dialog {handle} {dialog}"));
        Ok(())
    }
    type Error = &'static str;
    fn playing_avatar(&mut self, slot: usize) -> Result<Option<EntityFacts>, Self::Error> {
        Ok(self.avatars[slot])
    }
    fn entity_by_id(&mut self, id: i32) -> Result<Option<EntityFacts>, Self::Error> {
        Ok(self
            .entities
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, e)| *e))
    }
    fn locator(&mut self, _: i32) -> Result<[f32; 16], Self::Error> {
        Ok(locator())
    }
    fn level_segment(&mut self, o: [f32; 3], d: [f32; 3]) -> Result<Option<[f32; 3]>, Self::Error> {
        self.ops.push(format!("segment {o:?} {d:?}"));
        Ok(self.hit)
    }
    fn body_orientation(&mut self, h: u16) -> Result<[f32; 16], Self::Error> {
        self.ops.push(format!("orientation {h}"));
        let mut m = [0.0; 16];
        m[0] = 7.0;
        m[12] = 99.0;
        Ok(m)
    }
    fn set_desired_direction(&mut self, h: u16, d: [f32; 3]) -> Result<(), Self::Error> {
        self.ops.push(format!("desired {h} {d:?}"));
        Ok(())
    }
    fn move_body(&mut self, h: u16, p: [f32; 3], o: [f32; 16]) -> Result<(), Self::Error> {
        self.ops
            .push(format!("move {h} {p:?} m11={} t={:?}", o[0], &o[12..15]));
        Ok(())
    }
    fn present_characters(&mut self, area: i32) -> Result<Option<Vec<AreaCharacter>>, Self::Error> {
        Ok((area == custom_hash("yard")).then(|| self.area.clone()))
    }
    fn clear_cannot_die(&mut self, h: u16) -> Result<(), Self::Error> {
        self.ops.push(format!("mortal {h}"));
        Ok(())
    }
    fn kill(&mut self, h: u16) -> Result<(), Self::Error> {
        self.ops.push(format!("kill {h}"));
        Ok(())
    }
    fn terminate(&mut self, h: u16) -> Result<(), Self::Error> {
        self.ops.push(format!("terminate {h}"));
        Ok(())
    }
    fn release_npc_grip(&mut self, h: u16) -> Result<(), Self::Error> {
        self.ops.push(format!("release {h}"));
        Ok(())
    }
}
const CHARACTER: EntityFacts = EntityFacts {
    handle: 1,
    character: Some(0.5),
};
const PROP: EntityFacts = EntityFacts {
    handle: 2,
    character: None,
};

#[test]
fn set_position_snaps_and_chooses_orientation() {
    let mut host = Host {
        avatars: [None, Some(CHARACTER), None, None],
        entities: vec![(custom_hash("crate"), PROP)],
        hit: Some([4.0, 1.0, 6.0]),
        ..Default::default()
    };
    read("setPosition", &[("id", "PLAYER2"), ("area", "spot")])
        .execute(&mut host)
        .unwrap();
    read(
        "setposition",
        &[
            ("id", "player2"),
            ("area", "spot"),
            ("facingDirection", "true"),
        ],
    )
    .execute(&mut host)
    .unwrap();
    read("setposition", &[("id", "crate"), ("area", "spot")])
        .execute(&mut host)
        .unwrap();
    host.hit = None;
    read("setposition", &[("id", "player2"), ("area", "spot")])
        .execute(&mut host)
        .unwrap();
    // Players without an avatar and unknown IDs are ignored before any lookup.
    read("setposition", &[("id", "player1"), ("area", "spot")])
        .execute(&mut host)
        .unwrap();
    read("setposition", &[("id", "ghost"), ("area", "spot")])
        .execute(&mut host)
        .unwrap();
    assert_eq!(
        host.ops,
        [
            "segment [4.0, 3.0, 6.0] [0.0, -5.0, 0.0]",
            "orientation 1",
            "move 1 [4.0, 0.5, 6.0] m11=7 t=[0.0, 0.0, 0.0]",
            "segment [4.0, 3.0, 6.0] [0.0, -5.0, 0.0]",
            "desired 1 [-0.0, -0.0, 1.0]",
            "move 1 [4.0, 0.5, 6.0] m11=1 t=[0.0, 0.0, 0.0]",
            "segment [4.0, 3.0, 6.0] [0.0, -5.0, 0.0]",
            "move 2 [4.0, 1.0, 6.0] m11=1 t=[0.0, 0.0, 0.0]",
            "segment [4.0, 3.0, 6.0] [0.0, -5.0, 0.0]",
            "move 1 [4.0, 2.0, 6.0] m11=1 t=[0.0, 0.0, 0.0]",
        ]
    );
    assert_eq!(
        read("setposition", &[]).execute(&mut host),
        Err(EntityCommandError::MissingId)
    );
}

#[test]
fn remove_and_kill_differ_in_mortality_and_termination() {
    let area = vec![
        AreaCharacter {
            handle: 5,
            type_id: custom_hash("orc"),
            original_faction: 0,
        },
        AreaCharacter {
            handle: 6,
            type_id: custom_hash("imp"),
            original_faction: 1,
        },
        AreaCharacter {
            handle: 7,
            type_id: custom_hash("elf"),
            original_faction: 4,
        },
    ];
    let mut host = Host {
        entities: vec![
            (custom_hash("boss"), CHARACTER),
            (custom_hash("crate"), PROP),
        ],
        area,
        ..Default::default()
    };
    for (tag, attrs) in [
        ("remove", vec![("id", "boss")]),
        ("remove", vec![("id", "boss"), ("force", "true")]),
        ("remove", vec![("id", "crate")]),
        ("kill", vec![("id", "boss")]),
        ("kill", vec![("id", "crate")]),
        ("kill", vec![("id", "nobody")]),
        (
            "remove",
            vec![("area", "yard"), ("type", "orc"), ("factions", "evil")],
        ),
        ("kill", vec![("area", "yard"), ("type", "any")]),
    ] {
        read(tag, &attrs).execute(&mut host).unwrap();
    }
    assert_eq!(
        host.ops,
        [
            "kill 1",
            "terminate 1",
            "mortal 1",
            "kill 1",
            "terminate 1",
            "kill 2",
            "mortal 1",
            "kill 1",
            "kill 2",
            "kill 5",
            "terminate 5",
            "kill 6",
            "terminate 6",
            "mortal 5",
            "kill 5",
            "mortal 6",
            "kill 6",
            "mortal 7",
            "kill 7",
        ]
    );
    // Empty ID is not specific; default area 0 is a missing dictionary key.
    assert_eq!(
        read("kill", &[("id", "")]).execute(&mut host),
        Err(EntityCommandError::MissingArea(0))
    );
    assert!(EntityCommand::read("kill", [("factions", "orcs")]).is_err());
    // FacingDirection is not a Kill property and is ignored.
    assert!(EntityCommand::read("kill", [("facingdirection", "x")]).is_ok());
}

#[test]
fn release_grip_targets_like_kill_without_mortality_changes() {
    let mut host = Host {
        entities: vec![(custom_hash("troll"), CHARACTER)],
        area: vec![
            AreaCharacter {
                handle: 5,
                type_id: custom_hash("orc"),
                original_faction: 0,
            },
            AreaCharacter {
                handle: 6,
                type_id: custom_hash("imp"),
                original_faction: 1,
            },
        ],
        ..Default::default()
    };
    for attrs in [
        vec![("id", "troll")],
        vec![("id", "nobody")],
        vec![("area", "yard"), ("type", "imp")],
        vec![("area", "yard"), ("type", "any")],
    ] {
        read("ReleaseGrip", &attrs).execute(&mut host).unwrap();
    }
    assert_eq!(
        host.ops,
        ["release 1", "release 6", "release 5", "release 6"]
    );
    assert_eq!(
        read("releasegrip", &[]).execute(&mut host),
        Err(EntityCommandError::MissingArea(0))
    );
}
