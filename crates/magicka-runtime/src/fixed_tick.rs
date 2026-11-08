//! Fixed-rate simulation clock for playable hosts (rewrite decision; the
//! original stepped once per rendered frame on the frame's delta).
//!
//! Many ported rules act once per update: 0.6 m per update for Amalgameddon's
//! units, one spiderling per update, 20 AI agents per update. They only keep
//! their original meaning at the original's 60 updates per second, so a player
//! on a 144 Hz display must not get them 2.4 times as often, and a host must not
//! impose its display on everyone in its session. `magicka-server` already
//! ticks at this rate. See `research/fixed-timestep.md`.

use crate::avatar::PlayerInput;
use magicka_network::control::Controls;

/// One simulation tick, in seconds.
pub const TICK_DT: f32 = 1.0 / 60.0;

const TICK: f64 = 1.0 / 60.0;
/// Longest frame the clock catches up on. A longer stall slows the game down
/// instead of running a burst of ticks that takes longer still. The original's
/// 0.05 s clamp slowed the game below 20 FPS; six ticks keeps real time down to
/// 10 FPS when simulation is cheap and rendering is not.
const MAX_FRAME: f64 = 6.0 * TICK;
/// Display periods this close to a whole number of ticks count as exact, so a
/// 59.94 or 60.02 Hz display gets one tick per frame, not an occasional 0 or 2.
const SNAP_TOLERANCE: f64 = 0.000_25;
const SNAP_PERIODS: [f64; 3] = [TICK / 2.0, TICK, TICK * 2.0];
/// Accumulated rounding must not delay a tick by a whole frame.
const ROUNDING: f64 = 1e-9;

/// Turns rendered frame deltas into a whole number of simulation ticks.
#[derive(Clone, Copy, Debug, Default)]
pub struct FixedClock {
    accumulator: f64,
}

impl FixedClock {
    /// The ticks to run for a frame of `frame_dt` seconds.
    pub fn advance(&mut self, frame_dt: f32) -> u32 {
        let frame = f64::from(frame_dt).clamp(0.0, MAX_FRAME);
        let frame = SNAP_PERIODS
            .into_iter()
            .find(|period| (frame - period).abs() < SNAP_TOLERANCE)
            .unwrap_or(frame);
        self.accumulator += frame;
        let ticks = ((self.accumulator + ROUNDING) / TICK).floor();
        self.accumulator = (self.accumulator - ticks * TICK).max(0.0);
        ticks as u32
    }

    /// How far the render time sits between the last two ticks, 0 to 1.
    pub fn alpha(&self) -> f32 {
        ((self.accumulator / TICK) as f32).clamp(0.0, 1.0)
    }
}

/// An input sample that can be held for the next tick. Frames and ticks no
/// longer pair up, so a press seen by a frame without a tick has to wait for one.
pub trait TickSample: Copy {
    /// Combines an older unconsumed sample with a newer one. A control pressed at
    /// any point counts, so a tap between two ticks still reaches the game.
    fn coalesce(self, newer: Self) -> Self;
    /// This sample for a further tick in the same frame: held controls stay held,
    /// and one-shot presses have already been consumed.
    fn repeated(self) -> Self;
}

impl TickSample for bool {
    fn coalesce(self, newer: Self) -> Self {
        self || newer
    }
    fn repeated(self) -> Self {
        false
    }
}

impl TickSample for PlayerInput {
    fn coalesce(self, newer: Self) -> Self {
        Self {
            cursor_ndc: newer.cursor_ndc.or(self.cursor_ndc),
            pad_movement: newer.pad_movement,
            pad_attack: self.pad_attack || newer.pad_attack,
            pad_block: self.pad_block || newer.pad_block,
            move_down: self.move_down || newer.move_down,
            cast_down: self.cast_down || newer.cast_down,
            cast_self_down: self.cast_self_down || newer.cast_self_down,
            shift_down: self.shift_down || newer.shift_down,
            boost_down: self.boost_down || newer.boost_down,
            elements: std::array::from_fn(|i| self.elements[i] || newer.elements[i]),
            magick_prev: self.magick_prev || newer.magick_prev,
            magick_next: self.magick_next || newer.magick_next,
            inventory: self.inventory || newer.inventory,
        }
    }
    fn repeated(self) -> Self {
        Self {
            magick_prev: false,
            magick_next: false,
            inventory: false,
            ..self
        }
    }
}

