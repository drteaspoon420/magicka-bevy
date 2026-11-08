//! Script-spawned NPCs: NonPlayerCharacter with a CharacterBody, the shared
//! Character Idle/Move motion, and an Agent driving AIStateIdle/AIStateMove
//! event playback, proximity reactions and avoidance. Scheduling follows
//! AIManager (at most 20 agents per frame, time accumulated in between).
//! Attack/Defend orders run Agent targeting, AIStateAttack and melee
//! (`npc_combat`, research/npc-combat.md). Wander and Panic orders run
//! AIStateWander/AIStatePanic (`avatar_ai`, `magicka_sim::agent_steering`);
//! the character-level PanicState panic/stumble walk, leaders/flocking and
//! the non-character danger sources are not wired yet. See
//! research/npc-agents.md.
use crate::{
    animation_controller::AnimationSet,
    avatar_ai::AvatarAi,
    character::RuntimeCharacter,
    character_motion::{
        ActorAi, ActorFrame, CharacterFacts, CharacterHost, CharacterMotion, CombatCtx,
    },
    npc_combat::{CombatAbility, CombatAgent, CombatView, MeleeWeapon, TargetWeights},
    play_content::CharacterRig,
    CharacterDefinition,
};
use magicka_sim::damage::CastType;
use magicka_sim::impact_motion::Vector;
use std::sync::Arc;

/// ReactTo flags (spawn_action::REACT_TO).
pub const REACT_ATTACK: u8 = 1;
pub const REACT_PROXIMITY: u8 = 2;
/// AIManager.STEPS.
pub const AGENTS_PER_FRAME: usize = 20;
/// Agent.Initialize: mOrder = Attack.
pub const ORDER_ATTACK: u8 = 2;

/// CharacterTemplate AI fields the reconstructed agent uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AgentWeights {
    pub alert_radius: f32,
    pub friendly_avoidance: f32,
    pub enemy_avoidance: f32,
    pub sight_avoidance: f32,
    pub danger_avoidance: f32,
    /// CharacterTemplate Group* (FollowGroup weights).
    pub group: magicka_sim::agent_steering::GroupWeights,
}

/// Everything a scene needs to give an NPC a body, animation and AI.
#[derive(Clone, Debug)]
pub struct NpcActorTemplate {
    pub definition: CharacterDefinition,
    pub animations: Arc<AnimationSet>,
    pub move_abilities: u8,
    pub weights: AgentWeights,
    /// CharacterTemplate.Abilities and the targeting weights.
    pub abilities: Arc<Vec<CombatAbility>>,
    pub target_weights: TargetWeights,
    /// Skeleton and equipment attachments (weapon attach points).
    pub rig: Option<CharacterRig>,
    /// Melee data of each equipped item: (equipment slot, weapon).
    pub weapons: Vec<(i32, MeleeWeapon)>,
    /// Equipped items with definition effects (Item.Update presentation).
    pub held_effects: Vec<HeldItemEffects>,
    pub summon_element_cue: Option<(u16, String)>,
}

/// An equipped item's definition effects and its world frame this update.
#[derive(Clone, Debug)]
pub struct HeldItemEffects {
    pub slot: i32,
    pub effects: Arc<Vec<String>>,
    /// Item.HideEffects.
    pub hide: bool,
    pub frame: Option<crate::item_effects::ItemFrame>,
}

/// An entity as GetAvoidance / proximity reaction see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Neighbour {
    pub handle: u16,
    pub position: Vector,
    pub direction: Vector,
    pub radius: f32,
    /// Characters have factions; other entities only contribute danger.
    pub faction: Option<i32>,
    pub danger: f32,
    /// Character.IsInvisibile.
    pub invisible: bool,
}

