//! Shared reader primitives and sub-records used by several content types.
//!
//! Every reader mirrors the statement order of the original ContentReader
//! code in `Magicka.exe` (ILSpy output; method names cited per item).

use magicka_xnb::xnb::{Cursor, XnbError};
use std::fmt;

#[derive(Debug)]
pub enum ContentError {
    Xnb(XnbError),
    /// The original code would throw here (unknown type name, invalid tag...).
    Invalid(String),
}

impl fmt::Display for ContentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContentError::Xnb(e) => write!(f, "{e}"),
            ContentError::Invalid(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for ContentError {}

impl From<XnbError> for ContentError {
    fn from(e: XnbError) -> Self {
        ContentError::Xnb(e)
    }
}

pub type Result<T> = std::result::Result<T, ContentError>;

pub fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(ContentError::Invalid(msg.into()))
}

/// `BinaryReader`/`ContentReader` primitives used by Magicka's readers.
pub struct Reader<'a> {
    pub c: Cursor<'a>,
    /// Assembly-qualified type-reader names from the XNB manifest, used to
    /// dispatch `ReadObject` (index = type id - 1).
    pub types: Vec<String>,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            c: Cursor::new(data),
            types: Vec::new(),
        }
    }
    pub fn with_types(data: &'a [u8], types: Vec<String>) -> Self {
        Self {
            c: Cursor::new(data),
            types,
        }
    }
    pub fn pos(&self) -> usize {
        self.c.pos
    }
    pub fn remaining(&self) -> usize {
        self.c.remaining()
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.c.u8()?)
    }
    /// `ReadBoolean`: any non-zero byte is true.
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.c.u8()? != 0)
    }
    pub fn u16(&mut self) -> Result<u16> {
        let b = self.c.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.c.i32()?)
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(self.c.f32()?)
    }
    pub fn string(&mut self) -> Result<String> {
        Ok(self.c.string()?)
    }
    /// `ContentReader.ReadVector3`.
    pub fn vec3(&mut self) -> Result<[f32; 3]> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
    /// `ContentReader.ReadExternalReference<T>`: an asset path relative to
    /// the referencing asset; empty means null.
    pub fn external(&mut self) -> Result<Option<String>> {
        let s = self.string()?;
        Ok((!s.is_empty()).then_some(s))
    }
    /// An `Int32` element count; negative counts would make the original
    /// throw (`new T[negative]`).
    pub fn count(&mut self) -> Result<usize> {
        let n = self.i32()?;
        if n < 0 {
            return invalid(format!("negative count {n} at {}", self.pos()));
        }
        Ok(n as usize)
    }
    pub fn strings(&mut self) -> Result<Vec<String>> {
        let n = self.count()?;
        (0..n).map(|_| self.string()).collect()
    }
}

/// `Magicka.GameLogic.Damage` as stored by readers: attack property bits,
/// element bits, amount, magnitude.
#[derive(Debug, Clone, PartialEq)]
pub struct Damage {
    pub attack_property: i32,
    pub element: i32,
    pub amount: f32,
    pub magnitude: f32,
}

impl Damage {
    /// Order used by `DamageEvent`, `BuffDealDamage`, `DamageGrip` (action):
    /// i32, i32, f32 amount, f32 magnitude.
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Damage {
            attack_property: r.i32()?,
            element: r.i32()?,
            amount: r.f32()?,
            magnitude: r.f32()?,
        })
    }
    /// `SplashEvent` and `ThrowGrip` (ability) read the amount as Int32.
    pub fn read_int_amount(r: &mut Reader) -> Result<Self> {
        Ok(Damage {
            attack_property: r.i32()?,
            element: r.i32()?,
            amount: r.i32()? as f32,
            magnitude: r.f32()?,
        })
    }
}

/// `Resistance` entry as read by `CharacterTemplate.Read` / `Item.Read`.
#[derive(Debug, Clone, PartialEq)]
pub struct Resistance {
    pub element: i32,
    pub multiplier: f32,
    pub modifier: f32,
    pub status_resistance: bool,
}

impl Resistance {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Resistance {
            element: r.i32()?,
            multiplier: r.f32()?,
            modifier: r.f32()?,
            status_resistance: r.bool()?,
        })
    }
}

/// Point light block shared by characters (after a joint name) and items.
#[derive(Debug, Clone, PartialEq)]
pub struct PointLight {
    pub radius: f32,
    pub diffuse: [f32; 3],
    pub ambient: [f32; 3],
    pub specular: f32,
    pub variation_type: u8,
    pub variation_amount: f32,
    pub variation_speed: f32,
}

impl PointLight {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(PointLight {
            radius: r.f32()?,
            diffuse: r.vec3()?,
            ambient: r.vec3()?,
            specular: r.f32()?,
            variation_type: r.u8()?,
            variation_amount: r.f32()?,
            variation_speed: r.f32()?,
        })
    }
}

/// `SpecialAbilities.SpecialAbility.Read`.
#[derive(Debug, Clone, PartialEq)]
pub struct SpecialAbility {
    pub type_name: String,
    pub animation: String,
    pub hash_string: String,
    pub elements: Vec<i32>,
}

