//! TitleRenderer: the fading title/subtitle pair shown by BossFight.SetTitles
//! (BossTitle) and Level.DisplayTitle (DisplayTitleInLevel). Drawn by
//! TitleRenderData (ZIndex 205) in Stonecross36 / Stonecross28, centred at
//! (w/2 + 0.5, 0.3h + 0.5), the subtitle one title height lower, both with
//! black (2, 2) shadows at the title alpha (research/title-renderer.md).

/// TitleRenderer's timer. The fade-in phase advances the timer in units of
/// the fade-in time but compares it with seconds (DisplayTime + FadeIn), so a
/// FadeIn other than 1 also scales the hold; the native quirk is kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TitleRenderer {
    /// Unlocalized `#key` strings as authored (lowercased by Action.Read).
    pub title: String,
    pub subtitle: String,
    fade_in_time: f32,
    fade_out_time: f32,
    display_time: f32,
    timer: f32,
    fading_in: bool,
    started: bool,
    stopped: bool,
}

impl TitleRenderer {
    /// SetTitles: fade times clamp to at least 0.01.
    pub fn set_titles(
        &mut self,
        title: &str,
        subtitle: &str,
        display_time: f32,
        fade_in: f32,
        fade_out: f32,
    ) {
        self.fade_in_time = fade_in.max(0.01);
        self.fade_out_time = fade_out.max(0.01);
        self.display_time = display_time;
        self.title = title.to_owned();
        self.subtitle = subtitle.to_owned();
        self.fading_in = true;
    }

    pub fn start(&mut self) {
        self.fading_in = true;
        self.started = true;
        self.stopped = false;
        self.timer = 0.0;
    }

    pub fn reset_timer(&mut self) {
        self.fading_in = true;
        self.timer = 0.0;
    }

    /// Update: the render alpha min(timer, 1), or None once stopped (or
    /// before Start). The update that stops still returns its last alpha.
    pub fn update(&mut self, dt: f32) -> Option<f32> {
        if self.stopped || !self.started {
            return None;
        }
        if self.fading_in {
            self.timer += dt * (1.0 / self.fade_in_time);
            if self.timer > self.display_time + self.fade_in_time {
                self.fading_in = false;
                self.timer = 1.0;
            }
        } else {
            self.timer -= dt * (1.0 / self.fade_out_time);
            if self.timer <= 0.0 {
                self.stopped = true;
            }
        }
        Some(self.timer.min(1.0))
    }
}

/// Whose title it is, which decides how its strings are localized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TitleKind {
    /// BossTitle.Initialize: GetString(hash).ToUpper() for each present
    /// string ("NOT FOUND: id" is kept).
    Boss,
    /// DisplayTitleInLevel.Initialize: translated unless NoTranslation, a
    /// "NOT FOUND:" result becomes empty; untranslated strings are upper-cased.
    Level {
        no_translation: bool,
        align: crate::text_geometry::TextAlign,
    },
}

/// One title submitted for drawing this update.
#[derive(Clone, Debug, PartialEq)]
pub struct ShownTitle {
    pub title: String,
    pub subtitle: String,
    pub alpha: f32,
    pub kind: TitleKind,
}

impl ShownTitle {
    pub fn of(renderer: &TitleRenderer, alpha: f32, kind: TitleKind) -> Self {
        Self {
            title: renderer.title.clone(),
            subtitle: renderer.subtitle.clone(),
            alpha,
            kind,
        }
    }

    /// The (title, subtitle) display strings.
    pub fn resolve(&self, strings: &crate::localization::LocalizationTable) -> (String, String) {
        let one = |key: &str| -> String {
            if key.is_empty() {
                return String::new();
            }
            let lookup = || {
                let id = crate::scene_loading::custom_hash(&key.to_lowercase());
                strings.get_string(id).unwrap_or_default().to_uppercase()
            };
            match self.kind {
                TitleKind::Boss => lookup(),
                TitleKind::Level {
                    no_translation: false,
                    ..
                } => {
                    let s = lookup();
                    if s.starts_with("NOT FOUND:") {
                        String::new()
                    } else {
                        s
                    }
                }
                TitleKind::Level { .. } => key.to_uppercase(),
            }
        };
        (one(&self.title), one(&self.subtitle))
    }
}

#[cfg(test)]
mod tests {
    use super::TitleRenderer;

    /// Updates of an exactly representable 1/8 s.
    fn run(r: &mut TitleRenderer, updates: usize) -> Vec<Option<f32>> {
        (0..updates).map(|_| r.update(0.125)).collect()
    }

    #[test]
    fn castle_beholder_title_fades_in_holds_and_fades_out() {
        // wc_s3: DisplayTime 3, FadeIn 1, FadeOut 0.5.
        let mut r = TitleRenderer::default();
        assert_eq!(r.update(0.125), None, "nothing renders before Start");
        r.set_titles("#boss_n16", "#boss_t16", 3.0, 1.0, 0.5);
        r.start();
        let f = run(&mut r, 38);
        assert_eq!(f[3], Some(0.5));
        assert_eq!(f[7], Some(1.0));
        // Timer 4 is not past DisplayTime + FadeIn; 4.125 restarts at 1.
        assert_eq!(f[31], Some(1.0));
        assert_eq!(f[32], Some(1.0));
        assert_eq!(f[33], Some(0.75));
        assert_eq!(f[36], Some(0.0), "the stopping update still renders");
        assert_eq!(f[37], None);
    }

    #[test]
    fn fade_in_time_scales_the_hold_like_native() {
        // Timer units are fade-ins; the hold compares them with seconds.
        let mut r = TitleRenderer::default();
        r.set_titles("a", "", 1.0, 0.5, 1.0);
        r.start();
        let f = run(&mut r, 8);
        assert_eq!(f[5], Some(1.0));
        assert_eq!(f[6], Some(1.0));
        assert_eq!(f[7], Some(0.875));
    }

    #[test]
    fn reset_timer_does_not_restart_a_stopped_title() {
        let mut r = TitleRenderer::default();
        r.set_titles("a", "b", 0.0, 1.0, 1.0);
        r.start();
        let f = run(&mut r, 18);
        assert_eq!(f[16], Some(0.0));
        assert_eq!(f[17], None);
        r.reset_timer();
        assert_eq!(r.update(0.125), None, "ResetTimer does not clear Stopped");
        r.start();
        assert_eq!(r.update(0.125), Some(0.125));
    }
}
