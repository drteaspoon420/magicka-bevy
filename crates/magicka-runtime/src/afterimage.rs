//! Avatar and Vlad afterimages: character AfterImageRenderData
//! timing and AfterImageRenderData.Draw's per-ghost alpha
//! (research/afterimages.md). The pose history itself is presentation data.

/// Skeleton snapshots kept (AfterImageRenderData.mSkeleton.Length); index 0
/// is the newest and is never drawn.
pub const SNAPSHOTS: usize = 5;
/// Snapshot period and intensity refill step.
pub const PERIOD: f32 = 0.05;
/// Draw: base alpha 0.333, step 0.333 / (snapshots + 1).
pub const BASE_ALPHA: f32 = 0.333;
/// Character.ColdColor, the Colorize tint of the ghosts.
pub const COLD_COLOR: [f32; 3] = [1.0, 1.6, 2.0];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AfterImages {
    /// mAfterImageTimer.
    pub timer: f32,
    /// mAfterImageIntensity (-1: idle, ResetAfterImages).
    pub intensity: f32,
}
impl Default for AfterImages {
    fn default() -> Self {
        Self {
            timer: 0.0,
            intensity: -1.0,
        }
    }
}

/// One character afterimage update's outcome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AfterImageStep {
    /// Not drawn this frame. Avatar resets its clock; Vlad preserves it.
    Hidden,
    /// Drawn: `snapshots` new poses were pushed (shifting older ones), and
    /// `intensity` is mAfterImageIntensity / 0.05.
    Drawn { snapshots: u32, intensity: f32 },
}

impl AfterImages {
    /// VladCharacter's fields are zero-initialized, unlike Avatar's reset.
    pub fn vlad() -> Self {
        Self {
            timer: 0.0,
            intensity: 0.0,
        }
    }

    /// VladCharacter.Update has no invisibility or TimeWarp gate, and does
    /// not reset its fields when the speed/intensity gate stops submission.
    pub fn step_vlad(
        &mut self,
        dt: f32,
        speed_multiplier: f32,
        flat_speed_sq: f32,
    ) -> AfterImageStep {
        if !(speed_multiplier > 1.0 || self.intensity > -1.0) {
            return AfterImageStep::Hidden;
        }
        self.advance(dt, speed_multiplier, flat_speed_sq)
    }

    /// Avatar.Update: active while visible, rendered, and either
    /// `SpeedMultiplier > TimeWarpModifier` or the intensity is above -1.
    /// Each elapsed 0.05 s pushes a pose; moving (flat velocity² > 0.001)
    /// with SpeedMultiplier > 1 refills the intensity to (0, 0.05].
    pub fn step(
        &mut self,
        dt: f32,
        invisible: bool,
        speed_multiplier: f32,
        time_warp_modifier: f32,
        flat_speed_sq: f32,
    ) -> AfterImageStep {
        if invisible || !(speed_multiplier > time_warp_modifier || self.intensity > -1.0) {
            *self = Self::default();
            return AfterImageStep::Hidden;
        }
        self.advance(dt, speed_multiplier, flat_speed_sq)
    }

    fn advance(&mut self, dt: f32, speed_multiplier: f32, flat_speed_sq: f32) -> AfterImageStep {
        self.timer -= dt;
        self.intensity -= dt;
        let mut snapshots = 0;
        while self.timer <= 0.0 {
            self.timer += PERIOD;
            if flat_speed_sq > 0.001 && speed_multiplier > 1.0 {
                while self.intensity <= 0.0 {
                    self.intensity += PERIOD;
                }
            }
            snapshots += 1;
        }
        AfterImageStep::Drawn {
            snapshots,
            intensity: self.intensity / PERIOD,
        }
    }
}

