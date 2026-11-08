//! RenderManager's current TransitionEffect (Transitions.Fade/CrossFade),
//! shared by script `<fade>` actions and Level.GoToScene. See
//! research/scene-transition.md.

/// RenderManager.mCurrentTransition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderTransition {
    /// TransitionDirections.Out (BeginTransition: towards `color`) or In.
    pub out: bool,
    pub color: [u8; 4],
    pub time: f32,
    pub elapsed: f32,
}

impl RenderTransition {
    /// BeginTransition / EndTransition. `Transitions.None` (or Invalid)
    /// clears the current transition.
    pub fn start(transition: i32, out: bool, color: [u8; 4], time: f32) -> Option<Self> {
        (transition > 0).then_some(Self {
            out,
            color,
            time,
            elapsed: 0.0,
        })
    }

    /// Finished after at least one update once the time has elapsed.
    pub fn finished(&self) -> bool {
        self.elapsed > 0.0 && self.elapsed >= self.time
    }

    /// Advance one frame. Returns true on the frame the effect finishes
    /// (RenderManager raises TransitionEnd).
    pub fn update(&mut self, dt: f32) -> bool {
        let was = self.finished();
        self.elapsed += dt;
        !was && self.finished()
    }

    /// Overlay amount (0 clear .. 1 fully `color`). A linear ramp is
    /// INFERRED; the TransitionEffect shader is not reconstructed.
    pub fn amount(&self) -> f32 {
        let p = if self.time > 0.0 {
            (self.elapsed / self.time).clamp(0.0, 1.0)
        } else {
            1.0
        };
        if self.out {
            p
        } else {
            1.0 - p
        }
    }
}

/// Advance `current`; a finished In transition is dropped (OnTransitionEnd
/// nulls it), a finished Out transition stays. Returns TransitionEnd.
pub fn update(current: &mut Option<RenderTransition>, dt: f32) -> bool {
    let Some(t) = current.as_mut() else {
        return false;
    };
    let ended = t.update(dt);
    if ended && !t.out {
        *current = None;
    }
    ended
}
