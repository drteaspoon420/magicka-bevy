//! Staff special abilities (research/staff-items.md): the SpecialAbility
//! subclasses reachable only from item data. Magick classes shared with the
//! magick list run through `cast_magick_from`.
use super::*;
use crate::generic_aura::{AuraKind, AuraTarget, BuffKind, BuffSpec, GenericAura};
use magicka_sim::damage::{attack, Damage};
use magicka_sim::spells::{Elements as E, Spell};

/// Spell.FIRECOLOR / ARCANECOLOR.
const FIRE_COLOR: [f32; 3] = [1.8, 0.6, 0.4];
const ARCANE_COLOR: [f32; 3] = [2.0, 0.4, 0.6];
/// VisualCategory.
const OFFENSIVE: u8 = 1;
const DEFENSIVE: u8 = 2;
const SPECIAL: u8 = 3;

pub(super) fn spell_of(parts: &[(E, f32)]) -> Spell {
    let mut spell = Spell::default();
    for &(element, magnitude) in parts {
        spell.element = E(spell.element.0 | element.0);
        spell.mag[element.index() as usize] = magnitude;
    }
    spell
}

impl<C: SceneContent> SceneWorld<C> {
    /// Character.CastSource translation (the staff orb), or the body.
    pub(super) fn cast_source_of(&self, handle: u16) -> Option<[f32; 3]> {
        if let Some(p) = self.players.iter().flatten().find(|p| p.handle == handle) {
            let orb = [p.staff_orb[12], p.staff_orb[13], p.staff_orb[14]];
            let body = &self.physics.bodies[p.body];
            return Some(if orb == [0.0; 3] { body.position } else { orb });
        }
        let n = self.npcs.get(&handle)?;
        Some(n.spray_source(&self.physics.bodies[n.body]).cast_position)
    }

    /// The magick classes a staff can name (SpecialAbility.Read resolves
    /// any SpecialAbility subclass by name).
    pub(super) fn staff_magick(name: &str) -> Option<magicka_sim::spells::MagickType> {
        use magicka_sim::spells::MagickType as M;
        Some(match name {
            "revive" => M::Revive,
            "grease" => M::Grease,
            "tornado" => M::Tornado,
            "charm" => M::Charm,
            "confuse" => M::Confuse,
            "tractorpull" => M::TractorPull,
            "summonundead" => M::SUndead,
            "performanceenchantment" => M::PerformanceEnchantment,
            "zap" => M::ChainLightning,
            _ => return None,
        })
    }

