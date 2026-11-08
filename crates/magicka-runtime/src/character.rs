//! Assembly of recovered character combat phases; not the entire entity Update.
use crate::CharacterDefinition;
use magicka_sim::{
    character_health::{StatusFrame, StatusUpdateContext, StatusUpdateEvent},
    character_impact::ImpactCharacter,
    damage_update::{DamageBookkeeping, DamageUpdateContext, DamageUpdateEvent},
};

/// One Character.Damage → StatisticsManager.AddDamageEvent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatisticsDamage {
    pub attacker: Option<u16>,
    pub amount: f32,
    pub magnitude: f32,
    pub result: u16,
}

#[derive(Clone, Debug)]
pub struct RuntimeCharacter {
    /// Character.SetStaticTransform's uniform size factor, relative to template.
    pub model_scale: f32,
    /// Character.ScaleGraphicModel (drawn size only; Cthulhu's devour).
    pub graphic_scale: f32,
    /// ApplyTemplate's mModelID (index into the template's models).
    pub model: usize,
    /// Character.Color, i.e. mMaterial.TintColor: the chosen model's tint
    /// until a script overrides it. Avatars draw their player colour instead.
    pub tint: [f32; 3],
    pub definition: CharacterDefinition,
    pub spell_power: f32,
    pub additional_push_radius: f32,
    pub combat: ImpactCharacter,
    pub status_frame: StatusFrame,
    /// Rewrite extension: mod modifier movement factor (1: none), applied
    /// after native status speed each frame.
    pub mod_speed: f32,
    pub damage_bookkeeping: DamageBookkeeping,
    pub normalized_hit_points: f32,
    /// Character.mAuras applying resistance buffs to this character
    /// (self-casted Shield auras), in AddAura order.
    pub auras: Vec<ActiveAura>,
    pub nonvisual_auras: Vec<crate::buffs::NonvisualAura>,
    /// Special-ability auras (AddAura with a general AuraStorage).
    pub generic_auras: Vec<crate::generic_aura::GenericAura>,
    /// Native mBubble flag. RemoveBubbleShield calls ClearAura without
    /// resetting this flag (summon-cross.md).
    pub bubble: bool,
    /// Character.mBuffs: the resistance buffs damage reads (aura copies).
    pub buffs: crate::buffs::Buffs,
    pub halo: crate::buffs::Halo,
    /// This frame's HaloAuraRenderData tint (None: not drawn).
    pub halo_draw: Option<[f32; 4]>,
    /// SelfShield events awaiting the owner's body (mass, effects, auras).
    pub shield_events: Vec<magicka_sim::self_shield::ShieldEvent>,
    pub blood_decals: Vec<crate::decals::BloodDamage>,
    /// Character.BlockItem: reset every Character.Update, set by the Block
    /// animation action while it runs.
    pub block_item: i32,
    /// Grip links (research/npc-grip.md).
    pub grip: crate::grip::GripLinks,
    /// Character.mEntaglement (web wrapping; research/boss-parker.md).
    pub entanglement: magicka_sim::entanglement::Entanglement,
    /// Entanglement presentation awaiting the scene (effects, cues).
    pub entangle_events: Vec<magicka_sim::entanglement::EntangleEvent>,
    /// The Entangle_web visibility drawn this frame.
    pub entangle_visibility: Option<f32>,
    /// "melee_block" effects of blocked hits: (attack position, direction).
    pub block_effects: Vec<([f32; 3], [f32; 3])>,
    /// FloatingTextManager requests (direct damage, UpdateDamage numbers).
    pub damage_numbers: Vec<crate::floating_text::NumberRequest>,
    /// StatisticsManager.AddDamageEvent requests from Character.Damage.
    pub statistics_events: Vec<StatisticsDamage>,
    /// Character.mHitList, shared by its collision callbacks.
    pub collision_hits: magicka_sim::spray_pulse::HitList,
    pub status_cues: magicka_sim::character_status::CueState,
    pub status_cues_stopping: u16,
    pub pending_status_audio: Vec<magicka_sim::character_status::StatusEvent>,
    /// Animation PlaySound one-shots, drained by the live scene after motion.
    pub pending_animation_audio: Vec<(u16, i32)>,
    /// Template attached sounds, footstep and gore hit cues (research/audio-xact.md).
    pub sounds: crate::character_sounds::CharacterSounds,
    /// Owner's current spell pointer presence (including inactive effects).
    pub status_current_spell: bool,
    pub immortal_time: f32,
    pub collision_ignore_time: f32,
    /// Item.BlockValue by equipment index, preserving native sum order.
    pub block_values: std::collections::BTreeMap<i32, i32>,
    /// Item.ExecuteSpecialAbility preserves invisibility for one Attack entry.
    pub just_cast_invisible: bool,
    /// HasPassiveAbility(WetLightning): an attached item has it.
    pub wet_lightning_passive: bool,
    /// Character.mUndying: the template's Undying, or a live Undying buff.
    pub undying: bool,
    /// Character.mUndieTimer (None: NaN). Die() arms it while Undying.
    pub undie_timer: Option<f32>,
    /// Avatar's virtual IsImmortal inputs, refreshed by the live owner.
    pub(crate) avatar_in_event: bool,
    pub(crate) avatar_in_cutscene: bool,
    /// Character.mFearTimer/mFearedBy/mFearPosition (research/fear.md).
    pub fear: magicka_sim::fear::Fear,
    /// This frame's resolved FearPosition while feared and not entangled
    /// (the world refreshes it before the state update).
    pub fear_source: Option<[f32; 3]>,
    /// Character.mCharmOwner/mCharmTimer (research/charm.md).
    pub charm: Option<Charm>,
    /// Character.mHypnotized/mHypnotizeDirection (Cthulhu's mesmerize):
    /// the flat direction the character is walked in.
    pub hypnotized: Option<[f32; 3]>,
    /// Character.mEthereal: the template's IsEthereal at Initialize, cleared
    /// by Corporealize (research/corporealize.md).
    pub ethereal: bool,
    pub ethereal_alpha: f32,
    /// Native Character.TimeWarpModifier (afterimage threshold, not update dt).
    pub time_warp_modifier: f32,
    ethereal_alpha_target: f32,
    ethereal_alpha_speed: f32,
    /// Character.mEtherealTimer (TimedEthereal). Native never assigns
    /// mEtherealTimedState, so expiry always ends ethereality (PROVEN).
    pub ethereal_timer: f32,
    /// Character.mLevitateTimer/mLevitationFreeFallTimer (research/levitate.md).
    pub levitation: magicka_sim::magicks::Levitation,
}

