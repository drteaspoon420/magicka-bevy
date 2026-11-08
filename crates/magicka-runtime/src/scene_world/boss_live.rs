//! BossFight singleton and PlayState.GenericHealthBar (research/boss-fight.md).
//!
//! Generic bosses are template NPCs (SpawnNPC's path) and prop bosses are
//! damageable physics entities (SpawnPhys's path); both feed the fight's
//! summed health. Jormungandr has a dedicated asset-backed actor; Vlad and
//! Warlord have specialized character-backed encounter rules; Machine,
//! Fafnir, Cult, CountryDefense, Death, Grimnir and Assatur have their own hosts. Other named boss
//! classes (Cthulhu, ...) are not reimplemented yet: a character-backed class spawns its template as a
//! plain NPC, and the others become a logged stand-in that is defeated
//! STAND_IN_SECONDS after the fight starts, firing the triggers the class
//! fires, so the campaign can progress (documented divergence).
use super::*;
use crate::boss_action::{BossAction, BossCommand, BossSetup, HealthBarConfig, HealthBarType};
use crate::world_condition::BossFacts;

/// Fight time after which a stand-in named boss counts as defeated.
pub const STAND_IN_SECONDS: f32 = 20.0;

#[derive(Clone, Debug, PartialEq)]
pub enum BossRef {
    Jormungandr(u16),
    Vlad(crate::boss_vlad::VladBoss),
    Warlord(crate::boss_warlord::WarlordBoss),
    /// GenericBoss or a character-backed named boss (an NPC handle).
    Npc(u16),
    /// PropBoss (a damageable physics entity handle).
    Prop(u16),
    /// Machine (Havindr): ported named class.
    Machine(super::machine_live::MachineHost),
    /// Fafnir (Ruins): ported named class.
    Fafnir(Box<super::fafnir_live::FafnirHost>),
    Death(Box<super::boss_death_live::DeathBossHost>),
    Grimnir(Box<super::grimnir_live::GrimnirHost>),
    Grimnir2(Box<super::grimnir2_live::Grimnir2Host>),
    /// Assatur (EndofWorld2): ported named class.
    Assatur(Box<super::assatur_live::AssaturHost>),
    /// Cult (Mines): ported named class.
    Cult(Box<super::cult_live::CultHost>),
    /// CountryDefense (Highlands): ported UI-only named class.
    CountryDefense(super::country_defense_live::CountryDefenseHost),
    /// Parker (Tsar_WizardCastle): ported named class.
    Parker(Box<super::parker_live::ParkerHost>),
    /// Cthulhu (Tsar_Rlyeh): ported named class with its tentacles.
    Cthulhu(Box<super::cthulhu_live::CthulhuHost>),
    /// A named boss class without an implementation.
    StandIn {
        class: String,
        fight_time: f32,
        dead: bool,
    },
}

#[derive(Clone, Debug, Default)]
pub struct BossFight {
    /// IsSetup (mGotPlayState).
    pub setup: bool,
    /// PlayState.BossFight != null (Initialize sets it, Reset clears it).
    pub active: bool,
    pub bosses: Vec<BossRef>,
    /// Native BattleState singletons share one countdown within each Vlad kind.
    pub vlad_taunt_timers: [f32; 3],
    pub running: bool,
    pub dead: bool,
    pub normalized_health: f32,
    pub freeze_time: f32,
    pub health_appear_delay: f32,
    pub health_bar_width: f32,
    /// The health bar is shown (HUDManager.OnBossFightStarted ..Ended).
    pub hud: bool,
    /// BossFight.mTitleRenderer and mBossNameSet.
    pub title: crate::title_renderer::TitleRenderer,
    pub name_set: bool,
    /// The title submitted for drawing this update.
    pub shown_title: Option<crate::title_renderer::ShownTitle>,
    /// BossFight.mHealthbar.
    pub bar: BossHealthBar,
    /// The bar submitted a render this update (running, appear delay over).
    pub bar_visible: bool,
}

/// BossHealthBar: fades in/out at 0.5 per second; once opaque (or while
/// fading out) the shown fill eases towards the health at 10 per second.
/// The fill colour is (Power, 0, 0) with Power 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BossHealthBar {
    pub alpha: f32,
    pub display_health: f32,
    pub normalized_health: f32,
    pub destroy: bool,
    pub power: f32,
}

impl Default for BossHealthBar {
    /// Reset (and the constructor).
    fn default() -> Self {
        Self {
            alpha: 0.0,
            display_health: 0.0,
            normalized_health: 1.0,
            destroy: false,
            power: 1.0,
        }
    }
}

impl BossHealthBar {
    pub fn update(&mut self, dt: f32) {
        self.alpha = if self.destroy {
            (self.alpha - dt * 0.5).max(0.0)
        } else {
            (self.alpha + dt * 0.5).min(1.0)
        };
        if self.alpha > 0.99 || self.destroy {
            self.display_health += (self.normalized_health - self.display_health) * 10.0 * dt;
        }
    }
}

#[cfg(test)]
mod boss_health_bar_tests {
    use super::BossHealthBar;

    #[test]
    fn fill_waits_for_opacity_then_tracks_damage() {
        let mut bar = BossHealthBar::default();
        for _ in 0..19 {
            bar.update(0.1);
            assert_eq!(bar.display_health, 0.0);
        }
        bar.update(0.1);
        assert_eq!(bar.alpha, 1.0);
        assert_eq!(bar.display_health, 1.0);
        bar.normalized_health = 0.4;
        bar.update(0.05);
        assert!((bar.display_health - 0.7).abs() < 1e-6);
    }

