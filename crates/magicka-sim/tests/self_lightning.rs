use magicka_sim::{
    damage::Damage,
    self_lightning::{element_index, SelfLightning, SelfLightningHost},
    spells::{Elements as E, Spell},
};

#[derive(Default)]
struct Host {
    log: Vec<String>,
    damages: Option<[Damage; 5]>,
}
impl SelfLightningHost for Host {
    fn start_effect(&mut self, index: usize, name: &'static str) {
        self.log.push(format!("effect {index} {name}"));
    }
    fn damage_self(&mut self, damages: [Damage; 5], _: f64) {
        self.log.push("damage".into());
        self.damages = Some(damages);
    }
}

#[test]
fn element_index_rounds_log2_of_the_mask() {
    assert_eq!(element_index(E::LIGHTNING.0), 4);
    assert_eq!(element_index(E::LIGHTNING.0 | E::WATER.0), 4); // log2 18 = 4.17
    assert_eq!(element_index(E::LIGHTNING.0 | E::FIRE.0), 5); // log2 24 = 4.58
    assert_eq!(element_index(0xFFFF), 11);
}

#[test]
fn self_lightning_starts_effect_then_damages_owner_once() {
    let mut spell = Spell {
        element: E::LIGHTNING,
        ..Default::default()
    };
    spell.mag[4] = 1.0;
    let mut host = Host::default();
    let s = SelfLightning::cast(spell, true, 3.0, &mut host).unwrap();
    assert_eq!(host.log, ["effect 4 self_lightning", "damage"]);
    let d = host.damages.unwrap();
    assert!(
        d.iter()
            .any(|d| d.element == E::LIGHTNING.0 && d.amount > 0.0),
        "{d:?}"
    );
    assert!(s.lifecycle.active);
}
