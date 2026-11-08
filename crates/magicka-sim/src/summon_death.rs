//! SummonDeath target selection and MagickDeath's host-independent state.
//! Animation clocks, ground queries and scythe collision belong to the host.
//! Evidence and unresolved integration: research/summon-death.md.
use crate::spells::Elements;

pub const QUERY_RANGE: f32 = 40.0;
pub const MODEL: &str = "Models/Bosses/Death/Death";
pub const CLIPS: [&str; 4] = [
    "move_glide",
    "attack_scythe_rise",
    "attack_scythe_fall",
    "hit",
];

/// Entries in the original entity query, including non-character entries.
/// The random rotation is drawn against the full list, not eligible targets.
#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    pub handle: u16,
    pub character: bool,
    pub dead: bool,
    pub undead: bool,
    pub hit_points: f32,
    pub max_hit_points: f32,
}

/// Lowest positive HP fraction <=1 wins. Equality replaces the previous
/// candidate in the randomly rotated traversal. Owner fallback has no filter.
pub fn select_target(
    entries: &[Candidate],
    start: usize,
    character_owner: Option<u16>,
) -> Option<u16> {
    let mut selected = None;
    let mut fraction = 1.0;
    if !entries.is_empty() {
        for offset in 0..entries.len() {
            let c = entries[(start % entries.len() + offset) % entries.len()];
            if !c.character || c.dead || c.undead {
                continue;
            }
            let health = c.hit_points / c.max_hit_points;
            if health > 0.0 && health <= fraction {
                selected = Some(c.handle);
                fraction = health;
            }
        }
    }
    selected.or(character_owner)
}

/// One native placement iteration, before Default navmesh snapping. Each
/// attempt adds to the previous snapped point, not the original target point.
pub fn placement_step(previous: [f32; 3], roll: f64) -> [f32; 3] {
    let angle = ((roll - 0.5) as f32 * 2.0) * std::f32::consts::PI;
    let (sin, cos) = crate::math_approximation::fast_sin_cos(angle);
    [
        previous[0] + cos * 7.0,
        previous[1],
        previous[2] + sin * 7.0,
    ]
}

pub fn placement_accepted(snapped: [f32; 3], target: [f32; 3]) -> bool {
    (0..3)
        .map(|i| (snapped[i] - target[i]).powi(2))
        .sum::<f32>()
        >= 4.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Move,
    ScytheRise,
    ScytheFall,
    Disappear,
}

/// AnimationController results after this frame's animation update.
#[derive(Clone, Copy, Debug, Default)]
pub struct AnimationClock {
    pub finished: bool,
    pub crossfade: bool,
    pub normalized_time: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub position: [f32; 3],
    pub dead: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Step {
    /// CrossFade(clip, seconds, loop:false) for the native animation host.
    pub crossfade: Option<(Phase, f32)>,
    /// Turn uses quaternion interpolation in the host; speed is per second.
    pub turn: Option<([f32; 3], f32)>,
    pub swing_cue: bool,
    pub scythe_window: bool,
    pub despawn_effect_and_cue: bool,
}

#[derive(Clone, Debug)]
pub struct MagickDeath {
    pub target: u16,
    pub position: [f32; 3],
    pub movement: [f32; 3],
    pub phase: Phase,
    pub ethereal: bool,
    pub dead: bool,
    pub alpha: f32,
    pub time_target: f32,
    pub speed: f32,
    pub spawn_delay: f32,
    pub range_squared: f32,
    pub is_hit: bool,
    scythe_cue_started: bool,
    death_cue_started: bool,
}

impl MagickDeath {
    /// Host ground-snaps the spawn point first; Initialize raises it 1.3 m.
    pub fn new(target: u16, ground_point: [f32; 3], target_radius: f32) -> Self {
        let mut position = ground_point;
        position[1] += 1.3000001;
        Self {
            target,
            position,
            movement: [0.0; 3],
            phase: Phase::Move,
            ethereal: true,
            dead: false,
            alpha: 0.0,
            time_target: 0.25,
            speed: 8.0,
            spawn_delay: 2.0,
            range_squared: (0.39600003 + target_radius * target_radius) * 1.2,
            is_hit: false,
            scythe_cue_started: false,
            death_cue_started: false,
        }
    }

    /// Native Update changes the global modifier before undoing scaled dt.
    /// Caller passes entity dt and the current PlayState TimeMultiplier.
    pub fn time_step(&self, entity_dt: f32, modifier: &mut f32, multiplier: f32) -> f32 {
        *modifier += (self.time_target - *modifier) * (entity_dt / self.time_target);
        entity_dt / (*modifier * multiplier)
    }

    /// Only Life with damage features, while corporeal, sets mIsHit. Native
    /// returns Hit even with features disabled; no amount threshold applies.
    pub fn receive_damage(&mut self, element: Elements, damage_features: bool) -> bool {
        if element.has(Elements::LIFE) && !self.ethereal {
            if damage_features {
                self.is_hit = true;
            }
            true
        } else {
            false
        }
    }

