//! Offline SurvivalRuleset host: wave action ownership and scene progression.
use super::*;
use crate::ruleset::{
    RulesetLayout, SceneRuleset, SurvivalCommand, SurvivalFacts, SurvivalRuleset,
};

impl<C: SceneContent> SceneWorld<C> {
    pub(super) fn initialize_survival(&mut self, layout: &RulesetLayout) -> Result<(), WorldError> {
        if layout.waves.is_empty() || layout.areas.is_empty() {
            return Err("Survival requires waves and spawn areas".into());
        }
        // Read template health without creating characters. Native wave
        // initialization pre-reads each Spawn's total hit points.
        let mut health = BTreeMap::new();
        for a in layout.waves.iter().flatten() {
            for (name, _) in &a.spawns {
                if !health.contains_key(name) {
                    let template = self
                        .content
                        .character(name)
                        .ok_or_else(|| format!("missing survival template {name}"))?;
                    health.insert(name.clone(), template.max_hit_points);
                }
            }
        }
        let players = self.players.iter().flatten().filter(|p| p.playing).count();
        let state = SurvivalRuleset::new(
            layout,
            |n| (self.random() * n as f32) as usize,
            |name| health[name],
            players,
        );
        for (alias, area) in state.area_aliases(layout) {
            let alias = custom_hash(&alias);
            let target = custom_hash(&area.to_ascii_lowercase());
            self.area_aliases.insert(alias, target);
            if let Some(geometry) = self.areas.get(&target).copied() {
                self.areas.insert(alias, geometry);
            }
            if let Some(locator) = self.locators.get(&target).copied() {
                self.locators.insert(alias, locator);
            }
            if let Some(info) = self.locator_info.get(&target).copied() {
                self.locator_info.insert(alias, info);
            }
            self.presence.alias(alias, target);
        }
        // Initialize: StatisticsManager.SurvivalReset.
        self.statistics.reset();
        self.ruleset = Some(SceneRuleset::Survival(Box::new(state)));
        Ok(())
    }

    pub(super) fn update_survival(
        &mut self,
        runtime: &mut ScriptRuntime,
        dt: f32,
    ) -> Result<(), ScriptRuntimeError<WorldError>> {
        if self.role == NetworkRole::Client {
            return Ok(());
        }
        let Some(r) = self.survival_mut() else {
            return Ok(());
        };
        let ids = r.current_wave_actions();
        let fired = r.begin_wave(dt);
        runtime.update_wave_actions(&ids, &fired, dt, self.role, self)?;
        // Use the same cached area membership as scene triggers; refreshing
        // here would change the native one-frame spawn/count relationship.
        let any = self
            .presence
            .get(custom_hash("any"))
            .map_err(|e| ScriptRuntimeError::World(format!("survival presence: {e:?}")))?;
        let luggage_item = any
            .type_count(custom_hash(crate::ruleset::LUGGAGE_ITEM))
            .max(0) as usize;
        let luggage_magick = any
            .type_count(custom_hash(crate::ruleset::LUGGAGE_MAGICK))
            .max(0) as usize;
        let facts = SurvivalFacts {
            hostile: any.faction_count(11).max(0) as usize,
            luggage: luggage_item + luggage_magick,
            luggage_item,
            luggage_magick,
            players_alive: self
                .players
                .iter()
                .flatten()
                .filter(|p| p.playing && !p.dead())
                .count(),
            game_ended: self.game_ended || self.pending_end_game.is_some(),
            stats_score: self.statistics.total_score,
            stats_damage: self.statistics.total_player_damage,
        };
        let commands = self
            .survival_mut()
            .unwrap()
            .finish_update(dt, facts, |id| runtime.wave_actions_finished(id));
        for command in commands {
            match command {
                SurvivalCommand::Victory => self.pending_end_game = Some((1, true, false)),
                SurvivalCommand::SpawnLuggage { item } => self.pending_survival_luggage.push(item),
                SurvivalCommand::FireTrigger(_) => unreachable!("begin_wave dispatched actions"),
            }
        }
        self.spawn_pending_luggage();
        // Update: mHealthBar.SetNormalizedHealth(CurrentWave.HitPointPercentage()).
        let percentage = self.survival().unwrap().hit_point_percentage(|h| {
            self.entities
                .iter()
                .find(|e| e.handle == h && e.added && !e.removed && e.faction == 1)
                .map(|e| e.hit_points)
        });
        let bar = self
            .survival_health_bar
            .get_or_insert_with(Default::default);
        bar.normalized_health = percentage;
        bar.update(dt);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        level_variables::LevelVariables, play_session::PlaySession, scene_script::SceneScriptLayout,
    };

    #[test]
    fn delayed_reinforcements_block_wave_advance_then_final_wave_requests_victory() {
        let root = magicka_content::retail_content_dir();
        if !root.is_dir() {
            return;
        }
        let mut s = PlaySession::load(&root, "Challenges/ch_glade.lvl", "Wizard").unwrap();
        let layout = SceneScriptLayout::read(br#"<Scene>
          <RuleSet type="Survival"><spawnAreas><area>west</area><area>east</area></spawnAreas></RuleSet>
          <wave>
            <waveActions area="west"><spawn type="goblin_rogue" order="idle" snapToNavMesh="false"/></waveActions>
            <waveActions area="east" delay="8"><spawn type="goblin_rogue" order="idle" snapToNavMesh="false" delay="1"/></waveActions>
          </wave>
          <wave><waveActions area="any"><spawn type="goblin_rogue" order="idle" snapToNavMesh="false"/></waveActions></wave>
        </Scene>"#, false).unwrap();
        // Wave progression only: no reward luggage among the counted NPCs.
        s.world.survival_packs = None;
        s.runtime = ScriptRuntime::new(layout, LevelVariables::default());
        s.runtime.initialize_actions(&mut s.world).unwrap();
        let layout = s.runtime.layout().ruleset.clone();
        s.world
            .initialize_ruleset(layout.as_ref(), &mut s.runtime.variables)
            .unwrap();
        s.runtime.initialize_wave_actions(&mut s.world).unwrap();
        let frame = |s: &mut PlaySession| s.world.step(&mut s.runtime, 0.05).unwrap();
        frame(&mut s);
        assert_eq!(s.world.npcs.len(), 1);
        let first = *s.world.npcs.keys().next().unwrap();
        EntityCommandHost::terminate(&mut s.world, first).unwrap();
        for _ in 0..130 {
            frame(&mut s);
        }
        assert_eq!(
            s.world.survival().unwrap().wave_number,
            1,
            "empty field must still wait for delayed reinforcements"
        );
        for _ in 0..60 {
            frame(&mut s);
        }
        let second = *s.world.npcs.keys().next().unwrap();
        assert_ne!(first, second);
        for _ in 0..120 {
            frame(&mut s);
        }
        assert_eq!(
            s.world.survival().unwrap().wave_number,
            1,
            "living reinforcement must hold the wave"
        );
        EntityCommandHost::terminate(&mut s.world, second).unwrap();
        for _ in 0..105 {
            frame(&mut s);
        }
        assert_eq!(s.world.survival().unwrap().wave_number, 2);
        let last = *s.world.npcs.keys().next().unwrap();
        EntityCommandHost::terminate(&mut s.world, last).unwrap();
        for _ in 0..105 {
            frame(&mut s);
        }
        assert!(s.world.survival().unwrap().victory);
        assert_eq!(s.world.pending_end_game, Some((1, true, false)));
    }
}
