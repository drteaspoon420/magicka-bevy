//! Live Polymorph for NPC targets: Execute(owner)'s FindClosestTarget, the
//! targeted overload, CreateNPCPolymorph's replacement with carried state
//! and OnRemove's restoration (research/polymorph.md). Avatar targets are
//! queued for PlaySession, which owns template content (PolymorphAvatar).
use super::*;
use magicka_sim::polymorph::{self as pm, PolymorphEffect};

/// PolymorphAvatar for PlaySession: `form` is a Polymorph avatar template,
/// or None to restore the avatar's own template and equipment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AvatarPolymorph {
    pub handle: u16,
    pub form: Option<&'static str>,
}

/// Polymorph.mStorage (NPCPolymorphData) plus the template key the rewrite
/// needs to rebuild the original actor.
#[derive(Clone, Debug, Default)]
pub struct PolymorphStorage {
    pub events: Option<Vec<crate::ai_event::AiEvent>>,
    pub event_index: usize,
    /// The original template's cache key (custom_hash of its file stem).
    pub template: i32,
}

impl<C: SceneContent> SceneWorld<C> {
    /// Polymorph.Execute(owner): the closest non-ethereal character to the
    /// point 3 m ahead within the radius-6 ignoreY query; a miss plays
    /// magick_fail in the Spells bank at the owner.
    pub fn polymorph_from(&mut self, owner: u16) -> bool {
        use magicka_sim::entity_query::{EntityQueryGrid, QueryEntity};
        let characters = self.combatants();
        let Some(caster) = characters.iter().find(|c| c.handle == owner) else {
            return false;
        };
        let center = pm::query_center(caster.position, caster.direction);
        let caster_position = caster.position;
        let query: Vec<_> = characters
            .iter()
            .map(|c| QueryEntity {
                position: c.position,
                radius: c.radius,
                dead: c.dead,
                shield: false,
                damageable: true,
            })
            .collect();
        let found = EntityQueryGrid::build(&query)
            .ok()
            .and_then(|g| {
                g.query(&query, center, pm::QUERY_RADIUS, false, true, |_, _| {
                    (false, [0.0; 3])
                })
                .ok()
            })
            .unwrap_or_default();
        let names: Vec<String> = found
            .iter()
            .map(|&i| self.character_name(characters[i].handle))
            .collect();
        let target = pm::find_closest(
            center,
            Some(owner),
            found.iter().zip(&names).map(|(&i, name)| pm::Candidate {
                handle: characters[i].handle,
                position: characters[i].position,
                character: true,
                ethereal: characters[i].ethereal,
                name,
            }),
        );
        let Some(target) = target else {
            self.play_cue(16, "magick_fail", Some(caster_position));
            return false;
        };
        self.polymorph_target(target)
    }

    /// Character.Name: the template name (players use their template too).
    fn character_name(&self, handle: u16) -> String {
        self.entities
            .iter()
            .find(|e| e.handle == handle)
            .and_then(|e| e.template_name.clone())
            .unwrap_or_default()
    }

    /// Polymorph.Execute(owner, target): an active match is removed (and
    /// restored) instead; otherwise the authority replaces an NPC with a
    /// random form, then the effect and cue play at the old target.
    pub fn polymorph_target(&mut self, target: u16) -> bool {
        const BANK_ADDITIONAL: u16 = 0x200;
        if let Some(effect) = self.polymorphs.take_for_recast(target) {
            self.remove_polymorph(effect);
            return true;
        }
        let avatar = self
            .players
            .iter()
            .flatten()
            .find(|p| p.handle == target)
            .map(|p| (p.body, p.template_type));
        let Some(body) = self
            .npcs
            .get(&target)
            .map(|n| n.body)
            .or(avatar.map(|a| a.0))
        else {
            return false;
        };
        let (position, forward) = {
            let b = &self.physics.bodies[body];
            (b.position, b.orientation.forward())
        };
        let mut effect_target = None;
        let mut original_type = 0;
        if let (Some((_, template_type)), true) = (avatar, self.role != NetworkRole::Client) {
            // mTypeBeforePolymorph = Template.ID. The avatar keeps its
            // identity; PlaySession swaps the template (PolymorphAvatar).
            let form = pm::AVATAR_FORMS[((self.random() * 4.0) as usize).min(3)];
            original_type = template_type;
            effect_target = Some(target);
            self.avatar_polymorphs.push(AvatarPolymorph {
                handle: target,
                form: Some(form),
            });
        } else if self.role != NetworkRole::Client {
            let form = pm::NPC_FORMS[((self.random() * 4.0) as usize).min(3)];
            original_type = self.npc_type(target);
            let mut storage = PolymorphStorage {
                template: self.npc_template(target),
                ..Default::default()
            };
            effect_target = self.replace_npc(target, form, &mut storage, true);
            if let Some(new) = effect_target {
                self.polymorph_storage.insert(new, storage);
            }
        }
        self.start_named_effect("magick_polymorph", position, forward);
        self.play_cue(BANK_ADDITIONAL, "magick_polymorph", Some(position));
        self.polymorphs
            .add(PolymorphEffect::new(effect_target, original_type));
        true
    }