impl SpecialAbility {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let type_name = r.string()?;
        let animation = r.string()?;
        let hash_string = r.string()?;
        let n = r.count()?;
        let elements = (0..n).map(|_| r.i32()).collect::<Result<_>>()?;
        Ok(SpecialAbility {
            type_name,
            animation,
            hash_string,
            elements,
        })
    }
}

/// `Items.EventCondition`.
#[derive(Debug, Clone, PartialEq)]
pub struct EventCondition {
    pub condition_type: u8,
    pub hitpoints: i32,
    pub elements: i32,
    pub threshold: f32,
    pub time: f32,
}

/// `Items.EventStorage`: tagged by `EventType` (byte).
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Damage {
        damage: Damage,
        velocity_based: bool,
    },
    Splash {
        damage: Damage,
        radius: f32,
    },
    Sound {
        bank: i32,
        sound: String,
        magnitude: f32,
        stop_on_remove: bool,
    },
    Effect {
        follow: bool,
        world_align: bool,
        effect: String,
    },
    Remove {
        bounce: i32,
    },
    CameraShake {
        ttl: f32,
        magnitude: f32,
        from_position: bool,
    },
    Decal {
        decal: i32,
        variant: i32,
        scale: i32,
    },
    Spawn {
        character: String,
        idle_animation: String,
        spawn_animation: String,
        health: f32,
        order: u8,
        react_to: u8,
        reaction: u8,
        rotation_deg: f32,
        offset: [f32; 3],
    },
    Overkill,
    SpawnGibs {
        start: i32,
        end: i32,
    },
    SpawnItem {
        item: String,
    },
    SpawnMagick {
        magick: String,
    },
    SpawnMissile {
        missile: String,
        velocity: [f32; 3],
        facing: bool,
    },
    Light(PointLight),
    CastMagick {
        magick: String,
        elements: Vec<i32>,
    },
    DamageOwner {
        damage: Damage,
        velocity_based: bool,
    },
}

impl Event {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let tag = r.u8()?;
        Ok(match tag {
            0 => Event::Damage {
                damage: Damage::read(r)?,
                velocity_based: r.bool()?,
            },
            1 => {
                let damage = Damage::read_int_amount(r)?;
                let radius = r.f32()?;
                if radius <= f32::EPSILON {
                    return invalid("SplashEvent radius <= epsilon (original throws \"WTH\")");
                }
                Event::Splash { damage, radius }
            }
            2 => Event::Sound {
                bank: r.i32()?,
                sound: r.string()?,
                magnitude: r.f32()?,
                stop_on_remove: r.bool()?,
            },
            3 => Event::Effect {
                follow: r.bool()?,
                world_align: r.bool()?,
                effect: r.string()?,
            },
            4 => Event::Remove { bounce: r.i32()? },
            5 => Event::CameraShake {
                ttl: r.f32()?,
                magnitude: r.f32()?,
                from_position: r.bool()?,
            },
            6 => Event::Decal {
                decal: r.i32()?,
                variant: r.i32()?,
                scale: r.i32()?,
            },
            7 => return invalid("BlastEvent (original throws NotImplementedException)"),
            8 => Event::Spawn {
                character: r.string()?,
                idle_animation: r.string()?,
                spawn_animation: r.string()?,
                health: r.f32()?,
                order: r.u8()?,
                react_to: r.u8()?,
                reaction: r.u8()?,
                rotation_deg: r.f32()?,
                offset: r.vec3()?,
            },
            9 => Event::Overkill,
            10 => Event::SpawnGibs {
                start: r.i32()?,
                end: r.i32()?,
            },
            11 => Event::SpawnItem { item: r.string()? },
            12 => Event::SpawnMagick {
                magick: r.string()?,
            },
            13 => Event::SpawnMissile {
                missile: r.string()?,
                velocity: r.vec3()?,
                facing: r.bool()?,
            },
            14 => Event::Light(PointLight::read(r)?),
            15 => {
                let magick = r.string()?;
                let n = r.count()?;
                let elements = (0..n).map(|_| r.i32()).collect::<Result<_>>()?;
                Event::CastMagick { magick, elements }
            }
            16 => Event::DamageOwner {
                damage: Damage::read(r)?,
                velocity_based: r.bool()?,
            },
            17 => return invalid("CallbackEvent (original throws)"),
            t => return invalid(format!("unknown EventType {t}")),
        })
    }
}

/// `Items.EventCollection.Read`.
#[derive(Debug, Clone, PartialEq)]
pub struct EventCollection {
    pub condition: EventCondition,
    pub repeat: bool,
    pub events: Vec<Event>,
}

impl EventCollection {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let condition = EventCondition {
            condition_type: r.u8()?,
            hitpoints: r.i32()?,
            elements: r.i32()?,
            threshold: r.f32()?,
            time: r.f32()?,
        };
        let repeat = r.bool()?;
        let n = r.count()?;
        let events = (0..n).map(|_| Event::read(r)).collect::<Result<_>>()?;
        Ok(EventCollection {
            condition,
            repeat,
            events,
        })
    }
}

