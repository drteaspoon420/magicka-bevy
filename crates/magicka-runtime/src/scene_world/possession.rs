//! NPC possession for invaders and dungeon masters (rewrite extension; the
//! original game has no such role, research/multiplayer-scaling.md).
//!
//! A possessed NPC keeps its own character, faction and abilities, and its AI
//! makes no decisions: the pilot is its brain. Movement input steers its
//! body directly every frame. Each action control runs one family of the
//! NPC's own authored abilities (npc_combat::PilotAction) immediately, with
//! that ability's animations, cooldown, busy state and spell chant, toward
//! the hostile character nearest the aim point (else straight ahead).
use super::*;
use magicka_sim::impact_motion::Vector;

/// The pilot's latest controls for one NPC.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Possession {
    /// World X/Z movement (length up to one).
    pub movement: Option<[f32; 2]>,
    /// Any action held, toward this aim point.
    pub attack: Option<Vector>,
    /// Facing toward this point while standing.
    pub aim: Option<Vector>,
    /// Held action controls (PilotAction order: primary, secondary,
    /// self cast, boost, block).
    pub actions: [bool; 5],
    /// Boost acts on its press edge only (previous frame's state).
    pub boost_was_down: bool,
}

const ACTIONS: [crate::npc_combat::PilotAction; 5] = [
    crate::npc_combat::PilotAction::Primary,
    crate::npc_combat::PilotAction::Secondary,
    crate::npc_combat::PilotAction::SelfCast,
    crate::npc_combat::PilotAction::Boost,
    crate::npc_combat::PilotAction::Block,
];

/// The NPC's own motion and body as the ability owner (the AI phase's
/// AttackHost without its navigation frame).
struct PilotHost<'a> {
    motion: &'a mut crate::character_motion::CharacterMotion,
    body: &'a mut magicka_sim::character_physics::CharacterBody,
    dead: bool,
}
impl crate::npc_combat::AttackOwner for PilotHost<'_> {
    fn set_movement(&mut self, movement: Vector) {
        self.body.set_movement(movement);
    }
    fn set_desired_direction(&mut self, direction: Vector) {
        self.body.set_desired_direction(direction);
    }
    fn attacking(&self) -> bool {
        self.motion.attacking
    }
    fn attack(&mut self, animation: u16, allow_rotate: bool) {
        self.motion.attack(animation, allow_rotate);
    }
    fn dashing(&self) -> bool {
        self.motion.dashing
    }
    fn dash(&mut self, animation: u16, next_dash_animation: u16) {
        self.motion.dash(animation, false);
        self.motion.next_dash_animation = next_dash_animation;
    }
    fn block(&mut self) {
        self.motion.block();
    }
    fn damage_gripped(&mut self, animation: u16) {
        self.motion.damage_gripped(animation);
    }
    fn jump(&mut self, delta: Vector, elevation: f32) {
        magicka_sim::jump::launch(
            self.body,
            self.dead,
            delta,
            elevation,
            magicka_sim::character_physics::GRAVITY[1],
        );
    }
    fn avoidance_length(&self) -> f32 {
        0.0
    }
    fn order(&self) -> u8 {
        crate::npc::ORDER_ATTACK
    }
}

/// Aim-point snap radius for choosing an attack target.
const AIM_SNAP: f32 = 4.0;
/// Fallback: nearest hostile within this distance in front of the NPC.
const FACING_REACH: f32 = 15.0;

impl<C: SceneContent> SceneWorld<C> {
    /// A living NPC that can be piloted (not dead, removed or a boss part).
    pub fn possessable(&self, handle: u16) -> bool {
        // A dying NPC (no hit points, death clip still playing) is released
        // at once so its pilot can move on.
        self.npcs
            .get(&handle)
            .is_some_and(|n| !n.dead() && n.character.combat.health.hit_points > 0.0)
            && self
                .entities
                .iter()
                .any(|e| e.handle == handle && e.added && !e.removed && !e.dead)
    }

    pub fn possess(&mut self, handle: u16) -> bool {
        if !self.possessable(handle) || self.possessed.contains_key(&handle) {
            return false;
        }
        self.possessed.insert(handle, Possession::default());
        if let Some(n) = self.npcs.get_mut(&handle) {
            n.combat.release_target();
        }
        true
    }

    pub fn release_possession(&mut self, handle: u16) {
        if self.possessed.remove(&handle).is_some() {
            if let Some(n) = self.npcs.get(&handle) {
                self.physics.bodies[n.body].set_movement([0.0; 3]);
            }
        }
    }

    pub fn set_possession_controls(&mut self, handle: u16, controls: Possession) {
        if let Some(p) = self.possessed.get_mut(&handle) {
            let boost_was_down = p.boost_was_down;
            *p = Possession {
                boost_was_down,
                ..controls
            };
        }
    }

    /// The hostile living character nearest `aim` (within the snap radius),
    /// else the nearest hostile in front of `handle`.
    fn possession_target(&self, handle: u16, aim: Vector) -> Option<u16> {
        let me = self.entities.iter().find(|e| e.handle == handle)?;
        let own = self.faction(me);
        let origin = me.position;
        let npc = self.npcs.get(&handle)?;
        let facing = self.physics.bodies[npc.body].direction();
        let hostile = |e: &&SceneEntity| {
            e.handle != handle
                && e.added
                && !e.removed
                && !e.dead
                && matches!(e.kind, EntityKind::Avatar { .. } | EntityKind::Npc { .. })
                && self.faction(e) & own == 0
        };
        let dist2 = |a: Vector, b: Vector| {
            let d = [a[0] - b[0], a[2] - b[2]];
            d[0] * d[0] + d[1] * d[1]
        };
        let near_aim = self
            .entities
            .iter()
            .filter(hostile)
            .filter(|e| dist2(e.position, aim) <= AIM_SNAP * AIM_SNAP)
            .min_by(|a, b| dist2(a.position, aim).total_cmp(&dist2(b.position, aim)));
        near_aim
            .or_else(|| {
                self.entities
                    .iter()
                    .filter(hostile)
                    .filter(|e| {
                        let d = [e.position[0] - origin[0], e.position[2] - origin[2]];
                        let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
                        l <= FACING_REACH && (d[0] * facing[0] + d[1] * facing[2]) >= 0.5 * l
                    })
                    .min_by(|a, b| dist2(a.position, origin).total_cmp(&dist2(b.position, origin)))
            })
            .map(|e| e.handle)
    }

