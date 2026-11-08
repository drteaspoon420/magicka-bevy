use magicka_content::skinned::IDENTITY;
use magicka_runtime::{scene_loading::custom_hash, spawn_item::*};

#[derive(Default)]
struct Host {
    client: bool,
    server: bool,
    calls: Vec<String>,
    placements: Vec<(u16, [f32; 3], [f32; 16])>,
    messages: Vec<SpawnItemMessage>,
}
impl SpawnItemHost for Host {
    type Error = String;
    fn is_client(&self) -> bool {
        self.client
    }
    fn is_server(&self) -> bool {
        self.server
    }
    fn new_placeholder(&mut self) -> Result<u16, String> {
        self.calls.push("allocate".into());
        Ok(42)
    }
    fn load_item(&mut self, path: &str) -> Result<(), String> {
        self.calls.push(format!("load:{path}"));
        Ok(())
    }
    fn locator(&mut self, area: i32) -> Result<[f32; 16], String> {
        self.calls.push(format!("locator:{area}"));
        let mut m = IDENTITY;
        m[0] = 0.0;
        m[2] = 1.0;
        m[8] = -1.0;
        m[10] = 0.0;
        m[12..15].copy_from_slice(&[3.0, 4.0, 5.0]);
        Ok(m)
    }
    fn copy_and_detach(
        &mut self,
        handle: u16,
        template: i32,
        on_pickup: i32,
    ) -> Result<(), String> {
        self.calls
            .push(format!("detach:{handle}:{template}:{on_pickup}"));
        Ok(())
    }
    fn move_to(&mut self, handle: u16, p: [f32; 3], m: [f32; 16]) -> Result<(), String> {
        self.calls.push("move".into());
        self.placements.push((handle, p, m));
        Ok(())
    }
    fn add_entity(&mut self, handle: u16) -> Result<(), String> {
        self.calls.push(format!("add:{handle}"));
        Ok(())
    }
    fn configure(
        &mut self,
        _: u16,
        immovable: bool,
        tractor: bool,
        id: Option<i32>,
    ) -> Result<(), String> {
        self.calls
            .push(format!("configure:{immovable}:{tractor}:{id:?}"));
        Ok(())
    }
    fn enable_body(&mut self, handle: u16) -> Result<(), String> {
        self.calls.push(format!("enable:{handle}"));
        Ok(())
    }
    fn attach_if_present(&mut self, _: u16, part: i32) -> Result<(), String> {
        self.calls.push(format!("attach:{part}"));
        Ok(())
    }
    fn send_spawn(&mut self, msg: SpawnItemMessage) -> Result<(), String> {
        self.calls.push("send".into());
        self.messages.push(msg);
        Ok(())
    }
}

#[test]
fn placeholder_is_allocated_once_and_reused_even_when_nr_is_many() {
    let a = SpawnItemAction::read([
        ("Item", "weapon_kris"),
        ("Area", "spawn_item6"),
        ("Nr", "8"),
        ("PhysicsEnabled", "true"),
        ("IgnoreTractorPull", "true"),
        ("OnPickup", "Collected"),
        ("ID", "UniqueItem"),
        ("AttachTo", "moving_floor"),
    ])
    .unwrap();
    let mut state = SpawnItemState::default();
    let mut host = Host {
        server: true,
        ..Default::default()
    };
    a.initialize(&mut state, &mut host).unwrap();
    a.execute(&state, &mut host).unwrap();
    a.execute(&state, &mut host).unwrap();
    assert_eq!(host.calls.iter().filter(|s| *s == "allocate").count(), 1);
    assert_eq!(
        host.calls.iter().filter(|s| s.starts_with("load:")).count(),
        1
    );
    assert_eq!(host.placements.len(), 2);
    assert_eq!(host.calls[1], "load:Data/Items/Wizard/weapon_kris");
    assert_eq!(
        &host.calls[4..9],
        &[
            "move",
            "add:42",
            &format!("configure:false:true:Some({})", custom_hash("uniqueitem")),
            "enable:42",
            &format!("attach:{}", custom_hash("moving_floor"))
        ]
    );
    for message in host.messages {
        assert_eq!(message.handle, 42);
        assert_eq!(message.position, [3.0, 4.0, 5.0]);
        assert_eq!(&message.orientation[12..15], &[0.0; 3]);
        assert_eq!(message.orientation[2], 1.0);
        assert_eq!(message.orientation[8], -1.0);
        assert_eq!(message.on_pickup, custom_hash("collected"));
        assert!(message.physics_enabled && message.ignore_tractor_pull);
    }
}

#[test]
fn clients_do_not_spawn_and_offline_defaults_are_immovable() {
    let a = SpawnItemAction::read([("item", "weapon_kris")]).unwrap();
    let mut host = Host {
        client: true,
        ..Default::default()
    };
    a.execute(&SpawnItemState::default(), &mut host).unwrap();
    assert!(host.calls.is_empty());
    host.client = false;
    assert_eq!(
        a.execute(&SpawnItemState::default(), &mut host),
        Err(SpawnItemError::MissingPlaceholder)
    );
    let mut state = SpawnItemState::default();
    a.initialize(&mut state, &mut host).unwrap();
    a.execute(&state, &mut host).unwrap();
    assert!(host.calls.contains(&"configure:true:false:None".into()));
    assert!(host.messages.is_empty());
    assert!(!host.calls.iter().any(|s| s.starts_with("attach:")));
}

#[test]
fn xml_read_lowercases_identifiers_before_native_setters_and_validates_attributes() {
    let a = SpawnItemAction::read([("Area", "Start"), ("item", "Weapon"), ("id", "")]).unwrap();
    assert_eq!(a.area, custom_hash("start"));
    assert_ne!(a.area, custom_hash("Start"));
    assert_eq!(a.item_hash, custom_hash("weapon"));
    assert_eq!(a.unique_id, None);
    assert!(SpawnItemAction::read([("PhysicsEnabled", "yes")]).is_err());
    assert!(SpawnItemAction::read([("Nr", "bad")]).is_err());
    assert!(SpawnItemAction::read([("Area", "é")]).is_err());
}
