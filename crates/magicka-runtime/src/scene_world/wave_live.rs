//! Live Wave magick (research/wave.md): Wave.Execute probing for the first
//! pillar, WaveEntity pillars as pooled Solid barriers chaining forward,
//! their once-per-cast lift on grounded characters, and the distortion
//! effect's lifetime.
use super::*;
use magicka_sim::wave::{self as wv, Contact, ContactTarget, FirstPillar, WaveEffect};

#[derive(Clone, Debug)]
pub struct LiveWave {
    pub serial: u64,
    pub effect: WaveEffect,
    /// The effect left SpellManager (TTL or ClearMagicks). The entry stays
    /// while pillars still read its hit list.
    pub removed: bool,
}

impl<C: SceneContent> SceneWorld<C> {
    /// Wave.Execute(owner): the effect 3 m ahead, then CreateEntity probes
    /// 4 m down from 2 m above the owner's front. The rewrite's authority
    /// places the pillars (the native client sends SpawnWaveRequest).
    pub fn wave_from(&mut self, owner: u16) -> bool {
        let Some(body) = self.character_body(owner) else {
            return false;
        };
        let (position, direction) = (
            self.physics.bodies[body].position,
            self.physics.bodies[body].direction(),
        );
        let radius = self
            .combatants()
            .iter()
            .find(|c| c.handle == owner)
            .map_or(0.0, |c| c.radius);
        let serial = self.add_wave(WaveEffect::from_owner(position, direction, self.play_time));
        if self.role == NetworkRole::Client {
            return true;
        }
        let first = FirstPillar::new(position, direction, radius);
        let probe = Segment {
            origin: first.probe_origin,
            delta: [0.0, -wv::PROBE_DROP, 0.0],
        };
        let Some(hit) = self.collision.collision().segment_intersect(probe, false) else {
            return true;
        };
        let damage = match wv::cast_damage() {
            Ok(d) => d,
            Err(e) => {
                self.log.push(format!("wave: {e:?}"));
                return true;
            }
        };
        let placement = first.placement(hit.hit.position);
        let list = self.barriers.new_hit_list();
        self.place_wave_pillar(
            Some(owner),
            list,
            serial,
            &placement,
            placement.probe_origin,
            wv::spell(),
            damage,
        );
        true
    }

    /// Wave.Execute(Vector3): the distortion only, in a random +X/+Z
    /// direction; no pillars.
    pub fn wave_at(&mut self, position: [f32; 3]) -> bool {
        let random = [f64::from(self.random()), f64::from(self.random())];
        self.add_wave(WaveEffect::from_position(position, random, self.play_time));
        true
    }

    fn add_wave(&mut self, effect: WaveEffect) -> u64 {
        self.effect_serial = self.effect_serial.wrapping_add(1);
        let serial = self.effect_serial;
        self.waves.push(LiveWave {
            serial,
            effect,
            removed: false,
        });
        serial
    }

    /// WaveEntity.Initialize + AddEntity (Initialize kills other casts'
    /// barriers within 0.8 · scale · radius and detonates mines first).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn place_wave_pillar(
        &mut self,
        owner: Option<u16>,
        list: usize,
        wave: u64,
        p: &magicka_sim::barrier::BarrierPlacement,
        hit: [f32; 3],
        spell: magicka_sim::spells::Spell,
        damage: [magicka_sim::damage::Damage; 5],
    ) {
        use magicka_sim::barrier::{Barrier, BarrierInit};
        let (Some(clips), Some(pillar_clip)) = (self.barrier_clips, self.wave_pillar_clip) else {
            self.log
                .push("wave: pillar clip durations unavailable".into());
            return;
        };
        let (slot, stale) = self.barriers.take();
        let init = BarrierInit {
            owner,
            hit_list: list,
            hit,
            direction: p.direction,
            scale: p.scale,
            range: p.range,
            next_direction: p.next_direction,
            next_yaw: p.next_yaw,
            distance: p.distance,
            timestamp: self.play_time,
        };
        let (barrier, cue) =
            Barrier::initialize_wave(init, spell, damage, &clips, pillar_clip, stale, wave);
        let kill_radius = barrier.query_radius(0.8);
        self.clear_for_barrier(list, hit, kill_radius);
        self.log.push(format!("wave pillar {slot} at {hit:?}"));
        if let Some(cue) = cue {
            let key = self.play_cue(crate::audio::banks::SPELLS, cue, Some(barrier.position));
            if let Some(old) = self.barrier_cues.insert(slot, key) {
                self.stop_cue(old);
            }
        }
        self.barriers.add(slot, barrier);
    }

    /// Wave.Update for every effect; entries end once removed and no
    /// pillar refers to them.
    pub(super) fn update_waves(&mut self, dt: f32) {
        for live in &mut self.waves {
            if live.removed {
                continue;
            }
            live.effect.update(dt);
            if live.effect.is_dead() {
                live.removed = true;
            }
        }
        let barriers = &self.barriers;
        self.waves.retain(|live| {
            !live.removed
                || barriers
                    .iter()
                    .any(|(_, b)| b.wave.is_some_and(|w| w.wave == live.serial))
        });
    }

    /// SpellManager.ClearMagicks: Wave.OnRemove stops the effect; pillars
    /// keep their parent's hit list.
    pub(super) fn clear_waves(&mut self) {
        for live in &mut self.waves {
            live.removed = true;
        }
        let barriers = &self.barriers;
        self.waves.retain(|live| {
            barriers
                .iter()
                .any(|(_, b)| b.wave.is_some_and(|w| w.wave == live.serial))
        });
    }

    /// WaveEntity.OnCollision for characters overlapping a pillar this
    /// step: grounded characters not yet in the parent Wave's hit list take
    /// the pillar damage and, below the weight threshold, a 10 m/s lift.
    pub(super) fn wave_contacts(&mut self, contacts: Vec<(usize, u16)>) {
        use magicka_sim::earthquake::EarthquakeHost;
        for (slot, handle) in contacts {
            let Some(b) = self.barriers.get(slot) else {
                continue;
            };
            let Some(pillar) = b.wave else {
                continue;
            };
            let (owner, damage, timestamp, position) = (b.owner, b.damage, b.timestamp, b.position);
            let Some(wave) = self.waves.iter().position(|w| w.serial == pillar.wave) else {
                continue;
            };
            let Some(target) = EarthquakeHost::target(self, handle) else {
                continue;
            };
            let Some(c) = target.character else {
                continue;
            };
            let facts = ContactTarget {
                dead: false,
                ethereal: c.ethereal,
                grounded: c.grounded,
                levitating: c.levitating,
                immortal: c.immortal,
                in_event: c.in_event,
                mass: target.mass,
            };
            let Contact::Hit { lift } =
                wv::contact(facts, self.waves[wave].effect.in_hit_list(handle))
            else {
                continue;
            };
            let delivered = self.with_rail_world(|world| {
                world.deliver_collection(handle, owner, damage, timestamp, position);
            });
            if delivered.is_none() {
                self.log
                    .push("wave contact: entity grid unavailable".into());
            }
            self.waves[wave].effect.hit_list.push(handle);
            if lift {
                EarthquakeHost::impulse_velocity(self, handle, [0.0, wv::LIFT, 0.0]);
            }
        }
    }
}
