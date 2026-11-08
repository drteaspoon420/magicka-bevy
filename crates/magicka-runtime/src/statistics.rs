//! `Magicka.GameLogic.Statistics.StatisticsManager` (research/rulesets.md):
//! per-player score values and the Survival totals fed by character damage
//! and kill events. Pure: hosts resolve entities to `Combatant` facts
//! (charm owner / summon master already substituted) and apply the
//! returned Survival AddScore requests.
use magicka_sim::character_impact::result::{DAMAGED, DEFLECTED, HEALED, KILLED, OVERKILLED};

/// StatisticsManager.ScoreValues.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScoreValues {
    pub kills: u32,
    pub deaths: u32,
    pub over_killed: u32,
    pub over_kills: u32,
    pub healing_done: u64,
    pub healing_received: u64,
    pub damage_done: u64,
    pub damage_received: u64,
    pub suicides: u32,
    pub team_kills: u32,
    pub team_killed: u32,
}

/// The score facts of a non-player character.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NpcScore {
    /// CharacterTemplate.DisplayName (lowercase display id hash).
    pub display_name: i32,
    pub score_value: i32,
    pub undying: bool,
}

/// One side of a damage or kill event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Combatant {
    /// The local player slot of an avatar.
    pub player: Option<usize>,
    /// Factions flags.
    pub faction: i32,
    /// A non-player character (None: avatar or non-character).
    pub npc: Option<NpcScore>,
    /// Character (avatar or NPC) rather than another damageable.
    pub character: bool,
}

/// SurvivalRuleset.AddScore(displayName, score).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScoreRequest {
    pub name: i32,
    pub score: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Statistics {
    pub scores: crate::slots::PerSlot<ScoreValues>,
    /// SurvivalTotalScore.
    pub total_score: i32,
    /// SurvivalTotalDamage.
    pub total_player_damage: f32,
}

/// C# (ulong) of a float: truncation, negative and NaN to 0 here (native
/// out-of-range conversion is unspecified; UNKNOWN).
fn ulong(v: f32) -> u64 {
    if v.is_finite() && v > 0.0 {
        v as u64
    } else {
        0
    }
}

impl Statistics {
    /// SurvivalReset.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// InternalDamageEvent. `multiplier` is the Survival ScoreMultiplier
    /// (1 otherwise); `survival` selects the AddScore request.
    #[allow(clippy::too_many_arguments)]
    pub fn damage_event(
        &mut self,
        attacker: Option<Combatant>,
        target: Combatant,
        amount: f32,
        magnitude: f32,
        result: u16,
        multiplier: i32,
        survival: bool,
    ) -> Option<ScoreRequest> {
        if result == 0 || result & DEFLECTED == DEFLECTED {
            return None;
        }
        let mut request = None;
        if let Some((a, slot)) = attacker.and_then(|a| a.player.map(|s| (a, s))) {
            if result & HEALED == HEALED {
                self.scores[slot].healing_done += ulong(-amount);
            }
            if target.character {
                if result & DAMAGED == DAMAGED {
                    self.total_player_damage += amount * magnitude;
                    self.scores[slot].damage_done += ulong(amount * magnitude);
                }
                if result & KILLED != 0 {
                    let overkilled = result & OVERKILLED == OVERKILLED;
                    if let Some(npc) = target.npc {
                        if npc.undying && !overkilled && npc.score_value == 0 {
                            return None;
                        }
                        if survival {
                            let score = if a.faction & target.faction != 0 {
                                -npc.score_value
                            } else {
                                npc.score_value
                            };
                            request = Some(ScoreRequest {
                                name: npc.display_name,
                                score: score.wrapping_mul(multiplier),
                            });
                        }
                        self.total_score = self
                            .total_score
                            .wrapping_add(npc.score_value.wrapping_mul(multiplier));
                        self.total_player_damage += amount * magnitude;
                    }
                }
            }
        }
        if let Some(slot) = target.player {
            let dealt = amount * magnitude;
            if dealt < 0.0 {
                self.scores[slot].healing_received += ulong(-amount);
            }
            if result & DAMAGED == DAMAGED && dealt > 0.0 {
                self.scores[slot].damage_received += ulong(dealt);
            }
        }
        request
    }

    /// AddKillEvent with a Versus ruleset: deaths count, and a kill between
    /// avatars of disjoint factions credits the attacker. Returns the
    /// (attacker, target) slots for VersusRuleset.OnPlayerKill.
    pub fn versus_kill_event(
        &mut self,
        target: Combatant,
        attacker: Option<Combatant>,
    ) -> Option<(usize, usize)> {
        let t = target.player?;
        self.scores[t].deaths += 1;
        let a = attacker?;
        let slot = a.player?;
        if a.faction & target.faction != 0 {
            return None;
        }
        self.scores[slot].kills += 1;
        Some((slot, t))
    }

    /// AddKillEvent (Survival and campaign; the Versus branch is
    /// `versus_kill_event`). `survival_multiplier` is Some(ScoreMultiplier) when the
    /// scene's ruleset is Survival.
    pub fn kill_event(
        &mut self,
        target: Combatant,
        attacker: Option<Combatant>,
        survival_multiplier: Option<i32>,
    ) -> Option<ScoreRequest> {
        if let Some(slot) = target.player {
            self.scores[slot].deaths += 1;
        }
        let (multiplier, attacker, npc) = (
            survival_multiplier?,
            attacker.filter(|a| a.player.is_some())?,
            target.npc?,
        );
        self.scores[attacker.player.unwrap()].kills += 1;
        let score = if attacker.faction & target.faction != 0 {
            -npc.score_value
        } else {
            npc.score_value
        };
        Some(ScoreRequest {
            name: npc.display_name,
            score: score.wrapping_mul(multiplier),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYER: Combatant = Combatant {
        player: Some(0),
        faction: 2,
        npc: None,
        character: true,
    };
    fn goblin(score: i32) -> Combatant {
        Combatant {
            player: None,
            faction: 1,
            npc: Some(NpcScore {
                display_name: 77,
                score_value: score,
                undying: false,
            }),
            character: true,
        }
    }

    #[test]
    fn killing_damage_scores_and_counts_player_damage() {
        let mut s = Statistics::default();
        let r = s.damage_event(
            Some(PLAYER),
            goblin(40),
            100.0,
            1.5,
            DAMAGED | KILLED,
            6,
            true,
        );
        assert_eq!(
            r,
            Some(ScoreRequest {
                name: 77,
                score: 240
            })
        );
        assert_eq!(s.total_score, 240);
        assert_eq!(s.total_player_damage, 300.0, "damage and kill both add");
        assert_eq!(s.scores[0].damage_done, 150);
        assert_eq!(
            s.damage_event(Some(PLAYER), goblin(40), 0.0, 1.0, DEFLECTED, 6, true),
            None
        );
    }

    #[test]
    fn friendly_kills_subtract_and_deaths_count() {
        let mut s = Statistics::default();
        let ally = Combatant {
            faction: 2,
            ..goblin(30)
        };
        let r = s.kill_event(ally, Some(PLAYER), Some(3));
        assert_eq!(
            r,
            Some(ScoreRequest {
                name: 77,
                score: -90
            })
        );
        assert_eq!(s.scores[0].kills, 1);
        s.kill_event(PLAYER, Some(goblin(1)), Some(3));
        assert_eq!(s.scores[0].deaths, 1);
        assert_eq!(
            s.kill_event(goblin(5), Some(PLAYER), None),
            None,
            "no Survival"
        );
    }
}
