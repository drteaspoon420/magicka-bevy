//! Rewrite extension: the world operations mod scripts compose Magicks
//! from (docs/MODDING.md §5.5). Each one is a native routine the built-in
//! Magicks already use (missiles, hits, bolts, statuses, weather, native
//! magick entities), so a Lua Magick and a native one share the same code.
//! Clients never execute them: the host's results replicate.
use super::*;
use magicka_sim::damage::Damage;
use magicka_sim::spells::Spell;

/// The native simulated entities a Magick can start (`Entities.SpawnNative`).
/// Each takes its pose from its owner, as the built-in Magick does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeKind {
    Tornado,
    Vortex,
    Wave,
    Portal,
    Phoenix,
    Death,
    Elemental,
    Earthquake,
    Cross,
    Slime,
    SlimeOverkill,
    Clone,
    Napalm,
    Conflagration,
    Propp,
    Amalgameddon,
    Judgement,
    GreaseSpray,
}
impl NativeKind {
    pub const ALL: [(&'static str, NativeKind); 18] = [
        ("tornado", Self::Tornado),
        ("vortex", Self::Vortex),
        ("wave", Self::Wave),
        ("portal", Self::Portal),
        ("phoenix", Self::Phoenix),
        ("death", Self::Death),
        ("elemental", Self::Elemental),
        ("earthquake", Self::Earthquake),
        ("cross", Self::Cross),
        ("slime", Self::Slime),
        ("slime_overkill", Self::SlimeOverkill),
        ("clone", Self::Clone),
        ("napalm", Self::Napalm),
        ("conflagration", Self::Conflagration),
        ("propp", Self::Propp),
        ("amalgameddon", Self::Amalgameddon),
        ("judgement_spray", Self::Judgement),
        ("grease_spray", Self::GreaseSpray),
    ];
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, k)| *k)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weather {
    Rain,
    Blizzard,
}

/// Status names Lua uses (lowercase).
pub const STATUSES: [(&str, magicka_sim::status::Status); 9] = {
    use magicka_sim::status::Status as S;
    [
        ("burning", S::Burning),
        ("wet", S::Wet),
        ("frozen", S::Frozen),
        ("cold", S::Cold),
        ("poisoned", S::Poisoned),
        ("healing", S::Healing),
        ("greased", S::Greased),
        ("steamed", S::Steamed),
        ("bleeding", S::Bleeding),
    ]
};

/// One world operation with resolved handles.
#[derive(Clone, Debug, PartialEq)]
pub enum WorldOp {
    /// IDamageable.Damage through the public hit route.
    Hit {
        target: u16,
        damage: Damage,
        source: Option<u16>,
        from: Option<[f32; 3]>,
    },
    /// Character.AddStatusEffect (InflictBleedDamage, InflictPoisonDamage).
    Status {
        target: u16,
        status: magicka_sim::status::Status,
        dps: f32,
        magnitude: f32,
    },
    /// A native spell missile (MeteorShower, JudgementSpray).
    Projectile {
        owner: Option<u16>,
        spell: Spell,
        position: [f32; 3],
        velocity: [f32; 3],
        splash: f32,
    },
    /// LightningBolt.Cast (ChainLightning's Zap) along a direction, or the
    /// VladZap bolt at `target`; from the cast source when `from` is None.
    Lightning {
        owner: u16,
        target: Option<u16>,
        from: Option<[f32; 3]>,
        direction: Option<[f32; 3]>,
        damage: f32,
        range: f32,
        hit_owner: bool,
    },
    /// Thunderbolt.Execute's aimed bolt (8 m ahead of `position`).
    Thunderbolt {
        owner: Option<u16>,
        position: [f32; 3],
        direction: [f32; 3],
    },
    /// One Thunderstorm strike at a point.
    Strike {
        owner: Option<u16>,
        position: [f32; 3],
    },
    /// TractorPull (`pull`) or TractorPush around a centre.
    Push {
        owner: Option<u16>,
        center: [f32; 3],
        pull: bool,
    },
    GreaseField {
        owner: Option<u16>,
        position: [f32; 3],
    },
    SetInvisible {
        target: u16,
        seconds: f32,
    },
    SetEthereal {
        target: u16,
        ethereal: bool,
    },
    Levitate {
        target: u16,
    },
    Haste {
        target: u16,
        seconds: Option<f32>,
    },
    Resize {
        target: u16,
        grow: bool,
    },
    Charm {
        target: u16,
        owner: u16,
        seconds: f32,
    },
    Fear {
        target: u16,
        source: magicka_sim::fear::FearSource,
    },
    /// `from`: the caster's position (Ok) or the caster (Err).
    Confuse {
        target: u16,
        from: Result<[f32; 3], u16>,
    },
    Polymorph {
        target: u16,
    },
    Terminate {
        target: u16,
    },
    Revive {
        from: Option<[f32; 3]>,
        position: [f32; 3],
        health: f32,
        slot: Option<usize>,
    },
    /// Teleport.DoTeleport: effects at both ends, needs ground there.
    Blink {
        target: u16,
        position: [f32; 3],
    },
    /// A summoned NPC placed like SummonUndead's (navmesh, then ground).
    Summon {
        owner: Option<u16>,
        unit: String,
        position: [f32; 3],
        undead: bool,
    },
    SpawnNative {
        kind: NativeKind,
        owner: u16,
    },
    SetLight {
        intensity: f32,
    },
    Flash {
        seconds: f32,
    },
    Shake {
        magnitude: f32,
        seconds: f32,
    },
    Cue {
        name: String,
        bank: u16,
        position: Option<[f32; 3]>,
    },
    Effect {
        name: String,
        position: [f32; 3],
        direction: [f32; 3],
    },
    Weather {
        kind: Weather,
        owner: Option<u16>,
    },
    TimeWarp {
        owner: Option<u16>,
    },
    Nullify,
    Corporealize {
        position: [f32; 3],
    },
}