/// Character.Charm state: the charmer and the remaining time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Charm {
    pub owner: u16,
    pub timer: f32,
}

/// An aura buff on this character: its resistance and remaining TTL.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActiveAura {
    pub resistance: magicka_sim::resistance::Resistance,
    pub ttl: f32,
    /// ActiveAura.StartTTL: the TTL at AddAura, the aura HUD bar's full width.
    pub start_ttl: f32,
    pub self_casted: bool,
    /// AuraTarget.Self bypasses the area query and ethereal target filter.
    pub self_only: bool,
    /// Radius, colour, buff UniqueID, period and decal ring.
    pub visual: crate::buffs::AuraVisual,
}
#[derive(Debug, PartialEq)]
pub enum CombatTickEvent {
    Damage(DamageUpdateEvent),
    Status(StatusUpdateEvent),
}
impl RuntimeCharacter {
    /// Character.Ethereal / IsEthereal setter: becoming ethereal stops the
    /// Burning, Wet, Frozen, Cold, Poisoned, Healing, Greased and Steamed
    /// statuses (with their stop cues); the alpha fade is presentation only.
    pub fn set_ethereal(&mut self, ethereal: bool) {
        self.ethereal(ethereal, 1.0, 1.0);
    }

    /// Ethereal animation action: a zero speed sets current and target alpha
    /// without changing the gameplay flag (native EtherealAlpha setter).
    pub fn execute_ethereal_action(&mut self, ethereal: bool, alpha: f32, speed: f32) {
        if speed == 0.0 {
            self.ethereal_alpha = alpha;
            self.ethereal_alpha_target = alpha;
        } else {
            self.ethereal(ethereal, alpha, speed);
        }
    }

    /// Character.AddBuff's Resistance branch for a new general buff: the
    /// Water/Cold/Fire/Life/Steam/Poison statuses scale by its multiplier.
    pub fn apply_resistance_buff_to_statuses(&mut self, kind: &crate::generic_aura::BuffKind) {
        use magicka_sim::spells::Elements as E;
        use magicka_sim::status::Status as S;
        let crate::generic_aura::BuffKind::Resistance(r) = kind else {
            return;
        };
        for (element, status) in [
            (E::WATER, S::Wet),
            (E::COLD, S::Cold),
            (E::FIRE, S::Burning),
            (E::LIFE, S::Healing),
            (E::STEAM, S::Steamed),
            (E::POISON, S::Poisoned),
        ] {
            if r.against.0 & element.0 != 0 {
                self.combat.statuses.scale_magnitude(status, r.multiplier);
            }
        }
    }

    /// Character.AddSelfShield: self-casted auras expire; without Earth or
    /// Ice an active pure Shield toggles off, otherwise it is replaced.
    pub fn add_self_shield(&mut self, spell: &magicka_sim::spells::Spell) {
        use magicka_sim::character_health::{SelfShield, SelfShieldType};
        use magicka_sim::spells::Elements as E;
        let mass = self.definition.mass;
        for aura in &mut self.auras {
            if aura.self_casted {
                aura.ttl = 0.0;
            }
        }
        let events = &mut self.shield_events;
        let shield = &mut self.combat.health.shield;
        if spell.get(E::EARTH) == 0.0 && spell.get(E::ICE) == 0.0 {
            let create = !(shield.active() && shield.kind == SelfShieldType::Shield);
            if shield.active() {
                shield.remove(mass, false, events);
            }
            if create {
                *shield = SelfShield::from_spell(spell, events);
            }
        } else {
            *shield = SelfShield::from_spell(spell, events);
        }
    }

