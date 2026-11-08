//! NPC combat, statement for statement where reconstructed:
//! Agent.ChooseTarget / ChooseAbility / AddTarget / ChangeTarget /
//! ReleaseTarget / ReduceAggro / AddAttackedBy, AIStateAttack.OnExecute,
//! Melee, Ranged, CastSpell, RemoveStatus, Block, Dash, Jump and DamageGrip, and Item.Execute melee hits with their
//! Hit-condition events (Item.ExecuteRanged missiles are spawned by the
//! scene). Other ability types are listed but not executable (they never
//! win ChooseAbility). See research/npc-combat.md.
use crate::animation_controller::animation_value;
use magicka_content::common::{Event, EventCollection};
use magicka_sim::{
    ai_expression::{
        fuzzy_anger, fuzzy_distance_exponential, Expr, ExpressionArgs, ExpressionHost, Subject,
    },
    damage::Damage,
    entity_query::{EntityQueryGrid, QueryEntity},
    impact_motion::Vector,
    resistance::Resistance,
};
use std::sync::Arc;

pub const TARGET_SELF: u8 = 1;
pub const TARGET_ENEMY: u8 = 2;
pub const TARGET_FRIENDLY: u8 = 3;
/// NonPlayerCharacter.mAbilityCooldown length.
pub const COOLDOWN_SLOTS: usize = 16;

/// Ordered Item.Execute / Item.StopExecute calls from animation actions.
#[derive(Clone, Copy, Debug)]
pub enum MeleeRequest {
    Execute {
        weapon: i32,
        targets: u8,
        first: bool,
    },
    Stop(i32),
}

/// PlayEffectEvent references belong to their item when Follow is authored.
#[derive(Clone, Debug)]
pub enum ItemEffectRequest {
    Start {
        owner: u16,
        weapon: i32,
        name: String,
        position: Vector,
        direction: Vector,
        follow: bool,
    },
    Stop {
        owner: u16,
        weapon: i32,
    },
}
/// Agent.mFuzzySortValues length (half enemies, half others).
const FUZZY_SLOTS: usize = 8;

fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn length(a: Vector) -> f32 {
    dot(a, a).sqrt()
}

#[derive(Clone, Debug)]
pub enum AbilityBody {
    Special {
        min_range: f32,
        max_range: f32,
        arc: f32,
        weapon: i32,
    },
    Jump {
        min_range: f32,
        max_range: f32,
        arc: f32,
        elevation: f32,
    },
    Dash {
        min_range: f32,
        max_range: f32,
        arc: f32,
        /// Read by the native constructor but not used by execution.
        vector: Vector,
    },
    Block {
        arc: f32,
        shield: i32,
    },
    Melee {
        min_range: f32,
        max_range: f32,
        arc: f32,
        rotate: bool,
        weapons: Vec<i32>,
    },
    /// Ranged: the stored arc is replaced by π when read.
    Ranged {
        min_range: f32,
        max_range: f32,
        elevation: f32,
        accuracy: f32,
        weapons: Vec<i32>,
    },
    CastSpell(crate::npc_spellcasting::SpellAbility),
    RemoveStatus,
    /// DamageGrip: always in range and facing while gripping.
    DamageGrip,
    /// An ability type without a reconstruction (by name).
    Unreconstructed(String),
}

#[derive(Clone, Debug)]
pub struct CombatAbility {
    pub cooldown: f32,
    pub target: u8,
    pub expression: Option<Expr>,
    pub animations: Vec<u16>,
    pub body: AbilityBody,
}

impl CombatAbility {
    pub fn from_content(a: &magicka_content::character::Ability) -> Result<Self, String> {
        use magicka_content::character::AbilityKind;
        let expression = a
            .fuzzy_expression
            .as_deref()
            .map(Expr::read)
            .transpose()
            .map_err(|e| format!("{e:?}"))?;
        let animations = a
            .animation_keys
            .iter()
            .map(|n| animation_value(n).ok_or_else(|| format!("animation {n}")))
            .collect::<Result<_, _>>()?;
        let body = match &a.kind {
            AbilityKind::SpecialAbilityAbility {
                min_range,
                max_range,
                angle,
                weapon,
            } => AbilityBody::Special {
                min_range: *min_range,
                max_range: *max_range,
                arc: *angle,
                weapon: *weapon,
            },
            AbilityKind::Jump {
                min_range,
                max_range,
                angle,
                elevation,
            } => AbilityBody::Jump {
                min_range: *min_range,
                max_range: *max_range,
                arc: *angle,
                elevation: *elevation,
            },
            AbilityKind::Dash {
                min_range,
                max_range,
                arc,
                vector,
            } => AbilityBody::Dash {
                min_range: *min_range,
                max_range: *max_range,
                arc: *arc,
                vector: *vector,
            },
            AbilityKind::Block { arc, shield } => AbilityBody::Block {
                arc: *arc,
                shield: *shield,
            },
            AbilityKind::Melee {
                min_range,
                max_range,
                arc,
                weapons,
                rotate,
            } => AbilityBody::Melee {
                min_range: *min_range,
                max_range: *max_range,
                arc: *arc,
                rotate: *rotate,
                weapons: weapons.clone(),
            },
            AbilityKind::Ranged {
                min_range,
                max_range,
                elevation,
                accuracy,
                weapons,
                ..
            } => AbilityBody::Ranged {
                min_range: *min_range,
                max_range: *max_range,
                elevation: *elevation,
                accuracy: *accuracy,
                weapons: weapons.clone(),
            },
            AbilityKind::CastSpell { .. } => AbilityBody::CastSpell(
                crate::npc_spellcasting::SpellAbility::from_content(a)?.unwrap(),
            ),
            AbilityKind::RemoveStatus => AbilityBody::RemoveStatus,
            AbilityKind::DamageGrip => AbilityBody::DamageGrip,
            other => AbilityBody::Unreconstructed(
                format!("{other:?}")
                    .split([' ', '{', '('])
                    .next()
                    .unwrap_or("?")
                    .to_owned(),
            ),
        };
        Ok(Self {
            cooldown: a.cooldown,
            target: a.target,
            expression,
            animations,
            body,
        })
    }
    fn ranges(&self) -> Option<(f32, f32, f32)> {
        match &self.body {
            AbilityBody::Special {
                min_range,
                max_range,
                arc,
                ..
            } => Some((*min_range, *max_range, *arc)),
            AbilityBody::Jump {
                min_range,
                max_range,
                arc,
                ..
            } => Some((*min_range, *max_range, *arc)),
            AbilityBody::Dash {
                min_range,
                max_range,
                arc,
                ..
            } => Some((*min_range, *max_range, *arc)),
            AbilityBody::Block { arc, .. } => Some((0., f32::MAX, *arc)),
            AbilityBody::Melee {
                min_range,
                max_range,
                arc,
                ..
            } => Some((*min_range, *max_range, *arc)),
            AbilityBody::Ranged {
                min_range,
                max_range,
                ..
            } => Some((*min_range, *max_range, std::f32::consts::PI)),
            AbilityBody::CastSpell(spell) => Some((spell.min_range, spell.max_range, spell.arc)),
            AbilityBody::RemoveStatus => Some((0., f32::MAX, std::f32::consts::PI)),
            AbilityBody::DamageGrip => Some((0., 0., std::f32::consts::PI)),
            AbilityBody::Unreconstructed(_) => None,
        }
    }
}

/// A character as the combat code sees it (snapshot per frame).
#[derive(Clone, Copy, Debug)]
pub struct Combatant {
    /// Native Character equipment has seven slots; no special means not ready.
    pub special_ready: [bool; 7],
    pub handle: u16,
    pub unique_id: i32,
    pub position: Vector,
    pub velocity: Vector,
    /// Character.Direction (body orientation forward).
    pub direction: Vector,
    /// CharacterBody.Movement.
    pub movement: Vector,
    pub radius: f32,
    pub length: f32,
    pub faction: i32,
    pub hit_points: f32,
    pub max_hit_points: f32,
    pub dead: bool,
    pub avatar: bool,
    pub status_mask: u16,
    pub ethereal: bool,
    /// Character.IsInvisibile.
    pub invisible: bool,
    pub on_ground: bool,
    /// CurrentState is AttackState / CastState or ChargeState.
    pub attack_state: bool,
    pub casting: bool,
    /// IsGripping / IsGripped and GripDamageAccumulation / HitTolerance.
    pub gripping: bool,
    pub gripped: bool,
    pub grip_ratio: f32,
    pub current_animation: u16,
    /// SpellManager.Combine(SpellQueue) magnitudes per element index.
    pub queue: [f32; 11],
    pub resistances: [Resistance; 11],
    /// Avatar staff resistances (identity without items).
    pub equipment: [Resistance; 11],
    /// False for a non-Character IDamageable (a damageable prop) added only
    /// as some agent's priority target; ChooseTarget skips such entries
    /// unless they are its own priority target.
    pub character: bool,
    /// Character.GetAgroMultiplier (ReduceAgro buffs); 1 for props.
    pub agro_multiplier: f32,
}