#[derive(Clone, Debug)]
pub struct NpcCharacter {
    pub special_requests: Vec<crate::item_special::SpecialInvocation>,
    /// Refreshed from live bodies before each animation update.
    pub jump_target: Option<magicka_sim::jump::JumpTarget>,
    pub handle: u16,
    pub body: usize,
    /// Specialized VladCharacter subclass, including its speed/OverKill rules.
    pub vlad_character: bool,
    /// Item.Visible for the two native equipment slots.
    pub equipment_visible: [bool; 2],
    pub definition: CharacterDefinition,
    /// Health, statuses and damage bookkeeping (Character combat state).
    pub character: RuntimeCharacter,
    /// Spawn OnDeath / OnDamage triggers.
    pub on_death: i32,
    pub on_damage: i32,
    rng: u64,
    pub motion: CharacterMotion,
    pub ai: AvatarAi,
    pub move_abilities: u8,
    pub weights: AgentWeights,
    pub react_to: u8,
    pub reaction_order: u8,
    pub reaction_trigger: i32,
    /// Agent.mTimeSinceLastUpdate.
    pub time_since_update: f32,
    /// AIStateMove group queries for the next agent update: the live
    /// same-type agents other than this one, and the facts of `ai.leader`
    /// when it still exists (filled by the scene, cleared after use).
    pub group_candidates: Vec<magicka_sim::agent_steering::GroupCandidate>,
    pub group_leader: Option<(u16, crate::avatar_ai::GroupLeader)>,
    /// Agent targets/abilities/aggro and the ability cooldowns.
    pub combat: CombatAgent,
    pub rig: Option<CharacterRig>,
    pub weapons: Vec<(i32, MeleeWeapon)>,
    pub held_effects: Vec<HeldItemEffects>,
    /// DealDamage executions of the last motion update: (slot, targets, first).
    pub melee_requests: Vec<crate::npc_combat::MeleeRequest>,
    /// SpawnMissile executions of the last motion update:
    /// (slot, action velocity, item aligned).
    pub missile_requests: Vec<(i32, Option<Vector>, bool)>,
    /// DetachItem executions of the last motion update: (slot, local
    /// velocity).
    pub detach_requests: Vec<(i32, Vector)>,
    /// Grip/DamageGrip/ReleaseGrip executions of the last motion update.
    pub grip_requests: Vec<crate::grip::GripRequest>,
    /// Character.Color (material TintColor) when code sets it, e.g. the
    /// Cult's per-element cultists; None keeps the model's own tint.
    pub tint: Option<[f32; 3]>,
    pub animation_events: Vec<crate::avatar::AvatarEvent>,
    pub spell_requests: Vec<crate::avatar::AvatarEvent>,
    pub current_spell: Option<crate::avatar::SpellEffect>,
    pub current_spell_definition: Option<(CastType, magicka_sim::spells::Spell)>,
    pub rail_kills: Vec<magicka_sim::railgun::RailId>,
    pub staff_orb: magicka_content::skinned::Mat,
    pub weapon_transform: magicka_content::skinned::Mat,
    /// Character.GetHipAttachOrientation.
    pub hip_transform: magicka_content::skinned::Mat,
    pub play_time: f64,
    pub summon_element_cue: Option<(u16, String)>,
    pub lifecycle_dispatched: u8,
    /// Charm/EndCharm or Confuse/its OnRemove happened: pop the AI to idle
    /// before its next update.
    pub charm_reset: bool,
    /// NonPlayerCharacter.mFaction when it differs from the template's:
    /// Confuse sets NONE (Some(0)) and its OnRemove restores the template
    /// (None, dropping a summon's faction as the original does); Summoned
    /// sets the master's faction.
    pub faction_override: Option<i32>,
    /// NonPlayerCharacter.Summoned: the master and the summon kind.
    pub summon: Option<(u16, magicka_sim::summons::SummonKind)>,
}

impl NpcCharacter {
    /// NonPlayerCharacter.Initialize: Character.Initialize, then
    /// Agent.Initialize (order Attack, react to nothing, [AIStateIdle]).
    pub fn new(handle: u16, body: usize, template: &NpcActorTemplate) -> Self {
        let mut ai = AvatarAi::default();
        ai.agent = true;
        ai.set_order(ORDER_ATTACK, None);
        let mut npc = Self {
            special_requests: Vec::new(),
            jump_target: None,
            handle,
            body,
            vlad_character: false,
            equipment_visible: [true; 2],
            definition: template.definition.clone(),
            character: RuntimeCharacter::new(template.definition.clone()),
            on_death: 0,
            on_damage: 0,
            rng: 0x2545_F491_4F6C_DD1D ^ u64::from(handle),
            motion: CharacterMotion::new(template.animations.clone()),
            ai,
            move_abilities: template.move_abilities,
            weights: template.weights,
            react_to: 0,
            reaction_order: 0,
            reaction_trigger: 0,
            time_since_update: 0.0,
            combat: CombatAgent::new(
                template.abilities.clone(),
                template.target_weights,
                u64::from(handle),
            ),
            rig: template.rig.clone(),
            weapons: template.weapons.clone(),
            held_effects: template.held_effects.clone(),
            melee_requests: Vec::new(),
            missile_requests: Vec::new(),
            detach_requests: Vec::new(),
            grip_requests: Vec::new(),
            tint: None,
            animation_events: Vec::new(),
            spell_requests: Vec::new(),
            current_spell: None,
            current_spell_definition: None,
            rail_kills: Vec::new(),
            staff_orb: magicka_content::skinned::IDENTITY,
            weapon_transform: magicka_content::skinned::IDENTITY,
            hip_transform: magicka_content::skinned::IDENTITY,
            play_time: 0.,
            summon_element_cue: template.summon_element_cue.clone(),
            lifecycle_dispatched: 0,
            charm_reset: false,
            faction_override: None,
            summon: None,
            group_candidates: Vec::new(),
            group_leader: None,
        };
        // Character.Initialize: mDeadTimer = 20 + 20·random.
        npc.character.block_values = npc
            .weapons
            .iter()
            .map(|(slot, w)| (*slot, w.block_value))
            .collect();
        npc.motion.dead_timer = 20.0 + 20.0 * npc.random() as f32;
        // Agent.Initialize: mWanderAngle = random · 2π (own Random stream).
        npc.ai.rng = 0x9E37_79B9_7F4A_7C15 ^ (u64::from(handle) << 32);
        let r = npc.random() as f32;
        npc.ai.wander_angle = magicka_sim::agent_steering::initial_wander_angle(r);
        npc
    }

