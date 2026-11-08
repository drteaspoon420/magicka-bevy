//! Native three-Vlad ring and the custom player's cursor-target extension.
//! See research/vlad-zap.md for the distinction and remaining host gaps.
use super::*;
use magicka_sim::lightning::LightningTarget;

/// A boss host identifies actual VladCharacter instances and supplies their
/// CastSource translation. Template names alone do not establish native type.
#[derive(Clone, Copy, Debug)]
pub struct VladParticipant {
    pub handle: u16,
    pub cast_source: [f32; 3],
}

impl<C: SceneContent> SceneWorld<C> {
    /// Native Execute(owner): exactly three VladCharacter instances in the
    /// 12 m entity query form a directed cycle in entity order. Zero, one,
    /// two or more than three still play the cue and shake, but cast no bolts.
    pub fn vlad_zap_native(&mut self, owner: u16, vlads: &[VladParticipant]) -> bool {
        let Some(body) = self.character_body(owner) else {
            return false;
        };
        let at = self.physics.bodies[body].position;
        let combatants = self.combatants();
        let Some(view) = crate::npc_combat::CombatView::new(&combatants, None) else {
            return false;
        };
        let nearby: Vec<_> = view
            .entities(at, 12.0, false)
            .into_iter()
            .filter_map(|i| {
                let c = &combatants[i];
                vlads.iter().find(|v| v.handle == c.handle).map(|v| {
                    (
                        v.cast_source,
                        LightningTarget {
                            handle: c.handle,
                            position: c.position,
                            dead: c.dead,
                            eligible: !c.ethereal,
                            frozen: c.status_mask & magicka_sim::status::Status::Frozen as u16 != 0,
                            ends_chain: false,
                        },
                    )
                })
            })
            .collect();
        let casts = if nearby.len() == 3 {
            (0..3)
                .map(|i| (nearby[i].0, nearby[(i + 1) % 3].1))
                .collect()
        } else {
            Vec::new()
        };
        self.execute_vlad_zap(owner, at, casts, 850.0)
    }

    /// Authorized rewrite extension: use the established mouse-target rule
    /// to cast one native-payload bolt from the player, without boss identity.
    pub fn vlad_zap_player(&mut self, owner: u16, target: u16) -> bool {
        if !self.players.iter().flatten().any(|p| p.handle == owner) {
            return false;
        }
        self.targeted_bolt(owner, target, None, 850.0)
    }

    /// The Vlad zap's bolt from any character to one target: from `from`,
    /// else a player's staff orb, else the owner's position; `amount`
    /// Lightning damage (850 for VladZap). Also the mod
    /// `Spells.Lightning(owner, {target})`.
    pub fn targeted_bolt(
        &mut self,
        owner: u16,
        target: u16,
        from: Option<[f32; 3]>,
        amount: f32,
    ) -> bool {
        let Some(body) = self.character_body(owner) else {
            return false;
        };
        let at = self.physics.bodies[body].position;
        let orb = self
            .players
            .iter()
            .flatten()
            .find(|p| p.handle == owner)
            .map(|p| [p.staff_orb[12], p.staff_orb[13], p.staff_orb[14]])
            .filter(|o| *o != [0.0; 3]);
        let source = from.or(orb).unwrap_or(at);
        let Some(c) = self.combatants().into_iter().find(|c| c.handle == target) else {
            return false;
        };
        let target = LightningTarget {
            handle: c.handle,
            position: c.position,
            dead: c.dead,
            eligible: !c.ethereal,
            frozen: c.status_mask & magicka_sim::status::Status::Frozen as u16 != 0,
            ends_chain: false,
        };
        self.execute_vlad_zap(owner, at, vec![(source, target)], amount)
    }

    fn execute_vlad_zap(
        &mut self,
        owner: u16,
        at: [f32; 3],
        casts: Vec<([f32; 3], LightningTarget)>,
        amount: f32,
    ) -> bool {
        use magicka_sim::damage::{attack, Damage};
        let Some(character) = self.character_ref(owner) else {
            return false;
        };
        let mut hits = magicka_sim::spray_pulse::HitList::default();
        if !character
            .combat
            .statuses
            .has(magicka_sim::status::Status::Wet)
        {
            hits.add(owner);
        }
        let mut damages = [Damage::default(); 5];
        damages[0] = Damage {
            attack: attack::DAMAGE,
            element: magicka_sim::spells::Elements::LIGHTNING.0,
            amount,
            magnitude: 1.0,
        };
        let play_time = self.play_time;
        let color = LIGHTNING_COLOR.map(|c| c * 2.0);
        let mut bolts = Vec::new();
        let mut rng = self.rng;
        self.with_rail_world(|world| {
            let mut host = BarrierLightning {
                world,
                bolts: &mut bolts,
                rng: &mut rng,
            };
            for (source, target) in casts {
                // A single shared hit list persists across all three calls.
                // The explicit target overload always applies its initial hit.
                magicka_sim::lightning::cast_bolt_at(
                    &mut host, owner, play_time, source, target, &mut hits, color, 1.0, 15.0,
                    damages,
                );
            }
        });
        self.rng = rng;
        self.barrier_visuals.extend(bolts);
        self.play_cue(16, "magick_thunderbolt", Some(at));
        self.camera.shake(1.5, 0.333);
        true
    }
}
