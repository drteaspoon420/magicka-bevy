//! Boss-fight trigger actions (research/boss-fight.md): SpawnBoss,
//! SpawnGenericBoss, ActivateBoss, EndBossFight, BossMessage, BossTitle and
//! the GenericHealthBar actions (HealthBar, ActivateHealthBar,
//! RemoveHealthBar). Attribute sets follow the native classes' properties.
use crate::{
    ai_event::clr_enum,
    script_attributes::{boolean, identifier, number, ScriptReadError},
    spawn_action::SpawnAction,
};

pub const TAGS: &[&str] = &[
    "spawnboss",
    "spawngenericboss",
    "activateboss",
    "endbossfight",
    "bossmessage",
    "bosstitle",
    "healthbar",
    "activatehealthbar",
    "removehealthbar",
];

/// BossMessages (Enum.Parse ignoring case).
pub const BOSS_MESSAGES: &[(&str, i32)] = &[
    ("OrcBipedal", 0),
    ("GunterFight", 1),
    ("KingTalk1", 2),
    ("KingTalk2", 3),
    ("FafnirFight", 4),
    ("VladMortal", 5),
    ("CloneDone", 6),
    ("GrimnirHurt", 7),
    ("FutureVladDone", 8),
    ("KillGrimnir", 9),
    ("Assatur_Cut", 10),
    ("KillAssatur", 11),
    ("StartFight", 12),
];

/// GenericHealthBarTypes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HealthBarType {
    #[default]
    Hp,
    Counter,
    TimerDecreasing,
    TimerIncreasing,
}
const HEALTH_BAR_TYPES: &[(&str, i32)] = &[
    ("HP", 0),
    ("Counter", 1),
    ("TimerDecreasing", 2),
    ("TimerIncreasing", 3),
];
/// GenericHealthBarGraphics.
const HEALTH_BAR_GRAPHICS: &[(&str, i32)] = &[("Default", 0), ("Dynamite", 1)];

/// BossFight.Setup parameters shared by SpawnBoss and SpawnGenericBoss.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BossSetup {
    pub freeze_time: f32,
    pub health_appear_delay: f32,
    pub health_bar_width: f32,
    pub delayed: bool,
}

