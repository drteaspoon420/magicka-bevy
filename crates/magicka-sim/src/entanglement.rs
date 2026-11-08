//! `Magicka.GameLogic.Entities.Entanglement` (Character.mEntaglement): the
//! web wrapped around an entangled character. Magnitude caps at 20; fire
//! is its weakness. Presentation (the Entangle_web model, web_hit effect,
//! misc_spider_web_cut cue) is returned as events.
use crate::spells::Elements as E;

pub const MAX_MAGNITUDE: f32 = 20.0;
pub const BREAK_SOUND: &str = "misc_spider_web_cut";
pub const HIT_EFFECT: &str = "web_hit";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntangleEvent {
    /// EffectManager.StartEffect(web_hit) at the owner, facing its direction.
    HitEffect,
    /// AudioManager.PlayCue(Misc, misc_spider_web_cut) at the owner.
    BreakSound,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Entanglement {
    pub magnitude: f32,
    pub fade_timer: f32,
}

impl Entanglement {
    /// Character.IsEntangled.
    pub fn entangled(&self) -> bool {
        self.magnitude > f32::EPSILON
    }

    /// Character.Entangle (host side): immortal or burning characters are
    /// not entangled; a fresh web Initializes (fade 0.5) before adding.
    pub fn entangle(&mut self, magnitude: f32, immortal: bool, burning: bool) {
        if immortal || burning {
            return;
        }
        if self.magnitude <= f32::EPSILON {
            self.fade_timer = 0.5;
        }
        self.magnitude = (self.magnitude + magnitude).min(MAX_MAGNITUDE);
    }

    /// DecreaseEntanglement: fire or an overdraw releases the web.
    pub fn decrease(&mut self, amount: f32, element: E, events: &mut Vec<EntangleEvent>) {
        if self.magnitude <= 0.0 {
            return;
        }
        events.push(EntangleEvent::HitEffect);
        let rest = self.magnitude - amount;
        if element.intersects(E::FIRE) || rest < 0.0 {
            self.fade_timer = 0.5;
            self.release(events);
        } else {
            self.magnitude = rest;
        }
    }

    /// Release (also Character.ReleaseEntanglement when entangled).
    pub fn release(&mut self, events: &mut Vec<EntangleEvent>) {
        if self.magnitude > 0.0 {
            events.push(EntangleEvent::BreakSound);
            self.fade_timer = 0.5;
        }
        self.magnitude = 0.0;
    }

    /// Update's fade: once released the web fades over its timer.
    /// Returns the render visibility while drawn.
    pub fn update(&mut self, dt: f32) -> Option<f32> {
        if self.magnitude > f32::EPSILON || self.fade_timer > f32::EPSILON {
            if self.magnitude <= 0.0 {
                self.fade_timer -= dt;
            }
            Some(0.5 * (self.magnitude / MAX_MAGNITUDE) + self.fade_timer)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webs_stack_to_twenty_and_fire_releases_them() {
        let mut e = Entanglement::default();
        e.entangle(12.0, false, false);
        e.entangle(12.0, false, false);
        assert_eq!(e.magnitude, MAX_MAGNITUDE);
        let mut ev = Vec::new();
        e.decrease(5.0, E::EARTH, &mut ev);
        assert_eq!(e.magnitude, 15.0);
        e.decrease(1.0, E::FIRE, &mut ev);
        assert!(!e.entangled());
        assert_eq!(
            ev,
            vec![
                EntangleEvent::HitEffect,
                EntangleEvent::HitEffect,
                EntangleEvent::BreakSound
            ]
        );
        // Immortal or burning characters ignore webs.
        e.entangle(5.0, true, false);
        e.entangle(5.0, false, true);
        assert!(!e.entangled());
        // The released web fades for 0.5 s.
        assert!(e.update(0.25).is_some());
        assert!(e.update(0.3).is_none() || e.fade_timer <= 0.0);
    }
}