    #[test]
    fn death_fades_and_drains_fill_even_before_opacity() {
        let mut bar = BossHealthBar {
            alpha: 0.5,
            display_health: 0.8,
            normalized_health: 0.0,
            destroy: true,
            ..Default::default()
        };
        bar.update(0.05);
        assert_eq!(bar.alpha, 0.475);
        assert_eq!(bar.display_health, 0.4);
        bar.update(2.0);
        assert_eq!(bar.alpha, 0.0);
        // Native linear easing does not clamp or subdivide large time steps.
        assert!((bar.display_health - -7.6).abs() < 1e-6);
        bar = BossHealthBar::default();
        assert_eq!(bar.alpha, 0.0);
        assert_eq!(bar.display_health, 0.0);
        assert_eq!(bar.normalized_health, 1.0);
        assert_eq!(bar.power, 1.0);
        assert!(!bar.destroy);
    }
}

impl BossFight {
    pub fn facts(&self) -> Option<BossFacts> {
        self.active.then_some(BossFacts {
            normalized_health: self.normalized_health,
            dead: self.dead,
        })
    }
}

/// Slices of UI/HUD/CounterAnimation (Texture3D 32x32x8): the animated
/// sprite's frame count (the viewer checks the loaded asset against it).
pub const COUNTER_ANIMATION_SLICES: f32 = 8.0;
/// GenericHealthBar.mDynamiteBarOffset.
pub const DYNAMITE_BAR_OFFSET: f32 = 40.0;

/// PlayState.GenericHealthBar (timer and counter bars).
#[derive(Clone, Debug, Default)]
pub struct GenericHealthBar {
    pub config: HealthBarConfig,
    pub active: bool,
    pub removing: bool,
    /// TimeTicking / IsDone (the animated sprite draws while ticking and
    /// not done).
    pub ticking: bool,
    paused: bool,
    /// The fill position shift: Activate adds the dynamite offset for the
    /// Dynamite graphics (Reset, on scene change, removes it once).
    pub dynamite_shift: f32,
    /// mAnimationTimer and the frames mDepth has advanced past its
    /// initial half slice.
    sprite_timer: f32,
    pub sprite_frames: u32,
    initial_delay: f32,
    ttl: f32,
    start_time: f32,
    end_time: f32,
    counter: f32,
    counter_start: f32,
    counter_end: f32,
    pub done: bool,
    end_triggered: bool,
    fade: f32,
    /// Displayed fill and alpha.
    pub display_health: f32,
    pub alpha: f32,
}

impl GenericHealthBar {
    /// HealthBar.Execute.
    fn configure(&mut self, c: &HealthBarConfig) {
        self.config = c.clone();
        self.display_health = c.display_health;
        self.alpha = c.alpha;
        self.initial_delay = c.initial_timer_delay;
        match c.kind {
            HealthBarType::TimerDecreasing => {
                self.start_time = c.time;
                self.ttl = c.time;
                self.end_time = 0.0;
            }
            HealthBarType::TimerIncreasing => {
                self.start_time = 0.0;
                self.ttl = 0.0;
                self.end_time = c.time;
            }
            HealthBarType::Counter => {
                self.counter_start = 0.0;
                self.counter = 0.0;
                self.counter_end = c.count;
            }
            HealthBarType::Hp => {
                self.counter_start = c.count;
                self.counter = c.count;
                self.counter_end = 0.0;
            }
        }
        // OnEndTriggerID setter re-arms a non-zero trigger.
        if c.on_end_trigger != 0 {
            self.end_triggered = false;
        }
        self.done = false;
    }

    /// GenericHealthBar.Activate.
    fn activate(&mut self) {
        self.active = true;
        if matches!(
            self.config.kind,
            HealthBarType::TimerDecreasing | HealthBarType::TimerIncreasing
        ) && self.initial_delay <= 0.0
        {
            self.ticking = true;
        }
        if self.config.dynamite {
            self.dynamite_shift += DYNAMITE_BAR_OFFSET;
        }
        self.paused = false;
    }

    /// The animated sprite's Texture3D W coordinate (mDepth).
    pub fn sprite_depth(&self) -> f32 {
        (0.5 + self.sprite_frames as f32) / COUNTER_ANIMATION_SLICES
    }

    /// GenericHealthBar.Remove: fade out and stop the timer.
    fn remove(&mut self) {
        self.removing = true;
        self.ticking = false;
        self.fade = 0.0;
        self.config.has_animated_sprite = false;
    }

    /// GenericHealthBar.Update; returns the OnEndTrigger to execute once.
    /// The counter types' 4-second random test increments are omitted.
    fn update(&mut self, dt: f32) -> Option<i32> {
        if !self.active {
            return None;
        }
        let mut fire = None;
        if self.done && !self.end_triggered {
            self.end_triggered = true;
            fire = Some(self.config.on_end_trigger);
        }
        if self.initial_delay > 0.0 && !self.paused {
            self.initial_delay -= dt;
            if self.initial_delay <= 0.0 {
                self.ticking = true;
            }
        }
        match self.config.kind {
            HealthBarType::TimerDecreasing => {
                if self.ticking && !self.paused {
                    self.ttl -= dt;
                    if self.ttl <= self.end_time {
                        self.done = true;
                    }
                }
                self.display_health = if self.start_time > 0.0 {
                    (self.ttl / self.start_time).max(0.0)
                } else {
                    0.0
                };
            }
            HealthBarType::TimerIncreasing => {
                if self.ticking && !self.paused {
                    self.ttl += dt;
                    if self.ttl >= self.end_time {
                        self.done = true;
                    }
                }
                self.display_health = if self.end_time > 0.0 {
                    (self.ttl / self.end_time).min(1.0)
                } else {
                    1.0
                };
            }
            HealthBarType::Counter => {
                if self.counter >= self.counter_end {
                    self.done = true;
                }
            }
            HealthBarType::Hp => {
                if self.counter <= self.counter_end {
                    self.done = true;
                }
            }
        }
        if self.config.has_animated_sprite && !self.paused {
            // One slice per update at most, every 1/(slices·2.25) s.
            let step = 1.0 / COUNTER_ANIMATION_SLICES / 2.25;
            self.sprite_timer += dt;
            if self.sprite_timer > step {
                self.sprite_frames = self.sprite_frames.wrapping_add(1);
                self.sprite_timer -= step;
            }
        }
        self.fade += dt;
        let fade_time = self.config.fade_time.max(f32::EPSILON);
        self.alpha = if self.removing {
            (1.0 - self.fade / fade_time).max(0.0)
        } else {
            (self.fade / fade_time).min(1.0)
        };
        fire.filter(|&t| t != 0)
    }
}

