//! Magicks: `Magick.Effect` animations and instants, and SpellManager's
//! IAbilityEffect list for the ported magick effects (research/magicks.md).
use crate::spells::MagickType;

/// SpecialAbility.Animation of `Magick.Effect` (each class's default
/// constructor). None for types `Magick.Effect` does not map.
pub fn animation(t: MagickType) -> Option<&'static str> {
    use MagickType as M;
    Some(match t {
        M::Haste
        | M::Revive
        | M::Teleport
        | M::Fear
        | M::Invisibility
        | M::TimeWarp
        | M::Corporealize
        | M::Nullify
        | M::Levitate
        | M::PerformanceEnchantment => "cast_magick_self",
        M::Rain
        | M::MeteorS
        | M::ThunderS
        | M::Blizzard
        | M::Vortex
        | M::JudgementSpray
        | M::Amalgameddon => "cast_magick_global",
        M::ThunderB
        | M::SUndead
        | M::SPhoenix
        | M::Tornado
        | M::SElemental
        | M::SDeath
        | M::ChainLightning
        | M::Confuse => "cast_magick_direct",
        M::Charm | M::Conflagration | M::Grease | M::Ctd | M::Napalm | M::Wave => {
            "cast_magick_sweep"
        }
        M::Portal => "cast_area_fireworks",
        M::TractorPull => "cast_area_ground",
        M::ProppMagick => "cast_spell1",
        M::Grow | M::Shrink => "cast_spell0",
        M::Etherealize
        | M::Votal
        | M::KnockDownChance
        | M::InflictBleedDamage
        | M::InflictPoisonDamage => "cast_magick_self",
        M::Polymorph
        | M::VladZap
        | M::SpawnSlime
        | M::SpawnSlimeOverkill
        | M::GreaseSplash
        | M::EtherealClone => "cast_magick_direct",
        M::Earthquake | M::SummonCross => "cast_magick_global",
        // Rewrite extension: registered (mod) Magicks use the directed
        // Magick cast; their handler decides what the cast does.
        M::Registered => "cast_magick_direct",
        _ => return None,
    })
}

/// Magick.IsInstant (INSTANT_LOOKUP): cast at once by Avatar.Boost while
/// frozen, flying or knocked down.
pub fn is_instant(t: MagickType) -> bool {
    use MagickType as M;
    matches!(
        t,
        M::Revive | M::Haste | M::Invisibility | M::Teleport | M::Napalm | M::ChainLightning
    )
}

/// SpellManager.IsMagickAllowed for Campaign/Mythos: any playing player's
/// UnlockedMagicks bit.
pub fn allowed(unlocked_any_player: u64, t: MagickType) -> bool {
    t != MagickType::None && unlocked_any_player & (1u64 << t as u8) != 0
}

/// Invisibility.INVISIBILITY_TIME.
pub const INVISIBILITY_TIME: f32 = 15.0;
/// Haste.TTL.
pub const HASTE_TTL: f32 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Haste {
    pub owner: u16,
    pub ttl: f32,
    /// CustomTTL (CoffeeBoost) override; the magick itself uses None.
    pub custom_ttl: Option<f32>,
}

/// Revive.Execute(ISpellCaster): the point 4 units along the caster's
/// direction, revived at 0.1 of MaxHitPoints. The position overload (no
/// caster) revives at full health.
pub const REVIVE_DISTANCE: f32 = 4.0;
pub const REVIVE_CASTER_HEALTH: f32 = 0.1;
/// Revive.Update: mTimer += dt·4 until it exceeds 1 (the avatar spawns), then
/// the effect lingers 3 s.
pub const REVIVE_RATE: f32 = 4.0;
pub const REVIVE_LINGER: f32 = 3.0;

/// CLR float.Epsilon.
const CLR_EPSILON: f32 = f32::from_bits(1);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Revive {
    /// mRevivee: the player slot.
    pub revivee: usize,
    /// mPosition (already snapped to the navmesh).
    pub position: [f32; 3],
    /// mHealthPercentage.
    pub health: f32,
    pub timer: f32,
    pub ressed: bool,
}

/// TimeWarp.DURATION_TIME (real seconds), TIME_TARGET, SATURATION_TARGET
/// and mFadeTime.
pub const TIME_WARP_DURATION: f32 = 15.0;
pub const TIME_WARP_TIME_TARGET: f32 = 0.5;
pub const TIME_WARP_SATURATION_TARGET: f32 = 0.1;
pub const TIME_WARP_FADE: f32 = 1.0;