/// AfterImageRenderData.Draw: alphas of ghosts 1..SNAPSHOTS (index 0 is
/// skipped without consuming a step); drawing stops once the alpha is not
/// positive.
pub fn ghost_alphas(intensity: f32) -> [Option<f32>; SNAPSHOTS - 1] {
    let step = BASE_ALPHA / (SNAPSHOTS as f32 + 1.0);
    let mut alpha = BASE_ALPHA + intensity * step;
    let mut out = [None; SNAPSHOTS - 1];
    for slot in out.iter_mut() {
        if alpha <= 0.0 {
            break;
        }
        *slot = Some(alpha);
        alpha -= step;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vlad_starts_at_zero_and_retains_fields_when_submission_stops() {
        let mut a = AfterImages::vlad();
        assert_eq!(a.intensity, 0.0);
        assert!(matches!(
            a.step_vlad(0.025, 1.5, 4.0),
            AfterImageStep::Drawn { snapshots: 1, .. }
        ));
        for _ in 0..60 {
            a.step_vlad(0.025, 1.0, 0.0);
        }
        let stopped = a;
        assert_eq!(a.step_vlad(1.0, 1.0, 0.0), AfterImageStep::Hidden);
        assert_eq!(a, stopped);
        assert!(matches!(
            a.step_vlad(0.05, 1.5, 4.0),
            AfterImageStep::Drawn { .. }
        ));
        assert!(a.intensity > 0.0);
    }

    #[test]
    fn vlad_moving_history_fades_without_motion_and_catches_up() {
        let mut a = AfterImages::vlad();
        let AfterImageStep::Drawn {
            snapshots,
            intensity,
        } = a.step_vlad(0.12, 1.5, 1.0)
        else {
            panic!()
        };
        assert_eq!(snapshots, 3);
        assert!(ghost_alphas(intensity).iter().all(Option::is_some));
        let AfterImageStep::Drawn { intensity, .. } = a.step_vlad(0.5, 1.5, 0.001) else {
            panic!()
        };
        assert!(ghost_alphas(intensity).iter().all(Option::is_none));
    }

    #[test]
    fn hasted_movement_keeps_full_intensity_and_snapshots_every_period() {
        let mut a = AfterImages::default();
        let AfterImageStep::Drawn {
            snapshots,
            intensity,
        } = a.step(0.016, false, 1.9, 1.0, 4.0)
        else {
            panic!("haste draws")
        };
        assert_eq!(snapshots, 1);
        assert!(intensity > 0.0 && intensity <= 1.0);
        let mut pushed = 0;
        for _ in 0..60 {
            if let AfterImageStep::Drawn { snapshots, .. } =
                a.step(1.0 / 60.0, false, 1.9, 1.0, 4.0)
            {
                pushed += snapshots;
            }
        }
        assert!((19..=21).contains(&pushed), "{pushed} pushes in 1 s");
    }

    #[test]
    fn standing_still_fades_then_hides_after_haste() {
        let mut a = AfterImages::default();
        a.step(0.05, false, 1.9, 1.0, 4.0);
        // Haste over, standing: drawn until the intensity falls to -1.
        let mut frames = 0;
        while let AfterImageStep::Drawn { .. } = a.step(0.01, false, 1.0, 1.0, 0.0) {
            frames += 1;
            assert!(frames < 200);
        }
        assert!((100..=106).contains(&frames), "{frames}");
        assert_eq!(a, AfterImages::default());
    }

    #[test]
    fn invisibility_and_normal_speed_hide_them() {
        let mut a = AfterImages::default();
        assert_eq!(a.step(0.1, true, 2.0, 1.0, 4.0), AfterImageStep::Hidden);
        assert_eq!(a.step(0.1, false, 1.0, 1.0, 4.0), AfterImageStep::Hidden);
    }

    #[test]
    fn alphas_step_down_from_the_intensity() {
        let step = 0.333 / 6.0;
        let a = ghost_alphas(1.0);
        assert!((a[0].unwrap() - (0.333 + step)).abs() < 1e-6);
        assert!((a[3].unwrap() - (0.333 - 2.0 * step)).abs() < 1e-6);
        assert_eq!(ghost_alphas(-6.0), [None; 4]);
        let a = ghost_alphas(-4.5);
        assert!(a[0].is_some() && a[1].is_some() && a[2].is_none());
    }
}
