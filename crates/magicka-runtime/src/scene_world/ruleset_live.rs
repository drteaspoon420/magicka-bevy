//! GameScene.RuleSet host: creation from the scene layout, Initialize after
//! ordinary triggers, and Update after them each Level.Update
//! (research/rulesets.md). Ruleset-specific hosts live beside it
//! (survival_live).
use super::*;
use crate::level_variables::{LevelVariables, TimerFrame};
use crate::ruleset::{
    RulesetKind, RulesetLayout, RulesetResults, SceneRuleset, SurvivalPlayerResult,
    SurvivalResults, SurvivalRuleset, TimedObjectiveRuleset, VersusResults,
};

impl<C: SceneContent> SceneWorld<C> {
    pub fn survival(&self) -> Option<&SurvivalRuleset> {
        self.ruleset.as_ref().and_then(SceneRuleset::survival)
    }
    pub fn survival_mut(&mut self) -> Option<&mut SurvivalRuleset> {
        self.ruleset.as_mut().and_then(SceneRuleset::survival_mut)
    }
    pub fn timed_objective(&self) -> Option<&TimedObjectiveRuleset> {
        self.ruleset
            .as_ref()
            .and_then(SceneRuleset::timed_objective)
    }

    /// The ruleset's statistics sheet (InGameMenu*Statistics.OnEnter).
    pub fn ruleset_results(&self, variables: &LevelVariables) -> Option<RulesetResults> {
        Some(match self.ruleset.as_ref()? {
            SceneRuleset::Survival(r) => RulesetResults::Survival(SurvivalResults {
                players: self.player_results(),
                total_score: self.statistics.total_score,
                wave: r.wave_number,
            }),
            SceneRuleset::TimedObjective(r) => RulesetResults::TimedObjective(r.results(variables)),
            SceneRuleset::Versus(r) => RulesetResults::Versus(VersusResults {
                mode: r.settings.mode,
                teams: r.teams(),
                players: self.player_results(),
                scores: r.mode_scores(),
            }),
        })
    }

    fn player_results(&self) -> Vec<SurvivalPlayerResult> {
        self.players
            .iter()
            .flatten()
            .filter(|p| p.playing)
            .map(|p| SurvivalPlayerResult {
                slot: p.slot,
                kills: self.statistics.scores[p.slot].kills,
                deaths: self.statistics.scores[p.slot].deaths,
            })
            .collect()
    }

    /// Level.Update's timer frame: PlayState pause is the host's (offline
    /// sessions step only while running), IsGameEnded and IsInCutscene.
    pub(crate) fn timer_frame(&self) -> TimerFrame {
        TimerFrame {
            paused: false,
            game_ended: self.game_ended,
            cutscene: self.flags.cutscene,
        }
    }

    /// GameScene.Initialize's ruleset phase (after ordinary triggers).
    pub(crate) fn initialize_ruleset(
        &mut self,
        layout: Option<&RulesetLayout>,
        variables: &mut LevelVariables,
    ) -> Result<(), WorldError> {
        self.ruleset = None;
        let Some(layout) = layout else {
            return Ok(());
        };
        match layout.kind {
            RulesetKind::Survival => self.initialize_survival(layout)?,
            RulesetKind::TimedObjective => {
                let mut r = TimedObjectiveRuleset::read(&layout.node);
                r.initialize(variables);
                self.ruleset = Some(SceneRuleset::TimedObjective(Box::new(r)));
            }
            RulesetKind::Versus => self.initialize_versus(layout),
        }
        Ok(())
    }

    /// GameScene.Update's script phase after Level.Update's timers: presence,
    /// scene triggers, then the ruleset.
    pub(crate) fn run_scene_scripts(
        &mut self,
        runtime: &mut ScriptRuntime,
        dt: f32,
    ) -> Result<(), ScriptRuntimeError<WorldError>> {
        self.refresh_presence();
        let role = self.role;
        runtime.update(dt, role, self)?;
        self.update_ruleset(runtime, dt)
    }

    fn update_ruleset(
        &mut self,
        runtime: &mut ScriptRuntime,
        dt: f32,
    ) -> Result<(), ScriptRuntimeError<WorldError>> {
        match self.ruleset.as_ref().map(SceneRuleset::kind) {
            Some(RulesetKind::Survival) => self.update_survival(runtime, dt),
            Some(RulesetKind::TimedObjective) => {
                let frame = self.timer_frame();
                if let Some(SceneRuleset::TimedObjective(r)) = &mut self.ruleset {
                    self.timed_objective_hud = r.update(&runtime.variables, frame);
                }
                Ok(())
            }
            Some(RulesetKind::Versus) => {
                self.update_versus(dt);
                Ok(())
            }
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::play_session::PlaySession;
    use crate::scene_loading::custom_hash;

    #[test]
    fn vietnam_challenge_runs_its_timed_objective_clock() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../target/Magicka/Content");
        if !root.is_dir() {
            return;
        }
        let mut s = PlaySession::load(&root, "Challenges/ch_vietnam.lvl", "Wizard").unwrap();
        let r = s.world.timed_objective().expect("timed objective attached");
        assert_eq!(r.objectives.len(), 5);
        let timer = r.timer;
        assert_eq!(timer, custom_hash("timetimer"));
        for _ in 0..42 {
            s.world.flags.cutscene = false;
            s.world.step(&mut s.runtime, 0.05).unwrap();
        }
        let r = s.world.timed_objective().unwrap();
        assert!((s.runtime.variables.timer(timer) - 2.1).abs() < 1e-3);
        assert_eq!(r.time_text, "0:00:02");
        s.world.game_ended = true;
        s.world.step(&mut s.runtime, 1.0).unwrap();
        assert!(
            (s.runtime.variables.timer(timer) - 2.1).abs() < 1e-3,
            "played timer stops once the game has ended"
        );
    }
}
