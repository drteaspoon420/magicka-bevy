//! `Magicka.Levels.SurvivalRuleset` with its `Wave`s and `WaveActions` as a
//! deterministic state machine (research/rulesets.md).
use super::*;

/// SurvivalRuleset.TIMEBETWEENLEVELS.
pub const TIME_BETWEEN_LEVELS: f32 = 5.0;
/// TIMEBETWEENLUGGAGE.
pub const TIME_BETWEEN_LUGGAGE: f32 = 30.0;
pub const LUGGAGE_MAGICK: &str = "luggage_magick";
pub const LUGGAGE_ITEM: &str = "luggage_item";
const SCORE_QUEUE_DELAY: f32 = 0.5;
const TIME_MULTIPLIER_DECAY: f32 = 30.0;
const DAMAGE_MULTIPLIER_CAP: f32 = 10000.0;

/// ScrollScore: one "Name xN +score" line.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollScore {
    time_alive: f32,
    pub position: f32,
    pub score: i32,
    pub name: i32,
    pub multi_kills: i32,
    pub alpha: f32,
}

impl ScrollScore {
    /// A host's line as a client mirrors it.
    pub fn remote(name: i32, score: i32, multi_kills: i32, position: f32, alpha: f32) -> Self {
        Self {
            time_alive: 0.25,
            position,
            score,
            name,
            multi_kills,
            alpha,
        }
    }
    pub fn dead(&self) -> bool {
        self.position <= 0.0
    }
    /// Update: true when it died this frame.
    // `min(..).max(..)` maps NaN to 0.25 like Math.Min/Max; `clamp`
    // would propagate NaN instead.
    #[allow(clippy::manual_clamp)]
    fn update(&mut self, dt: f32) -> bool {
        self.time_alive += dt;
        self.position -= dt * 2.0;
        self.alpha = self.time_alive.min(0.25) * 4.0;
        self.alpha *= self.position.min(0.25).max(0.0) * 4.0;
        self.dead()
    }
}

#[derive(Clone, Debug)]
struct WaveActionsState {
    trigger: i32,
    has_actions: bool,
    delay: f32,
    initial_delay: f32,
    executed: bool,
    area: String,
    spawned_hit_points: f32,
}

#[derive(Clone, Debug, Default)]
struct WaveState {
    started: bool,
    executing: bool,
    actions: Vec<WaveActionsState>,
    /// (handle, item event) of TrackCharacter.
    characters: Vec<u16>,
    pre_read_hit_points: f32,
    total_hit_points: f32,
}

/// What the host must do after an update.
#[derive(Clone, Debug, PartialEq)]
pub enum SurvivalCommand {
    /// Execute a WaveActions' hidden trigger (its actions' OnTrigger).
    FireTrigger(i32),
    /// PlayState.Endgame(Victory, freeze, not phony).
    Victory,
    /// SpawnLuggage(item?).
    SpawnLuggage { item: bool },
}

/// The scene facts one update reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct SurvivalFacts {
    /// GetTriggerArea(any).GetFactionCount(EVIL|WILD|DEMON).
    pub hostile: usize,
    /// GetCount(Luggage_item) + GetCount(Luggage_magick).
    pub luggage: usize,
    pub luggage_item: usize,
    pub luggage_magick: usize,
    /// Playing players with a living avatar.
    pub players_alive: usize,
    pub game_ended: bool,
    /// StatisticsManager.SurvivalTotalScore / SurvivalTotalDamage.
    pub stats_score: i32,
    pub stats_damage: f32,
}

#[derive(Clone, Debug)]
pub struct SurvivalRuleset {
    waves: Vec<WaveState>,
    pub areas: Vec<String>,
    /// mCurrentWaveNr (1-based).
    pub wave_number: usize,
    current: usize,
    level_change_countdown: f32,
    wave_time: f32,
    pub time_multiplier: i32,
    pub damage_multiplier: i32,
    total_damage_start: f32,
    new_total_multiplier: bool,
    pub total_multiplier: i32,
    players_alive_last: usize,
    luggage_timer: f32,
    spawned_treats: u32,
    score_queue: Vec<(ScrollScore, f32)>,
    pub scroll_scores: [ScrollScore; 16],
    active_texts: usize,
    /// mTotalScore (ruleset-side, multi-kills counted).
    pub total_score: i32,
    /// mCurrentTotalScore shown in the HUD (eases to the statistics total).
    pub shown_total_score: i32,
    shown_total_score_float: f32,
    pub current_damage_amount: f32,
    /// Ended (Victory requested).
    pub victory: bool,
}