/// `Items.ConditionCollection(ContentReader)`: at most 5 collections (fixed
/// array in the original; more would throw IndexOutOfRange).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConditionCollection(pub Vec<EventCollection>);

impl ConditionCollection {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let n = r.count()?;
        if n > 5 {
            return invalid(format!("ConditionCollection with {n} > 5 entries"));
        }
        Ok(ConditionCollection(
            (0..n)
                .map(|_| EventCollection::read(r))
                .collect::<Result<_>>()?,
        ))
    }
}

/// `Buffs.BuffStorage(ContentReader)`, tagged by `Buffs.BuffType` (byte).
#[derive(Debug, Clone, PartialEq)]
pub struct BuffStorage {
    pub visual_category: u8,
    pub color: [f32; 3],
    pub ttl: f32,
    pub effect: String,
    pub kind: Buff,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Buff {
    BoostDamage(Damage),
    DealDamage(Damage),
    /// Note the field order: element, *modifier*, *multiplier*, status.
    Resistance {
        element: i32,
        modifier: f32,
        multiplier: f32,
        status_resistance: bool,
    },
    Undying,
    Boost {
        amount: f32,
    },
    ReduceAgro {
        amount: f32,
    },
    ModifyHitPoints {
        multiplier: f32,
        modifier: f32,
    },
    ModifySpellTtl {
        multiplier: f32,
        modifier: f32,
    },
    ModifySpellRange {
        multiplier: f32,
        modifier: f32,
    },
    /// Unknown tag: the original switch has no default, so no body is read.
    Unknown(u8),
}

impl BuffStorage {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let tag = r.u8()?;
        let visual_category = r.u8()?;
        let color = r.vec3()?;
        let ttl = r.f32()?;
        let effect = r.string()?;
        let kind = match tag {
            0 => Buff::BoostDamage(Damage::read(r)?),
            1 => Buff::DealDamage(Damage::read(r)?),
            2 => Buff::Resistance {
                element: r.i32()?,
                modifier: r.f32()?,
                multiplier: r.f32()?,
                status_resistance: r.bool()?,
            },
            3 => Buff::Undying,
            4 => Buff::Boost { amount: r.f32()? },
            5 => Buff::ReduceAgro { amount: r.f32()? },
            6 => Buff::ModifyHitPoints {
                multiplier: r.f32()?,
                modifier: r.f32()?,
            },
            7 => Buff::ModifySpellTtl {
                multiplier: r.f32()?,
                modifier: r.f32()?,
            },
            8 => Buff::ModifySpellRange {
                multiplier: r.f32()?,
                modifier: r.f32()?,
            },
            t => Buff::Unknown(t),
        };
        Ok(BuffStorage {
            visual_category,
            color,
            ttl,
            effect,
            kind,
        })
    }
}

/// `Buffs.AuraStorage(ContentReader)`, tagged by `AuraType` (byte).
#[derive(Debug, Clone, PartialEq)]
pub struct AuraStorage {
    pub target: u8,
    pub visual_category: u8,
    pub color: [f32; 3],
    pub effect: String,
    /// TTL as stored; `CharacterTemplate.Read` replaces values <= 0 with +inf.
    pub ttl: f32,
    pub radius: f32,
    /// Comma-separated target type names (hashed individually by the game).
    pub target_types: String,
    pub target_faction: i32,
    pub kind: Aura,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Aura {
    Buff(BuffStorage),
    Deflect { strength: f32 },
    Boost { magnitude: f32 },
    LifeSteal { amount: f32 },
    Love { radius: f32, ttl: f32 },
    Unknown(u8),
}

impl AuraStorage {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let target = r.u8()?;
        let tag = r.u8()?;
        let visual_category = r.u8()?;
        let color = r.vec3()?;
        let effect = r.string()?;
        let ttl = r.f32()?;
        let radius = r.f32()?;
        let target_types = r.string()?;
        let target_faction = r.i32()?;
        let kind = match tag {
            0 => Aura::Buff(BuffStorage::read(r)?),
            1 => Aura::Deflect { strength: r.f32()? },
            2 => Aura::Boost {
                magnitude: r.f32()?,
            },
            3 => Aura::LifeSteal { amount: r.f32()? },
            4 => Aura::Love {
                radius: r.f32()?,
                ttl: r.f32()?,
            },
            t => Aura::Unknown(t),
        };
        Ok(AuraStorage {
            target,
            visual_category,
            color,
            effect,
            ttl,
            radius,
            target_types,
            target_faction,
            kind,
        })
    }
}

/// `Items.Attachment(ContentReader, Skeleton)`: bone name, rotation (radians,
/// X/Y/X order applied by the game), item asset reference.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub bone: String,
    pub rotation: [f32; 3],
    pub item: Option<String>,
}

impl Attachment {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Attachment {
            bone: r.string()?,
            rotation: r.vec3()?,
            item: r.external()?,
        })
    }
}
