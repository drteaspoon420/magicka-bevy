//! SpawnEffect/StopEffect/StartEffect trigger actions and GameScene's named
//! visual-effect registry (mEffects) with its per-frame range clipping.
use crate::script_attributes::{identifier, number, ScriptReadError};
use magicka_content::skinned::mul;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EffectActionKind {
    Spawn {
        effect: i32,
        name: i32,
        area: i32,
        range: f32,
    },
    Stop {
        name: i32,
    },
    /// `None` when Name is null or empty: Execute asks for a generated ID.
    Start {
        name: Option<i32>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectAction {
    pub delay: f32,
    pub kind: EffectActionKind,
}

pub const TAGS: &[&str] = &["spawneffect", "stopeffect", "starteffect"];

pub trait EffectActionHost {
    type Error;
    /// GameScene.GetLocator(int, out Matrix), including its exception.
    fn locator(&mut self, area: i32) -> Result<[f32; 16], Self::Error>;
    /// EffectManager.GetEffect(effect).Type == VisualEffectType.Single.
    fn effect_is_single(&mut self, effect: i32) -> Result<bool, Self::Error>;
    fn add_effect(
        &mut self,
        id: i32,
        effect: i32,
        transform: [f32; 16],
        range: f32,
    ) -> Result<(), Self::Error>;
    fn stop_effect(&mut self, id: i32) -> Result<bool, Self::Error>;
    fn start_effect(&mut self, id: i32) -> Result<(), Self::Error>;
    fn generated_effect_id(&mut self) -> Result<i32, Self::Error>;
}

impl EffectAction {
    pub fn read<'a>(
        tag: &str,
        attrs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ScriptReadError> {
        use EffectActionKind::*;
        let mut kind = match tag.to_ascii_lowercase().as_str() {
            "spawneffect" => Spawn {
                effect: 0,
                name: 0,
                area: 0,
                range: 0.0,
            },
            "stopeffect" => Stop { name: 0 },
            "starteffect" => Start { name: None },
            _ => return Err(ScriptReadError::UnsupportedTag(tag.into())),
        };
        let mut delay = 0.0;
        for (key, value) in attrs {
            match (&mut kind, key.to_ascii_lowercase().as_str()) {
                (_, "delay") => delay = number(value, key)?,
                (Spawn { effect, .. }, "effect") => *effect = identifier(value)?,
                (Spawn { name, .. }, "name") | (Stop { name }, "name") => {
                    *name = identifier(value)?
                }
                (Spawn { area, .. }, "area") => *area = identifier(value)?,
                (Spawn { range, .. }, "range") => *range = number(value, key)?,
                (Start { name }, "name") => {
                    *name = (!value.is_empty()).then(|| identifier(value)).transpose()?
                }
                _ => {}
            }
        }
        Ok(Self { delay, kind })
    }

    pub fn execute<H: EffectActionHost + ?Sized>(&self, host: &mut H) -> Result<(), H::Error> {
        match self.kind {
            EffectActionKind::Spawn {
                effect,
                name,
                area,
                range,
            } => {
                let transform = host.locator(area)?;
                host.add_effect(name, effect, transform, range)
            }
            EffectActionKind::Stop { name } => host.stop_effect(name).map(|_| ()),
            EffectActionKind::Start { name } => {
                let id = match name {
                    Some(id) => id,
                    None => host.generated_effect_id()?,
                };
                host.start_effect(id)
            }
        }
    }

    /// SpawnEffect respawns only non-Single effect types; Stop/Start repeat Execute.
    pub fn quick_execute<H: EffectActionHost + ?Sized>(
        &self,
        host: &mut H,
    ) -> Result<(), H::Error> {
        if let EffectActionKind::Spawn { effect, .. } = self.kind {
            if host.effect_is_single(effect)? {
                return Ok(());
            }
        }
        self.execute(host)
    }
}

/// VisualEffect value operations (EffectManager.GetEffect returns a value copy).
pub trait EffectBackend {
    type Error;
    type Effect;
    fn get_effect(&mut self, effect: i32) -> Result<Self::Effect, Self::Error>;
    fn is_active(&self, effect: &Self::Effect) -> bool;
    /// VisualEffect.Start(ref transform); also stores the effect's own Transform.
    fn start(&mut self, effect: &mut Self::Effect, transform: [f32; 16])
        -> Result<(), Self::Error>;
    fn stop(&mut self, effect: &mut Self::Effect) -> Result<(), Self::Error>;
    /// The effect's own Transform field, used by StartEffect.
    fn transform(&self, effect: &Self::Effect) -> [f32; 16];
    fn set_transform(&mut self, effect: &mut Self::Effect, transform: [f32; 16]);
    fn update(&mut self, effect: &mut Self::Effect, dt: f32) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectStorage<E> {
    pub effect: E,
    pub transform: [f32; 16],
    /// AnimatedLevelPart owner for level-placed effects (None for SpawnEffect).
    pub animation: Option<i32>,
    pub range: f32,
}

/// GameScene.mEffects (SortedList by signed ID) and mLastGeneratedEffectID.
pub struct SceneEffects<E> {
    pub effects: BTreeMap<i32, EffectStorage<E>>,
    pub last_generated: i32,
}
impl<E> Default for SceneEffects<E> {
    fn default() -> Self {
        Self {
            effects: BTreeMap::new(),
            last_generated: 0,
        }
    }
}
impl<E> SceneEffects<E> {
    /// AddEffect: only when the ID is absent or its effect is inactive. The
    /// stored transform is the locator; any previous animation link is cleared.
    pub fn add<B: EffectBackend<Effect = E>>(
        &mut self,
        backend: &mut B,
        id: i32,
        effect: i32,
        transform: [f32; 16],
        range: f32,
    ) -> Result<(), B::Error> {
        if self
            .effects
            .get(&id)
            .is_some_and(|s| backend.is_active(&s.effect))
        {
            return Ok(());
        }
        let mut value = backend.get_effect(effect)?;
        backend.start(&mut value, transform)?;
        self.effects.insert(
            id,
            EffectStorage {
                effect: value,
                transform,
                animation: None,
                range,
            },
        );
        Ok(())
    }
    /// StartEffect restarts an existing inactive effect at its own Transform.
    pub fn start<B: EffectBackend<Effect = E>>(
        &mut self,
        backend: &mut B,
        id: i32,
    ) -> Result<(), B::Error> {
        if let Some(s) = self.effects.get_mut(&id) {
            if !backend.is_active(&s.effect) {
                let transform = backend.transform(&s.effect);
                backend.start(&mut s.effect, transform)?;
            }
        }
        Ok(())
    }
    pub fn stop<B: EffectBackend<Effect = E>>(
        &mut self,
        backend: &mut B,
        id: i32,
    ) -> Result<bool, B::Error> {
        match self.effects.get_mut(&id) {
            Some(s) => backend.stop(&mut s.effect).map(|_| true),
            None => Ok(false),
        }
    }
    /// Scans upward from the last generated ID past occupied keys, stopping at
    /// zero even if occupied (int overflow wraps like unchecked C#).
    pub fn generated_id(&mut self) -> i32 {
        let mut i = self.last_generated.wrapping_add(1);
        while self.effects.contains_key(&i) && i != 0 {
            i = i.wrapping_add(1);
        }
        self.last_generated = i;
        i
    }
    /// GameScene.Update's effect loop. `segment_fraction(origin, delta)` returns
    /// the clipped fraction (level hit, then any closer shield hit; 1 if none).
    pub fn update<B: EffectBackend<Effect = E>>(
        &mut self,
        backend: &mut B,
        dt: f32,
        mut animated: impl FnMut(i32) -> [f32; 16],
        mut segment_fraction: impl FnMut([f32; 3], [f32; 3]) -> f32,
    ) -> Result<(), B::Error> {
        for s in self.effects.values_mut() {
            if !backend.is_active(&s.effect) {
                continue;
            }
            let mut m = match s.animation {
                Some(part) => mul(&s.transform, &animated(part)),
                None => s.transform,
            };
            if s.range > 0.0 {
                let origin = [m[12], m[13], m[14]];
                let delta = [-m[8] * s.range, -m[9] * s.range, -m[10] * s.range];
                let fraction = segment_fraction(origin, delta);
                for row in 0..3 {
                    for col in 0..3 {
                        m[row * 4 + col] *= fraction;
                    }
                }
            }
            backend.set_transform(&mut s.effect, m);
            backend.update(&mut s.effect, dt)?;
        }
        Ok(())
    }
}