/// TimeWarp (research/time-warp.md): eases PlayState.TimeMultiplier and the
/// render saturation in real time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeWarp {
    /// mOwner's handle (None: the position overload).
    pub owner: Option<u16>,
    pub ttl: f32,
    pub multiplier: f32,
    pub multiplier_target: f32,
    pub saturation: f32,
    pub saturation_target: f32,
}
impl TimeWarp {
    /// TimeWarp.Update. `scaled_dt` is SpellManager's delta (already
    /// multiplied by `time_scale` = TimeModifier·TimeMultiplier, the values
    /// of this frame). Writes the new TimeMultiplier and saturation.
    pub fn update(&mut self, scaled_dt: f32, time_scale: f32, scene_saturation: f32) {
        if self.ttl <= TIME_WARP_FADE {
            self.multiplier_target = 1.0;
            self.saturation_target = scene_saturation;
        }
        let dt = scaled_dt / time_scale;
        self.ttl -= dt;
        self.multiplier += (self.multiplier_target - self.multiplier) * dt;
        self.saturation += (self.saturation_target - self.saturation) * dt;
    }
}

/// Rain.MAGICK_TTL, the wet pulse period and its Status/Water magnitude,
/// and GameScene.LightTargetIntensity while it rains.
pub const RAIN_TTL: f32 = 17.5;
pub const RAIN_WET_PERIOD: f32 = 0.25;
pub const RAIN_MAGNITUDE: f32 = 0.25;
pub const RAIN_LIGHT_INTENSITY: f32 = 0.333;

/// Rain (research/rain.md), a singleton: casting again refreshes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rain {
    /// mCaster's handle (None: the position overload).
    pub caster: Option<u16>,
    /// mTimeStamp: the caster's PlayTime at the cast (0 without one).
    pub timestamp: f64,
    pub ttl: f32,
}

/// Thunderstorm (research/thunderstorm.md): the first bolt after 2 s, then
/// every 0.5-1 s; bolts land within 20 m of the caster and strike the
/// tallest damageable within 5 m (Y ignored) of the landing point.
pub const THUNDERSTORM_FIRST_BOLT: f32 = 2.0;
pub const THUNDERSTORM_RADIUS: f32 = 20.0;
pub const THUNDERSTORM_TARGET_RANGE: f32 = 5.0;

/// Thunderstorm, a singleton riding on Rain (IsDead is the Rain's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thunderstorm {
    /// mOwner's handle (None: the position overload).
    pub owner: Option<u16>,
    /// mPosition: the bolt centre without an owner.
    pub position: [f32; 3],
    /// mTimeStamp: the owner's PlayTime at the cast (0 without one).
    pub timestamp: f64,
    pub bolt_ttl: f32,
}

/// Blizzard's singleton state (research/blizzard.md). Damage and liquid
/// pulses continue during the fade after TTL reaches zero.
pub const BLIZZARD_TTL: f32 = 10.0;
pub const BLIZZARD_PERIOD: f32 = 0.25;
pub const BLIZZARD_SCROLL: [[f32; 2]; 2] = [[0.738724, 1.234238], [1.324678, 0.647534]];
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blizzard {
    pub caster: Option<u16>,
    pub timestamp: f64,
    pub ttl: f32,
    pub alpha: f32,
    pub offsets: [[f32; 2]; 2],
}
impl Blizzard {
    pub fn is_dead(&self) -> bool {
        self.ttl <= 0.0 && self.alpha <= 0.0
    }
}

/// Levitate.Execute's SetLevitate TTL (research/levitate.md).
pub const LEVITATE_TTL: f32 = 30.0;
/// Character.Update's levitation probe reach below the body.
pub const LEVITATE_PROBE: f32 = 0.75;