    /// Character.AddGenericSelfShield: self-casted auras expire and a new
    /// SelfShield replaces any current one, then TimeSensitive is assigned
    /// (false resets health and capacity to 250).
    pub fn add_generic_self_shield(
        &mut self,
        spell: &magicka_sim::spells::Spell,
        time_sensitive: bool,
    ) {
        use magicka_sim::character_health::SelfShield;
        for aura in &mut self.auras {
            if aura.self_casted {
                aura.ttl = 0.0;
            }
        }
        let mut shield = SelfShield::from_spell(spell, &mut self.shield_events);
        shield.set_time_sensitive(time_sensitive);
        self.combat.health.shield = shield;
    }

    /// Character.Die's undie branch: Undying and not drowning arms the
    /// timer with UndieTime (0.5 when below epsilon), otherwise NaN.
    pub fn arm_undie(&mut self, drowning: bool) {
        self.undie_timer = (self.undying && !drowning).then(|| {
            let time = self.definition.undie_time;
            if time < f32::EPSILON {
                0.5
            } else {
                time
            }
        });
    }

    /// Character.Update while mDead: an armed undie timer (not overkilled)
    /// counts down, then the character is alive with UndieHitPoints.
    pub fn tick_undie(&mut self, dt: f32) {
        if !self.combat.dead || self.combat.overkilled {
            return;
        }
        if let Some(timer) = self.undie_timer.as_mut() {
            *timer -= dt;
            if *timer <= 0.0 {
                self.combat.dead = false;
                self.combat.health.hit_points = self.definition.undie_hit_points;
            }
        }
    }

    /// Native Character.Ethereal(state, alpha, speed).
    pub fn ethereal(&mut self, ethereal: bool, alpha: f32, speed: f32) {
        self.ethereal_alpha_target = alpha;
        self.ethereal_alpha_speed = speed;
        use magicka_sim::status::Status as S;
        self.ethereal = ethereal;
        if ethereal {
            const STATUSES: u16 = S::Burning as u16
                | S::Wet as u16
                | S::Frozen as u16
                | S::Cold as u16
                | S::Poisoned as u16
                | S::Healing as u16
                | S::Greased as u16
                | S::Steamed as u16;
            let mut events = Vec::new();
            self.combat.statuses.stop_effects(STATUSES, &mut events);
            self.pending_status_audio.extend(events);
        }
    }

    /// Character.AddBuff's extra NPC-only current-health change. Refreshes
    /// and rejected dead-state insertions leave current health unchanged.
    pub fn add_nonvisual_buff(
        &mut self,
        id: u32,
        kind: crate::buffs::NonvisualKind,
        ttl: f32,
        npc: bool,
    ) {
        if self.buffs.add_nonvisual(id, kind, ttl, self.combat.dead) && npc {
            if let crate::buffs::NonvisualKind::ModifyHitPoints { multiplier, .. } = kind {
                self.combat.health.hit_points *= multiplier;
            }
        }
    }

    /// AnimationAction.RemoveStatus -> Character.StopStatusEffects. The cached
    /// mask is intentionally rebuilt by the subsequent status update.
    pub fn stop_status_effects(&mut self, mask: u16, avatar: bool) {
        let mut events = Vec::new();
        self.combat.statuses.stop_effects(mask, &mut events);
        use magicka_sim::character_status::StatusEvent;
        self.retain_status_events(events.into_iter().flat_map(|event| {
            let hud = match event {
                StatusEvent::StopCue(status) if avatar => Some(StatusEvent::Hud {
                    status,
                    added: false,
                }),
                _ => None,
            };
            hud.into_iter().chain(std::iter::once(event))
        }));
    }

    /// Preserve AddStatusEffect notifications and their synchronous cue facts
    /// for subsequent damage entries before presentation drains requests.
    pub fn retain_status_events(
        &mut self,
        events: impl IntoIterator<Item = magicka_sim::character_status::StatusEvent>,
    ) {
        use magicka_sim::character_status::StatusEvent;
        for event in events {
            match event {
                StatusEvent::PlayStatusCue(kind) => {
                    let bit = kind as u16;
                    self.status_cues.exists |= bit;
                    self.status_cues.playing |= bit;
                    self.status_cues_stopping &= !bit;
                }
                StatusEvent::StopCue(kind) => {
                    let bit = kind as u16;
                    self.status_cues.playing &= !bit;
                    self.status_cues_stopping |= bit;
                }
                _ => {}
            }
            self.pending_status_audio.push(event);
        }
    }
    /// CharacterBody liquid contact -> Character.AddStatusEffect, preserving
    /// resistance, immunity and the round-versus-solid self-shield gate.
    pub fn liquid_status(
        &mut self,
        depth: f32,
        dt: f32,
        lava: bool,
        equipment: Option<&[magicka_sim::resistance::Resistance; 11]>,
        events: &mut Vec<magicka_sim::character_status::StatusEvent>,
    ) -> Result<
        magicka_sim::character_status::StatusResult,
        magicka_sim::character_status::StatusError,
    > {
        use magicka_sim::character_status::StatusResult;
        let shield = &self.combat.health.shield;
        if depth < magicka_sim::liquid_contact::STATUS_DEPTH || (shield.active() && !shield.solid())
        {
            return Ok(StatusResult::None);
        }
        self.add_status_effect(
            magicka_sim::liquid_contact::status(depth, dt, lava),
            equipment,
            events,
        )
    }