/// Triggers a named class executes when it is defeated, in order.
fn stand_in_death_triggers(class: &str) -> &'static [&'static str] {
    match class {
        "assatur" => &["assatur_defeated", "assatur_killed"],
        "cthulhu" => &["bossdead"],
        _ => &[],
    }
}

/// Triggers a named class executes when its fight starts (intro states).
fn stand_in_start_triggers(class: &str) -> &'static [&'static str] {
    let _ = class;
    &[]
}

/// Character template wrapped by a named class (its Initialize).
fn named_template(class: &str) -> Option<&'static str> {
    let _ = class;
    None
}

impl<C2: SceneContent> SceneWorld<C2> {
    /// BossStatusEffected presentation: the class's BloodEffect at the
    /// attack position (direction Right) and the damage numbers at its
    /// NotifierTextPostion.
    pub(super) fn present_boss_event(
        &mut self,
        event: magicka_sim::boss_status::BossEvent,
        notifier: [f32; 3],
        blood: &'static str,
    ) {
        let mut number = self.boss_number.take();
        self.present_boss_event_number(event, notifier, blood, &mut number);
        self.boss_number = number;
    }

    /// [`Self::present_boss_event`] with the locked number held elsewhere
    /// (one per damageable, e.g. Cthulhu's tentacles).
    pub(super) fn present_boss_event_number(
        &mut self,
        event: magicka_sim::boss_status::BossEvent,
        notifier: [f32; 3],
        blood: &'static str,
        number: &mut Option<i32>,
    ) {
        use magicka_sim::boss_status::BossEvent;
        match event {
            BossEvent::Blood { position } => {
                self.status_contact_effects
                    .push((blood, position, [1.0, 0.0, 0.0]))
            }
            BossEvent::LockedNumber { amount } => {
                *number = Some(self.floating_text.add(amount, notifier, 0.4, true));
            }
            BossEvent::AddToNumber { amount } => {
                if let Some(i) = *number {
                    self.floating_text.add_to(i, amount);
                }
            }
            BossEvent::MoveNumber => {
                if let Some(i) = *number {
                    self.floating_text.update_position(i, notifier);
                }
            }
            BossEvent::ReleaseNumber => {
                if let Some(i) = number.take() {
                    self.floating_text.release(i);
                }
            }
            BossEvent::Number { amount } => {
                self.floating_text.add(amount, notifier, 0.7, false);
            }
        }
    }

    fn deinitialize_vlad(&mut self, handle: u16, spirit_effect: bool) {
        let position = self
            .npcs
            .get(&handle)
            .map(|n| self.physics.bodies[n.body].position);
        if let Some(n) = self.npcs.get_mut(&handle) {
            n.character.combat.cannot_die_without_explicit_kill = false;
        }
        if let Ok(e) = self.entity_mut(handle) {
            e.cannot_die_without_explicit_kill = false;
            e.hit_points = 0.;
        }
        if spirit_effect {
            self.play_cue(16, "spell_arcane_area", None);
            if let Some(position) = position {
                self.mine_effects
                    .push(("special_fairy_crash", position, [1., 0., 0.]));
            }
        }
        let _ = EntityCommandHost::kill(self, handle);
        let _ = EntityCommandHost::terminate(self, handle);
    }

    fn vlad_taunt(&mut self, handle: u16, dialog: &str) {
        // Native StartDialog accepts an Entity, not only a scripted unique ID.
        // Swamp Vlad's ID is zero; keep a stable private alias to that handle.
        let owner = if let Some((&owner, _)) = self
            .dialog_handle_owners
            .iter()
            .find(|(_, h)| **h == handle)
        {
            owner
        } else {
            let mut owner = i32::MIN + i32::from(handle);
            while self.dialog_handle_owners.contains_key(&owner)
                || self.entities.iter().any(|e| e.unique_id == owner)
            {
                owner += 1;
            }
            self.dialog_handle_owners.insert(owner, handle);
            owner
        };
        let Some(mut manager) = self.dialogs.take() else {
            return;
        };
        let Some(mut text) = self.dialog_text.take() else {
            self.dialogs = Some(manager);
            return;
        };
        self.refresh_dialog_world(&mut text);
        let result = manager.start(
            custom_hash(dialog),
            crate::interaction_playback::InteractionLocation::Entity(Some(owner)),
            None,
            self.flags.cutscene,
            &mut DialogBridge {
                host: &mut text,
                world: &*self,
            },
        );
        if let Err(error) = result {
            self.log.push(format!("Vlad taunt: {error:?}"));
        }
        self.dialogs = Some(manager);
        self.dialog_text = Some(text);
        self.apply_dialog_commands();
    }

