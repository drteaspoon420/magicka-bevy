//! Rewrite-only, ordinary NPC attack captures with exact projectile controls.
use super::*;
use magicka_runtime::particles::EffectRef;
use std::collections::HashMap;

pub(super) const EARTH: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "before"),
    (1, [0., 0.5], Act::NpcEarthCaster, "spawn"),
    (600, [0., 0.5], Act::NpcProjectileWait, "flight"),
    (
        30,
        [0., 0.5],
        Act::NpcProjectileVisible,
        "projectile_visible",
    ),
    (30, [0., 0.5], Act::NpcModelHidden, "model_hidden"),
    (30, [0., 0.5], Act::NpcTrailHidden, "trail_hidden"),
    (30, [0., 0.5], Act::NpcProjectileHidden, "projectile_hidden"),
    (
        30,
        [0., 0.5],
        Act::NpcProjectileVisible,
        "projectile_restored",
    ),
];
pub(super) const LASER: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "before"),
    (1, [0., 0.5], Act::NpcBeholder, "spawn"),
    (600, [0., 0.5], Act::NpcProjectileWait, "flight"),
    (
        30,
        [0., 0.5],
        Act::NpcProjectileVisible,
        "projectile_visible",
    ),
    (30, [0., 0.5], Act::NpcModelHidden, "model_hidden"),
    (30, [0., 0.5], Act::NpcTrailHidden, "trail_hidden"),
    (30, [0., 0.5], Act::NpcProjectileHidden, "projectile_hidden"),
    (
        30,
        [0., 0.5],
        Act::NpcProjectileVisible,
        "projectile_restored",
    ),
];
pub(super) const BOMB: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "before"),
    (1, [0., 0.5], Act::NpcBomber, "spawn"),
    (600, [0., 0.5], Act::NpcProjectileWait, "flight"),
    (
        30,
        [0., 0.5],
        Act::NpcProjectileVisible,
        "projectile_visible",
    ),
    (30, [0., 0.5], Act::NpcModelHidden, "model_hidden"),
    (30, [0., 0.5], Act::NpcTrailHidden, "trail_hidden"),
    (30, [0., 0.5], Act::NpcProjectileHidden, "projectile_hidden"),
    (
        30,
        [0., 0.5],
        Act::NpcProjectileVisible,
        "projectile_restored",
    ),
];

#[derive(Resource, Default)]
pub(super) struct Audit {
    owner: Option<u16>,
    origins: HashMap<u32, [f32; 3]>,
    pub(super) missile: Option<u32>,
    pub(super) hidden: bool,
    pub(super) hide_model: bool,
    hide_particles: bool,
    refs: Vec<EffectRef>,
    template: &'static str,
    travel: f32,
}
impl Audit {
    /// Only ordinary authored AI and animation callbacks create the missiles.
    /// Pin after observing movement and emitted particles, rather than forcing
    /// a launch or injecting a replacement visual into the session.
    pub(super) fn prepare(&mut self, session: &mut PlaySession, act: Option<Act>) -> bool {
        self.hidden = act == Some(Act::NpcProjectileHidden);
        self.hide_model = matches!(act, Some(Act::NpcProjectileHidden | Act::NpcModelHidden));
        self.hide_particles = matches!(act, Some(Act::NpcProjectileHidden | Act::NpcTrailHidden));
        let template = match act {
            Some(Act::NpcEarthCaster) => Some("Boss_Cult_Fire"),
            Some(Act::NpcBeholder) => Some("Beholder_benign"),
            Some(Act::NpcBomber) => Some("Goblin_bomber"),
            _ => None,
        };
        if let Some(template) = template {
            session.stop_level_effects();
            session.world.flags.cutscene = false;
            session.world.flags.input_limited = false;
            self.owner = Some(
                session
                    .spawn_npc(template, "attack", 8.)
                    .expect("NPC projectile fixture"),
            );
            self.template = template;
        }
        if act == Some(Act::NpcProjectileWait) && self.missile.is_none() {
            for missile in session
                .world
                .missiles
                .iter()
                .filter(|m| Some(m.body.owner) == self.owner)
            {
                let origin = self
                    .origins
                    .entry(missile.body.id)
                    .or_insert(missile.body.position);
                let travel = (0..3)
                    .map(|i| (missile.body.position[i] - origin[i]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                let refs = session.missile_effect_refs(missile.body.id);
                let particles = session
                    .effects
                    .particles
                    .active()
                    .iter()
                    .zip(session.effects.particles.active_sources())
                    .filter(|(p, source)| {
                        p.radius > 0.
                            && p.normalized_lifetime < 1.
                            && source.is_some_and(|r| refs.contains(&r))
                    })
                    .count();
                if travel > 2.5 && particles > 0 {
                    self.missile = Some(missile.body.id);
                    self.refs = refs;
                    self.travel = travel;
                    break;
                }
            }
        }
        if matches!(
            act,
            Some(
                Act::NpcProjectileVisible
                    | Act::NpcProjectileHidden
                    | Act::NpcModelHidden
                    | Act::NpcTrailHidden
            )
        ) {
            assert!(
                self.missile.is_some(),
                "authored {} attack produced no travelling missile with particles",
                self.template
            );
        }
        self.missile.is_some()
    }

    pub(super) fn copy_particles(
        &self,
        session: &PlaySession,
        frame: &mut crate::particles::ParticleFrame,
    ) {
        frame.0.extend(
            session
                .effects
                .particles
                .active()
                .iter()
                .zip(session.effects.particles.active_sources())
                .filter(|(_, source)| {
                    !self.hide_particles || !source.is_some_and(|r| self.refs.contains(&r))
                })
                .map(|(p, _)| *p),
        );
    }

    pub(super) fn report(&self, session: &PlaySession) -> serde_json::Value {
        let missile = self
            .missile
            .and_then(|id| session.world.missiles.iter().find(|m| m.body.id == id));
        let particles: Vec<_> = session.effects.particles.active().iter()
            .zip(session.effects.particles.active_sources())
            .filter(|(p, source)| p.radius > 0. && p.normalized_lifetime < 1. && source.is_some_and(|r| self.refs.contains(&r)))
            .map(|(p, _)| serde_json::json!({"position":p.position,"radius":p.radius,"sprite":p.sprite}))
            .collect();
        let effects: Vec<_> = session.effects.presentation().into_iter()
            .filter(|p| self.refs.contains(&p.reference))
            .map(|p| serde_json::json!({"definition":p.definition,"transform":p.transform,"elapsed":p.elapsed}))
            .collect();
        serde_json::json!({
            "template":self.template,"owner":self.owner,"missile":self.missile,"hidden":self.hidden,
            "hide_model":self.hide_model,"hide_particles":self.hide_particles,
            "play_time":session.play_time,"travel":self.travel,
            "position":missile.map(|m| m.body.position),"velocity":missile.map(|m| m.body.velocity),
            "model":missile.and_then(|m| m.body.model.map(magicka_runtime::missiles::model_name)),
            "item_model":missile.and_then(|m| m.body.item_model.as_deref()),
            "particles":particles,"effects":effects,
        })
    }
}