impl Combatant {
    /// Character.ResistanceAgainst (buffs are not modelled).
    pub fn resistance_against(&self, elements: u32) -> f32 {
        let (mut modifier, mut multiplier) = (0.0f32, 0.0f32);
        for i in 0..11 {
            let element = 1u32 << i;
            if elements & element == 0 {
                continue;
            }
            let mut m = self.resistances[i].multiplier;
            let mut d = self.resistances[i].modifier;
            if self.status_mask & 4 == 4 && elements & 1 != 0 {
                d -= 350.0;
            }
            if self.status_mask & 0x40 == 0x40 && elements & 8 != 0 {
                m *= 2.0;
            }
            if self.avatar {
                m *= self.equipment[i].multiplier;
                d += self.equipment[i].modifier;
            }
            modifier += d;
            multiplier += m;
        }
        1.0 - (modifier / 300.0 + multiplier).clamp(-1.0, 1.0)
    }
}

/// EntityManager.GetEntities over the combatants (character order).
pub struct CombatView<'a> {
    pub combatants: &'a [Combatant],
    entities: Vec<QueryEntity>,
    grid: EntityQueryGrid,
    pub scene: Option<&'a crate::level_collision::SceneCollision>,
}

impl<'a> CombatView<'a> {
    pub fn new(
        combatants: &'a [Combatant],
        scene: Option<&'a crate::level_collision::SceneCollision>,
    ) -> Option<Self> {
        let entities: Vec<QueryEntity> = combatants
            .iter()
            .map(|c| QueryEntity {
                position: c.position,
                radius: c.radius,
                // Priority props join only through ChooseTarget's explicit
                // priority append; spatial queries see characters.
                dead: c.dead || !c.character,
                shield: false,
                damageable: true,
            })
            .collect();
        let grid = EntityQueryGrid::build(&entities).ok()?;
        Some(Self {
            combatants,
            entities,
            grid,
            scene,
        })
    }
    pub fn index(&self, handle: u16) -> Option<usize> {
        self.combatants.iter().position(|c| c.handle == handle)
    }
    pub fn get(&self, handle: u16) -> Option<&Combatant> {
        self.combatants.iter().find(|c| c.handle == handle)
    }
    /// GetEntities(center, radius, ignoreProtected): live entities whose
    /// sphere touches the query sphere, in grid order.
    pub fn entities(&self, center: Vector, radius: f32, ignore_protected: bool) -> Vec<usize> {
        self.grid
            .query(
                &self.entities,
                center,
                radius,
                ignore_protected,
                false,
                |_, _| (false, [0.0; 3]),
            )
            .unwrap_or_default()
    }
    pub fn scene_hit(&self, origin: Vector, delta: Vector) -> bool {
        self.scene.is_some_and(|s| {
            s.segment_intersect(magicka_sim::spray_pulse::Segment { origin, delta }, false)
                .is_some()
        })
    }
}

/// CharacterTemplate AI weights used by targeting.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TargetWeights {
    pub alert_radius: f32,
    pub anger: f32,
    pub distance: f32,
    pub health: f32,
    /// CharacterTemplate CanSeeInvisible.
    pub can_see_invisible: bool,
}

/// What ChooseTarget/ChooseAbility pick: a character or the owner itself.
pub type TargetChoice = (f32, Option<u16>, Option<usize>);

/// Agent combat state (targets, abilities, aggro).
#[derive(Clone, Debug)]
pub struct CombatAgent {
    /// Warlord's Bash subclass adds queue.Count * 0.2 to ability zero.
    pub bash_spell_count: Option<usize>,
    pub pending_item_specials: Vec<i32>,
    pub abilities: Arc<Vec<CombatAbility>>,
    pub cooldowns: [f32; COOLDOWN_SLOTS],
    /// mTargets (top is last) with mTargetAges.
    pub targets: Vec<u16>,
    pub target_ages: Vec<f32>,
    pub next_ability: Option<usize>,
    pub busy_ability: Option<usize>,
    /// mBeenAttackedBy (insertion-ordered).
    pub attacked_by: Vec<(u16, f32)>,
    pub weights: TargetWeights,
    pub priority_target: Option<u16>,
    pub priority_ability: i32,
    /// Item.mSourceAbility of each equipment slot a Ranged ability
    /// prepared: (slot, elevation, accuracy).
    pub ranged_source: Vec<(i32, f32, f32)>,
    pub spellcasting: crate::npc_spellcasting::NpcSpellcasting,
    pub current_spell: bool,
    /// Agent's distinct fallback ability; its identity follows retail abilities.
    pub spell_recovery: Option<CombatAbility>,
    pub last_spell_ability: Option<usize>,
    pub pending_spell_power: Option<f32>,
    rng: u64,
}

/// What AIStateAttack asks of the agent's AI stack and character.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AttackStep {
    Stay,
    /// PopState (and ReleaseTarget when `release`).
    Pop {
        release: bool,
    },
    /// WayPoint = offset from the target; PushState(AIStateMove).
    Move(Vector),
}

/// Character operations AIStateAttack performs on its owner.
pub trait AttackOwner {
    fn set_movement(&mut self, movement: Vector);
    fn set_desired_direction(&mut self, direction: Vector);
    fn attacking(&self) -> bool;
    /// Character.Attack(animation, allowRotate).
    fn attack(&mut self, animation: u16, allow_rotate: bool);
    /// Character.Dashing.
    fn dashing(&self) -> bool {
        false
    }
    /// Dash.InternalExecute: Character.Dash(animation, false), then
    /// NextDashAnimation = a second draw.
    fn dash(&mut self, _animation: u16, _next_dash_animation: u16) {}
    /// NonPlayerCharacter.Block.
    fn block(&mut self) {}
    /// Character.DamageGripped.
    fn damage_gripped(&mut self, _animation: u16) {}
    fn jump(&mut self, _delta: Vector, _elevation: f32) {}
    /// Agent.GetAvoidance length.
    fn avoidance_length(&self) -> f32;
    fn order(&self) -> u8;
}

const ORDER_DEFEND: u8 = 3;

/// Rewrite extension (invader/dungeon-master pilots; the original has no
/// such role): the player control that drives each family of an NPC's own
/// authored abilities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PilotAction {
    /// Melee (and grip damage): Cross/A, R2 or right mouse.
    Primary,
    /// Ranged, CastSpell, item Special and RemoveStatus aimed at others:
    /// L2/LT or Shift.
    Secondary,
    /// The same families authored with Target=Self, cast on the NPC itself:
    /// Triangle/Y or middle mouse (the wizard's self-cast controls).
    SelfCast,
    /// Dash or Jump: Circle/B or Space.
    Boost,
    /// Block: R1/RB, held.
    Block,
}

impl CombatAbility {
    pub fn pilot_action(&self) -> Option<PilotAction> {
        match self.body {
            AbilityBody::Melee { .. } | AbilityBody::DamageGrip => Some(PilotAction::Primary),
            AbilityBody::Ranged { .. }
            | AbilityBody::CastSpell(_)
            | AbilityBody::Special { .. }
            | AbilityBody::RemoveStatus => Some(if self.target == TARGET_SELF {
                PilotAction::SelfCast
            } else {
                PilotAction::Secondary
            }),
            AbilityBody::Dash { .. } | AbilityBody::Jump { .. } => Some(PilotAction::Boost),
            AbilityBody::Block { .. } => Some(PilotAction::Block),
            AbilityBody::Unreconstructed(_) => None,
        }
    }
}

/// What a pilot's action did this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum PilotOutcome {
    /// The ability ran through Ability.Execute (index into the abilities).
    Executed(usize),
    /// A CastSpell ability conjured these elements (NPC chant).
    Chanting(Vec<(magicka_sim::spells::Elements, bool)>),
    /// Busy, cooling down, out of reach or no ability of this family.
    Unavailable,
}