    fn update_vlad_bosses(&mut self, dt: f32) {
        use crate::boss_vlad::{Kind, Phase};
        for index in 0..self.boss_fight.bosses.len() {
            let BossRef::Vlad(mut boss) = self.boss_fight.bosses[index].clone() else {
                continue;
            };
            if boss.phase == Phase::Intro && self.boss_fight.running {
                boss.phase = Phase::Battle;
                self.boss_fight.vlad_taunt_timers[boss.kind as usize] = 5. + self.random() * 5.;
                if let Some(npc) = self.npcs.get_mut(&boss.handle) {
                    npc.ai.set_order(crate::npc::ORDER_ATTACK, None);
                    npc.react_to = crate::npc::REACT_ATTACK | crate::npc::REACT_PROXIMITY;
                    npc.reaction_order = crate::npc::ORDER_ATTACK;
                }
            }
            if boss.phase == Phase::Battle {
                self.boss_fight.vlad_taunt_timers[boss.kind as usize] -= dt;
                if let Some(npc) = self.npcs.get_mut(&boss.handle) {
                    let hp = npc.character.combat.health.hit_points;
                    let fallen = self.physics.bodies[npc.body].position[1] < -5.;
                    if boss.kind != Kind::Swamp
                        && npc
                            .character
                            .combat
                            .statuses
                            .has(magicka_sim::status::Status::Frozen)
                    {
                        use magicka_sim::status::Status as S;
                        npc.character.stop_status_effects(
                            S::Wet as u16 | S::Frozen as u16 | S::Cold as u16,
                            false,
                        );
                    }
                    if hp <= 0. && boss.kind != Kind::Diplomat {
                        boss.phase = Phase::Outro;
                        if boss.kind == Kind::Spirit {
                            self.deinitialize_vlad(boss.handle, true);
                        }
                    } else if boss.kind == Kind::Swamp
                        && self.boss_fight.vlad_taunt_timers[boss.kind as usize] <= 0.
                    {
                        let choice = (self.random() * 4.) as usize;
                        self.vlad_taunt(
                            boss.handle,
                            ["vladbattle1", "vladbattle2", "vladbattle3", "vladbattle4"]
                                [choice.min(3)],
                        );
                        self.boss_fight.vlad_taunt_timers[boss.kind as usize] =
                            5. + self.random() * 5.;
                    }
                    if fallen && boss.kind != Kind::Swamp {
                        self.cast_magick_from(
                            boss.handle,
                            magicka_sim::spells::MagickType::Teleport,
                        );
                    }
                }
            }
            self.boss_fight.bosses[index] = BossRef::Vlad(boss);
        }
    }

    fn update_warlord_bosses(&mut self, dt: f32) {
        use crate::boss_warlord::Phase;
        for index in 0..self.boss_fight.bosses.len() {
            let BossRef::Warlord(mut boss) = self.boss_fight.bosses[index].clone() else {
                continue;
            };
            if self.role != NetworkRole::Client {
                if boss.phase == Phase::Intro && self.boss_fight.running {
                    boss.phase = Phase::Battle;
                    if let Some(n) = self.npcs.get_mut(&boss.handle) {
                        n.ai.set_order(crate::npc::ORDER_ATTACK, None);
                        n.react_to = crate::npc::REACT_ATTACK | crate::npc::REACT_PROXIMITY;
                        n.reaction_order = crate::npc::ORDER_ATTACK;
                    }
                }
                if self
                    .npcs
                    .get(&boss.handle)
                    .is_some_and(|n| n.character.combat.health.hit_points <= 0.)
                {
                    boss.phase = Phase::Death;
                    if let Some(n) = self.npcs.get_mut(&boss.handle) {
                        n.motion.special_idle = 8;
                    }
                }
                // Native's redundant state predicate restricts taunts to Server.
                if boss.phase == Phase::Battle && self.role == NetworkRole::Server {
                    boss.taunt_timer -= dt;
                    if boss.taunt_timer <= 0. {
                        boss.taunt_timer += 9.;
                        let choice = (self.random() * 3.) as usize;
                        self.vlad_taunt(
                            boss.handle,
                            ["warlordtaunt1", "warlordtaunt2", "warlordtaunt3"][choice.min(2)],
                        );
                    }
                }
            }
            self.boss_fight.bosses[index] = BossRef::Warlord(boss);
        }
    }

    /// A DialogAction through the world's DialogManager (script StartDialog
    /// and code-started dialogs such as boss taunts).
    pub(super) fn world_dialog_action(
        &mut self,
        action: &crate::dialog_action::DialogAction,
        sender: Option<u16>,
    ) -> Option<Result<(), WorldError>> {
        let mut manager = self.dialogs.take()?;
        let Some(mut text) = self.dialog_text.take() else {
            self.dialogs = Some(manager);
            return None;
        };
        self.refresh_dialog_world(&mut text);
        let result = {
            let mut bridge = DialogBridge {
                host: &mut text,
                world: &*self,
            };
            action
                .execute(sender, &mut manager, &mut bridge)
                .map(|_| ())
                .map_err(|e| format!("dialog action: {e:?}"))
        };
        self.dialogs = Some(manager);
        self.dialog_text = Some(text);
        self.apply_dialog_commands();
        Some(result)
    }

