//! AIEvent child decoding shared by Spawn/GiveOrder. XML decoding is level-free;
//! the Initialize-time level lookups of MoveEvent are applied by `resolve`.
use crate::{
    dialog_enums::ANIMATIONS,
    scene_loading::custom_hash,
    scene_script::XmlElement,
    script_attributes::{boolean, number, ScriptReadError},
};

pub const ANIMATION_MAX: i32 = u16::MAX as i32;

/// Enum.Parse(type, value, ignoreCase: true): numeric text or comma-separated
/// names whose values are ORed. Values outside the underlying range overflow.
pub(crate) fn clr_enum(
    value: &str,
    table: &[(&str, i32)],
    key: &str,
    max: i32,
) -> Result<i32, ScriptReadError> {
    let invalid = || ScriptReadError::InvalidAttribute(key.into());
    let trimmed = value.trim();
    let starts_numeric = trimmed
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+');
    let parsed = if starts_numeric {
        trimmed.parse::<i64>().map_err(|_| invalid())?
    } else {
        let mut combined = 0i64;
        for name in trimmed.split(',') {
            combined |= table
                .iter()
                .find(|(n, _)| name.trim().eq_ignore_ascii_case(n))
                .map(|(_, v)| *v as i64)
                .ok_or_else(invalid)?;
        }
        combined
    };
    if parsed < 0 || parsed > max as i64 {
        return Err(invalid());
    }
    Ok(parsed as i32)
}

/// Enum.Parse for an int-backed enum: like `clr_enum` but negative values are
/// representable.
pub(crate) fn clr_enum_signed(
    value: &str,
    table: &[(&str, i32)],
    key: &str,
) -> Result<i32, ScriptReadError> {
    let invalid = || ScriptReadError::InvalidAttribute(key.into());
    let trimmed = value.trim();
    if trimmed
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+')
    {
        return trimmed.parse::<i32>().map_err(|_| invalid());
    }
    let mut combined = 0;
    for name in trimmed.split(',') {
        combined |= table
            .iter()
            .find(|(n, _)| name.trim().eq_ignore_ascii_case(n))
            .map(|(_, v)| *v)
            .ok_or_else(invalid)?;
    }
    Ok(combined)
}

pub(crate) fn animation(value: &str, key: &str) -> Result<u16, ScriptReadError> {
    clr_enum(value, ANIMATIONS, key, ANIMATION_MAX).map(|v| v as u16)
}

/// XNA Vector3.Normalize: single reciprocal of a double square root. Zero
/// vectors become NaN rather than being guarded.
pub fn xna_normalize(v: [f32; 3]) -> [f32; 3] {
    let length_squared = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    let inverse = 1.0 / (length_squared as f64).sqrt() as f32;
    [v[0] * inverse, v[1] * inverse, v[2] * inverse]
}

fn triple(value: &str, key: &str) -> Result<Option<[f32; 3]>, ScriptReadError> {
    let parts: Vec<_> = value.split(',').collect();
    if parts.len() != 3 {
        return Ok(None);
    }
    Ok(Some([
        number(parts[0], key)?,
        number(parts[1], key)?,
        number(parts[2], key)?,
    ]))
}