impl CombatAgent {
    pub fn new(abilities: Arc<Vec<CombatAbility>>, weights: TargetWeights, seed: u64) -> Self {
        Self {
            bash_spell_count: None,
            pending_item_specials: Vec::new(),
            abilities,
            cooldowns: [0.0; COOLDOWN_SLOTS],
            targets: Vec::new(),
            target_ages: Vec::new(),
            next_ability: None,
            busy_ability: None,
            attacked_by: Vec::new(),
            weights,
            priority_target: None,
            priority_ability: -1,
            ranged_source: Vec::new(),
            spellcasting: Default::default(),
            current_spell: false,
            spell_recovery: None,
            last_spell_ability: None,
            pending_spell_power: None,
            rng: 0x853C_49E6_748F_EA9B ^ seed,
        }
    }

    /// Ability.sRandom stand-in (the original System.Random is time-seeded).
    fn random_below(&mut self, n: usize) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng % n.max(1) as u64) as usize
    }
    fn random_unit(&mut self) -> f32 {
        self.random_below(1 << 24) as f32 / (1u32 << 24) as f32
    }

    pub fn current_target(&self) -> Option<u16> {
        self.targets.last().copied()
    }

    /// AI.BusyAbility is a Block (the Block animation action releases it).
    pub fn busy_is_block(&self) -> bool {
        self.busy_ability
            .is_some_and(|i| matches!(self.ability(i).body, AbilityBody::Block { .. }))
    }

    /// Rewrite extension (mod `Spells.CastElements` on an NPC): cast these
    /// elements next, through the native recovery-ability slot and the
    /// NPC's own spell animations. Returns false when the NPC has no spell
    /// ability to borrow animations from (it cannot cast).
    pub fn script_cast(
        &mut self,
        elements: Vec<magicka_sim::spells::Elements>,
        cast_type: magicka_sim::damage::CastType,
    ) -> bool {
        let Some(animations) = self.abilities.iter().find_map(|a| match &a.body {
            AbilityBody::CastSpell(s) => Some(s.animations.clone()),
            _ => None,
        }) else {
            return false;
        };
        let spell = crate::npc_spellcasting::SpellAbility {
            min_range: 0.,
            max_range: 30.,
            arc: std::f32::consts::PI,
            chant_speed: 0.,
            power: 1.,
            cast_type,
            animations: animations.clone(),
            elements,
        };
        self.spell_recovery = Some(CombatAbility {
            cooldown: 0.,
            target: TARGET_ENEMY,
            expression: None,
            animations,
            body: AbilityBody::CastSpell(spell),
        });
        self.next_ability = Some(self.abilities.len());
        if let Some(age) = self.target_ages.last_mut() {
            *age = 0.0;
        }
        true
    }

    pub fn ability(&self, index: usize) -> &CombatAbility {
        if index == self.abilities.len() {
            self.spell_recovery
                .as_ref()
                .expect("selected spell recovery")
        } else {
            &self.abilities[index]
        }
    }
    pub fn current_target_age(&self) -> f32 {
        self.target_ages.last().copied().unwrap_or(0.0)
    }

    /// NonPlayerCharacter.Update: every cooldown slot ticks.
    pub fn tick_cooldowns(&mut self, dt: f32) {
        for c in &mut self.cooldowns {
            *c -= dt;
        }
    }
    /// Agent.Update ageing of targets.
    pub fn age_targets(&mut self, dt: f32) {
        for a in &mut self.target_ages {
            *a += dt;
        }
    }

    /// Agent.ReduceAggro, including the original's skip of the entry after
    /// a removed one (the loop index still advances).
    pub fn reduce_aggro(&mut self, dt: f32) {
        let mut i = 0;
        while i < self.attacked_by.len() {
            let (key, value) = self.attacked_by[i];
            let value = (value - 25.0 * dt).min(500.0);
            if value < 0.0 {
                self.attacked_by.remove(i);
            } else {
                self.attacked_by[i] = (key, value);
            }
            i += 1;
        }
    }

    /// Agent.Update ReactTo.Attack scan: the last attacker with aggro above
    /// zero (the running maximum is never updated in the original).
    pub fn attack_reaction(&self) -> Option<u16> {
        let mut found = None;
        for &(key, value) in &self.attacked_by {
            if value > 0.0 {
                found = Some(key);
            }
        }
        found
    }

    /// Agent.AddAttackedBy (own part). Returns whether nearby allies should
    /// also receive the aggro (the caller propagates it).
    pub fn add_attacked_by(
        &mut self,
        attacker: u16,
        attacker_faction: i32,
        owner_faction: i32,
        amount: f32,
    ) {
        let i = self.attacked_by.iter().position(|e| e.0 == attacker);
        let value = i.map_or(0.0, |i| self.attacked_by[i].1);
        let value = if attacker_faction & owner_faction == 0 {
            value + amount
        } else {
            value + amount * 0.0
        };
        if value > f32::from_bits(1) {
            match i {
                Some(i) => self.attacked_by[i].1 = value,
                None => self.attacked_by.push((attacker, value)),
            }
        } else if let Some(i) = i {
            self.attacked_by.remove(i);
        }
    }

    fn anger(&self, handle: u16) -> f32 {
        let value = self
            .attacked_by
            .iter()
            .find(|e| e.0 == handle)
            .map_or(0.0, |e| e.1);
        fuzzy_anger(value)
    }

    /// Agent.AddTarget.
    pub fn add_target(&mut self, target: u16, ability: Option<usize>) {
        if Some(target) != self.current_target() {
            self.targets.push(target);
            self.next_ability = ability;
            self.target_ages.push(0.0);
        }
    }
    /// Agent.ChangeTarget.
    pub fn change_target(&mut self, target: u16, ability: Option<usize>) {
        self.targets.pop();
        self.targets.push(target);
        self.next_ability = ability;
        if let Some(a) = self.target_ages.last_mut() {
            *a = 0.0;
        }
    }
    /// Agent.ReleaseTarget.
    pub fn release_target(&mut self) {
        if !self.targets.is_empty() {
            self.targets.pop();
            self.target_ages.pop();
        }
        self.next_ability = None;
    }

    /// Ability.GetDesirability then the ChooseAbility filters.
    #[allow(clippy::too_many_arguments)]
    pub fn choose_ability(
        &mut self,
        view: &CombatView<'_>,
        owner: usize,
        target: usize,
        args: &ExpressionArgs,
        gripping: bool,
    ) -> (f32, Option<usize>) {
        let mut best = f32::MIN;
        let mut chosen = None;
        let owner_c = view.combatants[owner];
        let target_c = view.combatants[target];
        let target_is_owner = owner == target;
        let abilities = self.abilities.clone();
        for (i, ability) in abilities.iter().enumerate() {
            if self.cooldowns.get(i).copied().unwrap_or(0.0) > 0.0 {
                continue;
            }
            let same_faction = target_c.faction & owner_c.faction != 0;
            let excluded = ((ability.target == TARGET_SELF) & !target_is_owner)
                | ((ability.target == TARGET_FRIENDLY) & (target_is_owner || !same_faction))
                | ((ability.target == TARGET_ENEMY) & same_faction);
            let spell = match &ability.body {
                AbilityBody::CastSpell(spell) => Some(spell),
                _ => None,
            };
            if excluded || !self.spellcasting.allows(spell) {
                continue;
            }
            let mut value = self.desirability(view, owner, target, args, i, gripping);
            if i == 0 {
                if let Some(count) = self.bash_spell_count {
                    value += count as f32 * 0.2;
                }
            }
            if (i as i32 == self.priority_ability) & (Some(target_c.handle) == self.priority_target)
            {
                value += 0.5;
            }
            if value > 0.0 && value > best {
                best = value;
                chosen = Some(i);
            }
        }
        if chosen.is_none()
            && !self.spellcasting.queue.is_empty()
            && self.next_ability != Some(self.abilities.len())
        {
            if let Some(last) = self.last_spell_ability {
                let previous = self.abilities[last].clone();
                if previous.target == TARGET_ENEMY && target_c.faction & owner_c.faction == 0 {
                    let AbilityBody::CastSpell(previous_spell) = previous.body else {
                        unreachable!()
                    };
                    let spell = crate::npc_spellcasting::SpellAbility {
                        min_range: 0.,
                        max_range: 10.,
                        arc: 1.,
                        chant_speed: 0.,
                        power: 1.,
                        cast_type: previous_spell.cast_type,
                        animations: previous.animations.clone(),
                        elements: self.spellcasting.queue.iter().map(|s| s.element).collect(),
                    };
                    self.spell_recovery = Some(CombatAbility {
                        cooldown: previous.cooldown,
                        target: previous.target,
                        expression: None,
                        animations: previous.animations,
                        body: AbilityBody::CastSpell(spell),
                    });
                    return (f32::from_bits(2), Some(self.abilities.len()));
                }
            }
        }
        (best, chosen)
    }

    /// Ability.GetDesirability.
    fn desirability(
        &mut self,
        view: &CombatView<'_>,
        owner: usize,
        target: usize,
        args: &ExpressionArgs,
        index: usize,
        gripping: bool,
    ) -> f32 {
        let ability = &self.abilities[index];
        if let AbilityBody::Special { weapon, .. } = ability.body {
            if !usize::try_from(weapon)
                .ok()
                .and_then(|slot| view.combatants[owner].special_ready.get(slot))
                .copied()
                .unwrap_or(false)
            {
                return f32::MIN;
            }
        }
        let Some((min, max, _)) = ability.ranges() else {
            return f32::MIN;
        };
        // GetDesirability: IsUseful first.
        if !is_useful(ability, gripping, view.combatants[owner].grip_ratio) {
            return f32::MIN;
        }
        if matches!(ability.body, AbilityBody::RemoveStatus)
            && ability
                .animations
                .contains(&view.combatants[owner].current_animation)
        {
            return f32::MIN;
        }
        if self.cooldowns[index] > 0.0 {
            return f32::MIN;
        }
        if let Some(expression) = &ability.expression {
            let mut host = ExprHost {
                view,
                owner,
                target,
                alert_radius: self.weights.alert_radius,
            };
            return expression.value(args, &mut host).unwrap_or(f32::MIN);
        }
        if matches!(ability.body, AbilityBody::Jump { .. }) {
            return if view.combatants[target].avatar {
                f32::MAX
            } else {
                f32::MIN
            };
        }
        if let AbilityBody::DamageGrip = ability.body {
            // DamageGrip.Desirability (IsUseful already requires gripping).
            return (-2.0 * view.combatants[owner].grip_ratio + 1.0) * 2.0;
        }
        if matches!(
            ability.body,
            AbilityBody::CastSpell(_) | AbilityBody::RemoveStatus | AbilityBody::Block { .. }
        ) {
            // These native desirability overrides require an authored expression.
            return f32::MIN;
        }
        fuzzy_distance_exponential(args.distance, min, max)
    }

    /// Agent.ChooseTarget: score characters within 40 m (plus avatars and
    /// the priority target), keep the best four enemies and four others,
    /// add each one's best ability, then compare with the owner as target.
    pub fn choose_target(
        &mut self,
        view: &CombatView<'_>,
        owner: usize,
        gripping: bool,
    ) -> TargetChoice {
        let me = view.combatants[owner];
        let mut values = [f32::MIN; FUZZY_SLOTS];
        let mut entities: [Option<usize>; FUZZY_SLOTS] = [None; FUZZY_SLOTS];
        let mut abilities: [Option<usize>; FUZZY_SLOTS] = [None; FUZZY_SLOTS];
        let mut list = view.entities(me.position, 40.0, false);
        for (i, c) in view.combatants.iter().enumerate() {
            if c.avatar && !list.contains(&i) {
                list.push(i);
            }
        }
        if let Some(p) = self.priority_target.and_then(|h| view.index(h)) {
            if !list.contains(&p) {
                list.push(p);
            }
        }
        let half = FUZZY_SLOTS / 2;
        for &i in &list {
            let c = view.combatants[i];
            let priority = Some(c.handle) == self.priority_target;
            if c.dead
                || i == owner
                || (!priority && !c.character)
                || (c.invisible && !self.weights.can_see_invisible)
            {
                continue;
            }
            let distance = length(sub(me.position, c.position));
            let mut value = 0.0;
            if Some(c.handle) == self.priority_target {
                value += 1.0;
            }
            value += self.anger(c.handle) * self.weights.anger;
            value += (1.0 - c.hit_points / c.max_hit_points) * self.weights.health;
            value += fuzzy_distance_exponential(
                distance,
                -self.weights.alert_radius,
                self.weights.alert_radius,
            ) * self.weights.distance;
            value *= c.agro_multiplier;
            // Only enemy Characters fill the first half; a priority prop
            // sorts with the others.
            let range = if c.character && c.faction & me.faction == 0 {
                0..half
            } else {
                half..FUZZY_SLOTS
            };
            let end = range.end;
            for slot in range {
                if value > values[slot] {
                    for k in (slot + 1..end).rev() {
                        entities[k] = entities[k - 1];
                        values[k] = values[k - 1];
                    }
                    entities[slot] = Some(i);
                    values[slot] = value;
                    break;
                }
            }
        }
        for n in 0..FUZZY_SLOTS {
            if let Some(t) = entities[n] {
                let c = view.combatants[t];
                let args = ExpressionArgs::new(me.position, me.direction, c.position, c.direction);
                let (v, a) = self.choose_ability(view, owner, t, &args, gripping);
                values[n] += v;
                abilities[n] = a;
            }
        }
        let mut best = f32::MIN;
        let (mut target, mut ability) = (None, None);
        for n in 0..FUZZY_SLOTS {
            if values[n] > best {
                target = entities[n];
                ability = abilities[n];
                best = values[n];
            }
        }
        let args = ExpressionArgs {
            ai_position: me.position,
            ai_direction: me.direction,
            target_position: me.position,
            target_direction: me.direction,
            ..Default::default()
        };
        let (own, own_ability) = self.choose_ability(view, owner, owner, &args, gripping);
        if own > best {
            best = own;
            ability = own_ability;
            target = Some(owner);
        }
        if ability != Some(self.abilities.len()) {
            self.last_spell_ability =
                ability.filter(|&i| matches!(self.abilities[i].body, AbilityBody::CastSpell(_)));
        }
        (best, target.map(|t| view.combatants[t].handle), ability)
    }

    /// AIStateAttack.OnExecute (grips and force-navmesh/camera are not
    /// modelled).
    pub fn attack_execute(
        &mut self,
        view: &CombatView<'_>,
        owner: usize,
        host: &mut dyn AttackOwner,
    ) -> AttackStep {
        let Some(target_handle) = self.current_target() else {
            return AttackStep::Pop { release: true };
        };
        let me = view.combatants[owner];
        if let Some(busy) = self.busy_ability {
            host.set_movement([0.0; 3]);
            if let Some(t) = view.get(target_handle) {
                host.set_desired_direction(ability_direction(self.ability(busy), me, *t));
            }
            return AttackStep::Stay;
        }
        if host.avoidance_length() > 20.0 {
            return AttackStep::Pop { release: true };
        }
        let target = view.index(target_handle);
        let mut ability = self.next_ability;
        let useful =
            ability.is_some_and(|a| is_useful(self.ability(a), me.gripping, me.grip_ratio));
        if !useful || self.current_target_age() > 1.0 {
            ability = target.and_then(|t| {
                let c = view.combatants[t];
                let args = ExpressionArgs::new(me.position, me.direction, c.position, c.direction);
                self.choose_ability(view, owner, t, &args, me.gripping).1
            });
            self.next_ability = ability;
        }
        let target_alive = target.is_some_and(|t| {
            let c = view.combatants[t];
            c.hit_points > 0.0 && !c.dead && (!c.invisible || self.weights.can_see_invisible)
        });
        let (Some(ability), true) = (ability, target_alive) else {
            host.set_movement([0.0; 3]);
            return AttackStep::Pop { release: true };
        };
        let t = view.combatants[target.unwrap()];
        let def = self.ability(ability).clone();
        let in_range = in_range(&def, me.position, me.radius, t.position, t.radius);
        if in_range {
            host.set_movement([0.0; 3]);
            host.set_desired_direction(ability_direction(&def, me, t));
            if !facing_target(&def, me.direction, me.position, t.position) {
                return AttackStep::Stay;
            }
            if self.execute(ability, host, view, owner, t) {
                // Recovery's inherited Index is the CLR default zero.
                self.cooldowns[if ability == self.abilities.len() {
                    0
                } else {
                    ability
                }] = def.cooldown;
                let (_, new_target, new_ability) = self.choose_target(view, owner, me.gripping);
                if let Some(nt) = new_target {
                    self.change_target(nt, new_ability);
                }
            } else if !host.attacking() {
                if let Some(d) = self.choose_attack_angle(view, owner, &def, t) {
                    return AttackStep::Move(d);
                }
                // A false ChooseAttackAngle leaves the zero waypoint.
                return AttackStep::Move([0.0; 3]);
            }
            AttackStep::Stay
        } else if host.order() != ORDER_DEFEND {
            if host.attacking() | host.dashing() {
                host.set_movement([0.0; 3]);
                host.set_desired_direction(ability_direction(&def, me, t));
                AttackStep::Stay
            } else if let Some(d) = self.choose_attack_angle(view, owner, &def, t) {
                AttackStep::Move(d)
            } else {
                host.set_movement([0.0; 3]);
                AttackStep::Pop { release: true }
            }
        } else {
            host.set_movement([0.0; 3]);
            AttackStep::Pop { release: true }
        }
    }

    /// Indices of this agent's abilities driven by `action`. An NPC without
    /// melee uses its ranged/spell family for the primary control too.
    pub fn pilot_abilities(&self, action: PilotAction) -> Vec<usize> {
        let of = |a: PilotAction| -> Vec<usize> {
            (0..self.abilities.len())
                .filter(|&i| self.abilities[i].pilot_action() == Some(a))
                .collect()
        };
        let own = of(action);
        if own.is_empty() && action == PilotAction::Primary {
            of(PilotAction::Secondary)
        } else {
            own
        }
    }

    /// Rewrite pilot: run one of the agent's own `action` abilities now,
    /// toward `target`, with the native busy, cooldown, usefulness and
    /// facing/range data instead of the AI's ability choice and approach.
    /// In-range abilities are preferred; a CastSpell ability chants its
    /// elements (as NonPlayerCharacter.Update does for NextAbility) until
    /// the queue is complete, then casts.
    pub fn pilot_execute(
        &mut self,
        view: &CombatView<'_>,
        owner: usize,
        host: &mut dyn AttackOwner,
        action: PilotAction,
        target: Combatant,
        current_spell: bool,
    ) -> PilotOutcome {
        let me = view.combatants[owner];
        let busy = self.busy_ability;
        if busy.is_some() && !(action == PilotAction::Block && self.busy_is_block()) {
            return PilotOutcome::Unavailable;
        }
        // Agent.ChooseAbility's target-class and spell-queue filters.
        let same_faction = target.faction & me.faction != 0;
        let ready: Vec<usize> = self
            .pilot_abilities(action)
            .into_iter()
            .filter(|&i| {
                let a = self.ability(i);
                let spell = match &a.body {
                    AbilityBody::CastSpell(spell) => Some(spell),
                    _ => None,
                };
                let excluded = (a.target == TARGET_SELF) != (action == PilotAction::SelfCast)
                    || (a.target == TARGET_FRIENDLY && !same_faction)
                    || (a.target == TARGET_ENEMY && same_faction);
                self.cooldowns.get(i).is_none_or(|c| *c <= 0.0)
                    && is_useful(a, me.gripping, me.grip_ratio)
                    && !excluded
                    && self.spellcasting.allows(spell)
            })
            .collect();
        let reach = |agent: &Self, i: usize| {
            in_range(
                agent.ability(i),
                me.position,
                me.radius,
                target.position,
                target.radius,
            )
        };
        // A spell being chanted continues; otherwise the NPC's own
        // desirability (authored expressions) chooses within the family
        // against a real target, in-range abilities first.
        let chanting = self
            .next_ability
            .filter(|n| ready.contains(n) && !self.spellcasting.queue.is_empty());
        let target_index = if action == PilotAction::SelfCast {
            Some(owner)
        } else {
            (target.handle != 0)
                .then(|| view.index(target.handle))
                .flatten()
        };
        let ranked = target_index.and_then(|t| {
            let c = view.combatants[t];
            let args = ExpressionArgs::new(me.position, me.direction, c.position, c.direction);
            let mut best: Option<(bool, f32, usize)> = None;
            for &i in &ready {
                let value = self.desirability(view, owner, t, &args, i, me.gripping);
                if value <= f32::MIN {
                    continue;
                }
                let key = (reach(self, i), value, i);
                if best.is_none_or(|b| (key.0, key.1) > (b.0, b.1)) {
                    best = Some(key);
                }
            }
            best.map(|b| b.2)
        });
        let Some(ability) = chanting
            .or(ranked)
            .or_else(|| ready.iter().copied().find(|&i| reach(self, i)))
            .or(ready.first().copied())
        else {
            return PilotOutcome::Unavailable;
        };
        let def = self.ability(ability).clone();
        host.set_desired_direction(ability_direction(&def, me, target));
        if let AbilityBody::CastSpell(spell) = &def.body {
            if self.spellcasting.queue.len() < spell.elements.len() {
                self.next_ability = Some(ability);
                let context = crate::npc_spellcasting::AbilityContext {
                    busy_is_next: false,
                    current_spell,
                    attacking: host.attacking(),
                    distance_squared: 0.0,
                };
                return PilotOutcome::Chanting(self.spellcasting.update_ability(
                    spell,
                    context,
                    &mut Vec::new(),
                ));
            }
        }
        self.next_ability = Some(ability);
        if self.execute(ability, host, view, owner, target) {
            if let Some(c) = self.cooldowns.get_mut(ability) {
                *c = def.cooldown;
            }
            PilotOutcome::Executed(ability)
        } else {
            PilotOutcome::Unavailable
        }
    }

    /// Ability.Execute → InternalExecute; success makes it the busy ability.
    fn execute(
        &mut self,
        ability: usize,
        host: &mut dyn AttackOwner,
        view: &CombatView<'_>,
        owner: usize,
        target: Combatant,
    ) -> bool {
        let def = self.ability(ability).clone();
        let ok = match &def.body {
            AbilityBody::Special { weapon, .. } => {
                if def.animations.is_empty() {
                    self.pending_item_specials.push(*weapon);
                } else {
                    let key = self.random_below(def.animations.len());
                    host.attack(def.animations[key], false);
                }
                true
            }
            AbilityBody::Jump {
                min_range,
                max_range,
                elevation,
                ..
            } => {
                if let Some(delta) = magicka_sim::jump::ability_delta(
                    view.combatants[owner].position,
                    magicka_sim::jump::JumpTarget {
                        position: target.position,
                        velocity: target.velocity,
                    },
                    *min_range,
                    *max_range,
                ) {
                    host.jump(delta, *elevation);
                    true
                } else {
                    false
                }
            }
            AbilityBody::CastSpell(spell) => {
                let context = crate::npc_spellcasting::AbilityContext {
                    busy_is_next: self.busy_ability == self.next_ability,
                    current_spell: self.current_spell,
                    attacking: host.attacking(),
                    distance_squared: 0.,
                };
                if context.busy_is_next
                    || context.attacking
                    || self.spellcasting.queue.len() < spell.elements.len()
                {
                    return false;
                }
                let k = self.random_below(spell.animations.len());
                match self.spellcasting.execute(spell, context, |_| k) {
                    Ok(Some(animation)) => {
                        self.pending_spell_power = Some(self.spellcasting.power);
                        host.attack(animation, true);
                        true
                    }
                    _ => false,
                }
            }
            AbilityBody::Ranged {
                max_range,
                elevation,
                accuracy,
                weapons,
                ..
            } => {
                // Ranged.InternalExecute: face the target, then refuse when a
                // friendly character (not the owner) is within 0.75 of the
                // elevation-shortened segment to it.
                let me = view.combatants[owner];
                let delta = sub(target.position, me.position);
                host.set_desired_direction(delta);
                let delta = delta.map(|x| x * elevation.cos());
                let middle = [
                    me.position[0] + delta[0] * 0.5,
                    me.position[1] + delta[1] * 0.5,
                    me.position[2] + delta[2] * 0.5,
                ];
                let blocked = view
                    .entities(middle, *max_range, false)
                    .into_iter()
                    .any(|i| {
                        let c = &view.combatants[i];
                        i != owner
                            && c.faction & me.faction != 0
                            && capsule_segment_hit(c, me.position, delta, 0.75)
                    });
                if blocked || def.animations.is_empty() {
                    false
                } else {
                    // Item.PrepareToExecute(this) on every weapon.
                    for &slot in weapons {
                        self.ranged_source.retain(|(s, ..)| *s != slot);
                        self.ranged_source.push((slot, *elevation, *accuracy));
                    }
                    let k = self.random_below(def.animations.len());
                    host.attack(def.animations[k], true);
                    true
                }
            }
            AbilityBody::Melee { rotate, .. } => {
                if def.animations.is_empty() {
                    false
                } else {
                    let k = self.random_below(def.animations.len());
                    host.attack(def.animations[k], *rotate);
                    true
                }
            }
            AbilityBody::RemoveStatus => {
                if def.animations.is_empty() {
                    false
                } else {
                    let k = self.random_below(def.animations.len());
                    host.attack(def.animations[k], false);
                    true
                }
            }
            AbilityBody::Dash { .. } => {
                if view.combatants[owner].gripping || def.animations.is_empty() {
                    false
                } else {
                    let k = self.random_below(def.animations.len());
                    let next = self.random_below(def.animations.len());
                    host.dash(def.animations[k], def.animations[next]);
                    true
                }
            }
            AbilityBody::Block { .. } => {
                host.block();
                true
            }
            AbilityBody::DamageGrip => {
                if def.animations.is_empty() {
                    false
                } else {
                    let k = self.random_below(def.animations.len());
                    host.damage_gripped(def.animations[k]);
                    true
                }
            }
            AbilityBody::Unreconstructed(_) => false,
        };
        if ok {
            self.busy_ability = Some(ability);
        }
        ok
    }

    /// Ability.ChooseAttackAngle: up to three random yaws (±90°) of the
    /// owner's side at the mid range; the first unobstructed one (scene
    /// segment from the target) is the waypoint offset.
    fn choose_attack_angle(
        &mut self,
        view: &CombatView<'_>,
        owner: usize,
        def: &CombatAbility,
        target: Combatant,
    ) -> Option<Vector> {
        let me = view.combatants[owner];
        let (min, max, _) = def.ranges()?;
        if let AbilityBody::Ranged { elevation, .. } = def.body {
            return self.choose_ranged_angle(view, me, min, max, elevation, target);
        }
        let reach = (min + max) * 0.5 + target.radius;
        let mut v = sub(me.position, target.position);
        v[1] = 0.0;
        if dot(v, v) <= 1e-6 {
            return None;
        }
        let l = length(v);
        let v = v.map(|x| x / l * reach);
        for _ in 0..3 {
            let yaw = self.random_unit() * std::f32::consts::PI - std::f32::consts::FRAC_PI_2;
            let (s, c) = yaw.sin_cos();
            let delta = [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c];
            if !view.scene_hit(target.position, delta) {
                return Some(delta);
            }
        }
        None
    }

    /// Ranged.ChooseAttackAngle: the mid range without the target radius;
    /// a yaw passes when neither the half segment out from the target nor
    /// the half segment back from the waypoint (both shortened by the
    /// elevation) hits the scene. Its character loop has an empty body.
    fn choose_ranged_angle(
        &mut self,
        view: &CombatView<'_>,
        me: Combatant,
        min: f32,
        max: f32,
        elevation: f32,
        target: Combatant,
    ) -> Option<Vector> {
        let reach = (min + max) * 0.5;
        let mut v = sub(me.position, target.position);
        v[1] = 0.0;
        if dot(v, v) <= 1e-6 {
            return None;
        }
        let l = length(v);
        let v = v.map(|x| x / l * reach);
        let f = (1.0 - elevation / std::f32::consts::FRAC_PI_2) * 0.5;
        for _ in 0..3 {
            let yaw = self.random_unit() * std::f32::consts::PI - std::f32::consts::FRAC_PI_2;
            let (s, c) = yaw.sin_cos();
            let d = [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c];
            let back = [
                target.position[0] + d[0],
                target.position[1] + d[1],
                target.position[2] + d[2],
            ];
            if !view.scene_hit(target.position, d.map(|x| x * f))
                && !view.scene_hit(back, d.map(|x| -x * f))
            {
                return Some(d);
            }
        }
        None
    }
}