impl SurvivalRuleset {
    /// Construction + Initialize. `pick(n)` is MagickaMath.Random.Next(n)
    /// for the WaveActions without an area; `hit_points(template)` is the
    /// cached template's MaxHitpoints.
    pub fn new(
        layout: &RulesetLayout,
        mut pick: impl FnMut(usize) -> usize,
        mut hit_points: impl FnMut(&str) -> f32,
        players: usize,
    ) -> Self {
        let waves = layout
            .waves
            .iter()
            .map(|wave| {
                let mut w = WaveState::default();
                for a in wave {
                    let area = a.area.clone().unwrap_or_else(|| {
                        if layout.areas.is_empty() {
                            "any".into()
                        } else {
                            layout.areas[pick(layout.areas.len())].clone()
                        }
                    });
                    let spawned: f32 = a
                        .spawns
                        .iter()
                        .map(|(t, n)| hit_points(t) * *n as f32)
                        .sum();
                    w.pre_read_hit_points += spawned;
                    w.total_hit_points += spawned;
                    w.actions.push(WaveActionsState {
                        trigger: a.trigger,
                        has_actions: a.has_actions,
                        delay: a.delay,
                        initial_delay: a.delay,
                        executed: false,
                        area,
                        spawned_hit_points: spawned,
                    });
                }
                w
            })
            .collect();
        Self {
            waves,
            areas: layout.areas.clone(),
            wave_number: 1,
            current: 0,
            level_change_countdown: TIME_BETWEEN_LEVELS,
            wave_time: 0.0,
            time_multiplier: 5,
            damage_multiplier: 1,
            total_damage_start: 0.0,
            new_total_multiplier: true,
            total_multiplier: 6,
            players_alive_last: players,
            luggage_timer: TIME_BETWEEN_LUGGAGE * 0.25,
            spawned_treats: 0,
            score_queue: Vec::new(),
            scroll_scores: [ScrollScore::default(); 16],
            active_texts: 0,
            total_score: 0,
            shown_total_score: 0,
            shown_total_score_float: 0.0,
            current_damage_amount: 0.0,
            victory: false,
        }
    }

    /// (placeholder, area) aliases for every WaveActions.
    pub fn area_aliases(&self, layout: &RulesetLayout) -> Vec<(String, String)> {
        layout
            .waves
            .iter()
            .zip(&self.waves)
            .flat_map(|(lw, w)| {
                lw.iter()
                    .zip(&w.actions)
                    .map(|(l, a)| (l.placeholder.clone(), a.area.clone()))
            })
            .collect()
    }

    pub fn wave_count(&self) -> usize {
        self.waves.len()
    }
    /// ScoreMultiplier.
    pub fn score_multiplier(&self) -> i32 {
        self.time_multiplier * self.damage_multiplier
    }
    /// GetAnyArea: a random spawn area.
    pub fn any_area(&self, pick: impl FnOnce(usize) -> usize) -> Option<&str> {
        (!self.areas.is_empty()).then(|| self.areas[pick(self.areas.len())].as_str())
    }

    /// AddedCharacter for an EVIL/WILD/DEMON character: tracked by the
    /// current wave (an item event adds to the total, otherwise it
    /// replaces its pre-read share).
    pub fn added_character(&mut self, handle: u16, hit_points: f32, item_event: bool) {
        let Some(w) = self.waves.get_mut(self.current) else {
            return;
        };
        w.characters.push(handle);
        if item_event {
            w.total_hit_points += hit_points;
        } else {
            w.pre_read_hit_points -= hit_points;
        }
    }