/// Character.mLevitateTimer and mLevitationFreeFallTimer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Levitation {
    pub timer: f32,
    pub free_fall: f32,
}
impl Levitation {
    /// Character.IsLevitating.
    pub fn active(&self, dead: bool) -> bool {
        self.timer > 0.0 && !dead
    }
    /// Character.SetLevitate: a positive TTL (re)starts it (gravity off, the
    /// effect restarted) and returns true; otherwise StopLevitate.
    pub fn set(&mut self, ttl: f32) -> bool {
        self.timer = ttl;
        if ttl > 0.0 {
            true
        } else {
            self.stop();
            false
        }
    }
    /// Character.StopLevitate (gravity back on, effect stopped).
    pub fn stop(&mut self) {
        self.timer = 0.0;
    }
    /// The ray Character.Update casts while the timer runs: from half the
    /// height offset above the position, `-0.75 + height_offset / 2` down
    /// (height_offset is negative: centre to feet).
    pub fn probe(position: [f32; 3], height_offset: f32) -> ([f32; 3], [f32; 3]) {
        (
            [position[0], position[1] + height_offset * 0.5, position[2]],
            [0.0, -LEVITATE_PROBE + height_offset * 0.5, 0.0],
        )
    }
    /// Character.Update with the timer running. `hit` is the probe's
    /// fraction on a liquid, else on the scene. Returns the velocity Y
    /// change and whether the levitation ended (StopLevitate) this frame.
    pub fn tick(&mut self, dt: f32, hit: Option<f32>) -> (f32, bool) {
        self.timer -= dt;
        let dv = match hit {
            Some(fraction) => {
                self.free_fall = 0.0;
                (1.0 - fraction) * LEVITATE_PROBE
            }
            None => {
                let dv = -dt * (0.5 + 1.5 * self.free_fall);
                self.free_fall = (self.free_fall + dt * 0.5).min(1.0);
                dv
            }
        };
        let ended = self.timer <= 0.0;
        if ended {
            self.stop();
        }
        (dv, ended)
    }
}

/// Confuse.DEFAULT_TTL and RADIUS (research/confuse.md).
pub const CONFUSE_TTL: f32 = 15.0;
pub const CONFUSE_RADIUS: f32 = 14.0;

/// Confuse: one pooled instance per confused character.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Confuse {
    /// mTarget's handle.
    pub target: u16,
    pub ttl: f32,
}

/// One SpellManager.mEffects entry.
#[derive(Clone, Debug, PartialEq)]
pub enum AbilityEffect {
    Haste(Haste),
    Revive(Revive),
    TimeWarp(TimeWarp),
    Rain(Rain),
    Confuse(Confuse),
    Thunderstorm(Thunderstorm),
    Blizzard(Blizzard),
    Conflagration {
        sweep: crate::conflagration::Conflagration,
        /// Opaque identity for the host's retained particle reference.
        instance: u64,
    },
    /// Grease (research/grease.md): one pooled spray per cast, 0.5 s.
    Grease(GreaseSpray),
    /// Napalm (research/napalm.md), a singleton driven by the host's
    /// `update_napalm` (strikes need the scene).
    Napalm(crate::napalm::Napalm),
    /// MeteorShower (research/meteor-shower.md), a singleton.
    MeteorShower(MeteorShower),
    /// PerformanceEnchantment (research/performance-enchantment.md), one
    /// per owner.
    Performance(Performance),
    /// JudgementSpray (research/judgement-spray.md), a singleton: owner
    /// and TTL.
    Judgement {
        owner: u16,
        ttl: f32,
    },
    /// Amalgameddon (research/amalgameddon.md), a singleton driven by the
    /// host's `update_amalgameddon` (it moves the units).
    Amalgameddon(crate::amalgameddon::Amalgameddon),
}

/// JudgementSpray.TTL and NUM_OF_PROJECTILES: 20 missiles over 1 s.
pub const JUDGEMENT_TTL: f32 = 1.0;
pub const JUDGEMENT_MISSILES: f32 = 20.0;

/// PerformanceEnchantment: 10 s, boosted until 4 s remain.
pub const PERFORMANCE_TTL: f32 = 10.0;
pub const PERFORMANCE_WITHDRAWAL: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Performance {
    pub owner: u16,
    pub ttl: f32,
    pub boosted: bool,
}

/// MeteorShower.LIFE_TIME, the spawn cut-off, the light target and the
/// meteor spawn geometry.
pub const METEOR_TTL: f32 = 17.5;
pub const METEOR_LAST_SPAWN: f32 = 2.5;
pub const METEOR_LIGHT_INTENSITY: f32 = 0.4;
pub const METEOR_RADIUS: f32 = 20.0;
pub const METEOR_HEIGHT: f32 = 146.0;
pub const METEOR_VELOCITY: [f32; 3] = [0.01, -20.0, -0.01];
pub const METEOR_SPLASH: f32 = 4.0;

