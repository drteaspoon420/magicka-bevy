//! Item.ExecuteGun/StopGunfire/Update: animation-started timed clips.
#[derive(Clone, Debug, Default)]
pub struct GunState {
    pub firing: bool,
    pub clip: i32,
    pub timer: f32,
    pub accuracy: f32,
    pub tracer_count: u32,
    pub start_presentation: bool,
    pub stop_presentation: bool,
    pub cue: Option<u32>,
}
impl GunState {
    pub fn execute(&mut self, clip: i32, accuracy: f32) {
        self.clip = clip;
        self.accuracy = accuracy;
        if self.timer <= 0. {
            self.timer = f32::from_bits(1);
            self.start_presentation = true;
        }
        self.firing = true;
    }
    pub fn stop(&mut self, avatar: bool, attacking: bool, class: u8) {
        if self.firing && (!avatar || (!attacking && class == 16) || class == 14 || class == 15) {
            self.clip = 0;
            self.timer = 0.;
            self.firing = false;
            self.stop_presentation = true;
        }
    }
    /// Native emits at most one shot per Item.Update; no catch-up loop.
    // `!(timer > 0)` transcribes the native test, NaN included.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    pub fn tick(&mut self, dt: f32, rate: i32) -> bool {
        if !(self.timer > 0.) || self.clip <= 0 {
            return false;
        }
        self.timer -= dt;
        if self.timer > 0. {
            return false;
        }
        if self.firing {
            self.timer += 1. / rate as f32;
            self.clip -= 1;
        }
        true
    }
}

pub fn spread(direction: [f32; 3], accuracy: f32, yaw_random: f32, pitch_random: f32) -> [f32; 3] {
    let width = (1. - accuracy) * 0.2;
    let angle =
        (direction[2] as f64).atan2(direction[0] as f64) as f32 + (yaw_random - 0.5) * width;
    let mut result = [
        (angle as f64).cos() as f32,
        direction[1] + (pitch_random * 4. - 2.) * width,
        (angle as f64).sin() as f32,
    ];
    let length = result.iter().map(|x| x * x).sum::<f32>().sqrt();
    for value in &mut result {
        *value /= length;
    }
    result
}

#[derive(Clone, Debug)]
pub struct Shot {
    pub owner: u16,
    pub weapon: i32,
    pub origin: [f32; 3],
    pub end: [f32; 3],
    pub target: Option<u16>,
    pub velocity: f32,
    pub sprite: Option<u8>,
}

pub fn sprite(spec: &str) -> Option<u8> {
    let (sheet, number) = spec.split_once('/').map_or((0, spec), |(sheet, number)| {
        (
            sheet
                .to_ascii_uppercase()
                .bytes()
                .next()
                .unwrap_or(b'A')
                .saturating_sub(b'A') as u16
                * 64,
            number,
        )
    });
    let number = number.parse::<f32>().ok()?;
    let index = sheet as f32 + number;
    (index >= 0. && index.is_finite()).then_some(index as u32 as u8)
}
