//! Session host of ChantSpellManager (research/chant-spells.md).
use super::*;
use crate::chant_spells::{ChantRequest, OwnerFacts};
use magicka_sim::spells::QueueEvent;

impl PlaySession {
    /// TryAddToQueue's ChantSpellManager.Add / Merge calls for `slot`.
    /// `queue_before` is the queue length when the orb was constructed.
    pub(super) fn chant_events(
        &mut self,
        slot: usize,
        events: Vec<QueueEvent>,
        queue_before: usize,
    ) {
        let Some(avatar) = self.world.players[slot].as_ref() else {
            return;
        };
        let (owner, body) = (avatar.handle, avatar.body);
        let cast = avatar.spray_source(&self.world.physics.bodies[body]);
        let mut queue_len = queue_before;
        for event in events {
            match event {
                QueueEvent::ChantAdded(element) => {
                    let world = &mut self.world;
                    let mut random = || world.random();
                    self.chants.add(
                        element,
                        owner,
                        queue_len,
                        cast.cast_position,
                        cast.owner_position,
                        cast.owner_direction,
                        &mut random,
                        &mut self.effects,
                    );
                    queue_len += 1;
                }
                QueueEvent::ChantMerge { a, b, out } => self.chants.merge(owner, a, b, out),
                _ => {}
            }
        }
    }

    /// ChantSpellManager.Update (SpellManager.Update) and the orbs' lights.
    pub(super) fn update_chants(&mut self, dt: f32) {
        use magicka_sim::status::Status;
        let mut requests = Vec::new();
        {
            let world = &mut self.world;
            let players: Vec<(u16, OwnerFacts)> = world
                .players
                .iter()
                .flatten()
                .map(|p| {
                    (
                        p.handle,
                        OwnerFacts {
                            position: world.physics.bodies[p.body].position,
                            chanting: !p.conjure.queue.is_empty(),
                            dead: p.dead(),
                            polymorphed: p.polymorphed,
                            wet: p.character.combat.statuses.has(Status::Wet)
                                && !p.character.wet_lightning_passive,
                            queue_len: p.conjure.queue.len(),
                        },
                    )
                })
                .collect();
            let mut facts = |h: u16| {
                players
                    .iter()
                    .find(|(handle, _)| *handle == h)
                    .map(|(_, f)| *f)
            };
            let mut random = || world.random();
            self.chants.update(
                dt,
                &mut facts,
                &mut random,
                &mut self.effects,
                &mut requests,
            );
        }
        self.chants.update_lights(dt);
        for request in requests {
            match request {
                ChantRequest::WetLightning { owner, .. } => {
                    // The first Lightning leaves the queue; the self damage
                    // and bolt are not reproduced yet.
                    if let Some(p) = self
                        .world
                        .players
                        .iter_mut()
                        .flatten()
                        .find(|p| p.handle == owner)
                    {
                        if let Some(i) =
                            p.conjure.queue.iter().position(|s| {
                                s.element.has(magicka_sim::spells::Elements::LIGHTNING)
                            })
                        {
                            p.conjure.queue.remove(i);
                        }
                    }
                }
            }
        }
    }
}
