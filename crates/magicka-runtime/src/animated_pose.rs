//! AnimatedLevelPart time and pose arithmetic; callbacks/world updates are pending.
use magicka_content::{
    level::Keyframe,
    skinned::{self, Mat},
};
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PartPlayback {
    pub time: f32,
    pub start: f32,
    pub end: f32,
    pub speed: f32,
    pub looping: bool,
    pub playing: bool,
    pub resting_timer: f32,
}
impl PartPlayback {
    /// Recovered constructor values; Default remains a neutral test value.
    pub fn new(duration: f32) -> Self {
        Self {
            end: duration,
            speed: 1.0,
            resting_timer: -1.0,
            ..Default::default()
        }
    }
    /// Local Stop. Recursive child commands and server notification belong to
    /// the owner; Stop does not reset time or the interval.
    pub fn stop(&mut self) {
        self.playing = false;
    }
    /// Local Resume, distinct from Play's resume flag. Preserve mutations before
    /// rejecting a negative length, as the original does before throwing.
    pub fn resume(
        &mut self,
        duration: f32,
        length: f32,
        speed: f32,
        looping: Option<bool>,
    ) -> Result<(), ResumeError> {
        self.speed = speed;
        if let Some(looping) = looping {
            self.looping = looping;
        }
        // CLR Single.Epsilon is the smallest positive subnormal, not Rust EPSILON.
        let epsilon = f32::from_bits(1);
        if length > epsilon {
            self.end = if speed < 0.0 {
                self.time - length
            } else {
                self.time + length
            };
        } else {
            if length < -epsilon {
                return Err(ResumeError::NegativeLength);
            }
            self.end = duration;
        }
        self.playing = true;
        Ok(())
    }
    /// Local Play operation. Caller applies all-children recursion first with
    /// resume=false, as in the original command.
    pub fn play(
        &mut self,
        duration: f32,
        mut start: f32,
        mut end: f32,
        speed: f32,
        looping: bool,
        resume: bool,
    ) {
        self.speed = speed;
        if start < 0.0 {
            start = 0.0;
        }
        if end < 0.0 {
            end = duration;
        }
        if speed < 0.0 {
            self.start = start.max(end);
            self.end = start.min(end);
        } else {
            self.start = start.min(end);
            self.end = start.max(end);
        }
        if resume {
            if speed >= 0.0 {
                if self.time < self.start {
                    self.time = self.start;
                } else if self.time > self.end {
                    self.time = self.end;
                }
            } else if self.time > self.start {
                self.time = self.start;
            } else if self.time < self.end {
                self.time = self.end;
            }
        } else {
            self.time = self.start;
        }
        self.looping = looping;
        self.playing = true;
    }
    pub fn advance(&mut self, dt: f32) {
        if !self.playing {
            return;
        }
        self.resting_timer = 1.0;
        let previous = self.time;
        self.time += dt * self.speed;
        if self.speed > 0.0 {
            if previous <= self.end && self.time > self.end {
                if self.looping {
                    self.time -= self.end - self.start;
                } else {
                    self.time = self.end;
                    self.playing = false;
                }
            }
        } else if previous >= self.end && self.time < self.end {
            if self.looping {
                self.time += self.end - self.start;
            } else {
                self.time = self.end;
                self.playing = false;
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumeError {
    NegativeLength,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoseError {
    EmptyChannel,
}
pub fn local_pose(keys: &[Keyframe], time: f32) -> Result<Mat, PoseError> {
    if keys.is_empty() {
        return Err(PoseError::EmptyChannel);
    }
    let (mut low, mut high, mut index) = (0isize, keys.len() as isize - 1, 0usize);
    while high >= low {
        index = ((low + high) / 2) as usize;
        if keys[index].time < time {
            low = index as isize + 1;
        } else if keys[index].time > time {
            high = index as isize - 1;
        } else {
            break;
        }
    }
    if index > 0 && keys[index].time > time {
        index -= 1;
    }
    let next = (index + 1) % keys.len();
    let a = &keys[index];
    let b = &keys[next];
    let span = if next != 0 { b.time - a.time } else { 0.0 };
    if span > 0.0 {
        let amount = ((time - a.time) / span).clamp(0.0, 1.0);
        let lerp =
            |x: [f32; 3], y: [f32; 3]| std::array::from_fn(|i| x[i] + (y[i] - x[i]) * amount);
        Ok(skinned::pose_matrix(
            lerp(a.translation, b.translation),
            skinned::xna_quat_lerp(a.orientation, b.orientation, amount),
            lerp(a.scale, b.scale),
        ))
    } else {
        Ok(skinned::pose_matrix(a.translation, a.orientation, a.scale))
    }
}
pub fn initial_world_pose(keys: &[Keyframe], time: f32, parent: &Mat) -> Result<Mat, PoseError> {
    Ok(skinned::mul(parent, &local_pose(keys, time)?))
}
pub fn updated_world_pose(keys: &[Keyframe], time: f32, parent: &Mat) -> Result<Mat, PoseError> {
    Ok(skinned::mul(&local_pose(keys, time)?, parent))
}