    /// Character.OrderPanic (GiveOrder with the Panic order):
    /// ChangeState(PanicState) and mPanic = 10. The state change is applied
    /// at the next motion update (INFERRED timing; the original switches at
    /// once), where IsPanicing would enter PanicState from Idle/Move anyway.
    pub fn order_panic(&mut self) {
        self.motion.forced_state = Some(crate::character_motion::CharacterState::Panic);
        self.character.status_frame.panic = magicka_sim::agent_steering::ORDER_PANIC_MAGNITUDE;
    }

    /// Character.IsPanicing.
    pub fn is_panicing(&self) -> bool {
        magicka_sim::agent_steering::is_panicing(
            self.character.status_frame.panic,
            self.character.status_frame.max_panic,
        )
    }

    /// Character.IsStumbling: gripped with GripType.Ride.
    pub fn is_stumbling(&self) -> bool {
        magicka_sim::agent_steering::is_stumbling(
            self.character.grip.gripper.is_some(),
            self.character.grip.grip_type == crate::grip::GRIP_RIDE,
        )
    }

    /// Seeded xorshift in [0, 1); not the native System.Random sequence.
    fn random(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn dead(&self) -> bool {
        self.character.combat.dead && !self.character.combat.cannot_die_without_explicit_kill
    }

    /// Agent.SetOrder.
    pub fn set_order(
        &mut self,
        order: u8,
        react_to: u8,
        reaction_order: u8,
        reaction_trigger: i32,
        events: Option<Vec<crate::ai_event::AiEvent>>,
    ) {
        // SetOrder pops back to idle: each AIStateAttack popped releases its
        // target and stops attacking (OnExit).
        for _ in 0..self.ai.set_order(order, events) {
            self.combat.release_target();
            self.motion.stop_attacking();
        }
        self.react_to = react_to;
        self.reaction_order = reaction_order;
        self.reaction_trigger = reaction_trigger;
    }

    /// Agent.Update: proximity reaction, then the top AI state with the time
    /// accumulated since the last update. `neighbours` are the alive entities
    /// within the alert radius (EntityManager.GetEntities semantics).
    pub fn agent_update(
        &mut self,
        mut frame: ActorFrame<'_>,
        own_faction: i32,
        neighbours: &[Neighbour],
        view: Option<(&CombatView<'_>, usize)>,
    ) -> AgentOutcome {
        self.combat.bash_spell_count = self
            .character
            .combat
            .absorb_shield
            .as_ref()
            .map(|queue| queue.queue().len());
        let dt = self.time_since_update;
        let mut outcome = AgentOutcome::default();
        // Agent.Update: aggro decay and target ageing before reactions.
        self.combat.reduce_aggro(dt);
        self.combat.age_targets(dt);
        let attacked =
            self.react_to & REACT_ATTACK == REACT_ATTACK && self.combat.attack_reaction().is_some();
        if attacked
            || (self.react_to & REACT_PROXIMITY == REACT_PROXIMITY
                && neighbours.iter().any(|n| {
                    n.handle != self.handle
                        && n.faction.is_some_and(|f| f & own_faction == 0)
                        && (!n.invisible || self.combat.weights.can_see_invisible)
                }))
        {
            // Agent.React.
            if self.reaction_order != 0 {
                self.ai.order = self.reaction_order;
            }
            if self.reaction_trigger != 0 {
                frame
                    .triggers
                    .push((self.reaction_trigger, Some(self.handle)));
            }
            outcome.reacted = true;
        }
        // AIStateWander/AIStatePanic owner facts for this update.
        self.ai.radius = frame.body.radius;
        self.ai.panicking = self.is_panicing();
        self.ai.stumbling = self.is_stumbling();
        self.ai.run_backward = frame.body.run_backwards;
        // AIStateMove group facts (AlertRadius, Group* weights).
        self.ai.alert_radius = self.weights.alert_radius;
        self.ai.group = self.weights.group;
        let mut ai = ActorAi {
            motion: &mut self.motion,
            frame: &mut frame,
            killed: None,
            combat: view.map(|(view, owner)| CombatCtx {
                agent: &mut self.combat,
                view,
                owner,
            }),
            group: &self.group_candidates,
            leader: self.group_leader,
        };
        // NonPlayerCharacter.Charm/EndCharm/Confuse pop the AI to idle at once; the
        // rewrite does it before the agent's next update (INFERRED timing).
        if std::mem::take(&mut self.charm_reset) {
            self.ai.pop_to_idle(&mut ai);
        }
        self.ai.agent_update(&mut ai, dt);
        outcome.killed = ai.killed;
        for slot in std::mem::take(&mut self.combat.pending_item_specials) {
            if let Some(animation) = crate::item_special::npc_item_cast(
                &mut self.character,
                &mut self.weapons,
                slot,
                &mut self.special_requests,
            ) {
                self.motion.attack(animation, false);
            }
        }
        if let Some((view, owner)) = view {
            if let (Some(next), Some(target)) = (
                self.combat.next_ability,
                self.combat.current_target().and_then(|h| view.get(h)),
            ) {
                let ability = self.combat.ability(next).clone();
                if let crate::npc_combat::AbilityBody::CastSpell(spell) = ability.body {
                    let delta: Vector = std::array::from_fn(|i| {
                        target.position[i] - view.combatants[owner].position[i]
                    });
                    let context = crate::npc_spellcasting::AbilityContext {
                        busy_is_next: self.combat.busy_ability == self.combat.next_ability,
                        current_spell: self.current_spell.is_some(),
                        attacking: self.motion.attacking,
                        distance_squared: delta.iter().map(|v| v * v).sum(),
                    };
                    let attempts =
                        self.combat
                            .spellcasting
                            .update_ability(&spell, context, &mut Vec::new());
                    for (element, _) in attempts {
                        self.animation_events
                            .push(crate::avatar::AvatarEvent::NpcConjure(element));
                        self.spell_requests
                            .push(crate::avatar::AvatarEvent::NpcConjure(element));
                    }
                }
            }
        }
        self.time_since_update = 0.0;
        outcome
    }

    pub(crate) fn motion_facts(
        &mut self,
        body: &magicka_sim::character_physics::CharacterBody,
        in_cutscene: bool,
        random: f64,
    ) -> CharacterFacts {
        CharacterFacts {
            undie: self.character.undie_timer.is_some(),
            magick_animation: None,
            hit_points: self.character.combat.health.hit_points,
            cannot_die_without_explicit_kill: self
                .character
                .combat
                .cannot_die_without_explicit_kill,
            dead: self.character.combat.dead,
            overkilled: self.character.combat.overkilled,
            avatar: false,
            cast_type: self.combat.spellcasting.cast_type,
            spell_type: self.combat.spellcasting.prepared.spell_type(),
            charge_unlocked: false,
            in_cutscene,
            frozen: self
                .character
                .combat
                .statuses
                .has(magicka_sim::status::Status::Frozen),
            random,
            weapon_spell: None,
            fear: self.character.fear_source,
            panicking: self.is_panicing(),
            stumbling: self.is_stumbling(),
            max_panic: self.character.status_frame.max_panic,
            stunned: self.character.combat.stun_timer > 0.0 && !self.character.combat.dead,
            knocked_down: self.character.combat.motion.knocked_down,
            on_ground: body.is_touching_ground(),
            grip: {
                let mut grip = self.character.grip.facts();
                grip.entangled = self.character.entanglement.entangled();
                grip
            },
        }
    }

    /// Spawn.Execute: enter the shared state through the real NPC action host,
    /// so frame-zero animation actions and spell presentation are retained.
    pub fn begin_resurrection(
        &mut self,
        body: &mut magicka_sim::character_physics::CharacterBody,
        in_cutscene: bool,
        animation: u16,
    ) {
        let source = self.spray_source(body);
        let facts = self.motion_facts(body, in_cutscene, 0.0);
        let (mut died, mut entered_dead) = (false, false);
        let first_event = self.animation_events.len();
        self.motion.begin_resurrection(
            body,
            &facts,
            animation,
            &mut NpcHost {
                specials: &mut self.special_requests,
                jump_target: self.jump_target,
                died: &mut died,
                entered_dead: &mut entered_dead,
                combat: &mut self.combat,
                requests: &mut self.melee_requests,
                missiles: &mut self.missile_requests,
                detaches: &mut self.detach_requests,
                grips: &mut self.grip_requests,
                animation_events: &mut self.animation_events,
                weapons: &mut self.weapons,
                character: &mut self.character,
                handle: self.handle,
                current_spell: &mut self.current_spell,
                current_spell_definition: &mut self.current_spell_definition,
                rail_kills: &mut self.rail_kills,
                source,
                play_time: self.play_time,
            },
        );
        self.spell_requests
            .extend_from_slice(&self.animation_events[first_event..]);
        if died {
            self.character.combat.dead = true;
            self.character.combat.death_status_mask = self.character.combat.statuses.current_mask;
            self.character.arm_undie(self.motion.drowning);
            // Character.Die: mBuffs.Clear() (general part).
            self.character.buffs.general.clear();
        }
    }

    /// Character.Update subset: animation, actions, states, dead timer.
    /// Returns (entered DeadState, Die() ran).
    pub fn update_motion(
        &mut self,
        body: &mut magicka_sim::character_physics::CharacterBody,
        in_cutscene: bool,
        dt: f32,
    ) -> (bool, bool) {
        self.character.update_contact_timers(dt);
        self.combat.spellcasting.tick(dt);
        if let Some(power) = self.combat.pending_spell_power.take() {
            self.character.spell_power = power;
        }
        self.stop_status_spell();
        self.character.status_current_spell = self.current_spell.is_some();
        let source = self.spray_source(body);
        let random = self.random();
        let facts = self.motion_facts(body, in_cutscene, random);
        let (mut died, mut entered_dead) = (false, false);
        let first_event = self.animation_events.len();
        // NonPlayerCharacter.Update: ability cooldowns tick every frame.
        self.combat.tick_cooldowns(dt);
        self.motion.tick_block_timer(dt);
        // Character.Update: BlockItem = -1 before the animation actions.
        self.character.block_item = -1;
        self.melee_requests.clear();
        self.missile_requests.clear();
        self.detach_requests.clear();
        self.grip_requests.clear();
        self.motion.update(
            body,
            &facts,
            dt,
            &mut NpcHost {
                specials: &mut self.special_requests,
                jump_target: self.jump_target,
                died: &mut died,
                entered_dead: &mut entered_dead,
                combat: &mut self.combat,
                requests: &mut self.melee_requests,
                missiles: &mut self.missile_requests,
                detaches: &mut self.detach_requests,
                grips: &mut self.grip_requests,
                animation_events: &mut self.animation_events,
                weapons: &mut self.weapons,
                character: &mut self.character,
                handle: self.handle,
                current_spell: &mut self.current_spell,
                current_spell_definition: &mut self.current_spell_definition,
                rail_kills: &mut self.rail_kills,
                source,
                play_time: self.play_time,
            },
        );
        self.update_self_shield(body, dt);
        self.spell_requests
            .extend_from_slice(&self.animation_events[first_event..]);
        if died {
            let combat = &mut self.character.combat;
            combat.dead = true;
            combat.death_status_mask = combat.statuses.current_mask;
            self.character.arm_undie(self.motion.drowning);
            self.character.buffs.general.clear();
        }
        self.character.tick_stun(dt);
        (entered_dead, died)
    }

    fn update_self_shield(
        &mut self,
        body: &mut magicka_sim::character_physics::CharacterBody,
        dt: f32,
    ) {
        let mut events = std::mem::take(&mut self.character.shield_events);
        let shield = &mut self.character.combat.health.shield;
        if shield.active() {
            let mut speed = body.speed_multiplier();
            shield.update(
                dt,
                &mut speed,
                self.character.definition.mass,
                false,
                &mut events,
            );
            body.set_speed_multiplier(speed);
        }
        for event in events {
            use magicka_sim::self_shield::ShieldEvent as S;
            match event {
                S::SetMass(mass) => body.mass = mass,
                S::ClearAura => self.character.auras.clear(),
                S::CircleDamage { radius, damage } => self
                    .animation_events
                    .push(crate::avatar::AvatarEvent::ShieldCircleDamage { radius, damage }),
                _ => {}
            }
        }
    }

    pub fn finish(
        &mut self,
        body: &mut magicka_sim::character_physics::CharacterBody,
        _in_cutscene: bool,
        dt: f32,
    ) {
        self.motion
            .finish_dead(body, self.character.combat.dead, dt);
        for (_, weapon) in &mut self.weapons {
            if let Some(special) = &mut weapon.special {
                special.tick(dt);
            }
        }
        body.owner.hit_points = self.character.combat.health.hit_points;
        self.track_weapons(body);
    }

    pub fn spray_source(
        &self,
        body: &magicka_sim::character_physics::CharacterBody,
    ) -> magicka_sim::spray_pulse::SpraySource {
        let position = |m: &magicka_content::skinned::Mat| [m[12], m[13], m[14]];
        let direction = |m: &magicka_content::skinned::Mat| [-m[8], -m[9], -m[10]];
        magicka_sim::spray_pulse::SpraySource {
            owner_handle: self.handle,
            owner_position: body.position,
            owner_direction: body.direction(),
            cast_position: position(&self.staff_orb),
            cast_direction: direction(&self.staff_orb),
            weapon_position: position(&self.weapon_transform),
            weapon_direction: direction(&self.weapon_transform),
        }
    }

    pub fn cast_update<
        W: magicka_sim::spray_pulse::SprayWorld
            + magicka_sim::push_spell::PushWorld
            + magicka_sim::lightning::LightningScene
            + magicka_sim::railgun::RailControl,
    >(
        &mut self,
        body: &mut magicka_sim::character_physics::CharacterBody,
        dt: f32,
        world: &mut W,
    ) {
        let source = self.spray_source(body);
        let first_event = self.animation_events.len();
        let keep = crate::avatar::CharacterSpellHost {
            owner_is_character: true,
            avatar: false,
            handle: self.handle,
            equipment: &[magicka_sim::resistance::Resistance::IDENTITY; 11],
            character: &mut self.character,
            current_spell: &mut self.current_spell,
            current_spell_definition: &mut self.current_spell_definition,
            events: &mut self.animation_events,
            rail_kills: &mut self.rail_kills,
            source: Some(source),
            play_time: self.play_time,
        }
        .update(
            body,
            dt,
            world,
            self.combat.spellcasting.cast_type,
            true,
            self.motion.controller.has_finished(),
            &mut self.rng,
        );
        if !keep
            && self
                .current_spell_definition
                .is_some_and(|(cast, _)| cast == CastType::Weapon)
            && self.current_spell.as_ref().is_some_and(|s| !s.active())
            && self.motion.state != crate::character_motion::CharacterState::Cast
        {
            self.current_spell = None;
            self.current_spell_definition = None;
        }
        self.spell_requests
            .extend_from_slice(&self.animation_events[first_event..]);
        self.combat.current_spell = self.current_spell.is_some();
    }

    pub fn stop_current_spell(&mut self) -> bool {
        let had_spell = self.current_spell.is_some();
        let first_event = self.animation_events.len();
        self.spell_host().end();
        self.spell_requests
            .extend_from_slice(&self.animation_events[first_event..]);
        self.combat.current_spell = false;
        had_spell
    }

    pub fn stop_status_spell(&mut self) {
        let first_event = self.animation_events.len();
        self.spell_host().stop_for_status();
        self.spell_requests
            .extend_from_slice(&self.animation_events[first_event..]);
    }

    fn spell_host(&mut self) -> crate::avatar::CharacterSpellHost<'_> {
        crate::avatar::CharacterSpellHost {
            owner_is_character: true,
            avatar: false,
            handle: self.handle,
            equipment: &[magicka_sim::resistance::Resistance::IDENTITY; 11],
            character: &mut self.character,
            current_spell: &mut self.current_spell,
            current_spell_definition: &mut self.current_spell_definition,
            events: &mut self.animation_events,
            rail_kills: &mut self.rail_kills,
            source: None,
            play_time: self.play_time,
        }
    }

    /// CurrentSpell.CastArea(spell): an area cast outside the animation
    /// queue (FloorStomp).
    pub(crate) fn cast_area(
        &mut self,
        body: &magicka_sim::character_physics::CharacterBody,
        spell: magicka_sim::spells::Spell,
    ) {
        let first = self.animation_events.len();
        let source = self.spray_source(body);
        let mut host = self.spell_host();
        host.source = Some(source);
        host.cast(spell, CastType::Area, false);
        self.spell_requests
            .extend_from_slice(&self.animation_events[first..]);
    }

    /// AbsorbShield.Execute releases through Spell.Cast, bypassing the
    /// character's ordinary animation-cast queue and using equipment zero.
    pub(crate) fn release_absorbed_spell(
        &mut self,
        body: &magicka_sim::character_physics::CharacterBody,
    ) {
        let Some(queue) = &mut self.character.combat.absorb_shield else {
            return;
        };
        if queue.peek().element == magicka_sim::spells::Elements::NONE {
            return;
        }
        let spell = queue.retrieve(&mut Vec::new());
        let first = self.animation_events.len();
        let source = self.spray_source(body);
        let mut host = self.spell_host();
        host.source = Some(source);
        host.cast(spell, CastType::Area, false);
        self.spell_requests
            .extend_from_slice(&self.animation_events[first..]);
    }

    /// Item.Update: each weapon's attach0 and item position from this
    /// frame's pose (last attach kept for the melee sweep).
    fn track_weapons(&mut self, body: &magicka_sim::character_physics::CharacterBody) {
        let Some(rig) = &self.rig else {
            return;
        };
        let height_offset = -0.5 * body.length - body.radius;
        let parent = crate::character_pose::character_orientation(
            &self
                .character
                .scaled_static_transform(&rig.static_transform),
            &body.orientation,
            body.position,
            height_offset,
        );
        let skinned = rig.skeleton.skinned_for(&self.motion.controller, &parent);
        // NonPlayerCharacter.Update reads the hand joints, rather than the
        // Avatar's staff orb or an equipped item's attach0.
        self.staff_orb = rig.hand_joints[0]
            .as_ref()
            .map_or(magicka_content::skinned::IDENTITY, |joint| {
                joint.item_transform(&skinned)
            });
        self.weapon_transform = rig.hand_joints[1]
            .as_ref()
            .map_or(magicka_content::skinned::IDENTITY, |joint| {
                joint.item_transform(&skinned)
            });
        self.hip_transform = rig
            .hip_joint
            .as_ref()
            .map_or(magicka_content::skinned::IDENTITY, |joint| {
                joint.item_transform(&skinned)
            });
        for (slot, weapon) in &mut self.weapons {
            if let Some(a) = rig.attachments.iter().find(|a| a.slot == *slot) {
                let item = a.item_transform(&skinned);
                weapon.item_transform = item;
                let attach = a.attach_absolute(&skinned);
                weapon.track(
                    [attach[12], attach[13], attach[14]],
                    [item[12], item[13], item[14]],
                );
            }
        }
        for held in &mut self.held_effects {
            held.frame = rig
                .attachments
                .iter()
                .find(|a| a.slot == held.slot)
                .map(|a| crate::item_effects::ItemFrame::new(a, &skinned));
        }
    }
}

/// CharacterHost for NPCs: death, busy-ability release and DealDamage
/// requests (executed by the scene with world access after the update).
struct NpcHost<'a> {
    specials: &'a mut Vec<crate::item_special::SpecialInvocation>,
    jump_target: Option<magicka_sim::jump::JumpTarget>,
    handle: u16,
    current_spell: &'a mut Option<crate::avatar::SpellEffect>,
    current_spell_definition: &'a mut Option<(CastType, magicka_sim::spells::Spell)>,
    rail_kills: &'a mut Vec<magicka_sim::railgun::RailId>,
    source: magicka_sim::spray_pulse::SpraySource,
    play_time: f64,
    character: &'a mut RuntimeCharacter,
    died: &'a mut bool,
    entered_dead: &'a mut bool,
    combat: &'a mut CombatAgent,
    requests: &'a mut Vec<crate::npc_combat::MeleeRequest>,
    missiles: &'a mut Vec<(i32, Option<Vector>, bool)>,
    detaches: &'a mut Vec<(i32, Vector)>,
    grips: &'a mut Vec<crate::grip::GripRequest>,
    animation_events: &'a mut Vec<crate::avatar::AvatarEvent>,
    weapons: &'a mut Vec<(i32, MeleeWeapon)>,
}
impl NpcHost<'_> {
    fn spells(&mut self) -> crate::avatar::CharacterSpellHost<'_> {
        crate::avatar::CharacterSpellHost {
            owner_is_character: true,
            avatar: false,
            handle: self.handle,
            equipment: &[magicka_sim::resistance::Resistance::IDENTITY; 11],
            character: self.character,
            current_spell: self.current_spell,
            current_spell_definition: self.current_spell_definition,
            events: self.animation_events,
            rail_kills: self.rail_kills,
            source: Some(self.source),
            play_time: self.play_time,
        }
    }
}
impl CharacterHost for NpcHost<'_> {
    fn special_action(
        &mut self,
        weapon: i32,
        ability: Option<&magicka_content::common::SpecialAbility>,
    ) -> Option<u16> {
        if weapon < 0 {
            if let Some(ability) = ability {
                self.specials.push(crate::item_special::SpecialInvocation {
                    ability: ability.clone(),
                    buff_id: None,
                });
            }
            None
        } else {
            crate::item_special::npc_item_cast(self.character, self.weapons, weapon, self.specials)
        }
    }
    fn jump_animation(
        &mut self,
        body: &mut magicka_sim::character_physics::CharacterBody,
        elevation: f32,
        min: Option<f32>,
        max: Option<f32>,
    ) {
        if let Some(delta) = magicka_sim::jump::animation_delta(
            body.position,
            body.direction(),
            self.jump_target,
            min,
            max,
        ) {
            body.allow_move = true;
            magicka_sim::jump::launch(
                body,
                self.character.combat.dead,
                delta,
                elevation,
                magicka_sim::character_physics::GRAVITY[1],
            );
        }
    }
    fn remove_status(&mut self, mask: u16) {
        self.character.stop_status_effects(mask, false);
    }
    fn block_action(&mut self, weapon: i32) {
        self.character.block_item = weapon;
        if self.combat.busy_is_block() {
            self.combat.busy_ability = None;
        }
    }
    fn kill_block_action(&mut self) {
        self.character.block_item = -1;
    }
    fn grip_knock_down(&mut self) -> bool {
        // Character.KnockDown(): CanBeKnockedDown is a tolerance other
        // than 5000 (the horizontal velocity reset is not applied here).
        let motion = &mut self.character.combat.motion;
        if motion.knockdown_tolerance == 5000.0 {
            return false;
        }
        motion.knocked_down = true;
        true
    }
    fn grip_request(&mut self, request: crate::grip::GripRequest) {
        self.grips.push(request);
    }
    fn play_animation_effect(
        &mut self,
        index: usize,
        action: &magicka_content::character::AnimationAction,
        first: bool,
    ) {
        if let magicka_content::character::ActionKind::PlayEffect { bone, effect, .. } =
            &action.kind
        {
            self.animation_events
                .push(crate::avatar::AvatarEvent::AnimationEffect {
                    index,
                    bone: bone.clone(),
                    effect: effect.to_lowercase(),
                    retained: action.start_time < action.end_time,
                    first,
                });
        }
    }
    fn stop_animation_effect(&mut self, index: usize) {
        self.animation_events
            .push(crate::avatar::AvatarEvent::AnimationEffectStopped(index));
    }
    fn play_animation_sound(&mut self, bank: u16, sound: &str) {
        self.character
            .pending_animation_audio
            .push((bank, crate::scene_loading::custom_hash(sound)));
    }
    fn clear_knocked_down(&mut self) {
        self.character.combat.motion.knocked_down = false;
    }
    fn ethereal_action(&mut self, ethereal: bool, alpha: f32, speed: f32) {
        self.character
            .execute_ethereal_action(ethereal, alpha, speed);
    }
    fn entered_grip_attack_state(&mut self) {
        self.character.ethereal(false, 1.0, 1.0);
        self.character.set_invisible(0.0);
    }
    fn begin_charge(&mut self) {
        self.character.ethereal(false, 1.0, 1.0);
        self.character.set_invisible(0.0);
    }
    fn entered_attack_state(&mut self) {
        self.character.ethereal(false, 1.0, 1.0);
        self.character.enter_attack_visibility();
    }
    fn entered_cast_state(&mut self) {
        self.character.ethereal(false, 1.0, 1.0);
        self.character.set_invisible(0.0);
    }
    fn entered_panic_state(&mut self) {
        self.character.set_invisible(0.0);
    }
    fn immortal_action(&mut self, collide: bool) {
        self.character.execute_immortal_action(collide);
    }
    fn kill_immortal_action(&mut self) {
        self.character.kill_immortal_action();
    }
    fn cast_spell(&mut self, from_staff: bool, _: Option<&str>) {
        let Some(invocation) = self
            .combat
            .spellcasting
            .animation_cast(true, self.current_spell.is_some())
        else {
            return;
        };
        if invocation.spell.element == magicka_sim::spells::Elements::ALL {
            self.animation_events
                .push(crate::avatar::AvatarEvent::CastMagick(
                    invocation.spell.magick,
                ));
        } else {
            self.spells()
                .cast(invocation.spell, invocation.cast_type, from_staff);
        }
    }
    fn spell_animation_end(&mut self) {
        self.spells().animation_end();
    }
    fn current_spell(&self) -> Option<bool> {
        self.current_spell
            .as_ref()
            .map(crate::avatar::SpellEffect::active)
    }
    fn end_cast(&mut self) {
        self.spells().end();
        self.combat.spellcasting.set_cast_type(CastType::Blast);
        self.combat.current_spell = false;
    }
    fn entered_dead_state(&mut self) {
        *self.entered_dead = true;
    }
    fn die(&mut self) {
        *self.died = true;
    }
    fn release_busy_ability(&mut self) {
        self.combat.busy_ability = None;
    }
    fn stop_execute(&mut self, weapon: i32) {
        self.requests
            .push(crate::npc_combat::MeleeRequest::Stop(weapon));
    }
    fn deal_damage(&mut self, weapon: i32, targets: u8, first: bool) {
        if first {
            if let Some((_, w)) = self.weapons.iter_mut().find(|(s, _)| *s == weapon) {
                w.clear_hitlist();
            }
        }
        self.requests
            .push(crate::npc_combat::MeleeRequest::Execute {
                weapon,
                targets,
                first,
            });
    }
    fn spawn_missile(&mut self, weapon: i32, velocity: Option<Vector>, item_aligned: bool) {
        self.missiles.push((weapon, velocity, item_aligned));
    }
    fn detach_item(&mut self, slot: i32, velocity: Vector) {
        self.detaches.push((slot, velocity));
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AgentOutcome {
    pub reacted: bool,
    /// KillEvent: Some(remove).
    pub killed: Option<bool>,
}

fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn length(v: Vector) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}
fn scale(v: Vector, s: f32) -> Vector {
    v.map(|x| x * s)
}
fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// XNA Vector3.Normalize (zero becomes NaN).
fn normalize(v: Vector) -> Vector {
    scale(v, 1.0 / length(v))
}