    /// Wave.HitPointPercentage: `evil_hit_points(handle)` is the tracked
    /// character's HitPoints while it exists with faction EVIL.
    pub fn hit_point_percentage(&self, mut evil_hit_points: impl FnMut(u16) -> Option<f32>) -> f32 {
        let Some(w) = self.waves.get(self.current) else {
            return 0.0;
        };
        let alive: f32 = w
            .characters
            .iter()
            .filter_map(|&h| evil_hit_points(h))
            .map(|hp| hp.max(0.0))
            .sum();
        (alive + w.pre_read_hit_points) / w.total_hit_points
    }

    /// AddScore: a queued line, or one more multi-kill for a queued name.
    pub fn add_score(&mut self, name: i32, score: i32) {
        if score == 0 {
            return;
        }
        if let Some(entry) = self.score_queue.iter_mut().find(|(s, _)| s.name == name) {
            entry.0.multi_kills += 1;
            return;
        }
        let key = ScrollScore {
            score,
            name,
            multi_kills: 1,
            ..Default::default()
        };
        // SortedList by NameID.
        let at = self
            .score_queue
            .iter()
            .position(|(s, _)| s.name > name)
            .unwrap_or(self.score_queue.len());
        self.score_queue.insert(at, (key, SCORE_QUEUE_DELAY));
    }

    fn add_scroll_score(&mut self, mut score: ScrollScore) -> bool {
        let Some(slot) = self.scroll_scores.iter().position(|s| s.dead()) else {
            return false;
        };
        score.position = self.active_texts as f32 + 5.0;
        self.scroll_scores[slot] = score;
        self.active_texts += 1;
        self.total_score += score.score * score.multi_kills;
        true
    }

    /// Wave.Update phase, before the host updates the current wave's actions.
    pub fn begin_wave(&mut self, dt: f32) -> Vec<i32> {
        let mut fired = Vec::new();
        if let Some(w) = self.waves.get_mut(self.current) {
            self.wave_time += dt;
            w.started = true;
            for a in &mut w.actions {
                a.delay -= dt;
                if a.delay <= 0.0 && !a.executed {
                    fired.push(a.trigger);
                    a.executed = true;
                }
            }
        }
        fired
    }

    pub fn current_wave_actions(&self) -> Vec<i32> {
        self.waves
            .get(self.current)
            .map_or_else(Vec::new, |w| w.actions.iter().map(|a| a.trigger).collect())
    }

    /// Pure host adapter: returned FireTrigger requests execute after this
    /// call, so keep their wave busy until the next update.
    pub fn update(
        &mut self,
        dt: f32,
        facts: SurvivalFacts,
        mut trigger_idle: impl FnMut(i32) -> bool,
    ) -> Vec<SurvivalCommand> {
        let fired = self.begin_wave(dt);
        let mut out: Vec<_> = fired
            .iter()
            .copied()
            .map(SurvivalCommand::FireTrigger)
            .collect();
        out.extend(self.finish_update(dt, facts, |id| !fired.contains(&id) && trigger_idle(id)));
        out
    }

