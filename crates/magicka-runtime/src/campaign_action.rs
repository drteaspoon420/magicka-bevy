//! Assorted campaign trigger actions (research/campaign-actions.md): tutorial
//! telemetry, profile stats, conditional achievements, ambient/music focus,
//! camera bias/follow/position/effects, flash, highlights, in-level titles,
//! UI toggle, overkill, faction changes, status clearing, teleport, scripted
//! magicks and the avatar equipment sections/items. Each tag's attribute set
//! follows its native class's properties (Action.Read lowercases strings).
use crate::{
    ai_event::clr_enum,
    script_attributes::{boolean, identifier, number, ScriptReadError},
};

pub const TAGS: &[&str] = &[
    "telemetrytutorialbegin",
    "telemetrytutorialend",
    "telemetrytutorialskip",
    "telemetrytutorialstep",
    "foundmoose",
    "killedapony",
    "conditionalunlockachievement",
    "setvolume",
    "setmusicfocus",
    "camerabias",
    "camerafollow",
    "cameraposition",
    "flash",
    "highlightanim",
    "highlightphys",
    "displaytitleinlevel",
    "disableui",
    "enableui",
    "addcameraeffect",
    "removecameraeffect",
    "overkill",
    "setfaction",
    "removeallstatuseffects",
    "teleport",
    "magick",
    "beginunequipweaponssection",
    "endunequipweaponssection",
    "begindefaultavatarsection",
    "enddefaultavatarsection",
    "assignitem",
    "additemtoavatars",
    "removeitemfromavatars",
    "wizardcontrol",
    "scrollscore",
    "rollcredits",
];

/// An authored XML node kept for later synthesis (compared structurally).
#[derive(Clone, Debug)]
pub struct AuthoredNode(pub crate::scene_script::XmlElement);

impl PartialEq for AuthoredNode {
    fn eq(&self, other: &Self) -> bool {
        format!("{:?}", self.0) == format!("{:?}", other.0)
    }
}