/// Agent.GetAvoidance over the entities within 12 m (owner excluded).
pub fn avoidance(
    position: Vector,
    faction: i32,
    weights: &AgentWeights,
    neighbours: &[Neighbour],
) -> Vector {
    let (mut sight, mut friendly, mut enemy, mut danger) = ([0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3]);
    for n in neighbours {
        // Agent avoidance skips invisible characters entirely.
        if n.invisible && n.faction.is_some() {
            continue;
        }
        match n.faction {
            Some(f) if f & faction != 0 => {
                let d = sub(position, n.position);
                let len = length(d);
                let w = if len < 1e-6 { 1.0 } else { 1.0 - len / 12.0 };
                friendly = add(friendly, scale(scale(normalize(d), w), n.danger));
            }
            Some(_) => {
                let mut d = sub(position, n.position);
                let len = length(d);
                let w;
                if len >= 1e-6 {
                    w = 1.0 - len / 12.0;
                    d = scale(d, 1.0 / len);
                    let c = cross(d, n.direction);
                    let up = [0.0, if c[1] > 0.0 { 1.0 } else { -1.0 }, 0.0];
                    let side = normalize(cross(d, up));
                    let facing =
                        (d[0] * n.direction[0] + d[1] * n.direction[1] + d[2] * n.direction[2])
                            .clamp(0.0, 1.0)
                            .powf(0.25);
                    let r = len / 12.0;
                    let falloff = r * r * (1.0 - r) * 6.75;
                    sight = add(sight, scale(scale(side, facing), falloff));
                } else {
                    w = 1.0;
                }
                enemy = add(enemy, scale(scale(normalize(d), w), n.danger));
            }
            None => {
                let mut d = sub(position, n.position);
                d[1] = 0.0;
                let len = length(d);
                let w = 1.0 - len / 12.0;
                d = if len > 1e-6 { normalize(d) } else { [0.0; 3] };
                danger = add(danger, scale(scale(d, w), n.danger));
            }
        }
    }
    let mut out = add(
        add(
            scale(sight, weights.sight_avoidance),
            scale(friendly, weights.friendly_avoidance),
        ),
        add(
            scale(enemy, weights.enemy_avoidance),
            scale(danger, weights.danger_avoidance),
        ),
    );
    let len = length(out);
    if len.is_infinite() {
        out = [0.0; 3];
    }
    if len > 100_000.0 {
        out = scale(out, 1.0 / (len / 100_000.0));
    }
    out
}

/// AIManager.Update scheduling: which agents run this frame. Advances the
/// start index by STEPS first, then runs min(STEPS, n) agents from there.
pub fn scheduled(update_index: &mut usize, count: usize) -> Vec<usize> {
    if count == 0 {
        return Vec::new();
    }
    *update_index = (*update_index + AGENTS_PER_FRAME) % count;
    (0..AGENTS_PER_FRAME.min(count))
        .map(|i| (i + *update_index) % count)
        .collect()
}