    /// SurvivalRuleset.Update after WaveActions.Update. The live host supplies
    /// HasFinishedExecuting after this frame's actions have run.
    pub fn finish_update(
        &mut self,
        dt: f32,
        facts: SurvivalFacts,
        mut trigger_idle: impl FnMut(i32) -> bool,
    ) -> Vec<SurvivalCommand> {
        let mut out = Vec::new();
        let wave_count = self.waves.len();
        if let Some(w) = self.waves.get_mut(self.current) {
            w.executing = w
                .actions
                .iter()
                .any(|a| a.has_actions && (a.delay > 0.0 || !trigger_idle(a.trigger)));
            if w.started && !w.executing && facts.hostile <= facts.luggage {
                if self.level_change_countdown <= 0.0 {
                    if self.wave_number + 1 > wave_count {
                        if !facts.game_ended && !self.victory {
                            self.victory = true;
                            out.push(SurvivalCommand::Victory);
                        }
                    } else {
                        self.current = self.wave_number;
                        self.wave_number += 1;
                        self.level_change_countdown = TIME_BETWEEN_LEVELS;
                        self.spawned_treats = 0;
                        self.time_multiplier = 5;
                        self.wave_time = 0.0;
                    }
                } else {
                    self.level_change_countdown -= dt;
                }
            }
        }
        self.luggage_timer -= dt;
        if self.luggage_timer <= 0.0 && self.spawned_treats < 2 {
            if facts.luggage_item == 0 && facts.luggage_magick == 0 {
                out.push(SurvivalCommand::SpawnLuggage {
                    item: self.spawned_treats != 0,
                });
                self.spawned_treats += 1;
                self.luggage_timer += TIME_BETWEEN_LUGGAGE;
            } else {
                self.luggage_timer += TIME_BETWEEN_LUGGAGE * 0.5;
            }
        }
        let mut i = 0;
        while i < self.score_queue.len() {
            let left = self.score_queue[i].1 - dt;
            if left <= 0.0 {
                let score = self.score_queue[i].0;
                if self.add_scroll_score(score) {
                    self.score_queue.remove(i);
                    continue;
                }
                // The retry delay is not stored back in Update (native).
            } else {
                self.score_queue[i].1 = left;
            }
            i += 1;
        }
        for s in &mut self.scroll_scores {
            if !s.dead() && s.update(dt) {
                self.active_texts = self.active_texts.saturating_sub(1);
            }
        }
        self.shown_total_score_float +=
            (facts.stats_score as f32 - self.shown_total_score as f32) * dt * 5.0;
        self.shown_total_score = self.shown_total_score_float as i32;
        if facts.players_alive < self.players_alive_last {
            self.damage_multiplier = 1;
            self.total_damage_start = facts.stats_damage;
        }
        self.players_alive_last = facts.players_alive;
        if self.time_multiplier > 1 && self.wave_time > TIME_MULTIPLIER_DECAY {
            self.wave_time -= TIME_MULTIPLIER_DECAY;
            self.time_multiplier -= 1;
            self.new_total_multiplier = true;
        }
        let mut damage = facts.stats_damage - self.total_damage_start;
        if self.damage_multiplier < 5 && damage > DAMAGE_MULTIPLIER_CAP {
            self.total_damage_start += DAMAGE_MULTIPLIER_CAP;
            damage -= DAMAGE_MULTIPLIER_CAP;
            self.damage_multiplier += 1;
            self.new_total_multiplier = true;
        }
        if self.new_total_multiplier {
            self.total_multiplier = self.damage_multiplier * self.time_multiplier;
        }
        self.current_damage_amount += (damage - self.current_damage_amount) * dt * 5.0;
        out
    }

    /// Client mirror of the host's HUD values (RulesetMessage's role):
    /// wave, eased total score, [time, damage, total] multipliers and the
    /// bar/pie amounts.
    pub fn apply_remote(
        &mut self,
        wave: usize,
        shown_total_score: i32,
        multipliers: [i32; 3],
        damage_amount: f32,
        normalized_time: f32,
    ) {
        self.wave_number = wave;
        self.current = wave
            .saturating_sub(1)
            .min(self.waves.len().saturating_sub(1));
        self.shown_total_score = shown_total_score;
        [
            self.time_multiplier,
            self.damage_multiplier,
            self.total_multiplier,
        ] = multipliers;
        self.current_damage_amount = damage_amount * DAMAGE_MULTIPLIER_CAP;
        self.wave_time = (1.0 - normalized_time) * TIME_MULTIPLIER_DECAY;
    }