/// Ability.IsUseful overrides: Melee and Dash refuse while gripping,
/// DamageGrip needs a grip with accumulation below half the HitTolerance.
/// (RemoveStatus's animation test is applied in desirability.)
fn is_useful(def: &CombatAbility, gripping: bool, grip_ratio: f32) -> bool {
    match def.body {
        AbilityBody::Melee { .. } | AbilityBody::Dash { .. } => !gripping,
        AbilityBody::DamageGrip => gripping && grip_ratio < 0.5,
        _ => true,
    }
}

/// Melee.GetDesiredDirection.
pub fn desired_direction(_def: &CombatAbility, me: Vector, target: Vector) -> Vector {
    let d = sub(target, me);
    let l = length(d);
    if l > f32::from_bits(1) {
        d.map(|x| x / l)
    } else {
        [0.0, 0.0, -1.0]
    }
}

fn ability_direction(def: &CombatAbility, owner: Combatant, target: Combatant) -> Vector {
    match &def.body {
        AbilityBody::CastSpell(spell) => spell.desired_direction(
            owner.position,
            target.position,
            owner.direction,
            owner.handle == target.handle,
        ),
        AbilityBody::RemoveStatus => owner.direction,
        // DamageGrip.GetDesiredDirection: Vector3.Backward.
        AbilityBody::DamageGrip => [0.0, 0.0, 1.0],
        AbilityBody::Block { .. } => {
            // Block.GetDesiredDirection: a 1e-6 threshold and +Z fallback.
            let d = sub(target.position, owner.position);
            let l = length(d);
            if l > 1e-6 {
                d.map(|x| x / l)
            } else {
                [d[0], d[1], 1.0]
            }
        }
        _ => desired_direction(def, owner.position, target.position),
    }
}