    pub(super) fn boss(&mut self, a: &BossAction, _quick: bool) -> Result<(), WorldError> {
        match &a.command {
            BossCommand::Spawn {
                boss,
                prop,
                generic,
                area,
                unique_id,
                mesh,
                on_death,
                on_damage,
                setup,
            } => {
                self.boss_setup(setup);
                let boss_ref = if *prop {
                    // PropBoss only assigns OnDamage when OnDeath is set.
                    let on_damage = if *on_death != 0 { *on_damage } else { 0 };
                    BossRef::Prop(
                        self.spawn_prop_boss(boss, *area, *unique_id, *on_death, on_damage)?,
                    )
                } else if *generic {
                    BossRef::Npc(self.spawn_template_boss(boss, *area, *unique_id, *mesh)?)
                } else if boss == "jormungandr" {
                    let orientation = self.scene_locator(*area)?;
                    let asset = self
                        .content
                        .skinned_model(crate::jormungandr::MODEL)
                        .ok_or("Jormungandr model unavailable")?;
                    let handle = self.allocate_physics(true);
                    let actor = crate::jormungandr::Jormungandr::new(
                        asset,
                        orientation,
                        u64::from(handle),
                    )?;
                    let e = self.entity_mut(handle)?;
                    e.added = true;
                    e.unique_id = *unique_id;
                    e.hit_points = 10000.;
                    e.max_hit_points = 10000.;
                    e.radius = 1.1;
                    self.jormungandr.insert(handle, actor);
                    BossRef::Jormungandr(handle)
                } else if boss == "warlord" {
                    let orientation = self.scene_locator(*area)?;
                    let origin = [orientation[12], orientation[13], orientation[14]];
                    let mut position = self
                        .collision
                        .collision()
                        .segment_intersect(
                            Segment {
                                origin: [origin[0], origin[1] + 1., origin[2]],
                                delta: [0., -6., 0.],
                            },
                            false,
                        )
                        .map_or(origin, |hit| hit.hit.position);
                    SpawnLevel::load_character_template(self, Some("boss_warlord"))?;
                    let handle = SpawnHost::acquire_npc(self)?.handle;
                    SpawnHost::initialize_npc(
                        self,
                        handle,
                        custom_hash("boss_warlord"),
                        0,
                        position,
                        custom_hash("#boss_n06"),
                    )?;
                    SpawnHost::add_entity(self, handle)?;
                    let n = self
                        .npcs
                        .get_mut(&handle)
                        .ok_or("Warlord requires a live template")?;
                    n.character.combat.absorb_shield = Some(Default::default());
                    n.character.combat.cannot_die_without_explicit_kill = true;
                    let body = &mut self.physics.bodies[n.body];
                    position[1] += crate::boss_warlord::FRESH_SPAWN_HEIGHT;
                    body.move_to(
                        position,
                        magicka_sim::character_physics::Orientation {
                            right: [orientation[0], orientation[1], orientation[2]],
                            up: [orientation[4], orientation[5], orientation[6]],
                            backward: [orientation[8], orientation[9], orientation[10]],
                        },
                    );
                    body.set_desired_direction([
                        -orientation[8],
                        -orientation[9],
                        -orientation[10],
                    ]);
                    let e = self.entity_mut(handle)?;
                    e.cannot_die_without_explicit_kill = true;
                    e.position = position;
                    BossRef::Warlord(crate::boss_warlord::WarlordBoss::new(handle))
                } else if let Some(kind) = crate::boss_vlad::Kind::from_class(boss) {
                    let orientation = self.scene_locator(*area)?;
                    let id = if kind == crate::boss_vlad::Kind::Spirit {
                        *unique_id
                    } else {
                        0
                    };
                    let collision = self.collision.collision();
                    let origin = [orientation[12], orientation[13], orientation[14]];
                    let mut position = collision
                        .segment_intersect(
                            Segment {
                                origin: [origin[0], origin[1] + 1., origin[2]],
                                delta: [0., -8., 0.],
                            },
                            false,
                        )
                        .map_or(origin, |hit| hit.hit.position);
                    // Named Initialize takes this one locator and performs its
                    // own ground ray; GenericBoss's nav placement is different.
                    crate::spawn_action::SpawnLevel::load_character_template(
                        self,
                        Some(kind.template()),
                    )?;
                    let handle = SpawnHost::acquire_npc(self)?.handle;
                    SpawnHost::initialize_npc(
                        self,
                        handle,
                        custom_hash(kind.template()),
                        0,
                        position,
                        id,
                    )?;
                    SpawnHost::add_entity(self, handle)?;
                    let npc = self
                        .npcs
                        .get_mut(&handle)
                        .ok_or("Vlad requires a live character template")?;
                    npc.vlad_character = true;
                    npc.equipment_visible = kind.equipment_visible();
                    npc.character.combat.cannot_die_without_explicit_kill = true;
                    npc.ai.set_order(0, None);
                    npc.react_to = 0;
                    npc.reaction_order = 0;
                    let body = &mut self.physics.bodies[npc.body];
                    position[1] += body.radius + body.length * 0.5 + 0.1;
                    body.move_to(
                        position,
                        magicka_sim::character_physics::Orientation {
                            right: [orientation[0], orientation[1], orientation[2]],
                            up: [orientation[4], orientation[5], orientation[6]],
                            backward: [orientation[8], orientation[9], orientation[10]],
                        },
                    );
                    body.set_desired_direction([
                        -orientation[8],
                        -orientation[9],
                        -orientation[10],
                    ]);
                    let entity = self.entity_mut(handle)?;
                    entity.cannot_die_without_explicit_kill = true;
                    entity.position = position;
                    BossRef::Vlad(crate::boss_vlad::VladBoss::new(handle, kind))
                } else if boss.eq_ignore_ascii_case("machine") {
                    BossRef::Machine(self.spawn_machine(*area)?)
                } else if boss.eq_ignore_ascii_case("fafnir") {
                    BossRef::Fafnir(Box::new(self.spawn_fafnir(*area)?))
                } else if boss.eq_ignore_ascii_case("death") {
                    BossRef::Death(Box::new(self.spawn_death_boss(*area)?))
                } else if boss.eq_ignore_ascii_case("grimnir2") {
                    BossRef::Grimnir2(Box::new(self.spawn_grimnir2(*area)?))
                } else if boss.eq_ignore_ascii_case("assatur") {
                    BossRef::Assatur(Box::new(self.spawn_assatur(*area)?))
                } else if boss.eq_ignore_ascii_case("grimnir") {
                    BossRef::Grimnir(Box::new(self.spawn_grimnir()?))
                } else if boss.eq_ignore_ascii_case("cult") {
                    BossRef::Cult(Box::new(self.spawn_cult()?))
                } else if boss.eq_ignore_ascii_case("cthulhu") {
                    BossRef::Cthulhu(Box::new(self.spawn_cthulhu(*area)?))
                } else if boss.eq_ignore_ascii_case("parker") {
                    BossRef::Parker(Box::new(self.spawn_parker(*area)?))
                } else if boss.eq_ignore_ascii_case("countrydefense") {
                    BossRef::CountryDefense(self.spawn_country_defense())
                } else if let Some(template) = named_template(boss) {
                    self.log.push(format!(
                        "boss {boss}: stand-in NPC {template} (class not ported)"
                    ));
                    BossRef::Npc(self.spawn_template_boss(template, *area, *unique_id, *mesh)?)
                } else {
                    self.log.push(format!(
                        "boss {boss}: stand-in, defeated {STAND_IN_SECONDS}s after the fight starts"
                    ));
                    BossRef::StandIn {
                        class: boss.clone(),
                        fight_time: 0.0,
                        dead: false,
                    }
                };
                self.boss_initialize(boss_ref);
                if !setup.delayed {
                    self.boss_start();
                }
            }
            BossCommand::SpawnGeneric { spawn, setup } => {
                self.boss_setup(setup);
                let handle = self.spawn_generic_boss(spawn)?;
                self.boss_initialize(BossRef::Npc(handle));
                if !setup.delayed {
                    self.boss_start();
                }
            }
            BossCommand::Activate => self.boss_start(),
            BossCommand::End => {
                // PlayState.BossFight?.Reset().
                if self.boss_fight.active {
                    let f = &mut self.boss_fight;
                    f.bosses.clear();
                    f.active = false;
                    f.normalized_health = 0.0;
                    f.dead = false;
                    f.name_set = false;
                    f.title.reset_timer();
                    f.hud = false;
                    f.bar = BossHealthBar::default();
                    f.bar_visible = false;
                }
            }
            BossCommand::Message(m) => {
                // PassMessage: only named classes react; stand-ins log it.
                let name = crate::boss_action::BOSS_MESSAGES[*m as usize].0;
                if self
                    .boss_fight
                    .bosses
                    .iter()
                    .any(|b| matches!(b, BossRef::StandIn { .. }))
                {
                    self.log.push(format!("boss message {name} to stand-in"));
                }
                let mut bosses = std::mem::take(&mut self.boss_fight.bosses);
                for b in &mut bosses {
                    match b {
                        BossRef::Machine(m) => m.machine.message(name),
                        BossRef::Fafnir(f) => self.fafnir_message(f, name),
                        BossRef::Grimnir2(g) => {
                            for c in g.actor.message(name) {
                                self.grimnir2_command(g, c);
                            }
                        }
                        BossRef::Assatur(a) => self.assatur_message(a, name),
                        BossRef::Cthulhu(c) => self.cthulhu_message(c, name),
                        _ => {}
                    }
                }
                self.boss_fight.bosses = bosses;
                if name == "VladMortal" {
                    let handles: Vec<_> = self
                        .boss_fight
                        .bosses
                        .iter()
                        .filter_map(|boss| match boss {
                            BossRef::Vlad(v) if v.kind != crate::boss_vlad::Kind::Spirit => {
                                Some(v.handle)
                            }
                            _ => None,
                        })
                        .collect();
                    for handle in handles {
                        self.deinitialize_vlad(handle, false);
                    }
                }
            }
            BossCommand::Title {
                title,
                subtitle,
                display_time,
                fade_in,
                fade_out,
            } => {
                // BossTitle.Execute then BossFight.SetTitles.
                if !title.is_empty() || !subtitle.is_empty() {
                    let display_time = if *display_time < 0.0 {
                        2.0
                    } else {
                        *display_time
                    };
                    let f = &mut self.boss_fight;
                    f.title
                        .set_titles(title, subtitle, display_time, *fade_in, *fade_out);
                    f.title.start();
                    f.name_set = true;
                }
            }
            BossCommand::HealthBar(c) => self.generic_health_bar.configure(c),
            BossCommand::ActivateHealthBar => self.generic_health_bar.activate(),
            BossCommand::RemoveHealthBar => self.generic_health_bar.remove(),
        }
        Ok(())
    }