/// MeteorShower: the owner (None for the position overload), mPosition
/// and the TTL.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeteorShower {
    pub owner: Option<u16>,
    pub position: [f32; 3],
    pub ttl: f32,
}

/// Grease: the caster and the spray TTL (grease::SPRAY_TTL at the cast).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GreaseSpray {
    pub owner: u16,
    pub ttl: f32,
}

/// Revive.Execute's scan of Game.Players: `dead_ages[i]` is Player.DeadAge
/// when player `i` is eligible (playing, avatar absent, overkilled or dead
/// without Undying, versus CanRevive) and None otherwise. The greatest
/// DeadAge above float.Epsilon wins; ties keep the earlier slot.
pub fn select_revivee(dead_ages: impl IntoIterator<Item = Option<f32>>) -> Option<usize> {
    let mut best = CLR_EPSILON;
    let mut chosen = None;
    for (i, age) in dead_ages.into_iter().enumerate() {
        if let Some(age) = age.filter(|&a| a > best) {
            best = age;
            chosen = Some(i);
        }
    }
    chosen
}

#[derive(Clone, Debug, Default)]
pub struct AbilityEffects {
    pub effects: Vec<AbilityEffect>,
    /// Rain.mWetTimer: a field of the singleton, so it carries over between
    /// casts (zero before the first).
    pub rain_wet_timer: f32,
    /// Set by `update` when Rain's wet timer ran out this frame: the
    /// (caster, timestamp) of its Status/Water damage pass.
    pub rain_pulse: Option<(Option<u16>, f64)>,
    /// Targets of Confuse effects removed by `update` or `clear_magicks`
    /// whose OnRemove (controls back, template faction) the host still has
    /// to apply.
    pub confuse_ended: Vec<u16>,
    /// Retained particle references to stop after expiry or ClearMagicks.
    pub conflagration_ended: Vec<u64>,
    /// Set by `update_thunderstorm` when the bolt timer ran out (its state
    /// then): the host (not a client) rolls the next timer and 1-2 bolts,
    /// even when the Rain ended in the same pass.
    pub storm_bolts_due: Option<Thunderstorm>,
    /// MeteorShower.mMeteorSpawnTimer: a singleton field, so it carries
    /// over between casts (0 before the first: the first meteor falls at
    /// once).
    pub meteor_timer: f32,
    /// Set by `update` when the meteor timer ran out (the state then); the
    /// host rolls 1-2 meteors and adds 0.4-0.8 s to `meteor_timer`.
    pub meteors_due: Option<MeteorShower>,
    /// PerformanceEnchantment.Withdrawal ran this update (owners): the host
    /// stops the effect and plays the debuff cue.
    pub performance_withdrawn: Vec<u16>,
    /// PerformanceEnchantment.IsDead (owners): ClearAura on the owner.
    pub performance_ended: Vec<u16>,
    /// JudgementSpray.mInterval and mLeft: singleton fields, kept between
    /// casts.
    pub judgement_interval: f32,
    pub judgement_left: bool,
    /// Owners whose JudgementSpray interval ran out this update (one
    /// missile each; the host spawns it).
    pub judgement_due: Vec<u16>,
    /// Singleton cold timer; Execute does not reset it.
    pub blizzard_cold_timer: f32,
    /// One Cold/FreezeAll pass, including the final update before removal.
    pub blizzard_pulse: Option<(Option<u16>, f64)>,
    /// OnRemove leaves alpha and texture offsets in the singleton. A recast
    /// after Nullify can resume a still-visible overlay.
    blizzard_cached: Option<Blizzard>,
}
impl AbilityEffects {
    pub fn conflagration(&mut self, sweep: crate::conflagration::Conflagration, instance: u64) {
        // A cached instance per cast, not a singleton refresh.
        self.effects
            .push(AbilityEffect::Conflagration { sweep, instance });
    }

    pub fn update_conflagrations(
        &mut self,
        dt: f32,
        mut update: impl FnMut(&mut crate::conflagration::Conflagration, u64, f32),
    ) {
        let ended = &mut self.conflagration_ended;
        self.effects.retain_mut(|effect| {
            let AbilityEffect::Conflagration { sweep, instance } = effect else {
                return true;
            };
            update(sweep, *instance, dt);
            if sweep.is_dead() {
                ended.push(*instance);
                false
            } else {
                true
            }
        });
    }

