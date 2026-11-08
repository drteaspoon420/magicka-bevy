//! SprayEntity: web strands (Parker's web attack). A 48-entry cache handed
//! out round-robin; each strand flies, drops at 10 m/s², and is drawn as a
//! camera-facing Spray_web quad toward its child strand or, until it has one,
//! its owner (the mouth zone). Touching level geometry or a shield stops it;
//! an un-entangled avatar touching the strand capsule while it moves is
//! entangled (magnitude 12) and stops it. A stopped strand fades for 1 s.
//! See research/boss-parker.md.
use crate::xna_math::{self as m, V3};

/// PlayState.InitializeCache(48).
pub const CACHE: usize = 48;
/// The strand capsule radius.
pub const RADIUS: f32 = 0.5;
/// Character.Entangle magnitude on contact.
pub const ENTANGLE: f32 = 12.0;
const DECAY_TIME: f32 = 1.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WebStrand {
    pub position: V3,
    pub velocity: V3,
    pub immovable: bool,
    /// The owner entity (mOwner), tethering the strand until it has a child.
    pub owner: Option<u16>,
    /// mChild (a cache slot).
    pub child: Option<usize>,
    pub decay: f32,
    pub dead: bool,
    /// In the entity manager (initialized, not yet removed).
    pub added: bool,
    /// The far end this update (child or owner position + 0.01 X), when
    /// the strand is drawn.
    pub tail: Option<V3>,
}

impl WebStrand {
    /// The render alpha: (1 − decay)^0.2.
    pub fn alpha(&self) -> f32 {
        (1.0 - self.decay).max(0.0).powf(0.2)
    }
}

#[derive(Clone, Debug)]
pub struct WebPool {
    pub slots: Vec<WebStrand>,
    order: std::collections::VecDeque<usize>,
}

impl Default for WebPool {
    fn default() -> Self {
        Self {
            slots: vec![WebStrand::default(); CACHE],
            order: (0..CACHE).collect(),
        }
    }
}

impl WebPool {
    /// GetInstance: the front entry moves to the back.
    pub fn instance(&mut self) -> usize {
        let slot = self.order.pop_front().unwrap_or(0);
        self.order.push_back(slot);
        slot
    }

    /// Initialize(owner, position, direction, velocity): starts one unit
    /// along the direction, without a child.
    pub fn initialize(
        &mut self,
        slot: usize,
        owner: Option<u16>,
        position: V3,
        direction: V3,
        velocity: f32,
    ) {
        self.slots[slot] = WebStrand {
            position: m::add(position, direction),
            velocity: m::scale(direction, velocity),
            immovable: false,
            owner,
            child: None,
            decay: 0.0,
            dead: false,
            added: true,
            tail: None,
        };
    }

    /// The Child setter (a strand is never its own child).
    pub fn set_child(&mut self, slot: usize, child: usize) {
        self.slots[slot].child = (child != slot).then_some(child);
    }

    /// Update before the physics step: gravity on a moving strand, child
    /// re-linking past dead strands, decay of a stopped one, and the tail
    /// (child position, else owner position) offset 0.01 along X.
    pub fn update(&mut self, dt: f32, owner_position: impl Fn(u16) -> Option<V3>) {
        for i in 0..self.slots.len() {
            if !self.slots[i].added {
                continue;
            }
            let s = &mut self.slots[i];
            if m::length_squared(s.velocity) > f32::EPSILON {
                s.velocity[1] -= dt * 10.0;
            }
            if let Some(c) = self.slots[i].child {
                if self.slots[c].dead {
                    self.slots[i].child = self.slots[c].child;
                }
            }
            let s = &mut self.slots[i];
            if s.immovable {
                s.decay += dt;
                if s.decay >= DECAY_TIME {
                    s.dead = true;
                }
            }
            let end = match self.slots[i].child {
                Some(c) => Some(self.slots[c].position),
                None => self.slots[i].owner.and_then(&owner_position),
            };
            self.slots[i].tail = end.map(|e| m::add(e, [0.01, 0.0, 0.0]));
        }
    }

    /// Removable strands leave the entity manager.
    pub fn remove_dead(&mut self) {
        for s in &mut self.slots {
            if s.added && s.dead {
                s.added = false;
            }
        }
    }

    /// The strands drawn this frame: (position, tail, alpha).
    pub fn drawn(&self) -> impl Iterator<Item = (V3, V3, f32)> + '_ {
        self.slots
            .iter()
            .filter(|s| s.added && !s.dead)
            .filter_map(|s| s.tail.map(|t| (s.position, t, s.alpha())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_is_round_robin_and_strands_link_to_children() {
        let mut pool = WebPool::default();
        let a = pool.instance();
        let b = pool.instance();
        assert_ne!(a, b);
        pool.initialize(a, Some(7), [0.0; 3], [0.0, 0.0, -1.0], 10.0);
        pool.initialize(b, Some(7), [0.0; 3], [1.0, 0.0, 0.0], 10.0);
        pool.set_child(a, b);
        pool.update(0.1, |_| Some([0.0, 2.0, 0.0]));
        assert_eq!(pool.slots[a].tail, Some([1.01, 0.0, 0.0]));
        assert_eq!(pool.slots[b].tail, Some([0.01, 2.0, 0.0]));
        assert!(pool.slots[a].velocity[1] < 0.0);
        // A dead child is skipped; a stopped strand fades out in 1 s.
        pool.slots[b].dead = true;
        pool.slots[a].immovable = true;
        pool.update(0.5, |_| None);
        assert_eq!(pool.slots[a].child, None);
        pool.update(0.6, |_| None);
        assert!(pool.slots[a].dead);
        for _ in 0..CACHE - 2 {
            pool.instance();
        }
        assert_eq!(pool.instance(), a);
    }
}