    /// Returns false when `name` is not a staff ability handled here.
    pub(super) fn execute_staff_special(&mut self, owner: u16, name: &str) -> bool {
        let Some(body) = self.character_body(owner) else {
            return true;
        };
        let position = self.physics.bodies[body].position;
        let direction = self.physics.bodies[body].direction();
        let authority = self.role != NetworkRole::Client;
        match name {
            // Execute only stamps mTimeStamp.
            "damageimmunity" => {}
            "coffeeboost" => {
                // Two self heals through the public route from the cast
                // source, then Haste for 4 s without its cue.
                let source = self.cast_source_of(owner).unwrap_or(position);
                for damage in [
                    Damage {
                        attack: attack::DAMAGE,
                        element: E::LIFE.0,
                        amount: -100.0,
                        magnitude: 1.0,
                    },
                    Damage {
                        attack: attack::STATUS,
                        element: E::LIFE.0,
                        amount: -100.0,
                        magnitude: 2.0,
                    },
                ] {
                    self.hit_character(owner, damage, Some(owner), source);
                }
                self.ability_effects.haste(owner, Some(4.0));
            }
            "staffshield" => {
                if authority {
                    let spell = spell_of(&[(E::SHIELD, 1.0)]);
                    if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, owner) {
                        c.add_self_shield(&spell);
                    }
                }
            }
            "nontimesensitiveshield" => {
                if authority {
                    let spell = spell_of(&[(E::SHIELD, 1.0)]);
                    if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, owner) {
                        c.add_generic_self_shield(&spell, false);
                    }
                }
            }
            "radioactivedamage" => {
                self.start_named_effect("radioactive_damage", position, direction);
                let damage = Damage {
                    attack: attack::STATUS,
                    element: E::POISON.0,
                    amount: 20.0,
                    magnitude: 1.0,
                };
                self.arc_damage(
                    Some(owner),
                    Some(owner),
                    position,
                    direction,
                    7.5,
                    std::f32::consts::PI,
                    &[damage],
                    self.play_time,
                );
                self.play_cue(16, "magick_raise_dead", Some(position));
            }
            "zoom" => self.camera.set_player_magnification(2.0, 4.0),
            "fireimmunityaura" => {
                let buff = BuffSpec::new(
                    BuffKind::Resistance(magicka_sim::resistance::Resistance {
                        against: E::FIRE,
                        multiplier: 0.0,
                        modifier: 0.0,
                        status_resistance: false,
                    }),
                    DEFENSIVE,
                    FIRE_COLOR,
                );
                let aura = GenericAura::new(
                    AuraTarget::Friendly,
                    AuraKind::Buff(buff),
                    10.0,
                    10.0,
                    DEFENSIVE,
                )
                .with_color(FIRE_COLOR);
                self.add_generic_aura(owner, aura);
            }
            "lightningdamageaura" => {
                let buff = BuffSpec::new(
                    BuffKind::DealDamage(Damage {
                        attack: attack::DAMAGE,
                        element: E::LIGHTNING.0,
                        amount: 100.0,
                        magnitude: 1.0,
                    }),
                    0,
                    LIGHTNING_COLOR,
                );
                let aura = GenericAura::new(
                    AuraTarget::AllButSelf,
                    AuraKind::Buff(buff),
                    5.0,
                    10.0,
                    OFFENSIVE,
                )
                .with_color(LIGHTNING_COLOR)
                .with_effect("lightning_damage_aura");
                self.add_generic_aura(owner, aura);
            }
            "loveaura" => {
                // AuraLove(9 s charm, 3 m), aura TTL 5 and radius 3.
                let aura = GenericAura::new(
                    AuraTarget::Enemy,
                    AuraKind::Love {
                        radius: 3.0,
                        ttl: 9.0,
                    },
                    5.0,
                    3.0,
                    SPECIAL,
                )
                .with_color(FIRE_COLOR);
                self.add_generic_aura(owner, aura);
            }
            "deflectionaura" => {
                let aura = GenericAura::new(
                    AuraTarget::Own,
                    AuraKind::Deflect { strength: 5.0 },
                    8.0,
                    5.0,
                    SPECIAL,
                )
                .with_color(ARCANE_COLOR)
                .with_effect("deflection_aura");
                self.add_generic_aura(owner, aura);
            }
            "firebolt" | "arcanebolt" => {
                // ProjectileSpell.SpawnMissile ignores its homing argument.
                let local = authority || self.players.iter().flatten().any(|p| p.handle == owner);
                if !local {
                    return true;
                }
                let (spell, splash) = if name == "firebolt" {
                    (spell_of(&[(E::EARTH, 2.0), (E::FIRE, 5.0)]), 5.0)
                } else {
                    (spell_of(&[(E::EARTH, 2.0), (E::ARCANE, 5.0)]), 4.0)
                };
                let mut source = self.cast_source_of(owner).unwrap_or(position);
                let rogue = self.players.iter().flatten().any(|p| {
                    p.handle == owner
                        && p.character
                            .definition
                            .id
                            .eq_ignore_ascii_case("wizard_rogue")
                });
                if rogue {
                    source[1] += 1.0;
                }
                self.spawn_missiles(vec![(
                    owner,
                    magicka_sim::projectile_launch::ProjectileLaunch {
                        position: source,
                        velocity: direction.map(|v| v * 50.0),
                        spell,
                        splash,
                        total_projectiles: 1,
                    },
                )]);
            }
            "northpole" => {
                // Teleport 10 m ahead, then (next avatar update) three Cold
                // and an Area cast from the staff.
                let Some(p) = self.players.iter().flatten().find(|p| p.handle == owner) else {
                    return true;
                };
                let _ = p;
                let destination = std::array::from_fn(|i| position[i] + direction[i] * 10.0);
                self.teleport_character(owner, destination, direction);
                if let Some(p) = self
                    .players
                    .iter_mut()
                    .flatten()
                    .find(|p| p.handle == owner)
                {
                    p.pending_staff_cast = Some(crate::avatar::StaffCast::NorthPole);
                }
            }
            _ => return false,
        }
        true
    }
}
