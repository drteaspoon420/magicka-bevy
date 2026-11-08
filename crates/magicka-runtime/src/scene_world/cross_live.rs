//! SummonCross spawning and the caster's bubble aura (summon-cross.md).
use super::*;

const BUBBLE_TTL: f32 = 100_000_000.0;
const BUBBLE_EFFECT: &str = "paladin_bubble";

impl<C: SceneContent> SceneWorld<C> {
    /// Native owner Execute: cross at the nav point nearest owner +3Y, idle
    /// with no alert radius and random yaw, then AddBubbleShield on the owner.
    pub fn summon_cross(&mut self, owner: u16) -> Option<u16> {
        if self.role == NetworkRole::Client {
            return None;
        }
        let body = self.character_body(owner)?;
        let mut point = self.physics.bodies[body].position;
        point[1] += 3.0;
        let point = self
            .nav
            .as_ref()
            .and_then(|nav| nav.nearest(point, magicka_sim::navmesh::movement::DEFAULT))
            .map_or(point, |(_, p, _)| p);
        let yaw = self.random() * std::f32::consts::TAU;
        let (sin, cos) = yaw.sin_cos();
        let handle = self.summon_npc(
            Some(owner),
            "cross",
            point,
            [-sin, 0.0, -cos],
            magicka_sim::summons::SummonKind::Normal,
            crate::spawn_action::SpawnOrder {
                order: 0,
                react_to: 0,
                reaction: 0,
                priority_target: 0,
                priority_ability: 0,
                trigger: 0,
                events: &[],
            },
        )?;
        if let Some(cross) = self.npcs.get_mut(&handle) {
            cross.weights.alert_radius = 0.0;
        }
        self.add_bubble_shield(owner)?;
        self.crosses.push(handle);
        Some(handle)
    }

    /// Character.AddBubbleShield appends a new self aura. Recasting does not
    /// replace the previous aura; each has its own buff and particle instance.
    pub fn add_bubble_shield(&mut self, owner: u16) -> Option<u32> {
        let body = self.character_body(owner)?;
        let at = self.physics.bodies[body].position;
        let direction = self.physics.bodies[body].direction();
        let mut visual = crate::buffs::AuraVisual::new(0.0, [2.0, 1.5, 1.0]);
        // VisualCategory.Special: AuraOffensive (29) + 3 - 1.
        visual.decal_cell = 31;
        let aura = crate::character::ActiveAura {
            resistance: magicka_sim::resistance::Resistance {
                against: magicka_sim::spells::Elements::ALL,
                multiplier: 0.0,
                modifier: 0.0,
                status_resistance: false,
            },
            ttl: BUBBLE_TTL,
            start_ttl: BUBBLE_TTL,
            self_casted: true,
            self_only: true,
            visual,
        };
        let id = aura.visual.buff_id;
        let c = Self::character_mut(&mut self.players, &mut self.npcs, owner)?;
        c.bubble = true;
        c.auras.push(aura);
        let instance = self.start_effect_transform(
            custom_hash(BUBBLE_EFFECT),
            crate::particles::create_world(at, direction, [0.0, 1.0, 0.0]),
        );
        self.bubble_effects.insert((owner, id), instance);
        Some(id)
    }

    /// Native RemoveBubbleShield clears ALL self-cast auras and leaves mBubble
    /// set. Resistance buff copies expire independently after their last grant.
    pub fn remove_bubble_shield(&mut self, owner: u16) {
        if let Some(c) = Self::character_mut(&mut self.players, &mut self.npcs, owner) {
            if c.bubble {
                for aura in c.auras.iter_mut().filter(|a| a.self_casted) {
                    aura.ttl = 0.0;
                }
            }
        }
    }

    /// Bubble particle instances (kept by PlaySession's effect registry so
    /// their Move/Stop commands resolve).
    pub(crate) fn bubble_effect_instances(&self) -> impl Iterator<Item = u64> + '_ {
        self.bubble_effects.values().copied()
    }

    pub(super) fn update_crosses(&mut self) {
        self.crosses
            .retain(|h| self.npcs.get(h).is_some_and(|n| !n.dead()));
        let refs: Vec<_> = self
            .bubble_effects
            .iter()
            .map(|(key, instance)| (*key, *instance))
            .collect();
        for ((owner, id), instance) in refs {
            let alive = self.character_ref(owner).is_some_and(|c| {
                c.auras
                    .iter()
                    .any(|a| a.visual.buff_id == id && a.ttl > 0.0)
            });
            if !alive {
                self.effect_commands
                    .push(SceneEffectCommand::Stop { instance });
                self.bubble_effects.remove(&(owner, id));
            } else if let Some(body) = self.character_body(owner) {
                let b = &self.physics.bodies[body];
                self.effect_commands.push(SceneEffectCommand::Move {
                    instance,
                    position: b.position,
                    direction: b.direction(),
                });
            }
        }
    }
}