impl Default for BossSetup {
    fn default() -> Self {
        Self {
            freeze_time: 0.0,
            health_appear_delay: 0.0,
            health_bar_width: 0.8,
            delayed: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HealthBarConfig {
    pub display_name: String,
    pub normalized_health: f32,
    pub display_health: f32,
    pub time: f32,
    pub kind: HealthBarType,
    pub power: f32,
    pub alpha: f32,
    pub count: f32,
    pub dynamite: bool,
    pub initial_timer_delay: f32,
    pub is_scaled: bool,
    pub is_colored_red: bool,
    pub show_display_name: bool,
    pub has_animated_sprite: bool,
    pub animation_offset_y: f32,
    pub fade_time: f32,
    pub on_end_trigger: i32,
}

/// HealthBar action field initializers (ctor 0x06002aee): NormalizedHealth
/// 1, Power 1.1, AnimationSpriteOffsetY 16; everything else zero.
impl Default for HealthBarConfig {
    fn default() -> Self {
        Self {
            display_name: String::new(),
            normalized_health: 1.0,
            display_health: 0.0,
            time: 0.0,
            kind: HealthBarType::Hp,
            power: 1.1,
            alpha: 0.0,
            count: 0.0,
            dynamite: false,
            initial_timer_delay: 0.0,
            is_scaled: false,
            is_colored_red: false,
            show_display_name: false,
            has_animated_sprite: false,
            animation_offset_y: 16.0,
            fade_time: 0.0,
            on_end_trigger: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub enum BossCommand {
    /// SpawnBoss: a PropBoss (physics entity template), a GenericBoss
    /// (character template) or a named boss class.
    Spawn {
        boss: String,
        prop: bool,
        generic: bool,
        area: i32,
        unique_id: i32,
        mesh: i32,
        on_death: i32,
        on_damage: i32,
        setup: BossSetup,
    },
    /// SpawnGenericBoss: SpawnNPC's attribute set plus the fight setup.
    SpawnGeneric {
        spawn: Box<SpawnAction>,
        setup: BossSetup,
    },
    Activate,
    End,
    Message(u8),
    Title {
        title: String,
        subtitle: String,
        display_time: f32,
        fade_in: f32,
        fade_out: f32,
    },
    HealthBar(Box<HealthBarConfig>),
    ActivateHealthBar,
    RemoveHealthBar,
}

#[derive(Clone, Debug)]
pub struct BossAction {
    pub delay: f32,
    pub command: BossCommand,
}

fn setup_attr(setup: &mut BossSetup, key: &str, value: &str) -> Result<bool, ScriptReadError> {
    match key {
        "freezetime" => setup.freeze_time = number(value, key)?,
        "healthappeardelay" => setup.health_appear_delay = number(value, key)?,
        "healthbarwidth" => setup.health_bar_width = number(value, key)?,
        "delayed" => setup.delayed = boolean(value, key)?,
        _ => return Ok(false),
    }
    Ok(true)
}

impl BossAction {
    pub fn read(node: &crate::scene_script::XmlElement) -> Result<Self, ScriptReadError> {
        use BossCommand as C;
        let tag = node.name.to_ascii_lowercase();
        let mut delay = 0.0;
        let mut command = match tag.as_str() {
            "spawnboss" => C::Spawn {
                boss: String::new(),
                prop: false,
                generic: false,
                area: 0,
                unique_id: 0,
                mesh: -1,
                on_death: 0,
                on_damage: 0,
                setup: BossSetup::default(),
            },
            "spawngenericboss" => C::SpawnGeneric {
                spawn: Box::new(SpawnAction::read(node)?),
                setup: BossSetup::default(),
            },
            "activateboss" => C::Activate,
            "endbossfight" => C::End,
            "bossmessage" => C::Message(0),
            "bosstitle" => C::Title {
                title: String::new(),
                subtitle: String::new(),
                display_time: -1.0,
                fade_in: 1.0,
                fade_out: 1.0,
            },
            "healthbar" => C::HealthBar(Box::default()),
            "activatehealthbar" => C::ActivateHealthBar,
            "removehealthbar" => C::RemoveHealthBar,
            _ => return Err(ScriptReadError::UnsupportedTag(tag)),
        };
        for (key, value) in &node.attributes {
            let lower = key.to_ascii_lowercase();
            let k = lower.as_str();
            if k == "delay" {
                delay = number(value, key)?;
                continue;
            }
            if let C::Spawn { setup, .. } | C::SpawnGeneric { setup, .. } = &mut command {
                if setup_attr(setup, k, value)? {
                    continue;
                }
            }
            // Action.Read lowercases string properties.
            let text = || value.to_ascii_lowercase();
            match (&mut command, k) {
                (C::Spawn { boss, .. }, "boss") => *boss = text(),
                (C::Spawn { prop, .. }, "prop") => *prop = boolean(value, key)?,
                (C::Spawn { generic, .. }, "generic") => *generic = boolean(value, key)?,
                (C::Spawn { area, .. }, "area") => *area = identifier(value)?,
                (C::Spawn { unique_id, .. }, "id") => *unique_id = identifier(value)?,
                (C::Spawn { mesh, .. }, "meshid") => *mesh = number(value, key)?,
                (C::Spawn { on_death, .. }, "ondeath") => *on_death = identifier(value)?,
                (C::Spawn { on_damage, .. }, "ondamage") => *on_damage = identifier(value)?,
                // Template only feeds the network spawn of named bosses.
                (C::Spawn { .. }, "template") => {}
                // SpawnAction::read already consumed the NPC attributes.
                (C::SpawnGeneric { .. }, _) => {}
                (C::Message(m), "message") => {
                    *m = clr_enum(value, BOSS_MESSAGES, key, i32::MAX)? as u8
                }
                (C::Title { title, .. }, "title") => *title = text(),
                (C::Title { subtitle, .. }, "subtitle") => *subtitle = text(),
                (C::Title { display_time, .. }, "displaytime") => {
                    *display_time = number(value, key)?
                }
                (C::Title { fade_in, .. }, "fadein") => *fade_in = number(value, key)?,
                (C::Title { fade_out, .. }, "fadeout") => *fade_out = number(value, key)?,
                (C::HealthBar(h), k) => match k {
                    "displayname" => h.display_name = text(),
                    "normalizedhealth" => h.normalized_health = number(value, key)?,
                    "displayhealth" => h.display_health = number(value, key)?,
                    "time" => h.time = number(value, key)?,
                    "type" => {
                        h.kind = match clr_enum(value, HEALTH_BAR_TYPES, key, i32::MAX)? {
                            0 => HealthBarType::Hp,
                            1 => HealthBarType::Counter,
                            2 => HealthBarType::TimerDecreasing,
                            _ => HealthBarType::TimerIncreasing,
                        }
                    }
                    "power" => h.power = number(value, key)?,
                    "alpha" => h.alpha = number(value, key)?,
                    "count" => h.count = number(value, key)?,
                    "graphicstype" => {
                        h.dynamite = clr_enum(value, HEALTH_BAR_GRAPHICS, key, i32::MAX)? == 1
                    }
                    "initialtimerdelay" => h.initial_timer_delay = number(value, key)?,
                    "isscaled" => h.is_scaled = boolean(value, key)?,
                    "iscoloredred" => h.is_colored_red = boolean(value, key)?,
                    "showdisplayname" => h.show_display_name = boolean(value, key)?,
                    "hasanimatedsprite" => h.has_animated_sprite = boolean(value, key)?,
                    "animationoffsetpositiony" => h.animation_offset_y = number(value, key)?,
                    "fadetime" => h.fade_time = number(value, key)?,
                    "onendtrigger" => h.on_end_trigger = identifier(value)?,
                    _ => return Err(ScriptReadError::InvalidAttribute(key.clone())),
                },
                _ => return Err(ScriptReadError::InvalidAttribute(key.clone())),
            }
        }
        Ok(Self { delay, command })
    }

    /// Whether QuickExecute repeats Execute (BossTitle and SpawnGenericBoss
    /// have empty bodies).
    pub fn quick_executes(&self) -> bool {
        !matches!(
            self.command,
            BossCommand::Title { .. } | BossCommand::SpawnGeneric { .. }
        )
    }
}