    /// Confuse.Execute(owner, target, ...) for a character target: an active
    /// Confuse on the same target is refreshed to 15 s (false); otherwise a
    /// new effect starts (true) and the host inverts/re-factions the target.
    pub fn confuse(&mut self, target: u16) -> bool {
        for effect in &mut self.effects {
            if let AbilityEffect::Confuse(c) = effect {
                if c.target == target {
                    c.ttl = CONFUSE_TTL;
                    return false;
                }
            }
        }
        self.effects.push(AbilityEffect::Confuse(Confuse {
            target,
            ttl: CONFUSE_TTL,
        }));
        true
    }
    pub fn confused(&self, target: u16) -> bool {
        self.effects
            .iter()
            .any(|e| matches!(e, AbilityEffect::Confuse(c) if c.target == target))
    }

    /// SpellManager.ClearMagicks: every effect is a SpecialAbility; OnRemove
    /// of each Confuse is reported through `confuse_ended`.
    pub fn clear_magicks(&mut self) {
        self.storm_bolts_due = None;
        self.blizzard_pulse = None;
        for effect in self.effects.drain(..) {
            if let AbilityEffect::Conflagration { instance, .. } = &effect {
                self.conflagration_ended.push(*instance);
            }
            if let AbilityEffect::Confuse(c) = effect {
                self.confuse_ended.push(c.target);
            }
            if let AbilityEffect::Blizzard(mut b) = effect {
                b.ttl = 0.0;
                self.blizzard_cached = Some(b);
            }
        }
    }

    /// Blizzard.Execute after the caller's outdoor check. Unlike Rain,
    /// an active singleton preserves alpha and scroll offsets on recast.
    pub fn blizzard(&mut self, caster: Option<u16>, timestamp: f64) {
        if let Some(b) = self.effects.iter_mut().find_map(|e| match e {
            AbilityEffect::Blizzard(b) => Some(b),
            _ => None,
        }) {
            b.caster = caster;
            b.timestamp = timestamp;
            b.ttl = BLIZZARD_TTL;
            return;
        }
        let cached = self.blizzard_cached.filter(|b| !b.is_dead());
        self.effects.push(AbilityEffect::Blizzard(Blizzard {
            caster,
            timestamp,
            ttl: BLIZZARD_TTL,
            alpha: cached.map_or(0.0, |b| b.alpha),
            offsets: cached.map_or([[0.0, 0.5], [0.5, 0.0]], |b| b.offsets),
        }));
    }
    pub fn blizzard_state(&self) -> Option<&Blizzard> {
        self.effects.iter().find_map(|e| match e {
            AbilityEffect::Blizzard(b) => Some(b),
            _ => None,
        })
    }

    /// Haste.Execute for a character owner: refresh an active haste of the
    /// same owner (10 s, or max(custom, current)), else add a new effect.
    pub fn haste(&mut self, owner: u16, custom_ttl: Option<f32>) {
        for effect in &mut self.effects {
            let AbilityEffect::Haste(h) = effect else {
                continue;
            };
            if h.owner == owner {
                h.ttl = custom_ttl.map_or(HASTE_TTL, |c| c.max(h.ttl));
                return;
            }
        }
        self.effects.push(AbilityEffect::Haste(Haste {
            owner,
            ttl: custom_ttl.unwrap_or(HASTE_TTL),
            custom_ttl,
        }));
    }
    pub fn hasted(&self, owner: u16) -> bool {
        self.effects
            .iter()
            .any(|e| matches!(e, AbilityEffect::Haste(h) if h.owner == owner))
    }

    /// Revive.Execute after a revivee was chosen (SpellManager.AddSpellEffect).
    pub fn revive(&mut self, revivee: usize, position: [f32; 3], health: f32) {
        self.effects.push(AbilityEffect::Revive(Revive {
            revivee,
            position,
            health,
            timer: 0.0,
            ressed: false,
        }));
    }
    /// Player.Ressing: set by Revive.Execute, cleared when the avatar spawns
    /// or the effect is removed (OnRemove, e.g. SpellManager.ClearMagicks).
    pub fn ressing(&self, slot: usize) -> bool {
        self.effects
            .iter()
            .any(|e| matches!(e, AbilityEffect::Revive(r) if r.revivee == slot && !r.ressed))
    }

