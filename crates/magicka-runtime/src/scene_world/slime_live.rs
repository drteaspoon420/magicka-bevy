//! SpawnSlime and SpawnSlimeOverkill: faction-inheriting NPC creation, without
//! Summoned bookkeeping. Evidence and pending verification: spawn-slime.md.
use super::*;
use magicka_sim::{spawn_slime, spells::Elements};

#[derive(Clone, Copy, Debug, Default)]
pub struct SlimeCast {
    pub overkill: bool,
    pub elements: Elements,
    pub spawn_animation: u16,
    pub idle_animation: u16,
}

impl<C: SceneContent> SceneWorld<C> {
    /// Owner-only native ability: ordinary custom player casts use the native
    /// singleton defaults, not the elements typed in the magick combination.
    pub fn spawn_slimes(&mut self, owner: u16, cast: SlimeCast) -> Result<Vec<u16>, WorldError> {
        use crate::spawn_action::SpawnLevel;
        if self.role == NetworkRole::Client {
            return Ok(Vec::new());
        }
        let body = self
            .character_body(owner)
            .ok_or("slime parent is not a character")?;
        let origin = self.physics.bodies[body].position;
        let parent = self
            .character_ref(owner)
            .ok_or("missing slime parent")?
            .definition
            .id
            .clone();
        let batch = spawn_slime::batch(&parent, cast.elements, cast.overkill, || {
            ((self.random() * 11.0) as usize).min(10)
        })
        .ok_or("slime element mask has no supported element")?;
        for slime in &batch {
            self.load_character_template(Some(slime.template()))?;
        }
        let mut handles = Vec::with_capacity(batch.len());
        for slime in batch {
            let radial = f64::from(self.random());
            let angular = f64::from(self.random());
            let point = spawn_slime::position(origin, radial, angular);
            let point = self
                .nav
                .as_ref()
                .and_then(|nav| nav.nearest(point, magicka_sim::navmesh::movement::DEFAULT))
                .map_or(point, |(_, p, _)| p);
            handles.push(self.spawn_faction_child(
                owner,
                slime.template(),
                point,
                cast.spawn_animation,
                cast.idle_animation,
            )?);
        }
        Ok(handles)
    }
    /// Shared CreateEntities/SpawnClone lifecycle; neither calls Summoned.
    pub(super) fn spawn_faction_child(
        &mut self,
        owner: u16,
        template: &'static str,
        point: [f32; 3],
        spawn_animation: u16,
        idle_animation: u16,
    ) -> Result<u16, WorldError> {
        use crate::spawn_action::{SpawnHost, SpawnLevel};
        self.load_character_template(Some(template))?;
        let body = self
            .character_body(owner)
            .ok_or("missing child owner body")?;
        let direction = self.physics.bodies[body].direction();
        let faction = self
            .entities
            .iter()
            .find(|e| e.handle == owner)
            .map(|e| self.faction(e))
            .ok_or("missing child owner entity")?;
        let handle = self.acquire_npc()?.handle;
        self.initialize_npc(handle, custom_hash(template), 0, point, 0)?;
        self.configure(
            handle,
            &SpawnConfig {
                hit_points: None,
                on_death: 0,
                on_damage: 0,
                wander_speed: None,
                target_area: None,
                order: crate::spawn_action::SpawnOrder {
                    order: crate::npc::ORDER_ATTACK,
                    react_to: crate::npc::REACT_PROXIMITY,
                    reaction: crate::npc::ORDER_ATTACK,
                    priority_target: 0,
                    priority_ability: 0,
                    trigger: 0,
                    events: &[],
                },
                // CreateEntities reads owner Direction for the server message,
                // but does not assign it to the initialized local child body.
                orientation: magicka_content::skinned::IDENTITY,
                desired_direction: [0.0, 0.0, -1.0],
                remove_after_death: true,
                dialog: 0,
                force_camera: None,
                force_nav_mesh: None,
                cannot_die_without_explicit_kill: None,
            },
        )?;
        if let Some(npc) = self.npcs.get_mut(&handle) {
            npc.faction_override = Some(faction);
            npc.character.combat.health.hit_points = npc.character.combat.health.max_hit_points;
            // No Summoned(owner) call: no master link, cap or Nullify kill.
            npc.summon = None;
        }
        let entity = self.entity_mut(handle)?;
        entity.faction = faction;
        entity.hit_points = entity.max_hit_points;
        if spawn_slime::needs_resurrection(spawn_animation) {
            // The existing spawn host has no RessurectionState host yet.
            // Preserve its clip request while that state remains pending.
            self.force_animation(handle, spawn_animation)?;
            self.begin_resurrection(handle, spawn_animation)?;
        }
        if idle_animation != 0 {
            SpawnHost::set_special_idle(self, handle, idle_animation)?;
        }
        SpawnHost::add_entity(self, handle)?;
        if self.role == NetworkRole::Server {
            self.faction_child_spawns.push(FactionChildSpawn {
                handle,
                template,
                position: point,
                direction,
                spawn_animation,
                idle_animation,
            });
        }
        Ok(handle)
    }
}

/// Native SpawnNPC action fields. Transport/ruleset integration is pending.
#[derive(Clone, Debug)]
pub struct FactionChildSpawn {
    pub handle: u16,
    pub template: &'static str,
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub spawn_animation: u16,
    pub idle_animation: u16,
}