fn lower_hash(value: &str) -> Result<i32, ScriptReadError> {
    if !value.is_ascii() {
        return Err(ScriptReadError::UnsupportedName(value.into()));
    }
    Ok(custom_hash(&value.to_ascii_lowercase()))
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveEvent {
    pub waypoint: [f32; 3],
    /// Hash of a non-triple position. Zero when the position was a triple or omitted.
    pub waypoint_id: i32,
    pub direction: [f32; 3],
    pub fixed_direction: bool,
    pub delay: f32,
    pub speed: f32,
    pub trigger: i32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnimationEvent {
    pub animation: u16,
    pub idle_animation: u16,
    pub delay: f32,
    pub blend_time: f32,
    pub trigger: i32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FaceEvent {
    pub direction: [f32; 3],
    pub target_id: i32,
    pub delay: f32,
    pub speed: f32,
    pub trigger: i32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KillEvent {
    pub remove: bool,
    pub delay: f32,
    pub trigger: i32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopType {
    #[default]
    Loop,
    Reverse,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LoopEvent {
    pub kind: LoopType,
    pub delay: f32,
    pub trigger: i32,
}
/// AIEventType order: Move, Animation, Face, Kill, Loop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AiEvent {
    Move(MoveEvent),
    Animation(AnimationEvent),
    Face(FaceEvent),
    Kill(KillEvent),
    Loop(LoopEvent),
}

/// Level lookups performed by MoveEvent's constructor during Action.Initialize.
pub trait AiEventLevel {
    type Error;
    /// Locator transform in XNA row-major order.
    fn locator(&self, id: i32) -> Option<[f32; 16]>;
    /// TriggerArea.GetRandomLocation when an area with this ID exists.
    fn trigger_area_location(&mut self, id: i32) -> Result<Option<[f32; 3]>, Self::Error>;
}

impl AiEvent {
    /// Decode one non-comment AIEvent child. Unknown event names and unknown
    /// attributes fail, matching the original constructors' exceptions.
    pub fn read(node: &XmlElement) -> Result<Self, ScriptReadError> {
        let invalid_name = || ScriptReadError::UnsupportedTag(node.name.clone());
        let kind = clr_enum(
            &node.name,
            &[
                ("move", 0),
                ("animation", 1),
                ("face", 2),
                ("kill", 3),
                ("loop", 4),
            ],
            &node.name,
            i32::MAX,
        )
        .map_err(|_| invalid_name())?;
        let invalid = |key: &str| ScriptReadError::InvalidAttribute(key.into());
        let attributes = node
            .attributes
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()));
        match kind {
            0 => {
                let mut event = MoveEvent {
                    speed: 1.0,
                    ..Default::default()
                };
                for (key, value) in attributes {
                    match key.to_ascii_lowercase().as_str() {
                        "position" => match triple(value, key)? {
                            Some(p) => event.waypoint = p,
                            None => event.waypoint_id = lower_hash(value)?,
                        },
                        "facingdirection" => match triple(value, key)? {
                            Some(d) => {
                                event.fixed_direction = true;
                                event.direction = xna_normalize(d);
                            }
                            None => event.fixed_direction = boolean(value, key)?,
                        },
                        "delay" => event.delay = number(value, key)?,
                        "speed" => event.speed = number(value, key)?,
                        "trigger" => event.trigger = lower_hash(value)?,
                        _ => return Err(invalid(key)),
                    }
                }
                Ok(Self::Move(event))
            }
            1 => {
                let mut event = AnimationEvent::default();
                for (key, value) in attributes {
                    match key.to_ascii_lowercase().as_str() {
                        "animation" => event.animation = animation(value, key)?,
                        "idleanimation" => event.idle_animation = animation(value, key)?,
                        "delay" => event.delay = number(value, key)?,
                        "blendtime" => event.blend_time = number(value, key)?,
                        "trigger" => event.trigger = lower_hash(value)?,
                        _ => return Err(invalid(key)),
                    }
                }
                Ok(Self::Animation(event))
            }
            2 => {
                let mut event = FaceEvent {
                    speed: 1.0,
                    ..Default::default()
                };
                for (key, value) in attributes {
                    match key.to_ascii_lowercase().as_str() {
                        "facingdirection" | "target" => match triple(value, key)? {
                            Some(d) => event.direction = xna_normalize(d),
                            None => event.target_id = lower_hash(value)?,
                        },
                        "delay" => event.delay = number(value, key)?,
                        "speed" => event.speed = number(value, key)?,
                        "trigger" => event.trigger = lower_hash(value)?,
                        _ => return Err(invalid(key)),
                    }
                }
                Ok(Self::Face(event))
            }
            3 => {
                let mut event = KillEvent {
                    remove: true,
                    ..Default::default()
                };
                for (key, value) in attributes {
                    match key.to_ascii_lowercase().as_str() {
                        "remove" => event.remove = boolean(value, key)?,
                        "delay" => event.delay = number(value, key)?,
                        "trigger" => event.trigger = lower_hash(value)?,
                        _ => return Err(invalid(key)),
                    }
                }
                Ok(Self::Kill(event))
            }
            4 => {
                let mut event = LoopEvent::default();
                for (key, value) in attributes {
                    match key.to_ascii_lowercase().as_str() {
                        "type" => {
                            event.kind = match clr_enum(
                                value,
                                &[("loop", 0), ("reverse", 1)],
                                key,
                                i32::MAX,
                            )? {
                                0 => LoopType::Loop,
                                1 => LoopType::Reverse,
                                // Undefined numeric LoopType values are not modeled.
                                _ => return Err(invalid(key)),
                            }
                        }
                        "delay" => event.delay = number(value, key)?,
                        "trigger" => event.trigger = lower_hash(value)?,
                        _ => return Err(invalid(key)),
                    }
                }
                Ok(Self::Loop(event))
            }
            // Combined names (e.g. "move,face") parse but hit the original default throw.
            _ => Err(invalid_name()),
        }
    }

    /// Decode an action node's children. Only XmlComment is skipped; text,
    /// CDATA and processing-instruction nodes name invalid event types natively.
    pub fn read_children(node: &XmlElement) -> Result<Vec<Self>, ScriptReadError> {
        if node.non_comment_text_children {
            return Err(ScriptReadError::UnsupportedTag("#text".into()));
        }
        node.children.iter().map(Self::read).collect()
    }

    /// MoveEvent's constructor-time level lookup, run for every Initialize.
    /// Attribute order matters: a later triple position overwrites the waypoint
    /// but keeps an earlier locator direction and waypoint ID.
    pub fn resolve<L: AiEventLevel + ?Sized>(
        node: &XmlElement,
        level: &mut L,
    ) -> Result<Self, ResolveError<L::Error>> {
        let mut event = Self::read(node).map_err(ResolveError::Read)?;
        let Self::Move(ref mut movement) = event else {
            return Ok(event);
        };
        // Replay attribute order so lookups interleave with literal assignments.
        for (key, value) in &node.attributes {
            if key.eq_ignore_ascii_case("position") {
                if let Some(p) = triple(value, key).map_err(ResolveError::Read)? {
                    movement.waypoint = p;
                    continue;
                }
                let id = lower_hash(value).map_err(ResolveError::Read)?;
                if let Some(m) = level.locator(id) {
                    movement.waypoint = [m[12], m[13], m[14]];
                    movement.direction = [-m[8], -m[9], -m[10]];
                } else if let Some(p) = level
                    .trigger_area_location(id)
                    .map_err(ResolveError::Level)?
                {
                    movement.waypoint = p;
                }
            } else if key.eq_ignore_ascii_case("facingdirection") {
                if let Some(d) = triple(value, key).map_err(ResolveError::Read)? {
                    movement.direction = xna_normalize(d);
                }
            }
        }
        Ok(event)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResolveError<E> {
    Read(ScriptReadError),
    Level(E),
}