    /// Character.AddStatusEffect with the character's own status context
    /// (immortality, ethereal, resistances, buffs, cues).
    pub fn add_status_effect(
        &mut self,
        effect: magicka_sim::status::StatusEffect,
        equipment: Option<&[magicka_sim::resistance::Resistance; 11]>,
        events: &mut Vec<magicka_sim::character_status::StatusEvent>,
    ) -> Result<
        magicka_sim::character_status::StatusResult,
        magicka_sim::character_status::StatusError,
    > {
        use magicka_sim::character_status::StatusContext;
        let buffs = self.buffs.resistances();
        let ctx = StatusContext {
            immortal: self.is_immortal(),
            ethereal: self.ethereal,
            can_have_status: self.definition.can_have_status,
            avatar: equipment.is_some(),
            current_spell: self.status_current_spell,
            base: &self.definition.resistances,
            equipment,
            buffs: &buffs,
            cues: self.status_cues,
        };
        let start = events.len();
        let result = self.combat.statuses.add(effect, &ctx, events);
        self.retain_status_events(events[start..].iter().copied());
        result
    }

    /// ApplyTemplate's model selection: mModelID and the model's tint.
    pub fn set_model(&mut self, model: usize) {
        self.model = model;
        if let Some(&tint) = self.definition.model_tints.get(model) {
            self.tint = tint;
        }
    }

    pub fn new(definition: CharacterDefinition) -> Self {
        let combat = definition.combat_state();
        let status_frame = definition.status_frame(1.0);
        let sounds = crate::character_sounds::CharacterSounds::initialize(
            definition.attached_sounds.clone(),
        );
        Self {
            sounds,
            model_scale: 1.0,
            graphic_scale: 1.0,
            model: 0,
            tint: definition.model_tints.first().copied().unwrap_or([1.0; 3]),
            ethereal: definition.ethereal,
            ethereal_alpha: 1.0,
            time_warp_modifier: 1.0,
            ethereal_alpha_target: 1.0,
            ethereal_alpha_speed: 1.0,
            definition,
            spell_power: 0.,
            additional_push_radius: 0.,
            combat,
            status_frame,
            mod_speed: 1.0,
            damage_bookkeeping: DamageBookkeeping::default(),
            normalized_hit_points: 0.0,
            auras: Vec::new(),
            nonvisual_auras: Vec::new(),
            generic_auras: Vec::new(),
            bubble: false,
            buffs: Default::default(),
            halo: Default::default(),
            halo_draw: None,
            shield_events: Vec::new(),
            blood_decals: Vec::new(),
            block_item: -1,
            block_effects: Vec::new(),
            grip: Default::default(),
            entanglement: Default::default(),
            hypnotized: None,
            entangle_events: Vec::new(),
            entangle_visibility: None,
            damage_numbers: Vec::new(),
            statistics_events: Vec::new(),
            collision_hits: Default::default(),
            status_cues: Default::default(),
            status_cues_stopping: 0,
            pending_status_audio: Vec::new(),
            pending_animation_audio: Vec::new(),
            status_current_spell: false,
            immortal_time: 0.0,
            collision_ignore_time: 0.0,
            block_values: Default::default(),
            just_cast_invisible: false,
            wet_lightning_passive: false,
            undying: false,
            undie_timer: None,
            avatar_in_event: false,
            avatar_in_cutscene: false,
            fear: Default::default(),
            fear_source: None,
            charm: None,
            levitation: Default::default(),
            ethereal_timer: 0.0,
        }
    }