    /// Whether the AI phase runs for `handle` this frame: unpossessed NPCs
    /// only (a pilot makes a possessed NPC's decisions).
    pub(super) fn ai_runs_for(&self, handle: u16) -> bool {
        !self.possessed.contains_key(&handle)
    }

    /// Every frame after the AI phase: steer possessed NPCs, run the held
    /// actions' abilities and drop pilots of dead/removed NPCs.
    pub(super) fn update_possessed(&mut self) {
        let handles: Vec<u16> = self.possessed.keys().copied().collect();
        if handles.is_empty() {
            return;
        }
        let combatants = self.agent_combatants();
        for handle in handles {
            if !self.possessable(handle) {
                self.possessed.remove(&handle);
                continue;
            }
            let p = self.possessed[&handle];
            let target = p.attack.and_then(|aim| self.possession_target(handle, aim));
            let Some(mut npc) = self.npcs.remove(&handle) else {
                continue;
            };
            {
                let body = &mut self.physics.bodies[npc.body];
                match p.movement {
                    Some([x, z]) => body.set_movement([x, 0.0, z]),
                    None => {
                        body.set_movement([0.0; 3]);
                        if let Some(aim) = p.aim {
                            let d = [aim[0] - body.position[0], 0.0, aim[2] - body.position[2]];
                            body.set_desired_direction(d);
                        }
                    }
                }
            }
            let view =
                crate::npc_combat::CombatView::new(&combatants, Some(self.collision.collision()));
            let owner = view.as_ref().and_then(|v| v.index(handle));
            if let (Some(view), Some(owner), Some(aim)) = (view.as_ref(), owner, p.attack) {
                let me = view.combatants[owner];
                // The hostile nearest the aim point, else a stand-in at the
                // aim point (abilities read only its position/velocity).
                let target = target.and_then(|t| view.get(t).copied()).unwrap_or(
                    crate::npc_combat::Combatant {
                        handle: 0,
                        unique_id: 0,
                        // Hostile to everyone, so enemy abilities apply.
                        faction: 0,
                        position: [aim[0], me.position[1], aim[2]],
                        velocity: [0.0; 3],
                        movement: [0.0; 3],
                        ..me
                    },
                );
                if npc.combat.current_target() != Some(target.handle) && target.handle != 0 {
                    npc.combat.change_target(target.handle, None);
                }
                let current_spell = npc.current_spell.is_some();
                let dead = me.dead;
                for (k, action) in ACTIONS.into_iter().enumerate() {
                    let held = p.actions[k];
                    if !held
                        || (action == crate::npc_combat::PilotAction::Boost && p.boost_was_down)
                    {
                        continue;
                    }
                    // Self-targeted abilities are cast on the NPC itself.
                    let target = if action == crate::npc_combat::PilotAction::SelfCast {
                        me
                    } else {
                        target
                    };
                    let mut host = PilotHost {
                        motion: &mut npc.motion,
                        body: &mut self.physics.bodies[npc.body],
                        dead,
                    };
                    match npc.combat.pilot_execute(
                        view,
                        owner,
                        &mut host,
                        action,
                        target,
                        current_spell,
                    ) {
                        crate::npc_combat::PilotOutcome::Chanting(elements) => {
                            for (element, _) in elements {
                                npc.animation_events
                                    .push(crate::avatar::AvatarEvent::NpcConjure(element));
                                npc.spell_requests
                                    .push(crate::avatar::AvatarEvent::NpcConjure(element));
                            }
                        }
                        crate::npc_combat::PilotOutcome::Executed(_) => break,
                        crate::npc_combat::PilotOutcome::Unavailable => {}
                    }
                }
            } else if npc.combat.current_target().is_some() {
                npc.combat.release_target();
            }
            self.npcs.insert(handle, npc);
            if let Some(p) = self.possessed.get_mut(&handle) {
                p.boost_was_down = p.actions[3];
            }
        }
    }

    /// Living NPCs hostile to the players, nearest `near` first (invader
    /// respawn candidates), excluding already possessed ones.
    pub fn invasion_candidates(&self, near: Vector) -> Vec<u16> {
        let player_faction = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.playing)
            .filter_map(|p| self.entities.iter().find(|e| e.handle == p.handle))
            .map(|e| self.faction(e))
            .fold(0, |a, f| a | f);
        let mut out: Vec<(f32, u16)> = self
            .npcs
            .keys()
            .copied()
            .filter(|h| self.possessable(*h) && !self.possessed.contains_key(h))
            .filter_map(|h| {
                let e = self.entities.iter().find(|e| e.handle == h)?;
                let hostile = player_faction == 0 || self.faction(e) & player_faction == 0;
                hostile.then(|| {
                    let d = [e.position[0] - near[0], e.position[2] - near[2]];
                    (d[0] * d[0] + d[1] * d[1], h)
                })
            })
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        out.into_iter().map(|(_, h)| h).collect()
    }
}