    /// Real-time dt after time_step. Ground Y is the scene segment result,
    /// already raised 1.2 m by the host when the ground segment hits.
    pub fn update(
        &mut self,
        dt: f32,
        ground_y: f32,
        target: Option<Target>,
        clock: AnimationClock,
    ) -> Step {
        self.advance_position(dt, ground_y);
        self.update_after_movement(dt, target, clock)
    }

    /// Runtime moves the body before evaluating its animation and scythe pose.
    pub fn advance_position(&mut self, dt: f32, ground_y: f32) {
        self.position[0] += self.movement[0] * self.speed * dt;
        self.position[2] += self.movement[2] * self.speed * dt;
        self.position[1] += (ground_y - self.position[1]) * dt;
    }

    pub fn update_after_movement(
        &mut self,
        dt: f32,
        target: Option<Target>,
        clock: AnimationClock,
    ) -> Step {
        let mut out = Step::default();
        let Some(target) = target.filter(|t| !t.dead) else {
            self.dead = true;
            return out;
        };
        if self.dead {
            return out;
        }
        let dx = target.position[0] - self.position[0];
        let dz = target.position[2] - self.position[2];
        let distance_squared = dx * dx + dz * dz;
        let direction = if distance_squared > 0.0 {
            let inverse = distance_squared.sqrt().recip();
            [dx * inverse, 0.0, dz * inverse]
        } else {
            [0.0; 3]
        };
        match self.phase {
            Phase::Move => {
                self.alpha = (self.alpha + dt).min(1.0);
                out.turn = Some((direction, 8.0));
                self.spawn_delay -= dt;
                if self.spawn_delay <= 0.0 {
                    self.movement = direction;
                }
                if distance_squared <= self.range_squared {
                    self.speed = 0.0;
                    self.phase = Phase::ScytheRise;
                    self.ethereal = false;
                    out.crossfade = Some((self.phase, 0.33));
                    out.swing_cue = true;
                }
            }
            Phase::ScytheRise => {
                self.alpha = (self.alpha + dt).min(1.0);
                if clock.finished && !clock.crossfade {
                    self.phase = Phase::ScytheFall;
                    out.crossfade = Some((self.phase, 0.1));
                }
            }
            Phase::ScytheFall => {
                self.alpha = (self.alpha + dt).min(1.0);
                let amount = (distance_squared - self.range_squared * 1.1).clamp(0.0, 1.0);
                self.movement = direction.map(|v| v * amount);
                out.turn = Some((self.movement, amount * 4.0));
                out.scythe_window =
                    !clock.crossfade && (0.1..=0.8).contains(&clock.normalized_time);
                if out.scythe_window && !self.scythe_cue_started {
                    self.scythe_cue_started = true;
                    out.swing_cue = true;
                }
                if self.is_hit || (clock.finished && !clock.crossfade) {
                    self.phase = Phase::Disappear;
                    self.alpha = 1.0;
                    out.crossfade = Some((self.phase, 0.33));
                }
            }
            Phase::Disappear => {
                self.alpha = (self.alpha - dt).max(0.0);
                if !self.death_cue_started {
                    self.death_cue_started = true;
                    out.despawn_effect_and_cue = true;
                }
                self.time_target = 1.0;
                if clock.finished && !clock.crossfade {
                    self.dead = true;
                }
            }
        }
        out
    }

    /// Kill/Deinitialize resets the global modifier, not merely the target.
    pub fn remove(&mut self, modifier: &mut f32) {
        self.dead = true;
        *modifier = 1.0;
    }
}
/// Native MagickDeath's five shared render-buffer snapshots. Empty slots model
/// the invalid skeleton matrices used before a slot receives an actual pose.
#[derive(Clone, Debug)]
pub struct Afterimages<T> {
    pub poses: [Option<T>; 5],
    pub timer: f32,
    pub intensity: f32,
}

impl<T> Default for Afterimages<T> {
    fn default() -> Self {
        Self {
            poses: std::array::from_fn(|_| None),
            timer: 0.0,
            intensity: 0.0,
        }
    }
}

impl<T: Clone> Afterimages<T> {
    /// Initialize clears skeletons, but retains the singleton's timers.
    pub fn clear_poses(&mut self) {
        self.poses.fill(None);
    }

    /// Update IL_0838..09ce. Catch-up shifts the same current pose each time;
    /// no interpolated historical positions are invented.
    pub fn update(&mut self, dt: f32, movement_squared: f32, speed: f32, pose: T) -> bool {
        if movement_squared <= 0.5 || movement_squared.is_nan() {
            return false;
        }
        self.timer -= dt;
        self.intensity -= dt;
        while self.timer <= 0.0 {
            self.timer += 0.0375;
            if speed > 0.0 {
                while self.intensity <= 0.0 {
                    self.intensity += 0.05;
                }
            }
            self.poses.rotate_right(1);
            self.poses[0] = Some(pose.clone());
        }
        true
    }

    /// AfterImageRenderData.Draw decrements only when alpha is nonzero.
    /// Negative values are retained: native code neither clamps nor stops.
    pub fn draw_alphas(&self) -> [f32; 5] {
        let step = 0.333 / 6.0;
        let mut alpha = 0.333 + self.intensity * 20.0 * step;
        std::array::from_fn(|_| {
            let out = alpha;
            if alpha != 0.0 {
                alpha -= step;
            }
            out
        })
    }
}
