//! Per-wizard draw inputs for every wizard this machine shows: the local
//! simulation's own (host/offline) and the host's records for mirrored
//! wizards (network clients). The viewer's self-shield, weapon-trail and
//! arcane-blade systems read these accessors, so the same draw paths run
//! for remote wizards (network_world.rs applies the host's records).
use super::PlaySession;
use crate::avatar::ICE_BLADE_TRAIL;
use magicka_sim::character_health::SelfShield;
use magicka_sim::impact_motion::Vector;

/// A mirrored wizard's weapon trails (the host's `AvatarPresentation`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RemoteAvatarVisuals {
    pub ice_blade_trail: Option<Box<[Vector; ICE_BLADE_TRAIL]>>,
    pub arcane_blades: Vec<RemoteBlade>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteBlade {
    pub id: u32,
    pub color: Vector,
    pub vertices: Box<[Vector; crate::arcane_blade::VERTICES]>,
}

/// ArcaneBlade.RenderData inputs of one blade.
#[derive(Clone, Debug, PartialEq)]
pub struct ArcaneBladeDraw {
    pub owner: u16,
    pub id: u32,
    pub vertices: [Vector; crate::arcane_blade::VERTICES],
    /// Colour centre = colour + 1, edge = colour.
    pub center: Vector,
    pub edge: Vector,
}

/// Character.SelfShield draw inputs of one character.
#[derive(Clone, Debug)]
pub struct SelfShieldDraw<'a> {
    pub handle: u16,
    pub position: Vector,
    pub direction: Vector,
    pub shield: &'a SelfShield,
}

impl PlaySession {
    /// Live IceBlade trails by owner handle: this machine's simulated
    /// wizards and the host's records for mirrored ones.
    pub fn ice_blade_trails(&self) -> Vec<(u16, [Vector; ICE_BLADE_TRAIL])> {
        let mut out = Vec::new();
        for p in self.world.players.iter().flatten() {
            if let Some(trail) = p.ice_blade_trail() {
                out.push((p.handle, *trail));
            } else if let Some(trail) = self
                .remote_avatars
                .get(&p.handle)
                .and_then(|r| r.ice_blade_trail.as_deref())
            {
                out.push((p.handle, *trail));
            }
        }
        out
    }

    /// Live arcane blades of every wizard shown here.
    pub fn arcane_blade_draws(&self) -> Vec<ArcaneBladeDraw> {
        let mut out = Vec::new();
        for p in self.world.players.iter().flatten() {
            for blade in &p.arcane_blades {
                let (center, edge) = blade.colors();
                out.push(ArcaneBladeDraw {
                    owner: p.handle,
                    id: blade.id,
                    vertices: blade.vertices,
                    center,
                    edge,
                });
            }
            if let Some(remote) = self.remote_avatars.get(&p.handle) {
                for blade in &remote.arcane_blades {
                    out.push(ArcaneBladeDraw {
                        owner: p.handle,
                        id: blade.id,
                        vertices: *blade.vertices,
                        center: blade.color.map(|c| c + 1.0),
                        edge: blade.color,
                    });
                }
            }
        }
        out
    }

    /// Characters with an active SelfShield (any kind): every wizard shown
    /// here and the live NPCs, with the body pose the projection needs.
    pub fn self_shields(&self) -> Vec<SelfShieldDraw<'_>> {
        let bodies = &self.world.physics.bodies;
        let players = self
            .world
            .players
            .iter()
            .flatten()
            .map(|p| (p.handle, &p.character, &bodies[p.body]));
        let npcs = self
            .world
            .npcs
            .iter()
            .map(|(&handle, n)| (handle, &n.character, &bodies[n.body]));
        players
            .chain(npcs)
            .filter(|(_, character, _)| character.combat.health.shield.active())
            .map(|(handle, character, body)| SelfShieldDraw {
                handle,
                position: body.position,
                direction: body.direction(),
                shield: &character.combat.health.shield,
            })
            .collect()
    }
}