    /// BossFight.Setup when not yet set up.
    fn boss_setup(&mut self, s: &BossSetup) {
        let f = &mut self.boss_fight;
        if !f.setup {
            f.setup = true;
            f.freeze_time = s.freeze_time;
            f.health_appear_delay = 0.0;
            f.health_bar_width = s.health_bar_width;
            f.title.reset_timer();
            f.bar = BossHealthBar::default();
        }
    }

    /// BossFight.Initialize: add the boss, attach the fight, not running.
    fn boss_initialize(&mut self, boss: BossRef) {
        if !self.boss_fight.bosses.contains(&boss) {
            self.boss_fight.bosses.push(boss);
        }
        self.boss_fight.active = true;
        self.boss_fight.dead = false;
        let (hp, max) = self.boss_health();
        self.boss_fight.normalized_health = if max > 0.0 {
            (hp / max).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.boss_fight.running = false;
        self.boss_fight.health_appear_delay = 0.0;
    }

    /// BossFight.Start.
    fn boss_start(&mut self) {
        self.boss_fight.running = true;
        self.boss_fight.hud = true;
        let classes: Vec<String> = self
            .boss_fight
            .bosses
            .iter()
            .filter_map(|b| match b {
                BossRef::StandIn { class, .. } => Some(class.clone()),
                _ => None,
            })
            .collect();
        for class in classes {
            for t in stand_in_start_triggers(&class) {
                self.pending_triggers.push((custom_hash(t), None));
            }
        }
    }

    /// Summed (HitPoints clamped at 0, MaxHitPoints) of the bosses.
    fn boss_health(&self) -> (f32, f32) {
        let mut hp = 0.0;
        let mut max = 0.0;
        for b in &self.boss_fight.bosses {
            match b {
                BossRef::Npc(h) | BossRef::Prop(h) => {
                    if let Ok(e) = self.entity(*h) {
                        hp += e.hit_points.max(0.0);
                        max += e.max_hit_points;
                    }
                }
                BossRef::Vlad(v) => {
                    if let Some(npc) = self.npcs.get(&v.handle) {
                        hp += npc.character.combat.health.hit_points.max(0.);
                        max += npc.definition.max_hit_points;
                    } else if let Ok(e) = self.entity(v.handle) {
                        hp += e.hit_points.max(0.);
                        max += e.max_hit_points;
                    }
                }
                BossRef::Warlord(w) => {
                    if let Some(n) = self.npcs.get(&w.handle) {
                        hp += n.character.combat.health.hit_points.max(0.);
                        max += n.definition.max_hit_points;
                    }
                }
                BossRef::StandIn { dead, .. } => {
                    hp += if *dead { 0.0 } else { 1.0 };
                    max += 1.0;
                }
                BossRef::Jormungandr(h) => {
                    if let Some(b) = self.jormungandr.get(h) {
                        hp += b.health.hp.max(0.);
                        max += b.health.max;
                    }
                }
                BossRef::Machine(m) => {
                    let (h, m) = self.machine_health(m);
                    hp += h.max(0.0);
                    max += m;
                }
                BossRef::Fafnir(f) => {
                    let (h, m) = f.fafnir.health();
                    hp += h.max(0.0);
                    max += m;
                }
                BossRef::Death(d) => {
                    hp += d.actor.lock().unwrap().combat.hit_points().max(0.);
                    max += magicka_sim::boss_death::MAX_HIT_POINTS;
                }
                BossRef::Grimnir2(g) => {
                    hp += g.actor.status.hit_points.max(0.);
                    max += crate::boss_grimnir2::MAX_HP;
                }
                BossRef::Assatur(a) => {
                    let (h, m) = a.actor.health();
                    hp += h.max(0.);
                    max += m;
                }
                BossRef::Grimnir(g) => {
                    hp += g.dreams.health.max(0.);
                    max += 6.;
                }
                BossRef::Cult(c) => {
                    let (h, m) = self.cult_health(c);
                    hp += h.max(0.0);
                    max += m;
                }
                BossRef::CountryDefense(c) => {
                    let (h, m) = self.country_defense_health(c);
                    hp += h.max(0.0);
                    max += m;
                }
                BossRef::Cthulhu(c) => {
                    let (h, m) = c.cthulhu.health();
                    hp += h.max(0.0);
                    max += m;
                }
                BossRef::Parker(p) => {
                    let (h, m) = p.parker.health();
                    hp += h.max(0.0);
                    max += m;
                }
            }
        }
        (hp, max)
    }

    fn boss_dead(&self, b: &BossRef) -> bool {
        match b {
            BossRef::Vlad(v) => v.dead(),
            BossRef::Warlord(w) => self.npcs.get(&w.handle).is_none_or(|n| n.dead()),
            BossRef::Jormungandr(h) => self.jormungandr.get(h).is_none_or(|b| b.dead),
            BossRef::Npc(h) | BossRef::Prop(h) => self.entity(*h).map_or(true, |e| e.dead),
            BossRef::StandIn { dead, .. } => *dead,
            // Parker.Dead: set when the die clip ends.
            BossRef::Parker(p) => p.parker.dead,
            // Cthulhu.Dead: set when the die clip ends.
            BossRef::Cthulhu(c) => c.cthulhu.dead,
            // Machine.Dead (mDead) is never set by the class.
            // Cult.Dead (mDead) is never set either.
            BossRef::Machine(_)
            | BossRef::CountryDefense(_)
            | BossRef::Cult(_)
            | BossRef::Fafnir(_) => false,
            // Assatur.Dead (mDead) is never set by the class either.
            BossRef::Death(_)
            | BossRef::Grimnir(_)
            | BossRef::Grimnir2(_)
            | BossRef::Assatur(_) => false,
        }
    }

    /// BossFight.Update and GenericHealthBar.Update.
    pub(super) fn update_boss_fight(&mut self, dt: f32) {
        if let Some(t) = self.generic_health_bar.update(dt) {
            self.pending_triggers.push((t, None));
        }
        self.boss_fight.bar_visible = false;
        self.boss_fight.shown_title = None;
        if !self.boss_fight.active {
            return;
        }
        self.update_jormungandr(dt);
        self.update_vlad_bosses(dt);
        self.update_warlord_bosses(dt);
        let f = &mut self.boss_fight;
        if f.name_set {
            match f.title.update(dt) {
                Some(alpha) => {
                    f.shown_title = Some(crate::title_renderer::ShownTitle::of(
                        &f.title,
                        alpha,
                        crate::title_renderer::TitleKind::Boss,
                    ))
                }
                None => f.name_set = false,
            }
        }
        // Named classes: UpdateBoss(iFightStarted = running) every frame.
        let running = self.boss_fight.running;
        let mut bosses = std::mem::take(&mut self.boss_fight.bosses);
        for b in &mut bosses {
            match b {
                BossRef::Machine(m) => self.update_machine(m, dt, running),
                BossRef::CountryDefense(c) => self.update_country_defense(c, dt),
                BossRef::Cult(c) => self.update_cult(c, dt, running),
                BossRef::Fafnir(f) => self.update_fafnir(f, dt, running),
                BossRef::Death(d) => self.update_death_boss(d, dt, running),
                BossRef::Grimnir(g) => self.update_grimnir(g, dt),
                BossRef::Grimnir2(g) => self.update_grimnir2(g, dt, running),
                BossRef::Assatur(a) => self.update_assatur(a, dt, running),
                // Parker.UpdateBoss ignores iFightStarted.
                BossRef::Parker(p) => self.update_parker(p, dt),
                // Cthulhu.UpdateBoss ignores iFightStarted.
                BossRef::Cthulhu(c) => self.update_cthulhu(c, dt),
                _ => {}
            }
        }
        self.boss_fight.bosses = bosses;
        self.update_boss_spells(dt);
        if self.boss_fight.running {
            // Stand-ins (UpdateBoss with the fight started).
            let mut fired = Vec::new();
            for b in &mut self.boss_fight.bosses {
                if let BossRef::StandIn {
                    class,
                    fight_time,
                    dead,
                } = b
                {
                    *fight_time += dt;
                    if !*dead && *fight_time >= STAND_IN_SECONDS {
                        *dead = true;
                        fired.extend(stand_in_death_triggers(class).iter().copied());
                    }
                }
            }
            for t in fired {
                self.pending_triggers.push((custom_hash(t), None));
            }
            let was_dead = self.boss_fight.dead;
            let all_dead = self.boss_fight.bosses.iter().all(|b| self.boss_dead(b));
            let (hp, max) = self.boss_health();
            let f = &mut self.boss_fight;
            f.dead = all_dead && !f.bosses.is_empty();
            f.normalized_health = if max > 0.0 {
                (hp / max).clamp(0.0, 1.0)
            } else {
                0.0
            };
            if !f.bar.destroy && hp <= 0.0 {
                f.bar.destroy = true;
            }
            f.bar.normalized_health = f.normalized_health;
            f.health_appear_delay -= dt;
            if f.health_appear_delay <= 0.0 {
                f.bar.update(dt);
                f.bar_visible = true;
            }
            if hp <= 0.0 && !was_dead && f.dead {
                f.hud = false;
            }
        } else {
            self.boss_fight.normalized_health = 0.0001;
            self.boss_fight.bar.normalized_health = 0.0001;
        }
    }

    /// GenericBoss.Initialize through SpawnNPC's path: the template at the
    /// area locator facing its Forward.
    fn spawn_template_boss(
        &mut self,
        template: &str,
        area: i32,
        unique_id: i32,
        mesh: i32,
    ) -> Result<u16, WorldError> {
        let node = crate::scene_script::XmlElement::synthetic(
            "spawn",
            vec![
                ("type".into(), template.into()),
                ("meshid".into(), mesh.to_string()),
            ],
        );
        let mut spawn = crate::spawn_action::SpawnAction::read(&node)
            .map_err(|e| format!("boss {template}: {e:?}"))?;
        // The locator and unique ID are already hashed.
        spawn.area = area;
        spawn.unique_id = unique_id;
        self.spawn_generic_boss(&spawn)
    }

    pub(super) fn spawn_generic_boss(
        &mut self,
        spawn: &crate::spawn_action::SpawnAction,
    ) -> Result<u16, WorldError> {
        let mut state = crate::spawn_action::SpawnState::default();
        spawn
            .initialize(&mut state, self)
            .map_err(|e| format!("boss spawn initialize: {e:?}"))?;
        spawn
            .execute(&mut state, self)
            .map_err(|e| format!("boss spawn: {e:?}"))?;
        state
            .spawned
            .first()
            .and_then(|s| s.handle)
            .ok_or_else(|| "boss spawn produced no NPC".into())
    }

    /// PropBoss.Initialize through SpawnPhys's path.
    fn spawn_prop_boss(
        &mut self,
        object: &str,
        area: i32,
        unique_id: i32,
        on_death: i32,
        on_damage: i32,
    ) -> Result<u16, WorldError> {
        let action = crate::spawn_phys::SpawnPhysAction {
            delay: 0.0,
            area,
            object: Some(object.to_string()),
            on_death,
            on_damage,
            unique_id,
            anim: None,
        };
        let mut state = crate::spawn_phys::SpawnPhysState::default();
        action.initialize(&mut state, self)?;
        action
            .execute(&mut state, self)
            .map_err(|e| format!("prop boss {object}: {e:?}"))?;
        state
            .entity
            .map(|e| e.handle)
            .ok_or_else(|| "prop boss produced no entity".into())
    }
}