    /// TimeWarp.Execute: refused (false) while a TimeWarp is active
    /// (TimeWarpStaff is not reconstructed). Starts at multiplier 1 and the
    /// scene's saturation.
    pub fn time_warp(&mut self, owner: Option<u16>, scene_saturation: f32) -> bool {
        if self.time_warp_state().is_some() {
            return false;
        }
        self.effects.push(AbilityEffect::TimeWarp(TimeWarp {
            owner,
            ttl: TIME_WARP_DURATION,
            multiplier: 1.0,
            multiplier_target: TIME_WARP_TIME_TARGET,
            saturation: scene_saturation,
            saturation_target: TIME_WARP_SATURATION_TARGET,
        }));
        true
    }
    /// Rain.Execute (outdoors; the caller checks Indoors): TTL 17.5 and the
    /// caster/timestamp of the singleton. AddSpellEffect skips the instance
    /// already in the list, so a second cast only refreshes it.
    pub fn rain(&mut self, caster: Option<u16>, timestamp: f64) {
        let rain = Rain {
            caster,
            timestamp,
            ttl: RAIN_TTL,
        };
        for effect in &mut self.effects {
            if let AbilityEffect::Rain(r) = effect {
                *r = rain;
                return;
            }
        }
        self.effects.push(AbilityEffect::Rain(rain));
    }
    /// Thunderstorm.Execute after Rain.Execute (outdoors; the caller checks
    /// Indoors and runs the Rain first): owner, position and timestamp, and
    /// the first bolt 2 s later. Recasting reinitialises the singleton.
    pub fn thunderstorm(&mut self, owner: Option<u16>, position: [f32; 3], timestamp: f64) {
        let storm = Thunderstorm {
            owner,
            position,
            timestamp,
            bolt_ttl: THUNDERSTORM_FIRST_BOLT,
        };
        for effect in &mut self.effects {
            if let AbilityEffect::Thunderstorm(t) = effect {
                *t = storm;
                return;
            }
        }
        self.effects.push(AbilityEffect::Thunderstorm(storm));
    }
    /// Grease.Execute for a character owner (AddSpellEffect of a new pooled
    /// instance: two casts spray side by side).
    pub fn grease(&mut self, owner: u16) {
        self.effects.push(AbilityEffect::Grease(GreaseSpray {
            owner,
            ttl: crate::grease::SPRAY_TTL,
        }));
    }
    /// MeteorShower.Execute outdoors: an active shower only gets TTL 17.5
    /// (the position overload also moves it); otherwise a new one. Returns
    /// whether it was new (the host then dims the light).
    pub fn meteor_shower(&mut self, owner: Option<u16>, position: [f32; 3]) -> bool {
        for effect in &mut self.effects {
            if let AbilityEffect::MeteorShower(m) = effect {
                m.ttl = METEOR_TTL;
                if owner.is_none() {
                    m.position = position;
                }
                return false;
            }
        }
        self.effects.push(AbilityEffect::MeteorShower(MeteorShower {
            owner,
            position,
            ttl: METEOR_TTL,
        }));
        true
    }
    pub fn meteor_shower_state(&self) -> Option<&MeteorShower> {
        self.effects.iter().find_map(|e| match e {
            AbilityEffect::MeteorShower(m) => Some(m),
            _ => None,
        })
    }

    /// PerformanceEnchantment.Execute for a character: an owner's active
    /// enchantment is refreshed to 10 s and boosted again (false: no new
    /// auras); otherwise a new one (true: the host adds the boost auras).
    pub fn performance(&mut self, owner: u16) -> bool {
        for effect in &mut self.effects {
            if let AbilityEffect::Performance(p) = effect {
                if p.owner == owner {
                    p.ttl = PERFORMANCE_TTL;
                    p.boosted = true;
                    return false;
                }
            }
        }
        self.effects.push(AbilityEffect::Performance(Performance {
            owner,
            ttl: PERFORMANCE_TTL,
            boosted: true,
        }));
        true
    }
    pub fn performance_state(&self, owner: u16) -> Option<&Performance> {
        self.effects.iter().find_map(|e| match e {
            AbilityEffect::Performance(p) if p.owner == owner => Some(p),
            _ => None,
        })
    }

    /// JudgementSpray.Execute: the singleton takes the new owner and TTL 1
    /// (AddSpellEffect keeps one instance).
    pub fn judgement_spray(&mut self, owner: u16) {
        for effect in &mut self.effects {
            if let AbilityEffect::Judgement { owner: o, ttl } = effect {
                *o = owner;
                *ttl = JUDGEMENT_TTL;
                return;
            }
        }
        self.effects.push(AbilityEffect::Judgement {
            owner,
            ttl: JUDGEMENT_TTL,
        });
    }