    /// Character.TimedEthereal(time, state).
    pub fn timed_ethereal(&mut self, time: f32, state: bool) {
        if time <= 0.0 {
            self.ethereal = state;
            self.ethereal_timer = 0.0;
        } else {
            self.ethereal_timer = time;
        }
    }
    pub fn scaled_static_transform(
        &self,
        template: &magicka_content::skinned::Mat,
    ) -> magicka_content::skinned::Mat {
        let mut scale = magicka_content::skinned::IDENTITY;
        scale[0] = self.model_scale;
        scale[5] = self.model_scale;
        scale[10] = self.model_scale;
        magicka_content::skinned::mul(&scale, template)
    }
    /// Character.Update after the state update: while IsStunned the timer
    /// runs down (the `stunned` effect follows in PlaySession).
    pub fn tick_stun(&mut self, dt: f32) {
        if self.is_stunned() {
            self.combat.stun_timer -= dt;
        }
        self.combat.tick_last_attacker(dt);
        if self.ethereal_timer > 0.0 {
            self.ethereal_timer -= dt;
            if self.ethereal_timer <= 0.0 {
                self.ethereal_alpha = 1.0;
                self.ethereal(false, 1.0, 1.0);
            }
        }
    }

    /// Character.IsStunned: StunTimer > 0 and not Dead.
    pub fn is_stunned(&self) -> bool {
        self.combat.stun_timer > 0.0 && !self.combat.dead
    }

    /// Character.Update decrements these before animation actions refresh them.
    /// Native timers are not clamped at zero; unordered NaN fails the >0 test.
    pub fn update_contact_timers(&mut self, dt: f32) {
        self.ethereal_alpha +=
            (self.ethereal_alpha_target - self.ethereal_alpha) * self.ethereal_alpha_speed * dt;
        self.immortal_time -= dt;
        self.collision_ignore_time -= dt;
    }
    /// Character.SetInvisible: positive duration also removes an active shield.
    /// Character.IsInvisibile (mInvisibilityTimer > 0).
    pub fn is_invisible(&self) -> bool {
        self.combat.health.invisibility_timer > 0.0
    }
    /// Character.Hypnotize: the flat direction becomes the body's Movement
    /// and is held (input is ignored) until StopHypnotize.
    pub fn hypnotize(
        &mut self,
        body: &mut magicka_sim::character_physics::CharacterBody,
        mut direction: [f32; 3],
    ) {
        direction[1] = 0.0;
        body.set_movement(direction);
        self.hypnotized = Some(direction);
    }
    /// Character.StopHypnotize (positive damage, death, termination).
    pub fn stop_hypnotize(&mut self) {
        self.hypnotized = None;
    }
    /// Character.IsCharmed (mCharmTimer > float.Epsilon).
    pub fn is_charmed(&self) -> bool {
        self.charm.is_some_and(|c| c.timer > f32::from_bits(1))
    }
    /// Character.Charm: ignored for IsUncharmable templates. Returns whether
    /// it applied (the caller restarts the charm effect and, for NPCs, pops
    /// the AI to idle).
    pub fn apply_charm(&mut self, owner: u16, ttl: f32) -> bool {
        if self.definition.uncharmable {
            return false;
        }
        self.charm = Some(Charm { owner, timer: ttl });
        true
    }
    /// Character.Update after the state machine: the charm timer; EndCharm
    /// when it runs out. Returns true when the charm ended.
    pub fn tick_charm(&mut self, dt: f32) -> bool {
        if !self.is_charmed() {
            return false;
        }
        let charm = self.charm.as_mut().unwrap();
        charm.timer -= dt;
        if charm.timer <= 0.0 {
            self.charm = None;
            return true;
        }
        false
    }

    pub fn set_invisible(&mut self, duration: f32) {
        self.combat.health.invisibility_timer = duration;
        if duration > 0.0 && self.combat.health.shield.active() {
            self.combat
                .health
                .shield
                .remove(self.definition.mass, false, &mut self.shield_events);
        }
    }
    pub fn enter_attack_visibility(&mut self) {
        if self.just_cast_invisible {
            self.just_cast_invisible = false;
        } else {
            self.set_invisible(0.0);
        }
    }
    pub fn execute_immortal_action(&mut self, collide: bool) {
        self.immortal_time = 3.0;
        if !collide {
            self.collision_ignore_time = 0.15;
        }
    }
    pub fn kill_immortal_action(&mut self) {
        self.immortal_time = 0.0;
    }
    pub fn is_immortal(&self) -> bool {
        self.avatar_in_event || self.avatar_in_cutscene || self.immortal_time > 0.0
    }
    /// Tail of Character.OnCollision for owned skins. Level/liquid acceptance
    /// precedes this filter; shield side effects precede it too.
    pub fn accepts_owned_contact(&self) -> bool {
        !self.ethereal && (self.collision_ignore_time <= 0.0 || self.collision_ignore_time.is_nan())
    }
    /// Character.Update from the zap timer through UpdateStatusEffects on a
    /// live body: while `mZapTimer >= 0` the SpeedMultiplier becomes the
    /// template ZapModifier (unless NaN) and the timer counts down; then
    /// StatusEffect.Update caps the speed (Cold, Poisoned).
    pub fn update_damage_and_status_on(
        &mut self,
        body: &mut magicka_sim::character_physics::CharacterBody,
        dt: f32,
        is_play_state: bool,
        ctx: StatusUpdateContext,
        events: &mut Vec<CombatTickEvent>,
    ) -> crate::character_motion::StatusMotion {
        use magicka_sim::status::Status;
        let mut zap = None;
        if self.combat.zap_timer >= 0.0 {
            let modifier = self.definition.zap_modifier;
            if !modifier.is_nan() {
                body.set_speed_multiplier(modifier);
                zap = Some(modifier);
            }
            self.combat.zap_timer -= dt;
        }
        self.status_frame.speed_multiplier = body.speed_multiplier();
        self.update_damage_and_status(dt, is_play_state, ctx, events);
        body.set_speed_multiplier(self.status_frame.speed_multiplier * self.mod_speed);
        if self.hypnotized.is_some() {
            // Character.Update: SpeedMultiplier *= 0.4 while hypnotized.
            body.set_speed_multiplier(body.speed_multiplier() * 0.4);
        }
        let statuses = &self.combat.statuses;
        let cold = statuses
            .has(Status::Cold)
            .then(|| statuses.effect(Status::Cold).map(|e| e.slowdown()))
            .flatten();
        let frozen = statuses
            .effect(Status::Frozen)
            .is_some_and(|e| e.magnitude > 0.0);
        crate::character_motion::StatusMotion { cold, frozen, zap }
    }