impl<C: SceneContent> SceneWorld<C> {
    /// Where a Magick cast by `handle` aims: the caster's position, its
    /// mouse pick (players) and the native target (an NPC's combat target,
    /// else the character nearest the pick).
    pub fn magick_aim(&self, handle: u16) -> Option<([f32; 3], Option<[f32; 3]>, u16)> {
        let body = self.character_body(handle)?;
        let position = self.physics.bodies[body].position;
        let pick = self
            .players
            .iter()
            .flatten()
            .find(|p| p.handle == handle)
            .and_then(|p| p.mouse_pick);
        let target = self
            .npcs
            .get(&handle)
            .and_then(|n| n.combat.current_target())
            .unwrap_or_else(|| self.mouse_target(handle, pick));
        Some((position, pick, target))
    }

    /// Execute one operation (host or offline only).
    pub fn script_op(&mut self, op: WorldOp) -> Result<(), String> {
        if self.role == NetworkRole::Client {
            return Err("world operations run on the host".into());
        }
        let ok = |done: bool, what: &str| {
            if done {
                Ok(())
            } else {
                Err(format!("{what} did not apply"))
            }
        };
        match op {
            WorldOp::Hit {
                target,
                damage,
                source,
                from,
            } => {
                let from = from
                    .or_else(|| {
                        source
                            .and_then(|s| self.character_body(s))
                            .map(|b| self.physics.bodies[b].position)
                    })
                    .or_else(|| {
                        self.character_body(target)
                            .map(|b| self.physics.bodies[b].position)
                    })
                    .ok_or("unknown target")?;
                self.damage_any(target, damage, source, from);
            }
            WorldOp::Status {
                target,
                status,
                dps,
                magnitude,
            } => {
                let effect = magicka_sim::status::StatusEffect::new(status, dps, magnitude, true);
                let mut events = Vec::new();
                if let Some(p) = self
                    .players
                    .iter_mut()
                    .flatten()
                    .find(|p| p.handle == target)
                {
                    let _ = p.add_status_effect(effect, &mut events);
                } else if let Some(n) = self.npcs.get_mut(&target) {
                    let _ = n.character.add_status_effect(effect, None, &mut events);
                } else {
                    return Err("unknown character".into());
                }
            }
            WorldOp::Projectile {
                owner,
                spell,
                position,
                velocity,
                splash,
            } => self.spawn_missiles(vec![(
                owner.unwrap_or(0),
                magicka_sim::projectile_launch::ProjectileLaunch {
                    position,
                    velocity,
                    spell,
                    splash,
                    total_projectiles: 1,
                },
            )]),
            WorldOp::Lightning {
                owner,
                target: Some(target),
                from,
                damage,
                ..
            } => return ok(self.targeted_bolt(owner, target, from, damage), "lightning"),
            WorldOp::Lightning {
                owner,
                target: None,
                from,
                direction,
                damage,
                range,
                hit_owner,
            } => {
                let body = self.character_body(owner).ok_or("unknown caster")?;
                let (position, facing) = (
                    self.physics.bodies[body].position,
                    self.physics.bodies[body].direction(),
                );
                let orb = self
                    .players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == owner)
                    .map(|p| [p.staff_orb[12], p.staff_orb[13], p.staff_orb[14]])
                    .filter(|o| *o != [0.0; 3]);
                let source = from.or(orb).unwrap_or(position);
                self.special_bolt(
                    owner,
                    source,
                    direction.unwrap_or(facing),
                    !hit_owner,
                    damage,
                    range,
                );
            }
            WorldOp::Thunderbolt {
                owner,
                position,
                direction,
            } => {
                self.thunderbolt(owner, position, direction);
            }
            WorldOp::Strike { owner, position } => {
                let timestamp = self.play_time;
                self.strike_at(owner, position, timestamp);
            }
            WorldOp::Push {
                owner,
                center,
                pull,
            } => self.tractor(owner, center, !pull),
            WorldOp::GreaseField { owner, position } => {
                self.spawn_grease_field(owner, position, [0.0, 1.0, 0.0], None);
            }
            WorldOp::SetInvisible { target, seconds } => {
                let c = Self::character_mut(&mut self.players, &mut self.npcs, target)
                    .ok_or("unknown character")?;
                c.set_invisible(seconds);
            }
            WorldOp::SetEthereal { target, ethereal } => {
                let c = Self::character_mut(&mut self.players, &mut self.npcs, target)
                    .ok_or("unknown character")?;
                c.set_ethereal(ethereal);
            }
            WorldOp::Levitate { target } => return ok(self.levitate(target), "levitate"),
            WorldOp::Haste { target, seconds } => {
                self.character_body(target).ok_or("unknown character")?;
                self.ability_effects.haste(target, seconds);
            }
            WorldOp::Resize { target, grow } => {
                use magicka_sim::size_magicks::SizeKind;
                let kind = if grow {
                    SizeKind::Grow
                } else {
                    SizeKind::Shrink
                };
                return ok(self.size_magick(target, kind), "resize");
            }
            WorldOp::Charm {
                target,
                owner,
                seconds,
            } => return ok(self.charm_character(target, owner, seconds), "charm"),
            WorldOp::Fear { target, source } => {
                return ok(self.fear_character(target, source), "fear")
            }
            WorldOp::Confuse { target, from } => {
                let from = match from {
                    Ok(p) => p,
                    Err(h) => self
                        .character_body(h)
                        .map(|b| self.physics.bodies[b].position)
                        .ok_or("unknown caster")?,
                };
                return ok(self.confuse_character(from, target), "confuse");
            }
            WorldOp::Polymorph { target } => return ok(self.polymorph_target(target), "polymorph"),
            WorldOp::Terminate { target } => self.terminate_character(target),
            WorldOp::Revive {
                from,
                position,
                health,
                slot,
            } => {
                return ok(
                    self.execute_revive(from, position, health, slot, None),
                    "revive",
                )
            }
            WorldOp::Blink { target, position } => {
                let body = self.character_body(target).ok_or("unknown character")?;
                let direction = self.physics.bodies[body].direction();
                return ok(
                    self.teleport_character(target, position, direction),
                    "teleport (no ground below the destination)",
                );
            }
            WorldOp::Summon {
                owner,
                unit,
                position,
                undead,
            } => {
                use crate::npc::{ORDER_ATTACK, REACT_ATTACK, REACT_PROXIMITY};
                let position = self.summon_point(position);
                let direction = owner
                    .and_then(|o| self.character_body(o))
                    .map_or([0.0, 0.0, 1.0], |b| self.physics.bodies[b].direction());
                let kind = if undead {
                    magicka_sim::summons::SummonKind::Undead
                } else {
                    magicka_sim::summons::SummonKind::Normal
                };
                let events = Vec::new();
                let summoned = self.summon_npc(
                    owner,
                    &unit,
                    position,
                    direction,
                    kind,
                    crate::spawn_action::SpawnOrder {
                        order: ORDER_ATTACK,
                        react_to: REACT_ATTACK | REACT_PROXIMITY,
                        reaction: ORDER_ATTACK,
                        priority_target: 0,
                        priority_ability: 0,
                        trigger: 0,
                        events: &events,
                    },
                );
                return ok(summoned.is_some(), &format!("summon {unit}"));
            }
            WorldOp::SpawnNative { kind, owner } => return self.spawn_native(kind, owner),
            WorldOp::SetLight { intensity } => self.light_target_intensity = intensity,
            WorldOp::Flash { seconds } => self.flash(seconds),
            WorldOp::Shake { magnitude, seconds } => self.camera.shake(magnitude, seconds),
            WorldOp::Cue {
                name,
                bank,
                position,
            } => {
                self.play_cue(bank, &name, position);
            }
            WorldOp::Effect {
                name,
                position,
                direction,
            } => {
                self.start_named_effect(&name, position, direction);
            }
            WorldOp::Weather { kind, owner } => {
                if self.scene_indoors {
                    return Err("weather is refused indoors".into());
                }
                match kind {
                    Weather::Rain => {
                        self.rain_from(owner);
                    }
                    Weather::Blizzard => {
                        if self.blizzard_cue.is_none() {
                            self.blizzard_cue = Some(self.play_cue(16, "magick_blizzard", None));
                        }
                        self.ability_effects.blizzard(owner, self.play_time);
                    }
                }
            }
            WorldOp::TimeWarp { owner } => {
                if !self.ability_effects.time_warp(owner, self.scene_saturation) {
                    return Err("a time warp is already active".into());
                }
                self.time_warp_cue = Some(self.play_cue(16, "magick_timewarp", None));
            }
            WorldOp::Nullify => self.nullify(),
            WorldOp::Corporealize { position } => self.corporealize(position),
        }
        Ok(())
    }

    fn spawn_native(&mut self, kind: NativeKind, owner: u16) -> Result<(), String> {
        let body = self.character_body(owner).ok_or("unknown owner")?;
        let (position, direction) = (
            self.physics.bodies[body].position,
            self.physics.bodies[body].direction(),
        );
        let done = match kind {
            NativeKind::Tornado => self.tornado_from(owner),
            NativeKind::Vortex => self.vortex_from(owner),
            NativeKind::Wave => self.wave_from(owner),
            NativeKind::Portal => self.portal_from(owner),
            NativeKind::Phoenix => self.phoenix_from(owner),
            NativeKind::Death => self.summon_death_from(owner),
            NativeKind::Elemental => self
                .summon_elemental_from(owner)
                .map_err(|e| e.to_string())?
                .is_some(),
            NativeKind::Earthquake => self.earthquake_from(owner),
            NativeKind::Cross => self.summon_cross(owner).is_some(),
            NativeKind::Slime | NativeKind::SlimeOverkill => {
                let cast = slime_live::SlimeCast {
                    overkill: kind == NativeKind::SlimeOverkill,
                    ..Default::default()
                };
                !self
                    .spawn_slimes(owner, cast)
                    .map_err(|e| e.to_string())?
                    .is_empty()
            }
            NativeKind::Clone => self
                .ethereal_clone_player(owner)
                .map_err(|e| e.to_string())?
                .is_some(),
            NativeKind::Napalm => self.napalm(owner, position, direction),
            NativeKind::Conflagration => self.conflagration_from(owner),
            NativeKind::Propp => self.propp_magick(owner),
            NativeKind::Amalgameddon => self.amalgameddon(owner),
            NativeKind::Judgement => {
                self.ability_effects.judgement_spray(owner);
                true
            }
            NativeKind::GreaseSpray => {
                self.ability_effects.grease(owner);
                true
            }
        };
        if done {
            Ok(())
        } else {
            Err(format!("{kind:?} did not start"))
        }
    }
}