impl TickSample for Controls {
    fn coalesce(self, newer: Self) -> Self {
        Self {
            aim: newer.aim.or(self.aim),
            pad_movement: newer.pad_movement,
            pad_attack: self.pad_attack || newer.pad_attack,
            pad_block: self.pad_block || newer.pad_block,
            move_down: self.move_down || newer.move_down,
            cast_down: self.cast_down || newer.cast_down,
            cast_self_down: self.cast_self_down || newer.cast_self_down,
            shift_down: self.shift_down || newer.shift_down,
            boost_down: self.boost_down || newer.boost_down,
            elements: std::array::from_fn(|i| self.elements[i] || newer.elements[i]),
            magick_prev: self.magick_prev || newer.magick_prev,
            magick_next: self.magick_next || newer.magick_next,
            inventory: self.inventory || newer.inventory,
        }
    }
    fn repeated(self) -> Self {
        Self {
            magick_prev: false,
            magick_next: false,
            inventory: false,
            ..self
        }
    }
}

/// Frame samples waiting for the next tick.
#[derive(Clone, Copy, Debug)]
pub struct PendingSample<T> {
    merged: Option<T>,
    latest: Option<T>,
}

impl<T> Default for PendingSample<T> {
    fn default() -> Self {
        Self {
            merged: None,
            latest: None,
        }
    }
}

impl<T: TickSample> PendingSample<T> {
    /// Records this frame's sample.
    pub fn sample(&mut self, value: T) {
        self.merged = Some(match self.merged {
            Some(older) => older.coalesce(value),
            None => value,
        });
        self.latest = Some(value);
    }

    /// The input for one tick. The first tick after a sample gets everything since
    /// the last tick, and further ticks before the next sample get what is held now.
    pub fn take(&mut self) -> Option<T> {
        match self.merged.take() {
            Some(merged) => Some(merged),
            None => self.latest.map(T::repeated),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticks_over(frames: usize, frame_dt: f32) -> u32 {
        let mut clock = FixedClock::default();
        (0..frames).map(|_| clock.advance(frame_dt)).sum()
    }

    #[test]
    fn every_display_rate_gets_sixty_ticks_a_second() {
        for hz in [24u32, 30, 50, 59, 60, 61, 75, 90, 120, 144, 165, 240, 360] {
            let ticks = ticks_over(hz as usize * 10, 1.0 / hz as f32);
            assert!((599..=601).contains(&ticks), "{hz} Hz gave {ticks}");
        }
    }

    #[test]
    fn a_display_near_sixty_hertz_gets_exactly_one_tick_per_frame() {
        for hz in [59.94_f32, 60.0, 60.02] {
            let mut clock = FixedClock::default();
            let frame_dt = 1.0 / hz;
            for frame in 0..10_000 {
                assert_eq!(clock.advance(frame_dt), 1, "{hz} Hz, frame {frame}");
            }
        }
    }

    #[test]
    fn a_long_stall_is_clamped_and_a_zero_frame_runs_nothing() {
        let mut clock = FixedClock::default();
        assert_eq!(clock.advance(0.0), 0);
        assert_eq!(clock.advance(5.0), 6);
        assert_eq!(clock.advance(-1.0), 0);
    }

    #[test]
    fn alpha_is_the_fraction_of_the_next_tick() {
        let mut clock = FixedClock::default();
        assert_eq!(clock.advance(TICK_DT * 0.25), 0);
        assert!((clock.alpha() - 0.25).abs() < 1e-4, "{}", clock.alpha());
        assert_eq!(clock.advance(TICK_DT), 1);
        assert!((clock.alpha() - 0.25).abs() < 1e-4, "{}", clock.alpha());
    }

    #[test]
    fn a_tap_between_ticks_reaches_the_next_tick_once() {
        let mut pending = PendingSample::<PlayerInput>::default();
        pending.sample(PlayerInput {
            elements: [true; 8],
            magick_next: true,
            ..Default::default()
        });
        pending.sample(PlayerInput::default());
        let first = pending.take().unwrap();
        assert!(first.elements[0] && first.magick_next);
        let second = pending.take().unwrap();
        assert!(!second.elements[0] && !second.magick_next);
    }

    #[test]
    fn a_second_tick_in_one_frame_keeps_held_controls_but_not_one_shots() {
        let mut pending = PendingSample::<Controls>::default();
        pending.sample(Controls {
            cast_down: true,
            inventory: true,
            aim: Some([1.0, 0.0, 2.0]),
            ..Default::default()
        });
        let first = pending.take().unwrap();
        assert!(first.cast_down && first.inventory);
        let second = pending.take().unwrap();
        assert!(second.cast_down && !second.inventory);
        assert_eq!(second.aim, Some([1.0, 0.0, 2.0]));
    }

    #[test]
    fn the_latest_stick_position_wins() {
        let mut pending = PendingSample::<PlayerInput>::default();
        for x in [0.2, 0.9] {
            pending.sample(PlayerInput {
                pad_movement: Some([x, 0.0]),
                ..Default::default()
            });
        }
        assert_eq!(pending.take().unwrap().pad_movement, Some([0.9, 0.0]));
    }

    #[test]
    fn nothing_sampled_means_no_input() {
        assert!(PendingSample::<bool>::default().take().is_none());
    }
}
