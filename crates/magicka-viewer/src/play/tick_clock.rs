//! The viewer's side of the fixed simulation clock (`research/fixed-timestep.md`).
//!
//! Hosts and offline play tick at 60 Hz whatever the display does, so a frame runs
//! zero or more ticks. Frame input waits for the next tick, and the camera and the
//! moving roots are drawn between their last two tick states. That places them up
//! to one tick in the past; nothing here feeds back into the simulation.
use super::lifecycle::PlayAppExt;
use bevy::prelude::*;
use magicka_runtime::{
    avatar::PlayerInput,
    fixed_tick::{FixedClock, PendingSample},
};

/// Moving further than this in one tick (120 m/s) is a teleport or a camera cut.
const TELEPORT_DISTANCE: f32 = 2.0;

#[derive(Resource, Default)]
pub(super) struct TickClock {
    clock: FixedClock,
    input: PendingSample<PlayerInput>,
    select: PendingSample<bool>,
    ticks_this_frame: u32,
    /// Off for scripted QA and network clients, whose roots are already right
    /// for this frame when the sync systems finish.
    interpolating: bool,
    rate_window: f32,
    rate_frames: u32,
    rate_ticks: u32,
}

impl TickClock {
    /// The ticks to run for this rendered frame.
    pub(super) fn advance(&mut self, frame_dt: f32) -> u32 {
        let ticks = self.clock.advance(frame_dt);
        self.ticks_this_frame = ticks;
        self.interpolating = true;
        self.rate_window += frame_dt;
        self.rate_frames += 1;
        self.rate_ticks += ticks;
        if self.rate_window >= 1.0 {
            // RUST_LOG=tick_rate=debug
            debug!(target: "tick_rate", "{} frames, {} ticks in {:.3} s",
                self.rate_frames, self.rate_ticks, self.rate_window);
            self.rate_window = 0.0;
            self.rate_frames = 0;
            self.rate_ticks = 0;
        }
        ticks
    }
    /// The offline pause menu holds the world where it is drawn.
    pub(super) fn paused(&mut self) {
        self.ticks_this_frame = 0;
    }
    pub(super) fn unclocked(&mut self) {
        self.ticks_this_frame = 1;
        self.interpolating = false;
    }
    pub(super) fn sample(&mut self, input: PlayerInput, select: bool) {
        self.input.sample(input);
        self.select.sample(select);
    }
    /// The input for one tick.
    pub(super) fn take(&mut self) -> (PlayerInput, bool) {
        (
            self.input.take().unwrap_or_default(),
            self.select.take().unwrap_or_default(),
        )
    }
}

/// A root drawn between its last two tick states.
#[derive(Component, Default)]
pub(super) struct TickInterpolated {
    previous: Transform,
    current: Transform,
    seen: bool,
}

impl TickInterpolated {
    /// `written` is what the sync systems wrote this frame: the latest tick state.
    fn blend(&mut self, written: Transform, ticked: bool, alpha: f32) -> Transform {
        if !self.seen {
            self.seen = true;
            self.previous = written;
            self.current = written;
        } else if ticked {
            self.previous = self.current;
            self.current = written;
            if self.previous.translation.distance(self.current.translation) > TELEPORT_DISTANCE {
                self.previous = self.current;
            }
        }
        Transform {
            translation: self
                .previous
                .translation
                .lerp(self.current.translation, alpha),
            rotation: self.previous.rotation.slerp(self.current.rotation, alpha),
            scale: self.previous.scale.lerp(self.current.scale, alpha),
        }
    }
}

fn interpolate(clock: Res<TickClock>, mut roots: Query<(&mut Transform, &mut TickInterpolated)>) {
    let alpha = clock.clock.alpha();
    for (mut transform, mut state) in &mut roots {
        if clock.interpolating {
            *transform = state.blend(*transform, clock.ticks_this_frame > 0, alpha);
        } else {
            state.seen = false;
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_play_resource::<TickClock>();
    app.add_play_systems(
        PostUpdate,
        interpolate
            .after(bevy::app::AnimationSystems)
            .before(bevy::transform::TransformSystems::Propagate),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32) -> Transform {
        Transform::from_xyz(x, 0.0, 0.0)
    }

    #[test]
    fn draws_between_the_last_two_tick_states() {
        let mut state = TickInterpolated::default();
        assert_eq!(state.blend(at(0.0), true, 0.5).translation.x, 0.0);
        assert_eq!(state.blend(at(1.0), true, 0.25).translation.x, 0.25);
        // A frame without a tick moves on towards the same state, even though the
        // sync system wrote the tick state again.
        assert_eq!(state.blend(at(1.0), false, 0.75).translation.x, 0.75);
    }

    #[test]
    fn a_teleport_is_not_smeared_across_the_screen() {
        let mut state = TickInterpolated::default();
        state.blend(at(0.0), true, 0.0);
        assert_eq!(state.blend(at(50.0), true, 0.1).translation.x, 50.0);
    }

    #[test]
    fn scripted_frames_keep_the_tick_state_and_restart_cleanly() {
        let mut app = App::new();
        app.insert_resource(TickClock::default())
            .add_systems(Update, interpolate);
        let root = app
            .world_mut()
            .spawn((at(3.0), TickInterpolated::default()))
            .id();
        app.world_mut().resource_mut::<TickClock>().unclocked();
        app.update();
        assert_eq!(
            app.world().get::<Transform>(root).unwrap().translation.x,
            3.0
        );
        assert!(!app.world().get::<TickInterpolated>(root).unwrap().seen);
    }

    #[test]
    fn a_paused_frame_runs_no_ticks_and_holds_the_input() {
        let mut clock = TickClock::default();
        clock.sample(
            PlayerInput {
                inventory: true,
                ..default()
            },
            true,
        );
        clock.paused();
        assert_eq!(clock.ticks_this_frame, 0);
        let (input, select) = clock.take();
        assert!(input.inventory && select);
    }
}