    pub fn napalm_mut(&mut self) -> Option<&mut crate::napalm::Napalm> {
        self.effects.iter_mut().find_map(|e| match e {
            AbilityEffect::Napalm(n) => Some(n),
            _ => None,
        })
    }
    pub fn napalm_state(&self) -> Option<&crate::napalm::Napalm> {
        self.effects.iter().find_map(|e| match e {
            AbilityEffect::Napalm(n) => Some(n),
            _ => None,
        })
    }

    pub fn grease_sprays(&self) -> Vec<GreaseSpray> {
        self.effects
            .iter()
            .filter_map(|e| match e {
                AbilityEffect::Grease(g) => Some(*g),
                _ => None,
            })
            .collect()
    }

    pub fn thunderstorm_state(&self) -> Option<&Thunderstorm> {
        self.effects.iter().find_map(|e| match e {
            AbilityEffect::Thunderstorm(t) => Some(t),
            _ => None,
        })
    }
    pub fn thunderstorm_mut(&mut self) -> Option<&mut Thunderstorm> {
        self.effects.iter_mut().find_map(|e| match e {
            AbilityEffect::Thunderstorm(t) => Some(t),
            _ => None,
        })
    }

    /// Thunderstorm.Update returns before advancing its timer on clients.
    /// The host calls this before effect removal, allowing a last-frame bolt.
    pub fn update_thunderstorm(&mut self, dt: f32) {
        self.storm_bolts_due = None;
        if let Some(t) = self.thunderstorm_mut() {
            t.bolt_ttl -= dt;
            if t.bolt_ttl <= 0.0 {
                self.storm_bolts_due = Some(*t);
            }
        }
    }

    pub fn rain_state(&self) -> Option<&Rain> {
        self.effects.iter().find_map(|e| match e {
            AbilityEffect::Rain(r) => Some(r),
            _ => None,
        })
    }

    pub fn time_warp_state(&self) -> Option<&TimeWarp> {
        self.effects.iter().find_map(|e| match e {
            AbilityEffect::TimeWarp(w) => Some(w),
            _ => None,
        })
    }

