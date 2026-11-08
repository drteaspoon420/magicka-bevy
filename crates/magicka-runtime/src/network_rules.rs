//! Ruleset, statistics and end-of-game state across the modern protocol
//! (research/rulesets.md). Native servers send RulesetMessage (Survival
//! multipliers, wave and health; TimedObjective counters and timer; Versus
//! clock, scores and setup) and GameEndMessage; hosts here put the same
//! presentation state in each GameFrame and clients mirror it. Client
//! sessions run no scripts, so mirrored state is never advanced locally.
use crate::play_session::PlaySession;
use crate::ruleset::{SceneRuleset, VersusMode};
use magicka_network::session::{RulesState, RulesetState, ScrollScoreState};

/// The host's state for the next GameFrame.
pub fn rules_state(session: &PlaySession) -> RulesState {
    // Per-slot arrays cover the original four slots or every used slot.
    let slots = session
        .world
        .players
        .len()
        .max(crate::slots::ORIGINAL_PLAYER_LIMIT);
    let world = &session.world;
    let ruleset = world.ruleset.as_ref().map(|r| match r {
        SceneRuleset::Survival(s) => {
            let bar = world.survival_health_bar.unwrap_or_default();
            RulesetState::Survival {
                wave: s.wave_number as u32,
                shown_total_score: s.shown_total_score,
                time_multiplier: s.time_multiplier,
                damage_multiplier: s.damage_multiplier,
                total_multiplier: s.total_multiplier,
                damage_amount: s.damage_amount(),
                normalized_time: s.normalized_time(),
                health: bar.display_health,
                health_alpha: bar.alpha,
                scroll: s
                    .scroll_scores
                    .iter()
                    .filter(|x| !x.dead())
                    .map(|x| ScrollScoreState {
                        name: x.name,
                        score: x.score,
                        multi_kills: x.multi_kills,
                        position: x.position,
                        alpha: x.alpha,
                    })
                    .collect(),
            }
        }
        SceneRuleset::TimedObjective(t) => RulesetState::TimedObjective {
            counters: t
                .objectives
                .iter()
                .map(|o| o.counter)
                .chain(std::iter::once(t.time_bonus.counter))
                .map(|id| (id, session.runtime.variables.counter(id)))
                .collect(),
            timer: session.runtime.variables.timer(t.timer),
            time_text: t.time_text.clone(),
            hud: world.timed_objective_hud,
        },
        SceneRuleset::Versus(v) => RulesetState::Versus {
            mode: match v.settings.mode {
                VersusMode::DeathMatch => 3,
                VersusMode::Brawl => 4,
                VersusMode::Pyrite => 5,
                VersusMode::Krietor => 6,
                VersusMode::King => 7,
            },
            teams_enabled: v.settings.teams,
            waiting: world.versus_waiting,
            countdown: v.countdown_timer,
            time_limit: v.time_limit,
            time: v.time_limit_timer,
            scores: v.scores.to_vec(slots),
            team_scores: v.team_scores,
            respawns: v.respawns.to_vec(slots),
            team_respawns: v.team_respawns,
            respawn_timers: v.respawn_timers.to_vec(slots),
            sudden_death: v.sudden_death,
            unlocks: v.unlocks.clone(),
            teams: world.player_teams.to_vec(slots),
        },
    });
    RulesState {
        ruleset,
        kills: (0..slots)
            .map(|i| world.statistics.scores[i].kills)
            .collect(),
        deaths: (0..slots)
            .map(|i| world.statistics.scores[i].deaths)
            .collect(),
        total_score: world.statistics.total_score,
        end_condition: session.end_game.condition as u8,
        end_text_alpha: session.end_game.text_alpha,
        statistics: session.end_game.statistics,
    }
}