/// Ability.InRange (XZ plus radii), or Jump.InRange (3D center distance).
pub fn in_range(
    def: &CombatAbility,
    me: Vector,
    my_radius: f32,
    target: Vector,
    target_radius: f32,
) -> bool {
    if let AbilityBody::DamageGrip = def.body {
        return true;
    }
    let Some((min, max, _)) = def.ranges() else {
        return false;
    };
    if matches!(def.body, AbilityBody::Jump { .. }) {
        let delta = sub(me, target);
        let distance = dot(delta, delta);
        return (distance >= min * min) & (distance <= max * max);
    }
    let far = max + my_radius + target_radius;
    let near = (min - target_radius).max(0.0);
    let d = [me[0] - target[0], 0.0, me[2] - target[2]];
    let d2 = dot(d, d);
    d2 <= far * far && d2 >= near * near
}

/// Ability.FacingTarget: angle between the body direction and the
/// horizontal direction to the target below the arc.
pub fn facing_target(def: &CombatAbility, direction: Vector, me: Vector, target: Vector) -> bool {
    if let AbilityBody::DamageGrip = def.body {
        return true;
    }
    let Some((_, _, arc)) = def.ranges() else {
        return false;
    };
    let mut d = sub(target, me);
    d[1] = 0.0;
    let l = length(d);
    if l < 1e-6 {
        return true;
    }
    let d = d.map(|x| x / l);
    f64::from(dot(direction, d).clamp(-1.0, 1.0)).acos() < f64::from(arc)
}

