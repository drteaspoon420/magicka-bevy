//! Avatar.Boost queue and BoostState host operations.
#[derive(Clone, Debug, Default)]
pub struct ShieldBoost {
    pub count: i32,
    pub cooldown: f32,
    pub target: Option<u16>,
    pub was_down: bool,
}

impl ShieldBoost {
    pub fn press(
        &mut self,
        already_boosting: bool,
        passive: bool,
        multipliers: impl IntoIterator<Item = f32>,
    ) {
        self.count += 1;
        if passive {
            self.count += 1;
        }
        self.cooldown = 0.25;
        // Native first-press reset occurs AFTER the passive increment.
        if !already_boosting {
            self.count = 1;
        }
        for multiplier in multipliers {
            self.count *= multiplier as i32;
        }
    }
}
