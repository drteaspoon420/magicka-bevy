//! Spell-effect bookkeeping for BossSpellCasterZone owners. These owners are
//! physics entities, never NPCs; inactive CharacterBody entries supply the
//! shared spell engine's pose API and do not join character simulation/AI.
use super::*;
use crate::{
    avatar::{AvatarEvent, CharacterSpellHost, SpellEffect},
    character::RuntimeCharacter,
};
use magicka_sim::{damage::CastType, resistance::Resistance, spray_pulse::SpraySource};

pub struct BossCaster {
    pub body: usize,
    pub source: SpraySource,
    state: RuntimeCharacter,
    current: Option<SpellEffect>,
    definition: Option<(CastType, Spell)>,
    events: Vec<AvatarEvent>,
    rail_kills: Vec<magicka_sim::railgun::RailId>,
    rng: u64,
}
impl BossCaster {
    fn new(handle: u16, body: usize, radius: f32) -> Self {
        // Neutral storage for shared spell power/lifetime. No retail template,
        // NPC, character status eligibility or equipment is assigned to a zone.
        let definition = crate::CharacterDefinition {
            id: "boss_spell_zone".into(),
            faction: 0,
            ethereal: false,
            looks_ethereal: false,
            can_have_status: false,
            resistances: [Resistance::IDENTITY; 11],
            max_hit_points: 1.,
            health_bars: 0,
            event_conditions: Default::default(),
            hit_tolerance: 0.,
            knockdown_tolerance: 0.,
            max_panic: 0.,
            fearless: true,
            uncharmable: true,
            nonslippery: true,
            length: 1.5,
            radius,
            mass: 1.,
            speed: 0.,
            turn_speed: 0.,
            regeneration: 0,
            bleed_rate: 0.,
            stun_time: 0.,
            gib_count: 0,
            gibs: vec![],
            blood: 5,
            attached_sounds: Vec::new(),
            zap_modifier: 1.,
            break_free_strength: 0.,
            undying: false,
            undie_time: 0.0,
            undie_hit_points: 0.0,
            model_tints: Vec::new(),
            score_value: 0,
            display_id: String::new(),
        };
        Self {
            body,
            source: SpraySource {
                owner_handle: handle,
                owner_position: [0.; 3],
                owner_direction: [0., 0., -1.],
                cast_position: [0.; 3],
                cast_direction: [0., 0., -1.],
                weapon_position: [0.; 3],
                weapon_direction: [0., 0., -1.],
            },
            state: RuntimeCharacter::new(definition),
            current: None,
            definition: None,
            events: vec![],
            rail_kills: vec![],
            rng: u64::from(handle) + 1,
        }
    }
    fn host(&mut self, handle: u16, time: f64) -> CharacterSpellHost<'_> {
        CharacterSpellHost {
            owner_is_character: false,
            avatar: false,
            handle,
            equipment: &[Resistance::IDENTITY; 11],
            character: &mut self.state,
            current_spell: &mut self.current,
            current_spell_definition: &mut self.definition,
            events: &mut self.events,
            rail_kills: &mut self.rail_kills,
            source: Some(self.source),
            play_time: time,
        }
    }
}
impl<C: SceneContent> SceneWorld<C> {
    pub(super) fn add_boss_caster(&mut self, handle: u16, radius: f32) {
        let mut body = magicka_sim::character_physics::CharacterBody::new(radius, 1.5, 1., 0.);
        body.active = false;
        body.apply_gravity = false;
        let index = self.physics.bodies.len();
        self.physics.bodies.push(body);
        self.boss_casters
            .insert(handle, BossCaster::new(handle, index, radius));
    }
    pub(super) fn set_boss_caster_source(&mut self, handle: u16, source: SpraySource) {
        let Some(c) = self.boss_casters.get_mut(&handle) else {
            return;
        };
        c.source = source;
        let body = &mut self.physics.bodies[c.body];
        body.position = source.owner_position;
        body.orientation = magicka_sim::character_physics::Orientation::from_forward_up(
            source.owner_direction,
            [0., 1., 0.],
        );
    }
    pub(super) fn cast_boss_spell(
        &mut self,
        handle: u16,
        spell: Spell,
        cast: CastType,
        power: f32,
    ) {
        let Some(c) = self.boss_casters.get_mut(&handle) else {
            return;
        };
        c.state.spell_power = power;
        c.host(handle, self.play_time).cast(spell, cast, true);
        self.log.push(format!(
            "boss spell zone={handle} cast={cast:?} elements={:?} power={power}",
            spell.element
        ));
    }
    pub(super) fn stop_boss_spell(&mut self, handle: u16) {
        if let Some(c) = self.boss_casters.get_mut(&handle) {
            c.host(handle, self.play_time).end();
        }
    }
    pub(super) fn update_boss_spells(&mut self, dt: f32) {
        let handles: Vec<_> = self.boss_casters.keys().copied().collect();
        for handle in handles {
            let Some(mut c) = self.boss_casters.remove(&handle) else {
                continue;
            };
            if c.current.is_some() {
                let combatants = self.combatants();
                let (mut targets, _) = self.spray_entities();
                let shields: Vec<u16> = self.shields.iter().map(|s| s.handle).collect();
                let (before, after) = self.physics.bodies.split_at_mut(c.body);
                let (body, after) = after.split_first_mut().unwrap();
                let mut effects = SceneSprayEffects {
                    missiles: &mut self.missiles,
                    physics_damage: &mut self.physics_damage,
                    dispensers: &mut self.dispensers,
                    entities: &mut self.entities,
                    elemental_eggs: &mut self.elemental_eggs,
                    caster_boosts: vec![],
                    bodies_before: before,
                    bodies_after: after,
                    caster_index: c.body,
                    combatants,
                    npcs: &mut self.npcs,
                    players: &mut self.players,
                    triggers: &mut self.pending_triggers,
                    shields: &mut self.shields,
                    mines: &mut self.mines,
                    grease: &mut self.grease,
                    barriers: &mut self.barriers,
                    death: &mut self.death,
                    jormungandr: &mut self.jormungandr,
                    death_damage_features: self.role != NetworkRole::Client,
                    barrier_events: vec![],
                };
                if let Ok(world) = crate::spray_world::RuntimeSprayWorld::new(
                    Some(self.collision.collision()),
                    &mut targets,
                    &shields,
                    &mut effects,
                ) {
                    let mut world = world.with_rails(&mut self.rails);
                    let cast = c.definition.map_or(CastType::Blast, |d| d.0);
                    let mut rng = c.rng;
                    c.host(handle, self.play_time)
                        .update(body, dt, &mut world, cast, true, false, &mut rng);
                    c.rng = rng;
                }
                let events = std::mem::take(&mut effects.barrier_events);
                for (slot, events) in events {
                    self.barrier_events(slot, events);
                }
            }
            for rail in c.rail_kills.drain(..) {
                self.rails.kill(rail);
            }
            self.boss_casters.insert(handle, c);
        }
        self.apply_boss_spell_events();
    }
    pub(super) fn apply_boss_spell_events(&mut self) {
        let requests: Vec<_> = self
            .boss_casters
            .iter_mut()
            .flat_map(|(&h, c)| {
                std::mem::take(&mut c.events)
                    .into_iter()
                    .map(move |e| (h, e))
            })
            .collect();
        self.apply_owner_spell_events(requests);
    }
}
