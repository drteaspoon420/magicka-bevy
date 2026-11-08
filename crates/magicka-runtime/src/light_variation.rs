//! `PolygonHead.Lights.Light.GetIntensity(LightVariationType, dt, speed,
//! amount, ref timer, ref current)`: the per-frame intensity multiplier of
//! level lights (research/level-lighting.md). Enum order: None, Sine,
//! Flicker, Candle, Strobe. All lights share one `System.Random`
//! (Light.sRandom, time-seeded in the original; a fixed seed here).

// Negated comparisons keep the original's NaN behaviour.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::particles::DotNetRandom;

/// mVariationTimer and mCurrentVariation (both start at 0).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightVariation {
    pub timer: f32,
    pub current: f32,
}

fn wrap_angle(a: f32) -> f32 {
    let two_pi = std::f32::consts::TAU;
    let mut a = (a + std::f32::consts::PI).rem_euclid(two_pi) - std::f32::consts::PI;
    if a <= -std::f32::consts::PI {
        a += two_pi;
    }
    a
}

impl LightVariation {
    pub fn intensity(
        &mut self,
        kind: i32,
        dt: f32,
        speed: f32,
        amount: f32,
        random: &mut DotNetRandom,
    ) -> f32 {
        match kind {
            // Sine: timer = WrapAngle(timer + dt·speed); 1 + sin(timer)·amount.
            1 => {
                self.timer = wrap_angle(self.timer + dt * speed);
                self.timer.sin() * amount + 1.0
            }
            // Flicker: toggle 0/1 whenever the timer runs out, waiting a
            // random 0..1/speed seconds.
            2 => {
                self.timer -= dt;
                if !(self.timer > 0.0) {
                    self.current = if self.current > 0.5 { 0.0 } else { 1.0 };
                    self.timer = random.next_double() as f32 / speed;
                }
                self.current
            }
            // Candle: a damped random walk (timer is the velocity).
            3 => {
                self.timer += (random.next_double() as f32 - 0.5) * amount * dt;
                self.timer -= (speed * self.current + 0.8 * self.timer) * dt;
                self.current += self.timer * dt;
                1.0 + self.current
            }
            // Strobe: on for (1 − amount)/speed, off for amount/speed.
            4 => {
                self.timer -= dt;
                if !(self.timer > 0.0) {
                    if !(self.current >= 0.5) {
                        self.current = 1.0;
                        self.timer += (1.0 - amount) / speed;
                    } else {
                        self.current = 0.0;
                        self.timer += amount / speed;
                    }
                }
                self.current
            }
            _ => 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_and_strobe_follow_the_original_closed_forms() {
        let mut rng = DotNetRandom::new(1);
        let mut v = LightVariation::default();
        let i = v.intensity(1, 0.5, 2.0, 0.25, &mut rng);
        assert!((i - (1.0 + 1.0f32.sin() * 0.25)).abs() < 1e-6);

        let mut s = LightVariation::default();
        // First call turns on for (1 − 0.25)/2 = 0.375 s.
        assert_eq!(s.intensity(4, 0.1, 2.0, 0.25, &mut rng), 1.0);
        assert!((s.timer - 0.275).abs() < 1e-6);
        assert_eq!(s.intensity(4, 0.3, 2.0, 0.25, &mut rng), 0.0);
        assert!((s.timer - 0.1).abs() < 1e-6);
    }

    #[test]
    fn candle_stays_near_one_and_is_deterministic_per_seed() {
        let run = || {
            let mut rng = DotNetRandom::new(7);
            let mut v = LightVariation::default();
            (0..600)
                .map(|_| v.intensity(3, 1.0 / 60.0, 25.0, 0.2, &mut rng))
                .collect::<Vec<_>>()
        };
        let a = run();
        assert_eq!(a, run());
        assert!(a.iter().all(|i| (0.9..1.1).contains(i)), "{a:?}");
        assert!(a.iter().any(|&i| i != 1.0));
    }
}