    /// RenderData.NormalizedTime (the time-multiplier pie).
    pub fn normalized_time(&self) -> f32 {
        if self.time_multiplier > 1 {
            1.0 - self.wave_time / TIME_MULTIPLIER_DECAY
        } else {
            1.0
        }
    }
    /// RenderData.DamageAmount (the damage-multiplier bar).
    pub fn damage_amount(&self) -> f32 {
        (self.current_damage_amount / DAMAGE_MULTIPLIER_CAP).min(1.0)
    }
    /// The spawned hit points of the current wave's WaveActions.
    pub fn wave_spawned_hit_points(&self) -> f32 {
        self.waves.get(self.current).map_or(0.0, |w| {
            w.actions.iter().map(|a| a.spawned_hit_points).sum()
        })
    }
    /// Reset (ResetWave) is not reached by the class; initial delays are kept.
    pub fn initial_delays(&self) -> Vec<f32> {
        self.waves.get(self.current).map_or(Vec::new(), |w| {
            w.actions.iter().map(|a| a.initial_delay).collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(waves: usize) -> RulesetLayout {
        RulesetLayout {
            kind: RulesetKind::Survival,
            node: Default::default(),
            areas: vec!["north".into(), "south".into()],
            waves: (0..waves)
                .map(|w| {
                    vec![WaveActionsLayout {
                        trigger: w as i32 + 100,
                        has_actions: true,
                        area: None,
                        placeholder: wave_area_placeholder(w, 0),
                        delay: 1.0,
                        spawns: vec![("orc_regular".into(), 2)],
                    }]
                })
                .collect(),
        }
    }

    #[test]
    fn waves_advance_after_the_countdown_and_the_last_wins() {
        let l = layout(2);
        let mut r = SurvivalRuleset::new(&l, |_| 1, |_| 100.0, 1);
        assert_eq!(r.area_aliases(&l)[0].1, "south");
        let facts = SurvivalFacts {
            players_alive: 1,
            ..Default::default()
        };
        let dt = 0.1;
        let mut fired = vec![];
        for _ in 0..9 {
            fired.extend(r.update(dt, facts, |_| true));
        }
        assert!(fired.is_empty(), "WaveActions delay 1 s");
        fired.extend(r.update(dt, facts, |_| true));
        fired.extend(r.update(dt, facts, |_| true));
        assert_eq!(fired, vec![SurvivalCommand::FireTrigger(100)]);
        // Enemies alive: no countdown.
        let busy = SurvivalFacts {
            hostile: 2,
            ..facts
        };
        for _ in 0..100 {
            r.update(dt, busy, |_| true);
        }
        assert_eq!(r.wave_number, 1);
        for _ in 0..52 {
            r.update(dt, facts, |_| true);
        }
        assert_eq!(r.wave_number, 2);
        let mut out = vec![];
        for _ in 0..80 {
            out.extend(r.update(dt, facts, |_| true));
        }
        assert!(out.contains(&SurvivalCommand::FireTrigger(101)));
        assert!(out.contains(&SurvivalCommand::Victory));
    }

    #[test]
    fn empty_wave_groups_do_not_wait_for_their_authored_delay() {
        let mut l = layout(1);
        l.waves[0][0].has_actions = false;
        l.waves[0][0].delay = 50.0;
        let mut r = SurvivalRuleset::new(&l, |_| 0, |_| 100.0, 1);
        let mut commands = Vec::new();
        for _ in 0..53 {
            commands.extend(r.update(0.1, SurvivalFacts::default(), |_| true));
        }
        assert!(commands.contains(&SurvivalCommand::Victory));
        assert!(r.victory, "empty group must not impose a 50 second wait");
    }

    #[test]
    fn hit_points_and_scores() {
        let l = layout(1);
        let mut r = SurvivalRuleset::new(&l, |_| 0, |_| 100.0, 1);
        r.added_character(7, 100.0, false);
        r.added_character(8, 100.0, false);
        assert_eq!(r.hit_point_percentage(|h| (h == 7).then_some(50.0)), 0.25);
        r.add_score(3, 40);
        r.add_score(3, 40);
        let facts = SurvivalFacts {
            players_alive: 1,
            ..Default::default()
        };
        for _ in 0..6 {
            r.update(0.1, facts, |_| true);
        }
        assert_eq!(r.total_score, 80, "one line, two kills");
        assert_eq!(r.scroll_scores.iter().filter(|s| !s.dead()).count(), 1);
    }

    #[test]
    fn multipliers_decay_with_time_and_grow_with_damage() {
        let l = layout(1);
        let mut r = SurvivalRuleset::new(&l, |_| 0, |_| 100.0, 2);
        let mut facts = SurvivalFacts {
            players_alive: 2,
            hostile: 5,
            ..Default::default()
        };
        for _ in 0..310 {
            r.update(0.1, facts, |_| true);
        }
        assert_eq!(r.time_multiplier, 4);
        facts.stats_damage = 10500.0;
        r.update(0.1, facts, |_| true);
        assert_eq!(r.damage_multiplier, 2);
        assert_eq!(r.total_multiplier, 8);
        facts.players_alive = 1;
        r.update(0.1, facts, |_| true);
        assert_eq!(
            r.damage_multiplier, 1,
            "a death resets the damage multiplier"
        );
    }
}
