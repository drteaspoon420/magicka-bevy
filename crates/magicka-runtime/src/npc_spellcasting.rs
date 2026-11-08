//! NPC CastSpell queue and chant state, separate from animation/effect hosting.
//! Used by live combat selection and the shared Character spell-effect host.
use crate::animation_controller::animation_value;
use magicka_content::character::{Ability, AbilityKind};
use magicka_sim::{
    damage::CastType,
    spells::{self, Elements, QueueEvent, Spell},
};

#[derive(Clone, Debug, PartialEq)]
pub struct SpellAbility {
    pub min_range: f32,
    pub max_range: f32,
    pub arc: f32,
    pub chant_speed: f32,
    pub power: f32,
    pub cast_type: CastType,
    pub elements: Vec<Elements>,
    pub animations: Vec<u16>,
}

impl SpellAbility {
    /// CastSpell.GetDesiredDirection preserves the owner's direction for self
    /// targets, normalizes other deltas and sets only Z on a tiny delta.
    pub fn desired_direction(
        &self,
        owner: [f32; 3],
        target: [f32; 3],
        owner_direction: [f32; 3],
        target_is_owner: bool,
    ) -> [f32; 3] {
        if target_is_owner {
            return owner_direction;
        }
        let mut delta = std::array::from_fn(|i| target[i] - owner[i]);
        let length = delta.iter().map(|v| v * v).sum::<f32>().sqrt();
        if length > 1e-6 {
            delta = delta.map(|v| v / length);
        } else {
            delta[2] = 1.;
        }
        delta
    }

    pub fn from_content(ability: &Ability) -> Result<Option<Self>, String> {
        let AbilityKind::CastSpell {
            min_range,
            max_range,
            arc,
            chant_speed,
            power,
            cast_type,
            elements,
        } = &ability.kind
        else {
            return Ok(None);
        };
        let cast_type = match cast_type {
            0 => CastType::Blast,
            1 => CastType::Force,
            2 => CastType::Area,
            3 => CastType::SelfCast,
            4 => CastType::Weapon,
            5 => CastType::Magick,
            value => return Err(format!("unreconstructed CastType {value}")),
        };
        Ok(Some(Self {
            min_range: *min_range,
            max_range: *max_range,
            arc: *arc,
            chant_speed: *chant_speed,
            power: *power,
            cast_type,
            elements: elements.iter().map(|&e| Elements(e as u32)).collect(),
            animations: ability
                .animation_keys
                .iter()
                .map(|name| animation_value(name).ok_or_else(|| format!("animation {name}")))
                .collect::<Result<_, _>>()?,
        }))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AbilityContext {
    /// Reference equality of BusyAbility and NextAbility, not ability type.
    pub busy_is_next: bool,
    /// A non-null CurrentSpell blocks chanting even when inactive.
    pub current_spell: bool,
    pub attacking: bool,
    pub distance_squared: f32,
}

#[derive(Clone, Debug)]
pub struct NpcSpellcasting {
    pub queue: Vec<Spell>,
    pub chant_cooldown: f32,
    pub prepared: Spell,
    pub cast_type: CastType,
    pub power: f32,
}

impl Default for NpcSpellcasting {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            chant_cooldown: 1.,
            prepared: Spell::default(),
            cast_type: CastType::Blast,
            power: 0.,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Invocation {
    pub spell: Spell,
    pub cast_type: CastType,
}

impl NpcSpellcasting {
    /// NonPlayerCharacter.Update, separately from AIManager's scheduled dt.
    pub fn tick(&mut self, dt: f32) {
        self.chant_cooldown -= dt;
    }

    /// Agent.ChooseAbility's queue compatibility filter. A nonempty queue
    /// excludes non-casting abilities; Equatable accepts compatible prefixes.
    pub fn allows(&self, ability: Option<&SpellAbility>) -> bool {
        self.queue.is_empty()
            || ability.is_some_and(|a| spells::equatable(&a.elements, &self.queue))
    }

    /// CastSpell.Update runs after the agent's state update. Returns attempted
    /// conjures (including merges/rejections) for a future sound/network host.
    // The negated comparisons transcribe the native tests, NaN included.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    pub fn update_ability(
        &mut self,
        ability: &SpellAbility,
        context: AbilityContext,
        queue_events: &mut Vec<QueueEvent>,
    ) -> Vec<(Elements, bool)> {
        if context.busy_is_next
            || context.current_spell
            || !(self.chant_cooldown < 0. && self.queue.len() < ability.elements.len())
            || context.attacking
            || !(context.distance_squared <= ability.max_range * ability.max_range * 1.25 * 1.25)
        {
            return Vec::new();
        }
        self.chant_cooldown = ability.chant_speed;
        let elements = if ability.cast_type == CastType::Weapon {
            ability.elements.clone()
        } else {
            vec![ability.elements[self.queue.len()]]
        };
        elements
            .into_iter()
            .map(|element| {
                let grew = spells::try_add_to_queue(
                    &mut self.queue,
                    spells::SPELL_QUEUE_CAP,
                    Spell::default_for(element),
                    queue_events,
                );
                (element, grew)
            })
            .collect()
    }

    /// CastSpell.InternalExecute prepares the spell and chooses an attack clip.
    /// The caller handles Ability.Execute's BusyAbility and animation startup.
    pub fn execute(
        &mut self,
        ability: &SpellAbility,
        context: AbilityContext,
        choose_animation: impl FnOnce(usize) -> usize,
    ) -> Result<Option<u16>, String> {
        if context.busy_is_next || self.queue.len() < ability.elements.len() || context.attacking {
            return Ok(None);
        }
        self.power = ability.power;
        self.set_cast_type(ability.cast_type);
        self.chant_cooldown = ability.chant_speed;
        if ability.animations.is_empty() {
            return Err("CastSpell has no attack animations".into());
        }
        let index = choose_animation(ability.animations.len());
        let animation = *ability
            .animations
            .get(index)
            .ok_or("invalid animation selection")?;
        Ok(Some(animation))
    }

    /// NPC CastType setter combines only on a change to a non-Weapon,
    /// non-None type. Magicks use the NPC's null-player unlock bypass.
    pub fn set_cast_type(&mut self, cast_type: CastType) {
        if cast_type != CastType::Weapon
            && cast_type != CastType::Blast
            && cast_type != self.cast_type
        {
            if cast_type == CastType::Magick {
                static TREE: std::sync::OnceLock<spells::MagickTree> = std::sync::OnceLock::new();
                TREE.get_or_init(spells::MagickTree::default)
                    .combine_magick(&mut self.queue, |_| true);
            }
            self.prepared = spells::combine(&self.queue);
            self.queue.clear();
        }
        self.cast_type = cast_type;
    }

    /// Animation action's first-execution/current-pointer gate, followed by
    /// NonPlayerCharacter.CastSpell's special Weapon combination path.
    pub fn animation_cast(&mut self, first: bool, current_spell: bool) -> Option<Invocation> {
        if !first || current_spell {
            return None;
        }
        let spell = if self.cast_type == CastType::Weapon {
            let combined = spells::combine(&self.queue);
            self.queue.clear();
            combined
        } else {
            self.prepared
        };
        if spell.element == Elements::ALL && self.cast_type != CastType::Weapon {
            self.cast_type = CastType::Blast;
        }
        self.prepared = Spell::default();
        Some(Invocation {
            spell,
            cast_type: self.cast_type,
        })
    }
}
