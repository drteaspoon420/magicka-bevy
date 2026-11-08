//! Trigger Damage action (individual public damage calls, not a spell collection).
use crate::{
    ai_event::clr_enum,
    entity_command::CharacterSelector,
    presence::factions,
    scene_script::XmlElement,
    script_attributes::{boolean, identifier, number, ScriptReadError},
    toggle_action::ELEMENTS,
};
use magicka_sim::damage::Damage;

const ATTACKS: &[(&str, i32)] = &[
    ("damage", 1),
    ("knockdown", 2),
    ("pushed", 4),
    ("knockback", 6),
    ("piercing", 8),
    ("armourpiercing", 16),
    ("status", 32),
    ("entanglement", 64),
    ("stun", 128),
    ("bleed", 256),
    ("numberoftypes", 512),
];

#[derive(Clone, Debug)]
pub struct ScriptDamageAction {
    pub delay: f32,
    pub target: CharacterSelector,
    pub source: i32,
    pub ignore_shields: bool,
    pub damages: Vec<Damage>,
}
impl ScriptDamageAction {
    pub fn read(node: &XmlElement) -> Result<Self, ScriptReadError> {
        let (mut delay, mut id, mut area, mut type_id, mut faction_mask, mut source) =
            (0., None, 0, 0, 0, 0);
        let mut ignore_shields = true;
        for (key, value) in &node.attributes {
            match key.to_ascii_lowercase().as_str() {
                "delay" => delay = number(value, key)?,
                "id" => id = (!value.is_empty()).then(|| identifier(value)).transpose()?,
                "area" => area = identifier(value)?,
                "type" => type_id = identifier(value)?,
                "factions" => faction_mask = factions(value, key)?,
                "source" => source = identifier(value)?,
                "ignoreshields" => ignore_shields = boolean(value, key)?,
                _ => {}
            }
        }
        let mut damages = Vec::new();
        for child in &node.children {
            if !child.name.eq_ignore_ascii_case("damage") {
                continue;
            }
            let mut damage = Damage::default();
            for (key, value) in &child.attributes {
                match key.to_ascii_lowercase().as_str() {
                    "attackproperty" => {
                        damage.attack = clr_enum(value, ATTACKS, key, u16::MAX as i32)? as u16
                    }
                    "element" => damage.element = clr_enum(value, ELEMENTS, key, i32::MAX)? as u32,
                    "amount" => damage.amount = number(value, key)?,
                    "magnitude" => damage.magnitude = number(value, key)?,
                    _ => {}
                }
            }
            damages.push(damage);
        }
        Ok(Self {
            delay,
            target: id.map_or(
                CharacterSelector::Area {
                    area,
                    type_id,
                    factions: faction_mask,
                },
                CharacterSelector::Id,
            ),
            source,
            ignore_shields,
            damages,
        })
    }
}