/// ExpressionHost over the combat snapshot.
struct ExprHost<'a, 'b> {
    view: &'a CombatView<'b>,
    owner: usize,
    target: usize,
    alert_radius: f32,
}

impl ExprHost<'_, '_> {
    fn who(&self, s: Subject) -> &Combatant {
        &self.view.combatants[match s {
            Subject::Owner => self.owner,
            Subject::Target => self.target,
        }]
    }
    /// FuzzyMath.FuzzyThreat(owner, target, elements).
    fn threat_of(&self, target: &Combatant, elements: u32) -> f32 {
        let me = self.view.combatants[self.owner];
        let mut a = sub(me.position, target.position);
        let l = length(a);
        if l <= 1e-6 {
            return 1.0;
        }
        a = a.map(|x| x / l);
        let angle = (dot(a, target.direction) + 1.0) * 0.5;
        let distance = fuzzy_distance_exponential(l, -20.0, 20.0);
        if target.attack_state {
            return angle * distance;
        }
        let base = angle.max(distance);
        let mut magnitude = 0.0;
        for i in 0..11 {
            let e = 1u32 << i;
            if e & elements == e {
                magnitude += target.queue[i];
            }
        }
        magnitude *= 0.2;
        if target.casting {
            magnitude = f64::from(magnitude).powf(0.25) as f32;
        }
        base * magnitude
    }
}

