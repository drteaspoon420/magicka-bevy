//! Script SpawnPhys and SpawnAnimProp: Initialize allocates one physics entity
//! per action instance; every Execute re-initializes and re-adds that same
//! instance. SpawnAnimProp loads Data/AnimatedProps templates and, for a
//! template with clips and a skeleton, uses an AnimatedPhysicsEntity.
use crate::{
    ai_event::AiEventLevel,
    script_attributes::{identifier, number, ScriptReadError},
};

#[derive(Clone, Debug, PartialEq)]
pub struct SpawnPhysAction {
    pub delay: f32,
    pub area: i32,
    /// Lowercased Object; Initialize loads "Data/PhysicsEntities/" + this.
    pub object: Option<String>,
    pub on_death: i32,
    pub on_damage: i32,
    pub unique_id: i32,
    /// SpawnAnimProp's options; `None` for SpawnPhys.
    pub anim: Option<AnimProp>,
}

/// SpawnAnimProp's AnimatedPhysicsEntity settings.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimProp {
    /// Trimmed, lowercased Animation; `None` when empty or not an Animations
    /// name (Animations.None, whose clip slot is always empty).
    pub animation: Option<String>,
    pub damageable: bool,
    pub gravity: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysEntity {
    pub handle: u16,
    /// A DamageablePhysicsEntity (AnimatedPhysicsEntity derives from it).
    pub damageable: bool,
    /// An AnimatedPhysicsEntity.
    pub animated: bool,
}

/// HandleAndHealth; `handle` is an int natively (ReadTag stores 65535).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PhysHealth {
    pub handle: i32,
    pub health: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpawnPhysState {
    /// mEntity; `None` before Initialize or after a hot-join client Execute.
    pub entity: Option<PhysEntity>,
    pub health: PhysHealth,
}

pub trait SpawnPhysLevel: AiEventLevel {
    /// Content.Load<PhysicsEntityTemplate>; returns MaxHitpoints.
    fn load_physics_template(&mut self, object: Option<&str>) -> Result<i32, Self::Error>;
    /// DamageablePhysicsEntity.GetFromCache.
    fn damageable_from_cache(&mut self) -> Result<u16, Self::Error>;
    /// new PhysicsEntity(PlayState).
    fn new_physics_entity(&mut self) -> Result<u16, Self::Error>;
    /// The template has animation clips and a skeleton.
    fn physics_template_animated(&mut self, _object: Option<&str>) -> bool {
        false
    }
    /// new AnimatedPhysicsEntity(PlayState).
    fn new_animated_entity(&mut self) -> Result<u16, Self::Error> {
        self.damageable_from_cache()
    }
}

pub trait SpawnPhysHost {
    type Error;
    fn is_hot_join_client(&self) -> bool;
    fn return_damageable_to_cache(&mut self, handle: u16) -> Result<(), Self::Error>;
    fn locator(&mut self, area: i32) -> Result<[f32; 16], Self::Error>;
    /// PhysicsEntity.Initialize(template, locator, uniqueID).
    fn initialize_entity(
        &mut self,
        handle: u16,
        object: Option<&str>,
        transform: [f32; 16],
        unique_id: i32,
    ) -> Result<(), Self::Error>;
    fn set_damage_triggers(
        &mut self,
        handle: u16,
        on_death: i32,
        on_damage: i32,
    ) -> Result<(), Self::Error>;
    /// AnimatedPhysicsEntity's part of Initialize (after the base) and
    /// ForceAnimation.
    fn animate_entity(&mut self, _handle: u16, _anim: &AnimProp) -> Result<(), Self::Error> {
        Ok(())
    }
    fn add_entity(&mut self, handle: u16) -> Result<(), Self::Error>;
    fn hit_points(&mut self, handle: u16) -> Result<f32, Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum SpawnPhysError<E> {
    /// mEntity is null (never initialized or released by a hot-join Execute).
    MissingEntity,
    Host(E),
}
impl<E> From<E> for SpawnPhysError<E> {
    fn from(e: E) -> Self {
        Self::Host(e)
    }
}

impl SpawnPhysAction {
    pub fn read<'a>(
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        let mut action = Self {
            delay: 0.0,
            area: 0,
            object: None,
            on_death: 0,
            on_damage: 0,
            unique_id: 0,
            anim: None,
        };
        for (key, value) in attrs {
            match key.to_ascii_lowercase().as_str() {
                "delay" => action.delay = number(value, key)?,
                "area" => action.area = identifier(value)?,
                "object" => {
                    if !value.is_ascii() {
                        return Err(ScriptReadError::UnsupportedName(value.into()));
                    }
                    action.object = Some(value.to_ascii_lowercase())
                }
                "ondeath" => action.on_death = identifier(value)?,
                "ondamage" => action.on_damage = identifier(value)?,
                "id" => action.unique_id = identifier(value)?,
                _ => {}
            }
        }
        Ok(action)
    }