    /// Polymorph.Update for every active effect, then OnRemove for those
    /// that died (TTL out, missing or dead target).
    pub(super) fn update_polymorphs(&mut self, dt: f32) {
        let mut effects = std::mem::take(&mut self.polymorphs);
        let removed = effects.update(dt, |h| self.hit_points_of(h));
        self.polymorphs = effects;
        for effect in removed {
            self.remove_polymorph(effect);
        }
    }

    /// SpellManager.ClearEffects/ClearMagicks (Nullify): every effect's
    /// OnRemove, with the same restoration gate.
    pub(super) fn clear_polymorphs(&mut self) {
        for effect in self.polymorphs.clear() {
            self.remove_polymorph(effect);
        }
    }

    fn hit_points_of(&self, handle: u16) -> Option<f32> {
        self.npcs
            .get(&handle)
            .filter(|n| !n.dead())
            .map(|n| n.character.combat.health.hit_points)
            .or_else(|| {
                self.players
                    .iter()
                    .flatten()
                    .find(|p| p.handle == handle && !p.dead())
                    .map(|p| p.character.combat.health.hit_points)
            })
    }

    /// Polymorph.OnRemove: a living target is rebuilt from its current
    /// state as the original type; the storage is discarded either way.
    fn remove_polymorph(&mut self, effect: PolymorphEffect) {
        let storage = effect
            .target
            .and_then(|t| self.polymorph_storage.remove(&t));
        let hp = effect.target.and_then(|h| self.hit_points_of(h));
        let Some((target, original_type)) = effect.restoration(hp, true) else {
            return;
        };
        if self.players.iter().flatten().any(|p| p.handle == target) {
            self.avatar_polymorphs.push(AvatarPolymorph {
                handle: target,
                form: None,
            });
            return;
        }
        let Some(mut storage) = storage else {
            return;
        };
        // GetCachedTemplate(mTypeBeforePolymorph): the logical type when it
        // names a loaded template, otherwise the saved template key.
        let key = if self.template_names.contains_key(&original_type) {
            original_type
        } else {
            storage.template
        };
        if let Some(name) = self.template_names.get(&key).cloned() {
            self.replace_npc(target, &name, &mut storage, false);
        }
    }

    fn npc_type(&self, handle: u16) -> i32 {
        match self
            .entities
            .iter()
            .find(|e| e.handle == handle)
            .map(|e| &e.kind)
        {
            Some(EntityKind::Npc { type_id, .. }) => *type_id,
            _ => 0,
        }
    }

    fn npc_template(&self, handle: u16) -> i32 {
        match self
            .entities
            .iter()
            .find(|e| e.handle == handle)
            .map(|e| &e.kind)
        {
            Some(EntityKind::Npc { template, .. }) => *template,
            _ => 0,
        }
    }