    /// SpellManager.Update: each effect's Update, then removal when dead.
    /// `time_scale` and `scene_saturation` feed TimeWarp.Update; afterwards
    /// `time_warp_state()` is None once it was removed (OnRemove restores
    /// TimeMultiplier 1 and the scene saturation).
    /// `apply_speed(owner, factor)` multiplies the owner's SpeedMultiplier
    /// and returns false when its body is not enabled. `spawn(revive)` is
    /// Revive.Update's avatar creation once the timer exceeds 1 (the host
    /// checks that the revivee still plays). `alive(target)` is Confuse's
    /// check that its target still exists with HitPoints > 0.
    pub fn update(
        &mut self,
        dt: f32,
        time_scale: f32,
        scene_saturation: f32,
        mut apply_speed: impl FnMut(u16, f32) -> bool,
        mut spawn: impl FnMut(&Revive),
        mut alive: impl FnMut(u16) -> bool,
    ) {
        let wet_timer = &mut self.rain_wet_timer;
        let pulse = &mut self.rain_pulse;
        let ended = &mut self.confuse_ended;
        let cold_timer = &mut self.blizzard_cold_timer;
        let cold_pulse = &mut self.blizzard_pulse;
        let cached = &mut self.blizzard_cached;
        let judgement_interval = &mut self.judgement_interval;
        let judgement_due = &mut self.judgement_due;
        let withdrawn = &mut self.performance_withdrawn;
        let performance_ended = &mut self.performance_ended;
        let meteor_timer = &mut self.meteor_timer;
        let meteors_due = &mut self.meteors_due;
        self.effects.retain_mut(|effect| match effect {
            // JudgementSpray.Update: one missile whenever the interval drops
            // below zero (then 1/20 s); IsDead at TTL <= 0.
            AbilityEffect::Judgement { owner, ttl } => {
                *ttl -= dt;
                *judgement_interval -= dt;
                if *judgement_interval < 0.0 {
                    *judgement_interval = JUDGEMENT_TTL / JUDGEMENT_MISSILES;
                    judgement_due.push(*owner);
                }
                *ttl > 0.0
            }
            // PerformanceEnchantment.Update: boosted speed 1 + (1 - 0.5^(ttl-4)),
            // then (Withdrawal) 0.5 + (0.5 - 0.75^ttl); IsDead at ttl <= 0.
            AbilityEffect::Performance(p) => {
                p.ttl -= dt;
                if p.ttl < PERFORMANCE_WITHDRAWAL {
                    if p.boosted {
                        p.boosted = false;
                        withdrawn.push(p.owner);
                    }
                    let num = 0.75f64.powf(f64::from(p.ttl)) as f32;
                    apply_speed(p.owner, 0.5 + (0.5 - num));
                } else {
                    let num = 0.5f64.powf(f64::from(p.ttl - PERFORMANCE_WITHDRAWAL)) as f32;
                    apply_speed(p.owner, 1.0 + (1.0 - num));
                }
                if p.ttl <= 0.0 {
                    performance_ended.push(p.owner);
                }
                p.ttl > 0.0
            }
            // MeteorShower.Update: spawns only while more than 2.5 s remain
            // (clients ignore `meteors_due`).
            AbilityEffect::MeteorShower(m) => {
                m.ttl -= dt;
                if m.ttl > METEOR_LAST_SPAWN {
                    *meteor_timer -= dt;
                    if *meteor_timer <= 0.0 {
                        *meteors_due = Some(*m);
                    }
                }
                m.ttl > 0.0
            }
            // Grease.Update's world work runs in the host before this (it
            // needs the TTL before and after); here only the TTL.
            AbilityEffect::Grease(g) => {
                g.ttl -= dt;
                g.ttl > 0.0
            }
            // These need synchronous entity queries and damage delivery.
            // The world calls update_conflagrations in the same frame.
            AbilityEffect::Conflagration { .. }
            | AbilityEffect::Napalm(_)
            | AbilityEffect::Amalgameddon(_) => true,
            AbilityEffect::Blizzard(b) => {
                b.ttl -= dt;
                *cold_timer -= dt;
                b.alpha = if b.ttl > 0.0 {
                    (b.alpha + dt).min(1.0)
                } else {
                    (b.alpha - dt).max(0.0)
                };
                for (offset, speed) in b.offsets.iter_mut().zip(BLIZZARD_SCROLL) {
                    for i in 0..2 {
                        offset[i] -= dt * speed[i];
                    }
                }
                if *cold_timer <= 0.0 {
                    *cold_pulse = Some((b.caster, b.timestamp));
                    *cold_timer = BLIZZARD_PERIOD;
                }
                *cached = Some(*b);
                !b.is_dead()
            }
            AbilityEffect::Thunderstorm(_) => true,
            AbilityEffect::Confuse(c) => {
                // Confuse.Update: a missing or dead (HitPoints <= 0) target
                // ends it at once; the `confused` effect follows the target.
                if alive(c.target) {
                    c.ttl -= dt;
                } else {
                    c.ttl = 0.0;
                }
                if c.ttl <= 0.0 {
                    ended.push(c.target);
                }
                c.ttl > 0.0
            }
            AbilityEffect::Rain(r) => {
                // Rain.Update: TTL and the wet timer; every 0.25 s a damage
                // pass (only with mDoDamage, decided by the host).
                r.ttl -= dt;
                *wet_timer -= dt;
                if *wet_timer <= 0.0 {
                    *pulse = Some((r.caster, r.timestamp));
                    *wet_timer = RAIN_WET_PERIOD;
                }
                r.ttl > 0.0
            }
            AbilityEffect::Revive(r) => {
                if !r.ressed {
                    r.timer += dt * REVIVE_RATE;
                    if r.timer > 1.0 {
                        spawn(r);
                        r.timer = REVIVE_LINGER;
                        r.ressed = true;
                    }
                    true
                } else {
                    r.timer -= dt;
                    r.timer > 0.0
                }
            }
            AbilityEffect::TimeWarp(w) => {
                w.update(dt, time_scale, scene_saturation);
                w.ttl > 0.0
            }
            AbilityEffect::Haste(h) => {
                h.ttl -= dt;
                let num = 0.5f64.powf(f64::from(h.ttl)) as f32;
                if !apply_speed(h.owner, 1.0 + (1.0 - num)) {
                    h.ttl = 0.0;
                }
                h.ttl > 0.0
            }
        });
        // Thunderstorm.IsDead is its Rain's: both go in the same pass.
        if self.rain_state().is_none() {
            self.effects
                .retain(|e| !matches!(e, AbilityEffect::Thunderstorm(_)));
        }
    }
}