/// Apply the host's state on a client. Clients see their own avatar in
/// local slot 0 and the host's slot 0 in their own slot (RemotePlayers), so
/// per-slot arrays swap those two entries.
pub fn mirror_rules(session: &mut PlaySession, rules: &RulesState, own_slot: usize) {
    let local = |server: usize| match server {
        0 => own_slot,
        s if s == own_slot => 0,
        s => s,
    };
    fn remap<T: Copy + Default>(
        values: &[T],
        local: &dyn Fn(usize) -> usize,
    ) -> crate::slots::PerSlot<T> {
        let mut out = crate::slots::PerSlot::default();
        for (server, v) in values.iter().enumerate() {
            out[local(server)] = *v;
        }
        out
    }
    session.network_mode = crate::play_session::NetworkMode::Client;
    let world = &mut session.world;
    let kills = remap(&rules.kills, &local);
    let deaths = remap(&rules.deaths, &local);
    for slot in 0..kills.len().max(deaths.len()) {
        world.statistics.scores[slot].kills = kills[slot];
        world.statistics.scores[slot].deaths = deaths[slot];
    }
    world.statistics.total_score = rules.total_score;
    session.end_game.condition = crate::end_game::EndGameCondition::from_value(rules.end_condition);
    session.end_game.text_alpha = rules.end_text_alpha;
    session.end_game.statistics = rules.statistics;
    world.game_ended = rules.end_condition != 0;
    match (&rules.ruleset, world.ruleset.as_mut()) {
        (
            Some(RulesetState::Survival {
                wave,
                shown_total_score,
                time_multiplier,
                damage_multiplier,
                total_multiplier,
                damage_amount,
                normalized_time,
                health,
                health_alpha,
                scroll,
            }),
            Some(SceneRuleset::Survival(s)),
        ) => {
            s.apply_remote(
                *wave as usize,
                *shown_total_score,
                [*time_multiplier, *damage_multiplier, *total_multiplier],
                *damage_amount,
                *normalized_time,
            );
            for (i, slot) in s.scroll_scores.iter_mut().enumerate() {
                *slot = match scroll.get(i) {
                    Some(x) => crate::ruleset::ScrollScore::remote(
                        x.name,
                        x.score,
                        x.multi_kills,
                        x.position,
                        x.alpha,
                    ),
                    None => Default::default(),
                };
            }
            let bar = world
                .survival_health_bar
                .get_or_insert_with(Default::default);
            bar.display_health = *health;
            bar.alpha = *health_alpha;
        }
        (
            Some(RulesetState::TimedObjective {
                counters,
                timer,
                time_text,
                hud,
            }),
            Some(SceneRuleset::TimedObjective(t)),
        ) => {
            for &(id, value) in counters {
                session.runtime.variables.set_counter(id, value);
            }
            session.runtime.variables.add_timer(t.timer, true, *timer);
            session.runtime.variables.set_timer(t.timer, *timer);
            t.time_text = time_text.clone();
            world.timed_objective_hud = *hud;
        }
        (
            Some(RulesetState::Versus {
                mode,
                teams_enabled,
                waiting,
                countdown,
                time_limit,
                time,
                scores,
                team_scores,
                respawns,
                team_respawns,
                respawn_timers,
                sudden_death,
                unlocks,
                teams,
            }),
            Some(SceneRuleset::Versus(v)),
        ) => {
            v.settings.mode = match mode {
                4 => VersusMode::Brawl,
                5 => VersusMode::Pyrite,
                6 => VersusMode::Krietor,
                7 => VersusMode::King,
                _ => VersusMode::DeathMatch,
            };
            v.settings.teams = *teams_enabled;
            v.countdown_timer = *countdown;
            v.time_limit = *time_limit;
            v.time_limit_timer = *time;
            // Mode scores are per server slot except team entries.
            // Krietor's team rounds score team indices 0/1 in mScores.
            v.scores = if v.settings.mode == VersusMode::Krietor && v.settings.teams {
                crate::slots::PerSlot::filled(0, 0).with_values(scores)
            } else {
                remap(scores, &local)
            };
            v.team_scores = *team_scores;
            v.respawns = remap(respawns, &local);
            v.team_respawns = *team_respawns;
            v.respawn_timers = remap(respawn_timers, &local);
            v.sudden_death = *sudden_death;
            v.unlocks = unlocks.clone();
            world.player_teams = remap(teams, &local);
            world.versus_waiting = *waiting;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ruleset::{VersusMode, VersusSettings};

    fn root() -> Option<std::path::PathBuf> {
        let r = magicka_content::retail_content_dir();
        r.is_dir().then_some(r)
    }

    #[test]
    fn clients_mirror_survival_hud_state() {
        let Some(root) = root() else { return };
        let mut host = PlaySession::load(&root, "Challenges/ch_glade.lvl", "Wizard").unwrap();
        for _ in 0..120 {
            host.step(1.0 / 60.0);
        }
        host.world.survival_mut().unwrap().time_multiplier = 3;
        let state = rules_state(&host);
        let mut client = PlaySession::load(&root, "Challenges/ch_glade.lvl", "Wizard").unwrap();
        client.disable_scripts();
        mirror_rules(&mut client, &state, 0);
        let (h, c) = (
            host.world.survival().unwrap(),
            client.world.survival().unwrap(),
        );
        assert_eq!((c.wave_number, c.time_multiplier), (h.wave_number, 3));
        assert!((c.normalized_time() - h.normalized_time()).abs() < 1e-5);
        assert_eq!(
            client.network_mode,
            crate::play_session::NetworkMode::Client
        );
        // Serialized like a GameFrame field.
        let bytes = serde_json::to_vec(&state).unwrap();
        assert_eq!(serde_json::from_slice::<RulesState>(&bytes).unwrap(), state);
    }

    #[test]
    fn versus_slots_swap_into_the_clients_view() {
        let Some(root) = root() else { return };
        let mut host = PlaySession::load(&root, "Versus/vs_boat.lvl", "Wizard").unwrap();
        host.join_player_at_start(1, "Wizard").unwrap();
        host.set_versus(VersusSettings::defaults(VersusMode::DeathMatch), [0; 4])
            .unwrap();
        host.step(1.0 / 60.0);
        host.world.statistics.scores[1].kills = 4;
        if let Some(SceneRuleset::Versus(v)) = host.world.ruleset.as_mut() {
            v.scores = [1, 4, 0, 0].into();
        }
        let state = rules_state(&host);
        let mut client = PlaySession::load(&root, "Versus/vs_boat.lvl", "Wizard").unwrap();
        client.disable_scripts();
        client
            .set_versus(VersusSettings::defaults(VersusMode::Brawl), [0; 4])
            .unwrap();
        mirror_rules(&mut client, &state, 1);
        let v = client.world.versus().unwrap();
        assert_eq!(
            v.settings.mode,
            VersusMode::DeathMatch,
            "mode comes from the host"
        );
        assert_eq!(
            v.scores[0], 4,
            "the client's own (server slot 1) score is local slot 0"
        );
        assert_eq!(v.scores[1], 1);
        assert_eq!(client.world.statistics.scores[0].kills, 4);
    }
}