    /// SpawnAnimProp: the SpawnPhys attributes (Object under
    /// Data/AnimatedProps) plus Animation, Damageable (default false) and
    /// Gravity (default true). Anything but "true" reads false; a null or
    /// empty Gravity clears Damageable instead (native setter slip).
    pub fn read_anim_prop<'a>(
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        let attrs: Vec<_> = attrs.into_iter().collect();
        let mut action = Self::read(attrs.iter().copied())?;
        action.object = action
            .object
            .map(|o| format!("{}{o}", crate::play_content::ANIMATED_PROPS));
        let mut anim = AnimProp {
            animation: None,
            damageable: false,
            gravity: true,
        };
        let flag = |v: &str| v.trim().eq_ignore_ascii_case("true");
        for (key, value) in attrs {
            match key.to_ascii_lowercase().as_str() {
                "animation" => {
                    let name = value.trim().to_lowercase();
                    anim.animation = crate::dialog_enums::ANIMATIONS
                        .iter()
                        .any(|(n, _)| *n == name)
                        .then_some(name);
                }
                "damageable" => anim.damageable = flag(value),
                "gravity" if value.is_empty() => anim.damageable = false,
                "gravity" => anim.gravity = flag(value),
                _ => {}
            }
        }
        action.anim = Some(anim);
        Ok(action)
    }

    /// Initialize: positive MaxHitpoints uses a cached damageable entity and
    /// records its handle and full health; otherwise a plain PhysicsEntity.
    /// Each Initialize allocates again without releasing the previous entity.
    pub fn initialize<L: SpawnPhysLevel + ?Sized>(
        &self,
        state: &mut SpawnPhysState,
        level: &mut L,
    ) -> Result<(), L::Error> {
        let max = level.load_physics_template(self.object.as_deref())?;
        if let Some(anim) = &self.anim {
            // SpawnAnimProp records health only for a damageable prop.
            if level.physics_template_animated(self.object.as_deref()) {
                let handle = level.new_animated_entity()?;
                state.entity = Some(PhysEntity {
                    handle,
                    damageable: true,
                    animated: true,
                });
            } else {
                let damageable = max > 0;
                let handle = if damageable {
                    level.damageable_from_cache()?
                } else {
                    level.new_physics_entity()?
                };
                state.entity = Some(PhysEntity {
                    handle,
                    damageable,
                    animated: false,
                });
            }
            if max > 0 && anim.damageable {
                state.health = PhysHealth {
                    handle: state.entity.map_or(0, |e| e.handle as i32),
                    health: max as f32,
                };
            }
            return Ok(());
        }
        if max > 0 {
            let handle = level.damageable_from_cache()?;
            state.entity = Some(PhysEntity {
                handle,
                damageable: true,
                animated: false,
            });
            state.health = PhysHealth {
                handle: handle as i32,
                health: max as f32,
            };
        } else {
            state.entity = Some(PhysEntity {
                handle: level.new_physics_entity()?,
                damageable: false,
                animated: false,
            });
        }
        Ok(())
    }

    pub fn execute<H: SpawnPhysHost + ?Sized>(
        &self,
        state: &mut SpawnPhysState,
        host: &mut H,
    ) -> Result<(), SpawnPhysError<H::Error>> {
        // SpawnAnimProp.Execute has no hot-join branch.
        if self.anim.is_none() && host.is_hot_join_client() {
            if let Some(PhysEntity {
                handle,
                damageable: true,
                ..
            }) = state.entity
            {
                host.return_damageable_to_cache(handle)?;
            }
            state.entity = None;
            return Ok(());
        }
        let transform = host.locator(self.area)?;
        let entity = state.entity.ok_or(SpawnPhysError::MissingEntity)?;
        host.initialize_entity(
            entity.handle,
            self.object.as_deref(),
            transform,
            self.unique_id,
        )?;
        if entity.damageable {
            host.set_damage_triggers(entity.handle, self.on_death, self.on_damage)?;
        }
        if let (Some(anim), true) = (&self.anim, entity.animated) {
            host.animate_entity(entity.handle, anim)?;
        }
        host.add_entity(entity.handle)?;
        Ok(())
    }

    /// Update's bookkeeping before base Action.Update. QuickExecute is empty,
    /// so a restored tag health is overwritten here and never applied.
    /// SpawnAnimProp tracks only an AnimatedPhysicsEntity.
    pub fn track<H: SpawnPhysHost + ?Sized>(
        &self,
        state: &mut SpawnPhysState,
        host: &mut H,
    ) -> Result<(), H::Error> {
        if let Some(e) = state.entity {
            let tracked = if self.anim.is_some() {
                e.animated
            } else {
                e.damageable
            };
            if tracked {
                state.health.health = host.hit_points(e.handle)?;
            }
        }
        Ok(())
    }
}

pub fn write_tag(health: PhysHealth) -> [u8; 4] {
    health.health.to_le_bytes()
}
pub fn read_tag(bytes: &[u8]) -> Option<PhysHealth> {
    Some(PhysHealth {
        handle: 65535,
        health: f32::from_le_bytes(bytes.get(..4)?.try_into().ok()?),
    })
}