    /// Consecutive Character.UpdateDamage then UpdateStatusEffects calls.
    /// Movement, shield/state-machine updates happen earlier in the source and
    /// remain caller responsibilities. dt is already the caller's simulation dt.
    pub fn update_damage_and_status(
        &mut self,
        dt: f32,
        is_play_state: bool,
        mut ctx: StatusUpdateContext,
        events: &mut Vec<CombatTickEvent>,
    ) {
        self.collision_hits.update(dt);
        ctx.dead = self.combat.dead && !self.combat.cannot_die_without_explicit_kill;
        ctx.health.immortal |= self.is_immortal();
        ctx.health.entangled |= self.entanglement.entangled();
        ctx.cues_exist |= self.status_cues.exists;
        ctx.cues_stopping |= self.status_cues_stopping;
        let mut damage_events = vec![];
        self.damage_bookkeeping.update(
            &mut self.combat,
            dt,
            DamageUpdateContext {
                regeneration: self.definition.regeneration,
                is_play_state,
                in_cutscene: ctx.in_cutscene,
                dead: ctx.dead,
            },
            &mut damage_events,
        );
        self.damage_numbers.extend(
            damage_events
                .iter()
                .map(|e| crate::floating_text::NumberRequest::Update(*e)),
        );
        events.extend(damage_events.into_iter().map(CombatTickEvent::Damage));
        let mut status_events = vec![];
        self.combat.statuses.update(
            dt,
            &mut self.combat.health,
            &mut self.status_frame,
            ctx,
            &mut status_events,
        );
        self.pending_status_audio
            .extend(status_events.iter().filter_map(|e| {
                if let StatusUpdateEvent::StopCue(kind) = e {
                    Some(magicka_sim::character_status::StatusEvent::StopCue(*kind))
                } else {
                    None
                }
            }));
        for e in &status_events {
            match e {
                StatusUpdateEvent::Health(
                    magicka_sim::character_health::HealthEvent::DecreaseEntanglement {
                        amount,
                        element,
                    },
                ) => self
                    .entanglement
                    .decrease(*amount, *element, &mut self.entangle_events),
                StatusUpdateEvent::Health(
                    magicka_sim::character_health::HealthEvent::StopHypnotize,
                ) => self.stop_hypnotize(),
                _ => {}
            }
        }
        // DeadState.OnEnter: ReleaseEntanglement; Die: StopHypnotize.
        if self.combat.dead && self.entanglement.entangled() {
            self.entanglement.release(&mut self.entangle_events);
        }
        if self.combat.dead {
            self.stop_hypnotize();
        }
        self.entangle_visibility = self.entanglement.update(dt);
        events.extend(status_events.into_iter().map(CombatTickEvent::Status));
        self.normalized_hit_points = crate::hud::smooth_health(
            self.normalized_hit_points,
            self.combat.health.hit_points,
            self.combat.health.max_hit_points,
            dt,
        );
    }
}