    /// CreateNPCPolymorph: a new NPC of `template` with the old UniqueID,
    /// CopyPolymorphValuesFrom and Agent.CopyPolymorphValues, the feet kept
    /// through MoveTo with the old orientation, then the old actor
    /// Terminate(killItems: false). `entering` saves the AI events into the
    /// storage; otherwise the stored events return to the new actor.
    fn replace_npc(
        &mut self,
        old: u16,
        template: &str,
        storage: &mut PolymorphStorage,
        entering: bool,
    ) -> Option<u16> {
        use crate::spawn_action::SpawnHost;
        use crate::spawn_action::SpawnLevel;
        let (unique_id, logical_type, entity_faction) = {
            let e = self.entities.iter().find(|e| e.handle == old)?;
            let EntityKind::Npc { type_id, .. } = e.kind else {
                return None;
            };
            (e.unique_id, type_id, e.faction)
        };
        self.load_character_template(Some(template)).ok()?;
        let key = custom_hash(template);
        let handle = self.acquire_npc().ok()?.handle;
        if self
            .initialize_npc(handle, key, 0, [0.0; 3], unique_id)
            .is_err()
            || !self.npcs.contains_key(&handle)
        {
            self.entities.retain(|e| e.handle != handle);
            return None;
        }
        // mType = iSource.Type; mFaction = iSource.mFaction.
        if let Ok(e) = self.entity_mut(handle) {
            e.kind = EntityKind::Npc {
                type_id: logical_type,
                template: key,
            };
            e.faction = entity_faction;
        }
        let source = self.npcs.get(&old)?.clone();
        let old_body = self.physics.bodies[source.body].clone();
        let summon = source.summon;
        let fear = source.character.fear;
        let charm = source.character.charm;
        {
            let n = self.npcs.get_mut(&handle)?;
            n.faction_override = source.faction_override;
            let sc = &source.character.combat;
            let health = &mut n.character.combat.health;
            health.hit_points = pm::transferred_hit_points(
                sc.health.hit_points,
                sc.health.max_hit_points,
                health.max_hit_points,
            );
            n.character.combat.time_since_last_damage = sc.time_since_last_damage;
            if source.character.ethereal {
                n.character.set_ethereal(true);
            }
        }
        if fear.is_feared(false) {
            let by = fear.by.map(magicka_sim::fear::FearSource::Character);
            self.fear_character(
                handle,
                by.unwrap_or(magicka_sim::fear::FearSource::Position(fear.position)),
            );
            if let Some(n) = self.npcs.get_mut(&handle) {
                n.character.fear.timer = fear.timer;
            }
        }
        {
            let n = self.npcs.get_mut(&handle)?;
            let mut events = Vec::new();
            for effect in source.character.combat.statuses.slots() {
                let _ = n.character.add_status_effect(*effect, None, &mut events);
            }
        }
        if let Some((master, kind)) = summon {
            let list = self.summons.entry(master).or_default();
            let mut overkill = list.on_despawned(old);
            overkill.extend(list.on_spawned(handle, kind));
            if let Some(n) = self.npcs.get_mut(&old) {
                n.summon = None;
            }
            if let Some(n) = self.npcs.get_mut(&handle) {
                n.summon = Some((master, kind));
            }
            for h in overkill {
                self.overkill(h);
            }
        }
        if let Some(c) = charm.filter(|c| c.timer > f32::from_bits(1)) {
            self.charm_character(handle, c.owner, c.timer);
        }
        // Agent.CopyPolymorphValues: SetOrder (priority ability -1, no
        // events), retained timing and waypoint; path/targets cleared by the
        // fresh agent. Leader/flocking/wander state is not reconstructed.
        {
            let n = self.npcs.get_mut(&handle)?;
            n.set_order(
                source.ai.order,
                source.react_to,
                source.reaction_order,
                source.reaction_trigger,
                None,
            );
            n.combat.priority_target = source.combat.priority_target;
            n.combat.priority_ability = -1;
            n.time_since_update = source.time_since_update;
            n.ai.waypoint = source.ai.waypoint;
            if entering {
                storage.events = source.ai.events().map(<[_]>::to_vec);
                storage.event_index = source.ai.current_event;
            } else {
                n.ai.set_events(storage.events.take());
                n.ai.current_event = storage.event_index;
            }
        }
        let new_body = self.npcs[&handle].body;
        let (new_radius, new_length) = {
            let b = &self.physics.bodies[new_body];
            (b.radius, b.length)
        };
        let position = pm::replacement_position(
            old_body.position,
            old_body.radius,
            old_body.length,
            new_radius,
            new_length,
        );
        let body = &mut self.physics.bodies[new_body];
        body.move_to(position, old_body.orientation);
        body.set_desired_direction(old_body.desired_direction());
        // Polymorph.cs: Terminate(iKillItems: false, iNetwork: false).
        self.terminate_character_keeping_items(old);
        SpawnHost::add_entity(self, handle).ok()?;
        Some(handle)
    }
}