impl ExpressionHost for ExprHost<'_, '_> {
    fn has_status(&mut self, who: Subject, status: u32) -> Option<bool> {
        let s = u32::from(self.who(who).status_mask);
        Some(s & status == status)
    }
    fn gripped(&mut self, who: Subject) -> Option<bool> {
        Some(self.who(who).gripped)
    }
    fn gripping(&mut self, who: Subject) -> Option<bool> {
        Some(self.who(who).gripping)
    }
    fn entangled(&mut self, _: Subject) -> Option<bool> {
        Some(false)
    }
    fn speed(&mut self, who: Subject) -> Option<f32> {
        Some(length(self.who(who).movement))
    }
    fn health(&mut self, who: Subject) -> (f32, f32) {
        let c = self.who(who);
        (c.hit_points, c.max_hit_points)
    }
    fn density_count(&mut self, around: Subject, friendly: bool, range: f32) -> u32 {
        let center = *self.who(around);
        let own_faction = self.view.combatants[self.owner].faction;
        self.view
            .entities(center.position, range, false)
            .into_iter()
            .filter(|&i| {
                let c = &self.view.combatants[i];
                c.handle != center.handle && ((c.faction & own_faction != 0) == friendly)
            })
            .count() as u32
    }
    fn danger(&mut self, elements: u32) -> f32 {
        let me = self.view.combatants[self.owner];
        let mut total = 0.0;
        for i in self.view.entities(me.position, self.alert_radius, true) {
            let c = self.view.combatants[i];
            if i != self.owner && c.faction & me.faction == 0 {
                total += self.threat_of(&c, elements);
            }
        }
        // Character.GetDanger (nearby spell danger) and zap are not modelled.
        1.0 - 0.25f64.powf(f64::from(total)) as f32
    }
    fn threat(&mut self, elements: u32) -> Option<f32> {
        let t = self.view.combatants[self.target];
        Some(self.threat_of(&t, elements))
    }
    fn line_of_sight(&mut self, origin: Vector, delta: Vector, distance: f32) -> bool {
        let mid = [
            origin[0] + delta[0] * 0.5,
            origin[1] + delta[1] * 0.5,
            origin[2] + delta[2] * 0.5,
        ];
        let owner = self.view.combatants[self.owner].handle;
        let target = self.view.combatants[self.target].handle;
        !self
            .view
            .entities(mid, distance * 0.5, false)
            .into_iter()
            .any(|i| {
                let c = &self.view.combatants[i];
                c.handle != owner
                    && c.handle != target
                    && capsule_segment_hit(
                        c,
                        origin,
                        delta,
                        self.view.combatants[self.owner].radius,
                    )
            })
    }
    fn scene_hit(&mut self, origin: Vector, delta: Vector) -> bool {
        self.view.scene_hit(origin, delta)
    }
    fn is_character(&mut self) -> bool {
        true
    }
    fn ethereal(&mut self, who: Subject) -> Option<bool> {
        Some(self.who(who).ethereal)
    }
    fn resistance_against(&mut self, who: Subject, elements: u32) -> Option<f32> {
        Some(self.who(who).resistance_against(elements))
    }
    fn self_shielded(&mut self, _: Subject) -> Option<bool> {
        Some(false)
    }
    fn is_shield(&mut self) -> bool {
        false
    }
    fn is_barrier(&mut self) -> bool {
        false
    }
    fn grip_damage(&mut self) -> Option<(f32, f32)> {
        None
    }
    fn on_ground(&mut self, who: Subject) -> Option<bool> {
        Some(self.who(who).on_ground)
    }
}

/// Character.SegmentIntersect: the capsule axis (feet-up, centre ± L/2)
/// against the segment, with `radius` added to the capsule radius.
pub fn capsule_segment_hit(c: &Combatant, origin: Vector, delta: Vector, radius: f32) -> bool {
    if c.ethereal {
        return false;
    }
    let start = [c.position[0], c.position[1] - c.length * 0.5, c.position[2]];
    let axis = [0.0, c.length, 0.0];
    let d2 = magicka_sim::geometry::segment_segment_distance_sq(start, axis, origin, delta);
    let reach = radius + c.radius;
    d2 <= reach * reach
}

/// BlastDamage clips its query to the target skin before terrain/radius tests.
pub fn blast_target_delta(c: &Combatant, origin: Vector) -> Vector {
    let delta = std::array::from_fn(|i| c.position[i] - origin[i]);
    let start = [c.position[0], c.position[1] - c.length * 0.5, c.position[2]];
    let fraction = magicka_sim::capsule_intersection::segment_fraction(
        magicka_sim::spray_pulse::Segment { origin, delta },
        start,
        [0.0, 1.0, 0.0],
        c.length,
        c.radius,
    );
    fraction.map_or(delta, |f| delta.map(|v| v * f))
}

/// One equipped weapon's melee state (Item hit list and attach tracking).
#[derive(Clone, Debug)]
pub struct MeleeWeapon {
    pub special: Option<crate::item_special::ItemSpecial>,
    /// Item.mAuras of an NPC-held item (avatars keep theirs on AvatarItem).
    pub auras: Vec<crate::generic_aura::GenericAura>,
    pub block_value: i32,
    /// The item's ranged data (Item.ExecuteRanged), when it has any
    /// ranged condition.
    pub ranged: Option<Arc<RangedItem>>,
    /// The equipped item for Attachment.Release: (Data/Items/Wizard name
    /// when the item is pickable and in that directory, has a model).
    pub release: Option<(Option<String>, bool)>,
    /// Item world transform of the last pose (Item.Transform).
    pub item_transform: [f32; 16],
    pub range: f32,
    pub multi_hit: bool,
    pub conditions: Arc<Vec<EventCollection>>,
    pub hitlist: Vec<u16>,
    pub next_to_damage: usize,
    /// AbsorbShield's persistent continuation across contacts in one swing.
    pub continue_hitting: bool,
    pub attach: Vector,
    pub last_attach: Vector,
    pub item_position: Vector,
}

/// Item ranged fields used by ExecuteRanged.
#[derive(Clone, Debug)]
pub struct RangedItem {
    pub range: f32,
    /// mRangedElevation (radians).
    pub elevation: f32,
    pub homing: f32,
    pub facing: bool,
    pub danger: f32,
    /// Missile radius: the projectile model's bounding sphere, else 0.25.
    pub radius: f32,
    /// The projectile model, relative to Content without extension.
    pub model: Option<String>,
    pub conditions: Vec<EventCollection>,
}

/// A melee hit to deliver: (target handle, damage, attack position).
pub type MeleeHit = (u16, Damage, Vector);

#[derive(Clone, Copy, Debug)]
// A short-lived per-swing value: boxing the character variant would add an
// allocation to the combat hot path to save stack space.
#[allow(clippy::large_enum_variant)]
pub enum MeleeGeometry {
    /// BossDamageZone.SegmentIntersect uses the actual skin, ignoring melee range.
    BossCapsule {
        position: Vector,
        length: f32,
        radius: f32,
    },
    Character(Combatant),
    Dispenser(magicka_sim::spray_pulse::Segment),
    Prop(magicka_sim::box_intersection::OrientedBox),
}

#[derive(Clone, Copy, Debug)]
pub struct MeleeTarget {
    pub handle: u16,
    pub query: QueryEntity,
    pub geometry: MeleeGeometry,
}

impl MeleeWeapon {
    pub fn new(range: f32, multi_hit: bool, conditions: Arc<Vec<EventCollection>>) -> Self {
        Self {
            block_value: 0,
            special: None,
            auras: Vec::new(),
            ranged: None,
            release: None,
            item_transform: [0.0; 16],
            range,
            multi_hit,
            conditions,
            hitlist: Vec::new(),
            next_to_damage: 0,
            continue_hitting: false,
            attach: [0.0; 3],
            last_attach: [0.0; 3],
            item_position: [0.0; 3],
        }
    }
    /// Item.ClearHitlist.
    pub fn clear_hitlist(&mut self) {
        self.hitlist.clear();
        self.next_to_damage = 0;
        self.continue_hitting = false;
    }
    /// Item.Update attach tracking: last = current, then the new attach0.
    pub fn track(&mut self, attach: Vector, item_position: Vector) {
        self.last_attach = self.attach;
        self.attach = attach;
        self.item_position = item_position;
    }

    /// Item.Execute(targets) without shields, world-collision conditions,
    /// special items and passives: sweep attach0 back to its last position,
    /// add capsule hits to the hit list, then run the Hit conditions for
    /// the new entries. Returns the damage to deliver in order.
    pub fn execute(
        &mut self,
        view: &CombatView<'_>,
        owner: usize,
        current_target: Option<u16>,
        targets: u8,
    ) -> Vec<MeleeHit> {
        let candidates: Vec<_> = view
            .combatants
            .iter()
            .copied()
            .map(|c| MeleeTarget {
                handle: c.handle,
                query: QueryEntity {
                    position: c.position,
                    radius: c.radius,
                    dead: c.dead,
                    shield: false,
                    damageable: true,
                },
                geometry: MeleeGeometry::Character(c),
            })
            .collect();
        self.execute_targets(view.combatants[owner], current_target, targets, &candidates)
    }

