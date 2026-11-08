//! Character, player, boss, achievement and DLC trigger conditions evaluated
//! over explicit `WorldConditionFacts`.
use crate::{
    ai_event::clr_enum,
    script_attributes::{boolean, comparison, identifier, number, ScriptReadError},
    toggle_action::{CAST_TYPES, ELEMENTS},
};
use magicka_sim::spells::{Elements, Spell};

pub const STATUS_EFFECTS: &[(&str, i32)] = &[
    ("none", 0),
    ("burning", 1),
    ("wet", 2),
    ("frozen", 4),
    ("cold", 8),
    ("poisoned", 0x10),
    ("healing", 0x20),
    ("life", 0x20),
    ("greased", 0x40),
    ("steamed", 0x80),
    ("bleeding", 0x100),
    ("numberoftypes", 0x200),
];
pub const DLCS: &[(&str, i32)] = &[("wizardhat", 0), ("vietnam", 1)];

#[derive(Clone, Debug, PartialEq)]
pub enum WorldConditionKind {
    CharacterHealth {
        id: i32,
        compare: i8,
        health: f32,
    },
    InteractorIs {
        id: i32,
    },
    /// `name` is `None` for a null or empty Name (always false).
    RobeInUse {
        name: Option<i32>,
        player: i32,
    },
    BossHealth {
        compare: i8,
        value: f32,
    },
    AnyPlayerHasDied,
    BossDead,
    CharacterCasting {
        id: i32,
        cast_type: i32,
        element: i32,
    },
    HasAchievement {
        name: Option<String>,
    },
    HasDlc {
        dlc: i32,
    },
    /// The configured status is parsed but unused by InternalMet.
    PlayersHaveStatusEffect {
        status: i32,
    },
    PlayerHasElement {
        element: i32,
    },
    /// CurrentSaveData.Looped; throws outside GameType.Campaign.
    IsCampaignLooped,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorldCondition {
    pub invert: bool,
    pub kind: WorldConditionKind,
}

pub const TAGS: &[&str] = &[
    "characterhealth",
    "interactoris",
    "robeinuse",
    "bosshealth",
    "anyplayerhasdied",
    "bossdead",
    "charactercasting",
    "hasachievement",
    "hasdlc",
    "playershavestatuseffect",
    "playerhaselement",
    "iscampaignlooped",
];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConditionAvatar {
    /// Avatar.Template.ID.
    pub template_id: i32,
    /// StatusEffects bitmask from HasStatus.
    pub statuses: i32,
    /// SpellQueue element masks in queue order.
    pub queue: Vec<u32>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConditionPlayer {
    pub playing: bool,
    pub avatar: Option<ConditionAvatar>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterHealthFacts {
    pub dead: bool,
    pub hit_points: f32,
    pub max_hit_points: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BossFacts {
    pub normalized_health: f32,
    pub dead: bool,
}

pub trait WorldConditionFacts {
    /// Entity.GetByID(id) as Character.
    fn character_health(&self, id: i32) -> Option<CharacterHealthFacts>;
    /// Character.CurrentSpell (cast type and spell) of Entity.GetByID(id).
    fn current_spell(&self, id: i32) -> Option<(i32, Spell)>;
    /// UniqueID of the sending Character.
    fn sender_unique_id(&self, sender: u16) -> Option<i32>;
    /// Game.Instance.Players in slot order.
    fn players(&self) -> Vec<ConditionPlayer>;
    /// PlayState.BossFight, if any.
    fn boss_fight(&self) -> Option<BossFacts>;
    /// PlayState.RecentPlayState.DiedInLevel.
    fn died_in_level(&self) -> bool;
    fn has_achievement(&self, name: Option<&str>) -> bool;
    /// Helper.CheckDLC.
    fn has_dlc(&self, dlc: i32) -> bool;
    /// The current save slot's Looped flag; `None` outside GameType.Campaign.
    fn campaign_looped(&self) -> Option<bool>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum WorldConditionError {
    /// RobeInUse Player outside 0..=player count indexes out of range.
    PlayerIndex(i32),
    /// BossHealth's switch throws for compare methods other than LESS/EQUAL/GREATER.
    InvalidCompare(i8),
    MissingWorldFacts,
    /// "IsCampaignLooped can only be used in gametype campaign!"
    NotCampaign,
}

impl WorldCondition {
    pub fn read<'a>(
        tag: &str,
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        use WorldConditionKind::*;
        let mut kind = match tag.to_ascii_lowercase().as_str() {
            "characterhealth" => CharacterHealth {
                id: 0,
                compare: 0,
                health: 0.0,
            },
            "interactoris" => InteractorIs { id: 0 },
            "robeinuse" => RobeInUse {
                name: None,
                player: 0,
            },
            "bosshealth" => BossHealth {
                compare: 0,
                value: 0.0,
            },
            "anyplayerhasdied" => AnyPlayerHasDied,
            "bossdead" => BossDead,
            "charactercasting" => CharacterCasting {
                id: -1,
                cast_type: 0,
                element: 0,
            },
            "hasachievement" => HasAchievement { name: None },
            "hasdlc" => HasDlc { dlc: 0 },
            "iscampaignlooped" => IsCampaignLooped,
            "playershavestatuseffect" => PlayersHaveStatusEffect { status: 0 },
            "playerhaselement" => PlayerHasElement { element: 0 },
            _ => return Err(ScriptReadError::UnsupportedTag(tag.into())),
        };
        let mut invert = false;
        for (key, value) in attrs {
            match (&mut kind, key.to_ascii_lowercase().as_str()) {
                (_, "invert") => invert = boolean(value, key)?,
                (CharacterHealth { id, .. }, "id")
                | (InteractorIs { id }, "id")
                | (CharacterCasting { id, .. }, "id") => *id = identifier(value)?,
                (CharacterHealth { compare, .. }, "comparemethod")
                | (BossHealth { compare, .. }, "comparemethod") => {
                    *compare = comparison(value, key)?
                }
                (CharacterHealth { health, .. }, "health") => {
                    let v: f32 = number(value, key)?;
                    if v > 1.0 || v < 0.0 {
                        return Err(ScriptReadError::InvalidAttribute(key.into()));
                    }
                    *health = v;
                }
                (RobeInUse { name, .. }, "name") => {
                    *name = (!value.is_empty()).then(|| identifier(value)).transpose()?
                }
                (RobeInUse { player, .. }, "player") => *player = number(value, key)?,
                (BossHealth { value: v, .. }, "value") => *v = number(value, key)?,
                (CharacterCasting { cast_type, .. }, "casttype") => {
                    *cast_type = clr_enum(value, CAST_TYPES, key, i32::MAX)?
                }
                (CharacterCasting { element, .. }, "element")
                | (PlayerHasElement { element }, "element") => {
                    *element = clr_enum(value, ELEMENTS, key, i32::MAX)?
                }
                (HasAchievement { name }, "name") => {
                    if !value.is_ascii() {
                        return Err(ScriptReadError::UnsupportedName(value.into()));
                    }
                    *name = Some(value.to_ascii_lowercase())
                }
                (HasDlc { dlc }, "type") => *dlc = clr_enum(value, DLCS, key, 255)?,
                (PlayersHaveStatusEffect { status }, "statuseffect") => {
                    *status = clr_enum(value, STATUS_EFFECTS, key, i16::MAX as i32)?
                }
                _ => {}
            }
        }
        Ok(Self { invert, kind })
    }

    pub fn is_met(
        &self,
        facts: &dyn WorldConditionFacts,
        sender: Option<u16>,
    ) -> Result<bool, WorldConditionError> {
        use WorldConditionKind::*;
        // BossHealth/BossDead override IsMet: no boss fight is false even inverted.
        if matches!(self.kind, BossHealth { .. } | BossDead) && facts.boss_fight().is_none() {
            return Ok(false);
        }
        let met = match self.kind {
            CharacterHealth {
                id,
                compare,
                health,
            } => {
                let ratio = match facts.character_health(id) {
                    Some(c) if !c.dead => c.hit_points / c.max_hit_points,
                    _ => 0.0,
                };
                match compare {
                    -1 => health > ratio,
                    0 => (health - ratio).abs() < 0.0001,
                    1 => health < ratio,
                    _ => false,
                }
            }
            InteractorIs { id } => sender
                .and_then(|s| facts.sender_unique_id(s))
                .is_some_and(|u| u == id),
            RobeInUse { name, player } => match name {
                None => false,
                Some(name) => {
                    let players = facts.players();
                    let wears = |p: &ConditionPlayer| {
                        p.playing && p.avatar.as_ref().is_some_and(|a| a.template_id == name)
                    };
                    if player == 0 {
                        players.iter().any(wears)
                    } else {
                        let slot = usize::try_from(player - 1)
                            .ok()
                            .filter(|s| *s < players.len())
                            .ok_or(WorldConditionError::PlayerIndex(player))?;
                        wears(&players[slot])
                    }
                }
            },
            BossHealth { compare, value } => {
                let health = facts.boss_fight().unwrap().normalized_health;
                match compare {
                    -1 => health < value,
                    0 => (health - value).abs() <= 1e-6,
                    1 => health > value,
                    other => return Err(WorldConditionError::InvalidCompare(other)),
                }
            }
            AnyPlayerHasDied => facts.died_in_level(),
            BossDead => facts.boss_fight().unwrap().dead,
            CharacterCasting {
                id,
                cast_type,
                element,
            } => facts.current_spell(id).is_some_and(|(cast, spell)| {
                cast == cast_type && spell.contains_element(Elements(element as u32))
            }),
            HasAchievement { ref name } => facts.has_achievement(name.as_deref()),
            HasDlc { dlc } => facts.has_dlc(dlc),
            PlayersHaveStatusEffect { .. } => facts
                .players()
                .iter()
                .any(|p| p.playing && p.avatar.as_ref().is_some_and(|a| a.statuses & 0x1ff != 0)),
            IsCampaignLooped => facts
                .campaign_looped()
                .ok_or(WorldConditionError::NotCampaign)?,
            PlayerHasElement { element } => facts.players().iter().any(|p| {
                p.playing
                    && p.avatar
                        .as_ref()
                        .is_some_and(|a| a.queue.iter().any(|e| e & element as u32 != 0))
            }),
        };
        Ok(self.invert ^ met)
    }
}
