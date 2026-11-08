//! Native EtherealClone owner casting and the authorized custom-player caster
//! extension. The child is the native template, not a snapshot of the caster.
use super::*;
use magicka_sim::ethereal_clone as clone;

#[derive(Clone, Copy, Debug, Default)]
pub struct CloneCast {
    pub spawn_animation: u16,
    pub idle_animation: u16,
}

impl<C: SceneContent> SceneWorld<C> {
    /// Native Execute(owner): only the dungeon human adventurer qualifies.
    pub fn ethereal_clone_from(
        &mut self,
        owner: u16,
        cast: CloneCast,
    ) -> Result<Option<u16>, WorldError> {
        self.spawn_ethereal_clone(owner, cast, false)
    }

    /// Rewrite extension: a player cast uses the same fixed native clone
    /// template even when the player is not the dungeon adventurer.
    pub fn ethereal_clone_player(&mut self, owner: u16) -> Result<Option<u16>, WorldError> {
        let player = self
            .players
            .iter()
            .flatten()
            .any(|p| p.handle == owner && p.playing);
        if !player {
            return Ok(None);
        }
        self.spawn_ethereal_clone(owner, CloneCast::default(), true)
    }

    fn spawn_ethereal_clone(
        &mut self,
        owner: u16,
        cast: CloneCast,
        player_extension: bool,
    ) -> Result<Option<u16>, WorldError> {
        use crate::spawn_action::SpawnLevel;
        if self.role == NetworkRole::Client {
            return Ok(None);
        }
        let parent = self
            .character_ref(owner)
            .ok_or("clone parent is not a character")?;
        if !player_extension && !clone::native_eligible(&parent.definition.id) {
            return Ok(None);
        }
        let body = self
            .character_body(owner)
            .ok_or("missing clone parent body")?;
        let origin = self.physics.bodies[body].position;
        self.load_character_template(Some(clone::TEMPLATE))?;
        let x = f64::from(self.random());
        let z = f64::from(self.random());
        let point = clone::position(origin, x, z);
        let point = self
            .nav
            .as_ref()
            .and_then(|nav| nav.nearest(point, magicka_sim::navmesh::movement::DEFAULT))
            .map_or(point, |(_, p, _)| p);
        self.spawn_faction_child(
            owner,
            clone::TEMPLATE,
            point,
            cast.spawn_animation,
            cast.idle_animation,
        )
        .map(Some)
    }
}
