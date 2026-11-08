//! StatisticsManager host: resolves characters to `Combatant` facts and
//! forwards Survival AddScore requests (research/rulesets.md).
use super::*;
use crate::statistics::{Combatant, NpcScore, ScoreRequest};

impl<C: SceneContent> SceneWorld<C> {
    /// The statistics facts of an entity; NPC attackers resolve to their
    /// charm owner, else their summoning master (InternalDamageEvent and
    /// AddKillEvent).
    fn combatant(&self, handle: u16, resolve_owner: bool) -> Option<Combatant> {
        if resolve_owner {
            if let Some(n) = self.npcs.get(&handle) {
                if n.character.is_charmed() {
                    return self.combatant(n.character.charm?.owner, false);
                }
                if let Some((master, _)) = n.summon {
                    return self.combatant(master, false);
                }
            }
        }
        let entity = self.entities.iter().find(|e| e.handle == handle)?;
        let player = self
            .players
            .iter()
            .flatten()
            .find(|p| p.handle == handle)
            .map(|p| p.slot);
        let npc = self.npcs.get(&handle).map(|n| NpcScore {
            display_name: custom_hash(&n.definition.display_id.to_lowercase()),
            score_value: n.definition.score_value,
            undying: n.definition.undying,
        });
        Some(Combatant {
            player,
            faction: self.faction(entity),
            character: player.is_some() || npc.is_some(),
            npc,
        })
    }

    fn survival_score_multiplier(&self) -> Option<i32> {
        self.survival().map(|r| r.score_multiplier())
    }

    fn add_survival_score(&mut self, request: Option<ScoreRequest>) {
        if let (Some(r), Some(q)) = (self.survival_mut(), request) {
            r.add_score(q.name, q.score);
        }
    }

    /// Drain every character's Character.Damage statistics requests
    /// (AddDamageEvent; host only).
    pub(super) fn apply_damage_statistics(&mut self) {
        let mut queued = Vec::new();
        for (handle, c) in self
            .players
            .iter_mut()
            .flatten()
            .map(|p| (p.handle, &mut p.character))
            .chain(self.npcs.iter_mut().map(|(&h, n)| (h, &mut n.character)))
        {
            queued.extend(c.statistics_events.drain(..).map(|e| (handle, e)));
        }
        if self.role == NetworkRole::Client {
            return;
        }
        let multiplier = self.survival_score_multiplier();
        for (target, e) in queued {
            let Some(target) = self.combatant(target, false) else {
                continue;
            };
            let attacker = e.attacker.and_then(|a| self.combatant(a, true));
            let request = self.statistics.damage_event(
                attacker,
                target,
                e.amount,
                e.magnitude,
                e.result,
                multiplier.unwrap_or(1),
                multiplier.is_some(),
            );
            self.add_survival_score(request);
        }
    }

    /// DeadState: AddKillEvent(owner, LastAttacker) before Die (host only).
    pub(super) fn note_kill(&mut self, target: u16) {
        if self.role == NetworkRole::Client {
            return;
        }
        let last_attacker = self
            .npcs
            .get(&target)
            .map(|n| &n.character.combat)
            .or_else(|| {
                self.players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == target)
                    .map(|p| &p.character.combat)
            })
            .and_then(|c| c.last_attacker)
            .and_then(|h| u16::try_from(h).ok());
        let Some(target) = self.combatant(target, false) else {
            return;
        };
        let attacker = last_attacker.and_then(|a| self.combatant(a, true));
        if self.ruleset.as_ref().is_some_and(|r| r.versus().is_some()) {
            if let Some((a, t)) = self.statistics.versus_kill_event(target, attacker) {
                self.versus_player_kill(a, t);
            }
            return;
        }
        let multiplier = self.survival_score_multiplier();
        let request = self.statistics.kill_event(target, attacker, multiplier);
        self.add_survival_score(request);
    }
}

#[cfg(test)]
mod tests {
    use crate::play_session::PlaySession;
    use magicka_sim::damage_delivery::DamagePayload;

    #[test]
    fn avatar_kill_in_survival_scores_and_scrolls() {
        let root = magicka_content::retail_content_dir();
        if !root.is_dir() {
            return;
        }
        let mut s = PlaySession::load(&root, "Challenges/ch_glade.lvl", "Wizard").unwrap();
        s.step(1.0 / 60.0);
        let player = s.world.players[0].as_ref().unwrap().handle;
        let (&target, npc) = s.world.npcs.iter_mut().next().expect("first wave");
        let score_value = npc.definition.score_value;
        assert!(score_value > 0);
        let body = &mut s.world.physics.bodies[npc.body];
        let hp = npc.character.combat.health.hit_points;
        crate::avatar::deliver_to_body(
            &mut npc.character,
            body,
            &[magicka_sim::resistance::Resistance::IDENTITY; 11],
            false,
            target,
            DamagePayload::Single(magicka_sim::damage::Damage {
                attack: 1,
                element: 0,
                amount: hp + 10.0,
                magnitude: 1.0,
            }),
            Some(player),
            [0.0; 3],
            0.0,
        );
        let multiplier = s.world.survival().unwrap().score_multiplier();
        for _ in 0..600 {
            s.step(1.0 / 60.0);
        }
        assert_eq!(s.world.statistics.scores[0].kills, 1);
        assert_eq!(s.world.statistics.total_score, score_value * multiplier);
        let r = s.world.survival().unwrap();
        assert_eq!(
            r.total_score,
            2 * score_value * multiplier,
            "native queues AddScore from both the killing damage and the kill"
        );
        assert!(r.shown_total_score > 0);
    }
}