impl RuntimeCharacter {
    /// Uses the caller's current world position and loaded collision radius.
    pub fn arc_intersect(
        &self,
        position: [f32; 3],
        query: magicka_sim::arc_intersection::ArcQuery,
    ) -> Option<[f32; 3]> {
        magicka_sim::arc_intersection::character_arc(
            position,
            self.definition.radius,
            self.ethereal,
            query,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use magicka_sim::{
        resistance::Resistance,
        status::{Status, StatusEffect},
    };
    #[test]
    fn zero_speed_ethereal_action_sets_alpha_without_changing_gameplay_state() {
        let mut c = runtime();
        c.ethereal(false, 0.1, 2.0);
        c.execute_ethereal_action(true, 0.6, 0.0);
        assert!(!c.ethereal);
        c.update_contact_timers(0.25);
        assert_eq!(c.ethereal_alpha, 0.6);
        c.execute_ethereal_action(true, 0.0, 2.0);
        assert!(c.ethereal);
        c.update_contact_timers(0.25);
        assert_eq!(c.ethereal_alpha, 0.3);
    }

    #[test]
    fn ethereal_alpha_uses_native_unclamped_update_and_preserves_template_look() {
        let mut c = runtime();
        c.definition.looks_ethereal = true;
        c.ethereal(true, 0.0, 2.0);
        c.update_contact_timers(0.25);
        assert_eq!(c.ethereal_alpha, 0.5);
        c.update_contact_timers(0.75);
        assert_eq!(
            c.ethereal_alpha, -0.25,
            "native update does not clamp or exponentiate"
        );
        c.set_ethereal(false);
        assert!(!c.ethereal);
        assert!(c.definition.looks_ethereal);
        c.update_contact_timers(0.5);
        assert_eq!(c.ethereal_alpha, 0.375);
    }

    fn runtime() -> RuntimeCharacter {
        RuntimeCharacter::new(CharacterDefinition {
            id: "fixture".into(),
            event_conditions: std::sync::Arc::new(Vec::new()),
            faction: 4,
            ethereal: false,
            looks_ethereal: false,
            can_have_status: true,
            resistances: [Resistance::IDENTITY; 11],
            max_hit_points: 100.0,
            health_bars: 1,
            hit_tolerance: 0.0,
            knockdown_tolerance: 1.0,
            max_panic: 0.0,
            fearless: false,
            uncharmable: false,
            nonslippery: false,
            length: 1.0,
            radius: 0.5,
            mass: 70.0,
            speed: 1.0,
            turn_speed: 1.0,
            regeneration: 10,
            bleed_rate: 0.0,
            stun_time: 0.0,
            gib_count: 0,
            gibs: Vec::new(),
            blood: 5,
            attached_sounds: Vec::new(),
            zap_modifier: 1.0,
            break_free_strength: 1.0,
            undying: false,
            undie_time: 0.0,
            undie_hit_points: 0.0,
            model_tints: Vec::new(),
            score_value: 0,
            display_id: String::new(),
        })
    }
    #[test]
    fn size_health_buff_changes_new_npc_health_but_not_player_or_refresh() {
        use crate::buffs::NonvisualKind;
        for npc in [false, true] {
            let mut c = runtime();
            c.combat.health.hit_points = 40.0;
            let grow = NonvisualKind::ModifyHitPoints {
                multiplier: 2.0,
                modifier: 0.0,
            };
            c.add_nonvisual_buff(12, grow, 0.5, npc);
            assert_eq!(c.combat.health.hit_points, if npc { 80.0 } else { 40.0 });
            c.add_nonvisual_buff(12, grow, 0.5, npc);
            assert_eq!(c.combat.health.hit_points, if npc { 80.0 } else { 40.0 });
            c.buffs.update_nonvisual(0.1, 100.0, &mut c.combat.health);
            assert_eq!(c.combat.health.max_hit_points, 200.0);
            c.combat.dead = true;
            c.add_nonvisual_buff(12, grow, 0.5, npc);
            assert_eq!(c.buffs.nonvisual[0].ttl, 0.5);
            c.add_nonvisual_buff(13, grow, 0.5, npc);
            assert_eq!(c.buffs.nonvisual.len(), 1);
        }
    }

    #[test]
    fn cached_burning_suppresses_regen_before_expiry_then_next_frame_recovers() {
        let mut c = runtime();
        c.combat.health.hit_points = 50.0;
        c.combat
            .statuses
            .set_effect(StatusEffect::new(Status::Burning, 0.0, 0.01, false))
            .unwrap();
        c.combat.statuses.current_mask = Status::Burning as u16;
        c.update_damage_and_status(0.1, true, StatusUpdateContext::default(), &mut vec![]);
        assert_eq!(c.combat.health.hit_points, 50.0);
        assert_eq!(c.combat.statuses.current_mask, 0);
        c.update_damage_and_status(0.1, true, StatusUpdateContext::default(), &mut vec![]);
        assert_eq!(c.combat.health.hit_points, 51.0);
    }
    #[test]
    fn status_damage_accumulates_after_number_flush_and_is_shown_next_update() {
        let mut c = runtime();
        c.definition.regeneration = 0;
        c.combat
            .statuses
            .set_effect(StatusEffect::new(Status::Bleeding, 20.0, 1.0, false))
            .unwrap();
        let mut events = vec![];
        c.update_damage_and_status(0.5, true, StatusUpdateContext::default(), &mut events);
        assert_eq!(c.combat.health.hit_points, 90.0);
        assert_eq!(c.combat.health.bleed_damage, 10.0);
        assert!(!events
            .iter()
            .any(|e| matches!(e, CombatTickEvent::Damage(DamageUpdateEvent::Number { .. }))));
        events.clear();
        c.update_damage_and_status(0.5, true, StatusUpdateContext::default(), &mut events);
        assert!(matches!(
            events[0],
            CombatTickEvent::Damage(DamageUpdateEvent::Number { amount: 10.0, .. })
        ));
        assert_eq!(c.combat.health.hit_points, 80.0);
        assert_eq!(c.combat.health.bleed_damage, 10.0);
    }
    use crate::self_cast::{SelfCastError, SelfCastEvent};
    use magicka_sim::{
        character_impact::ImpactContext,
        character_status::{CueState, StatusContext},
        damage_delivery::{AttackerKind, DeliveryContext, DeliveryEvent, NetworkRole},
        impact_motion::MotionContext,
        spells::{Elements as E, Spell},
    };
    fn ctx(base: &[Resistance; 11]) -> ImpactContext<'_> {
        ImpactContext {
            status: StatusContext {
                immortal: false,
                ethereal: false,
                can_have_status: true,
                avatar: false,
                current_spell: false,
                base,
                equipment: None,
                buffs: &[],
                cues: CueState::default(),
            },
            motion: MotionContext {
                mass: 70.0,
                gravity_y: -19.64,
                ..Default::default()
            },
            attack_position: [0.0, 0.0, -1.0],
            forward: [0.0, 0.0, -1.0],
            block_values: &[],
            block_item: -1,
            features: 15,
            network_client: false,
            attacker: Some(7),
            attacker_is_self: false,
            attacker_avatar: false,
            attacker_local_avatar: false,
            attacker_is_gripped: false,
            grip_attack_state: false,
            hit_tolerance: 5.0,
            resurrection_state: false,
            stun_time: 2.0,
            bleed_rate: 20.0,
            blood_and_gore: false,
            has_blood: false,
            bloat_killed: false,
            has_damage_trigger: false,
        }
    }
    fn delivery(network: NetworkRole) -> DeliveryContext {
        DeliveryContext {
            network,
            attacker_kind: AttackerKind::Character,
            target_handle: 7,
            attacker_handle: Some(99),
            time_stamp: 2.0,
            target_position: [1.0, 2.0, 3.0],
            attack_position: [9.0; 3],
        }
    }
    #[test]
    fn self_spray_adds_burning_and_runtime_tick_applies_damage() {
        let base = [Resistance::IDENTITY; 11];
        let mut c = runtime();
        c.definition.regeneration = 0;
        let mut events = vec![];
        c.cast_self_spray(
            Spell::default_for(E::FIRE),
            false,
            delivery(NetworkRole::Offline),
            &[],
            ctx(&base),
            &mut events,
        )
        .unwrap();
        assert!(c.combat.statuses.has(Status::Burning));
        assert!(matches!(
            events[0],
            SelfCastEvent::StartEffect {
                name: "self_fire",
                ..
            }
        ));
        let before = c.combat.health.hit_points;
        c.update_damage_and_status(0.5, true, StatusUpdateContext::default(), &mut vec![]);
        assert!(c.combat.health.hit_points < before);
    }
    #[test]
    fn self_water_client_request_has_self_identity_and_zero_relative_position() {
        let base = [Resistance::IDENTITY; 11];
        let mut c = runtime();
        let mut events = vec![];
        let mut d = delivery(NetworkRole::Client);
        d.attacker_kind = AttackerKind::LocalAvatar;
        let mut context = ctx(&base);
        context.status.avatar = true;
        context.status.equipment = Some(&base);
        c.cast_self_spray(
            Spell::default_for(E::WATER),
            true,
            d,
            &[],
            context,
            &mut events,
        )
        .unwrap();
        assert!(!c.combat.statuses.has(Status::Wet));
        let request = events
            .iter()
            .find_map(|e| match e {
                SelfCastEvent::Delivery(DeliveryEvent::Request(r)) => Some(r),
                _ => None,
            })
            .unwrap();
        assert_eq!(request.attacker_handle, Some(7));
        assert_eq!(request.target_handle, 7);
        assert_eq!(request.relative_attack_position, [0.0; 3]);
        assert_eq!(request.time_stamp, 2.0);
        assert_eq!(c.combat.health.hit_points, 100.0);
    }
    #[test]
    fn self_cast_uses_loaded_status_eligibility_and_rejects_other_effect_families() {
        let base = [Resistance::IDENTITY; 11];
        let mut c = runtime();
        let mut events = vec![];
        c.definition.can_have_status = false;
        c.cast_self_spray(
            Spell::default_for(E::FIRE),
            false,
            delivery(NetworkRole::Offline),
            &[],
            ctx(&base),
            &mut events,
        )
        .unwrap();
        assert!(!c.combat.statuses.has(Status::Burning));
        events.clear();
        assert!(matches!(
            c.cast_self_spray(
                Spell::default_for(E::EARTH),
                false,
                delivery(NetworkRole::Offline),
                &[],
                ctx(&base),
                &mut events
            ),
            Err(SelfCastError::NotSpray)
        ));
        assert!(events.is_empty());
    }
}