    /// The native item sweep accepts non-character IDamageable targets with
    /// target bit 4, using SegmentBoxDistanceSq against their collision box.
    pub fn execute_targets(
        &mut self,
        me: Combatant,
        current_target: Option<u16>,
        targets: u8,
        candidates: &[MeleeTarget],
    ) -> Vec<MeleeHit> {
        self.execute_targets_inner(me, current_target, targets, candidates, false)
    }

    pub fn absorb_can_execute(&mut self) -> bool {
        if self.hitlist.is_empty() {
            self.continue_hitting = true;
        }
        self.multi_hit || self.continue_hitting
    }

    /// Return whether the native damage result requests attack_recoil.
    pub fn absorb_result(&mut self, result: u16) -> bool {
        self.continue_hitting &= result & 0x21c != 0;
        !self.continue_hitting && result & magicka_sim::character_impact::result::DEFLECTED != 0
    }

    pub fn absorb_shield_contact(
        &self,
        position: Vector,
        shields: &[(u16, magicka_sim::shield_intersection::ShieldGeometry)],
    ) -> Option<(u16, Vector)> {
        let sweep = magicka_sim::spray_pulse::Segment {
            origin: self.attach,
            delta: sub(self.last_attach, self.attach),
        };
        // Native's second segment uses Owner as origin and Owner - Attach as
        // delta. Shield.SegmentIntersect ignores its radius argument entirely.
        let other = magicka_sim::spray_pulse::Segment {
            origin: position,
            delta: sub(position, self.attach),
        };
        shields.iter().find_map(|(handle, geometry)| {
            let (segment, fraction) = if let Some(fraction) = geometry.segment_fraction(sweep) {
                (sweep, fraction)
            } else {
                (other, geometry.segment_fraction(other)?)
            };
            Some((
                *handle,
                std::array::from_fn(|i| segment.origin[i] + segment.delta[i] * fraction),
            ))
        })
    }

    pub fn execute_absorb_targets(
        &mut self,
        me: Combatant,
        candidates: &[MeleeTarget],
    ) -> Vec<MeleeHit> {
        self.execute_targets_inner(me, None, 7, candidates, true)
    }

    fn execute_targets_inner(
        &mut self,
        me: Combatant,
        current_target: Option<u16>,
        targets: u8,
        candidates: &[MeleeTarget],
        absorb: bool,
    ) -> Vec<MeleeHit> {
        let mut hits = Vec::new();
        if self.conditions.is_empty() && !absorb {
            return hits;
        }
        let continue_hitting = if absorb {
            self.absorb_can_execute()
        } else {
            self.hitlist.is_empty()
        };
        if !self.multi_hit && !continue_hitting {
            return hits;
        }
        let origin = self.attach;
        let delta = sub(self.last_attach, origin);
        let query: Vec<_> = candidates.iter().map(|c| c.query).collect();
        let Some(grid) = EntityQueryGrid::build(&query).ok() else {
            return hits;
        };
        for i in grid
            .query(
                &query,
                origin,
                self.range + if absorb { 0. } else { 5. },
                true,
                false,
                |_, _| (false, [0.0; 3]),
            )
            .unwrap_or_default()
        {
            let c = &candidates[i];
            if c.handle == me.handle || self.hitlist.contains(&c.handle) {
                continue;
            }
            if Some(c.handle) != current_target {
                match c.geometry {
                    MeleeGeometry::Character(character) => {
                        let friendly = character.faction & me.faction != 0;
                        if (friendly && targets & 1 == 0) || (!friendly && targets & 2 == 0) {
                            continue;
                        }
                    }
                    MeleeGeometry::BossCapsule { .. }
                    | MeleeGeometry::Dispenser(_)
                    | MeleeGeometry::Prop(_)
                        if targets & 4 == 0 =>
                    {
                        continue
                    }
                    MeleeGeometry::BossCapsule { .. }
                    | MeleeGeometry::Dispenser(_)
                    | MeleeGeometry::Prop(_) => {}
                }
            }
            let intersects = match c.geometry {
                MeleeGeometry::BossCapsule {
                    position,
                    length,
                    radius,
                } => magicka_sim::capsule_intersection::segment_fraction(
                    magicka_sim::spray_pulse::Segment { origin, delta },
                    position,
                    [0., 1., 0.],
                    length,
                    radius,
                )
                .is_some(),
                MeleeGeometry::Character(character) => {
                    capsule_segment_hit(&character, origin, delta, self.range)
                }
                MeleeGeometry::Dispenser(axis) => {
                    magicka_sim::geometry::jiglib_segment_segment(
                        axis.origin,
                        axis.delta,
                        origin,
                        delta,
                    )
                    .0 <= (1.25 + self.range).powi(2)
                }
                MeleeGeometry::Prop(bounds) => {
                    magicka_sim::capsule_box::segment_distance(
                        magicka_sim::spray_pulse::Segment { origin, delta },
                        bounds,
                    )
                    .0 <= self.range * self.range
                }
            };
            if intersects {
                self.hitlist.push(c.handle);
                if !self.multi_hit {
                    break;
                }
            }
        }
        while self.next_to_damage < self.hitlist.len() {
            let target = self.hitlist[self.next_to_damage];
            for collection in self.conditions.iter() {
                // EventCondition.IsMet for a Hit event (Repeat/Activated
                // bookkeeping is not shared state here).
                if collection.condition.condition_type & 2 == 0 {
                    continue;
                }
                for event in &collection.events {
                    if let Event::Damage { damage, .. } = event {
                        hits.push((
                            target,
                            Damage {
                                attack: damage.attack_property as u16,
                                element: damage.element as u32,
                                amount: damage.amount,
                                magnitude: damage.magnitude,
                            },
                            self.item_position,
                        ));
                    }
                }
            }
            self.next_to_damage += 1;
            if !self.multi_hit {
                break;
            }
        }
        hits
    }
}

/// Agent.AddAttackedBy on `victim` with its avatar-only propagation: when
/// the attacker is an Avatar, every NPC within the victim's alert radius
/// that shares a faction with it (the victim included, again) also gains
/// the aggro, unless it shares a faction with the attacker.
pub fn add_attacked_by(
    agents: &mut std::collections::BTreeMap<u16, crate::npc::NpcCharacter>,
    view: &CombatView<'_>,
    victim: u16,
    attacker: &Combatant,
    amount: f32,
) {
    let Some(npc) = agents.get_mut(&victim) else {
        return;
    };
    for (handle, amount) in attacked_by_alerts(npc, view, attacker, amount) {
        if let Some(n) = agents.get_mut(&handle) {
            add_alert_aggro(&mut n.combat, attacker.handle, amount);
        }
    }
}

/// Shared NonPlayerCharacter.AddAttackedBy route for maps owning NPCs and
/// synchronous contact adapters borrowing distinct NPC references.
pub fn attacked_by_alerts(
    npc: &mut crate::npc::NpcCharacter,
    view: &CombatView<'_>,
    attacker: &Combatant,
    amount: f32,
) -> Vec<(u16, f32)> {
    let Some(me) = view.get(npc.handle).copied() else {
        return vec![];
    };
    npc.combat
        .add_attacked_by(attacker.handle, attacker.faction, me.faction, amount);
    let has = npc
        .combat
        .attacked_by
        .iter()
        .any(|e| e.0 == attacker.handle && e.1 > f32::from_bits(1));
    if !has || !attacker.avatar {
        return vec![];
    }
    let alert = npc.combat.weights.alert_radius;
    let mut alerts = vec![];
    for i in view.entities(me.position, alert, false) {
        let c = view.combatants[i];
        if c.avatar || c.faction & me.faction == 0 {
            continue;
        }
        let num = if attacker.faction & c.faction != 0 {
            amount * 0.0
        } else {
            amount
        };
        if num > f32::from_bits(1) {
            alerts.push((c.handle, num));
        }
    }
    alerts
}

pub fn add_alert_aggro(agent: &mut CombatAgent, attacker: u16, amount: f32) {
    match agent.attacked_by.iter_mut().find(|e| e.0 == attacker) {
        Some(e) => e.1 += amount,
        None => agent.attacked_by.push((attacker, amount)),
    }
}