/// SetFaction.Action (OperationAction).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Add,
    Remove,
    Set,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CampaignCommand {
    /// TutorialUtils telemetry (Steam stats only): local no-op.
    Telemetry,
    /// Profile.FoundAMoose / KilledAPony (secret stats).
    FoundMoose(String),
    KilledAPony(String),
    ConditionalAchievement {
        achievement: String,
        victim: i32,
    },
    /// GameScene.ChangeAmbientSoundVolume(id, volume).
    SetVolume {
        id: i32,
        volume: f32,
    },
    /// AudioManager.SetMusicFocus(locator, radius, clearTo).
    SetMusicFocus {
        locator: i32,
        radius: f32,
        clear_to: f32,
    },
    CameraBias {
        bias: [f32; 3],
        time: f32,
    },
    CameraFollow(i32),
    /// Lowercased target locator (ground position when missing) + offset.
    CameraPosition {
        target: i32,
        offset: [f32; 3],
        magnification: f32,
    },
    Flash(f32),
    /// `part/child` path hashes.
    HighlightAnim {
        path: Vec<i32>,
        time: f32,
    },
    HighlightPhys {
        id: i32,
        time: f32,
    },
    DisplayTitle {
        title: String,
        subtitle: String,
        display_time: f32,
        fade_in: f32,
        fade_out: f32,
        no_translation: bool,
        align: crate::text_geometry::TextAlign,
    },
    SetUi(bool),
    /// SpawnCloneWizards: the authored node (its attributes and AI event
    /// children become one Spawn per living player).
    SpawnCloneWizards(AuthoredNode),
    /// ScrollScore: FloatingTextManager.AddNumber(value, at area/locator +
    /// offset, ttl, unlocked, white). Area hashes the lowercased name.
    ScrollScore {
        value: f32,
        area: i32,
        offset: [f32; 3],
        ttl: f32,
    },
    /// RollCredits: Credits.Initialize(File) then Start (File is lowercased
    /// by Action.Read; empty = Credits.xml).
    RollCredits(String),
    /// WizardControl: PlayState.StaffLight (StaffLightOn/Off; any other
    /// WizardAction value does nothing). Natively the flag is write-only.
    StaffLight(Option<bool>),
    /// Camera.StartVisualEffect / RemoveEffect (0 = all).
    CameraEffect {
        effect: i32,
        add: bool,
    },
    /// Specific ID, or area characters matching type ("any") or faction.
    Overkill {
        id: Option<i32>,
        type_hash: i32,
        area: i32,
        factions: i32,
    },
    SetFaction {
        /// player1..4 → 0..3.
        player: Option<usize>,
        id: Option<i32>,
        faction: i32,
        area: i32,
        operation: Operation,
        type_hash: i32,
    },
    RemoveAllStatusEffects,
    Teleport {
        id: i32,
        target: i32,
        effect: i32,
        sound: i32,
    },
    Magick {
        magick: u8,
        id: Option<i32>,
        position: [f32; 3],
    },
    UnequipWeaponsSection {
        begin: bool,
        animation: u16,
    },
    DefaultAvatarSection(bool),
    /// item name (lowercased); player 0 = any/area; animation 0 = none.
    AssignItem {
        item: String,
        player: usize,
        area: i32,
        animation: u16,
    },
    AddItemToAvatars {
        item: String,
        bone: String,
    },
    RemoveItemFromAvatars(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CampaignAction {
    pub delay: f32,
    pub command: CampaignCommand,
}

/// AssignItem.ALLID / Overkill.ANYID / SetFaction.ANYID.
pub fn any_id() -> i32 {
    crate::scene_loading::custom_hash("any")
}

fn vector(value: &str, key: &str) -> Result<[f32; 3], ScriptReadError> {
    let parts: Vec<f32> = value
        .split(',')
        .map(|p| p.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .map_err(|_| ScriptReadError::InvalidAttribute(key.into()))?;
    match parts.as_slice() {
        [x, y, z] => Ok([*x, *y, *z]),
        _ => Err(ScriptReadError::InvalidAttribute(key.into())),
    }
}

fn animation(value: &str, key: &str) -> Result<u16, ScriptReadError> {
    Ok(clr_enum(value, crate::dialog_enums::ANIMATIONS, key, i32::MAX)? as u16)
}

impl CampaignAction {
    /// SpawnCloneWizards keeps its node; Area, Animation and the Spawn
    /// properties are validated when each clone's Spawn is synthesized.
    pub fn read_clone_wizards(
        node: &crate::scene_script::XmlElement,
    ) -> Result<Self, ScriptReadError> {
        let mut delay = 0.0;
        for (key, value) in &node.attributes {
            if key.eq_ignore_ascii_case("delay") {
                delay = number(value, key)?;
            }
        }
        Ok(Self {
            delay,
            command: CampaignCommand::SpawnCloneWizards(AuthoredNode(node.clone())),
        })
    }

    pub fn read<'a>(
        tag: &str,
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        use CampaignCommand as C;
        let any = any_id();
        let mut command = match tag.to_ascii_lowercase().as_str() {
            "telemetrytutorialbegin"
            | "telemetrytutorialend"
            | "telemetrytutorialskip"
            | "telemetrytutorialstep" => C::Telemetry,
            "foundmoose" => C::FoundMoose(String::new()),
            "killedapony" => C::KilledAPony(String::new()),
            "conditionalunlockachievement" => C::ConditionalAchievement {
                achievement: String::new(),
                victim: 0,
            },
            "setvolume" => C::SetVolume { id: 0, volume: 0.0 },
            "setmusicfocus" => C::SetMusicFocus {
                locator: 0,
                radius: 0.0,
                clear_to: 1.0,
            },
            "camerabias" => C::CameraBias {
                bias: [0.0; 3],
                time: 20.0,
            },
            "camerafollow" => C::CameraFollow(0),
            "cameraposition" => C::CameraPosition {
                target: 0,
                offset: [0.0; 3],
                magnification: -1.0,
            },
            "flash" => C::Flash(0.5),
            "highlightanim" => C::HighlightAnim {
                path: Vec::new(),
                time: 1.0,
            },
            "highlightphys" => C::HighlightPhys { id: 0, time: 0.0 },
            "displaytitleinlevel" => C::DisplayTitle {
                title: String::new(),
                subtitle: String::new(),
                display_time: -1.0,
                fade_in: 1.0,
                fade_out: 1.0,
                no_translation: false,
                align: crate::text_geometry::TextAlign::Center,
            },
            "disableui" => C::SetUi(false),
            "enableui" => C::SetUi(true),
            "wizardcontrol" => C::StaffLight(Some(true)),
            "rollcredits" => C::RollCredits(String::new()),
            "scrollscore" => C::ScrollScore {
                value: 0.0,
                area: 0,
                offset: [0.0; 3],
                ttl: 0.0,
            },
            "addcameraeffect" => C::CameraEffect {
                effect: 0,
                add: true,
            },
            "removecameraeffect" => C::CameraEffect {
                effect: 0,
                add: false,
            },
            "overkill" => C::Overkill {
                id: None,
                type_hash: 0,
                area: 0,
                factions: 0,
            },
            "setfaction" => C::SetFaction {
                player: None,
                id: None,
                faction: 0,
                area: 0,
                operation: Operation::Add,
                type_hash: 0,
            },
            "removeallstatuseffects" => C::RemoveAllStatusEffects,
            "teleport" => C::Teleport {
                id: 0,
                target: 0,
                effect: 0,
                sound: 0,
            },
            "magick" => C::Magick {
                magick: 0,
                id: None,
                position: [0.0; 3],
            },
            "beginunequipweaponssection" => C::UnequipWeaponsSection {
                begin: true,
                animation: 0,
            },
            "endunequipweaponssection" => C::UnequipWeaponsSection {
                begin: false,
                animation: 0,
            },
            "begindefaultavatarsection" => C::DefaultAvatarSection(true),
            "enddefaultavatarsection" => C::DefaultAvatarSection(false),
            "assignitem" => C::AssignItem {
                item: String::new(),
                player: 0,
                area: any,
                animation: 0,
            },
            "additemtoavatars" => C::AddItemToAvatars {
                item: String::new(),
                bone: String::new(),
            },
            "removeitemfromavatars" => C::RemoveItemFromAvatars(String::new()),
            _ => return Err(ScriptReadError::UnsupportedTag(tag.into())),
        };
        let mut delay = 0.0;
        for (key, value) in attrs {
            let lower = key.to_ascii_lowercase();
            if lower == "delay" {
                delay = number(value, key)?;
                continue;
            }
            let text = || value.to_ascii_lowercase();
            match (&mut command, lower.as_str()) {
                (C::Telemetry, _) => {}
                (C::FoundMoose(id) | C::KilledAPony(id), "id") => *id = text(),
                (C::ConditionalAchievement { achievement, .. }, "achievement") => {
                    *achievement = text()
                }
                (C::ConditionalAchievement { victim, .. }, "victim") => {
                    *victim = identifier(value)?
                }
                (C::SetVolume { id, .. }, "id") => *id = identifier(value)?,
                (C::SetVolume { volume, .. }, "volume") => *volume = number(value, key)?,
                (C::SetMusicFocus { locator, .. }, "id") => *locator = identifier(value)?,
                (C::SetMusicFocus { radius, .. }, "radius") => *radius = number(value, key)?,
                (C::SetMusicFocus { clear_to, .. }, "clearto") => *clear_to = number(value, key)?,
                (C::CameraBias { bias, .. }, "bias") => *bias = vector(value, key)?,
                (C::CameraBias { time, .. }, "time") => *time = number(value, key)?,
                (C::CameraFollow(id), "id") => *id = identifier(value)?,
                (C::CameraPosition { target, .. }, "target") => *target = identifier(value)?,
                (C::CameraPosition { offset, .. }, "offset") => *offset = vector(value, key)?,
                (C::CameraPosition { magnification, .. }, "magnification") => {
                    *magnification = number(value, key)?
                }
                (C::Flash(duration), "duration") => *duration = number(value, key)?,
                (C::HighlightAnim { path, .. }, "name") => {
                    *path = text()
                        .split('/')
                        .map(crate::scene_loading::custom_hash)
                        .collect()
                }
                (C::HighlightAnim { time, .. } | C::HighlightPhys { time, .. }, "time") => {
                    *time = number(value, key)?
                }
                (C::HighlightPhys { id, .. }, "id") => *id = identifier(value)?,
                (C::DisplayTitle { title, .. }, "title") => *title = text().trim().to_owned(),
                (C::DisplayTitle { subtitle, .. }, "subtitle") => {
                    *subtitle = text().trim().to_owned()
                }
                (C::DisplayTitle { display_time, .. }, "displaytime") => {
                    *display_time = number(value, key)?
                }
                (C::DisplayTitle { fade_in, .. }, "fadein") => *fade_in = number(value, key)?,
                (C::DisplayTitle { fade_out, .. }, "fadeout") => *fade_out = number(value, key)?,
                (C::DisplayTitle { no_translation, .. }, "notranslation") => {
                    *no_translation = boolean(value, key)?
                }
                (C::DisplayTitle { align, .. }, "textalignment") => {
                    // L/LEFT, R/RIGHT, anything else centre.
                    use crate::text_geometry::TextAlign;
                    *align = match value.trim().to_uppercase().as_str() {
                        "L" | "LEFT" => TextAlign::Left,
                        "R" | "RIGHT" => TextAlign::Right,
                        _ => TextAlign::Center,
                    }
                }
                (C::CameraEffect { effect, .. }, "effect") => *effect = identifier(value)?,
                (C::ScrollScore { value: v, .. }, "value") => *v = number(value, key)?,
                (C::ScrollScore { ttl, .. }, "ttl") => *ttl = number(value, key)?,
                (C::ScrollScore { area, .. }, "area") => {
                    *area = crate::scene_loading::custom_hash(&value.to_lowercase())
                }
                (C::ScrollScore { offset, .. }, "offset") => {
                    // Only a three-part value is applied.
                    let parts: Vec<&str> = value.split(',').collect();
                    if parts.len() == 3 {
                        for (o, p) in offset.iter_mut().zip(parts) {
                            *o = number(p, key)?;
                        }
                    }
                }
                (C::RollCredits(file), "file") => *file = value.to_lowercase(),
                (C::StaffLight(on), "action") => {
                    // Enum.Parse(WizardAction, ignoreCase); default 0 = On.
                    let table = [("StaffLightOn", 0), ("StaffLightOff", 1)];
                    *on = match clr_enum(value, &table, key, i32::MAX)? {
                        0 => Some(true),
                        1 => Some(false),
                        _ => None,
                    }
                }
                (C::Overkill { id, .. }, "id") => {
                    *id = (!value.is_empty()).then(|| identifier(value)).transpose()?
                }
                (C::Overkill { type_hash, .. }, "type") => *type_hash = identifier(value)?,
                (C::Overkill { area, .. }, "area") => *area = identifier(value)?,
                (C::Overkill { factions, .. }, "factions") => {
                    *factions = crate::presence::factions(value, key)?
                }
                (C::SetFaction { player, id, .. }, "id") => {
                    let v = text();
                    *player = match v.as_str() {
                        "player1" => Some(0),
                        "player2" => Some(1),
                        "player3" => Some(2),
                        "player4" => Some(3),
                        _ => None,
                    };
                    *id = (!v.is_empty()).then(|| identifier(&v)).transpose()?;
                }
                (C::SetFaction { faction, .. }, "faction") => {
                    *faction = crate::presence::factions(value, key)?
                }
                (C::SetFaction { area, .. }, "area") => *area = identifier(value)?,
                (C::SetFaction { type_hash, .. }, "type") => *type_hash = identifier(value)?,
                (C::SetFaction { operation, .. }, "action") => {
                    *operation = match text().as_str() {
                        "add" | "0" => Operation::Add,
                        "remove" | "1" => Operation::Remove,
                        "set" | "2" => Operation::Set,
                        _ => return Err(ScriptReadError::InvalidAttribute(key.into())),
                    }
                }
                (C::Teleport { id, .. }, "id") => *id = identifier(value)?,
                (C::Teleport { target, .. }, "target") => *target = identifier(value)?,
                (C::Teleport { effect, .. }, "effect") => *effect = identifier(value)?,
                (C::Teleport { sound, .. }, "sound") => *sound = identifier(value)?,
                (C::Magick { magick, .. }, "magicktype" | "type" | "magick") => {
                    *magick = crate::magick_books::magick_id(value)
                        .ok_or_else(|| ScriptReadError::InvalidAttribute(key.into()))?
                }
                (C::Magick { id, .. }, "id") => {
                    *id = (!value.is_empty()).then(|| identifier(value)).transpose()?
                }
                (C::Magick { position, .. }, "position") => *position = vector(value, key)?,
                (C::UnequipWeaponsSection { animation: a, .. }, "animation") => {
                    *a = animation(value, key)?
                }
                (C::AssignItem { item, .. }, "id") => *item = text(),
                (C::AssignItem { player, .. }, "player") => *player = number(value, key)?,
                (C::AssignItem { area, .. }, "area") => *area = identifier(value)?,
                (C::AssignItem { animation: a, .. }, "animation") => *a = animation(value, key)?,
                (C::AddItemToAvatars { item, .. }, "item") => *item = text(),
                (C::AddItemToAvatars { bone, .. }, "bone") => *bone = text(),
                (C::RemoveItemFromAvatars(item), "item") => *item = text(),
                _ => return Err(ScriptReadError::InvalidAttribute(key.into())),
            }
        }
        Ok(Self { delay, command })
    }

    /// Whether the native QuickExecute repeats Execute (others are empty).
    pub fn quick_executes(&self) -> bool {
        use CampaignCommand as C;
        matches!(
            self.command,
            C::Telemetry
                | C::SetMusicFocus { .. }
                | C::Flash(_)
                | C::SetUi(_)
                | C::CameraEffect { .. }
                | C::Overkill { .. }
                | C::SetFaction { .. }
                | C::RemoveAllStatusEffects
                | C::Teleport { .. }
                | C::Magick { .. }
                | C::AssignItem { .. }
                | C::AddItemToAvatars { .. }
                | C::RemoveItemFromAvatars(_)
        )
    }
}
