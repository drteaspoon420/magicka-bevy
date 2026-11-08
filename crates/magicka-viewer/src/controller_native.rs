//! Retail XInput gesture and tutorial layout, recovered from static CIL.
//! Provenance: output/research/controller-support.md.
use bevy::prelude::Vec2;

// Right, Up, Left, Down. Output indexes follow PlayerInput::elements.
pub const ELEMENT_GESTURES: [(usize, usize); 8] = [
    (2, 1),
    (1, 2),
    (3, 0),
    (0, 1),
    (2, 3),
    (1, 0),
    (3, 2),
    (0, 3),
];
#[derive(Default)]
pub struct Gesture {
    pub direction: Option<usize>,
    locked: bool,
}
impl Gesture {
    pub fn update(&mut self, value: Vec2, help: bool) -> Option<usize> {
        if help {
            return None;
        }
        if !value.is_finite() || value.length() < 0.3 {
            self.direction = None;
            self.locked = false;
            return None;
        }
        if self.locked || value.length() < 0.6 {
            return None;
        }
        let next = if let Some(current) = self.direction {
            if (current % 2 == 0 && value.y.abs() > value.x.abs())
                || (current % 2 == 1 && value.x.abs() > value.y.abs())
            {
                cardinal(value)
            } else {
                current
            }
        } else {
            let candidates = [Vec2::X, Vec2::Y, Vec2::NEG_X, Vec2::NEG_Y];
            let found = candidates.iter().position(|v| v.distance(value) < 0.5);
            self.direction = found;
            return None;
        };
        let first = self.direction?;
        if first == next {
            return None;
        }
        self.locked = true;
        ELEMENT_GESTURES.iter().position(|p| *p == (first, next))
    }
}
fn cardinal(v: Vec2) -> usize {
    if v.x.abs() >= v.y.abs() {
        if v.x >= 0. {
            0
        } else {
            2
        }
    } else if v.y >= 0. {
        1
    } else {
        3
    }
}
pub fn hint_layout(
    position: i32,
    extent: [f32; 2],
    animation: i32,
    screen: [f32; 2],
) -> ([f32; 2], [f32; 2]) {
    let size = [
        extent[0] + if animation != 0 { 128. } else { 0. },
        extent[1],
    ];
    let [w, h] = screen;
    let center = match position {
        1 => [w * 0.5, h * 0.1],
        3 => [w + w * 0.05 - size[0] * 0.5, h * 0.7],
        4 => [w * 0.5, h * 0.7],
        5 => [w * 0.95 - size[0] * 0.5, h * 0.7],
        8 => [w * 0.95 - size[0] * 0.5, h * 0.95 - size[1] * 0.5],
        _ => [0.; 2],
    };
    (center.map(f32::floor), size)
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub animation_bit: usize,
    pub glyph: usize,
    /// Center relative to the native hint icon origin.
    pub offset: [f32; 2],
    pub size: [f32; 2],
    pub pressed: bool,
}
pub fn tutorial_slot(animation: i32, time: f32) -> Option<Slot> {
    let durations = [
        1., 1.5, 2., 5., 5., 5., 5., 5., 5., 5., 5., 1.5, 1.5, 1.5, 1.5, 1., 1.,
    ];
    let duration: f32 = durations
        .iter()
        .enumerate()
        .filter(|(i, _)| animation & (1 << i) != 0)
        .map(|(_, d)| d)
        .sum();
    if duration <= 0. {
        return None;
    }
    let mut phase = time.rem_euclid(duration);
    for (i, d) in durations.into_iter().enumerate() {
        if animation & (1 << i) == 0 {
            continue;
        }
        if phase >= d {
            phase -= d;
            continue;
        }
        let glyph = match i {
            0 => 2, // Interact: X / Square
            1 => {
                if phase < 0.5 {
                    14
                } else {
                    13
                }
            } // Magick next/previous
            2..=10 => 11, // right stick / right trackpad
            11 => 7,
            12 => 6,
            13 => 3,
            14 => 1,
            15 => 5,
            16 => 0,
            _ => unreachable!(),
        };
        let mut offset = [0.; 2];
        if (3..=10).contains(&i) {
            let (first, second) = ELEMENT_GESTURES[i - 3];
            let vectors = [[1., 0.], [0., -1.], [-1., 0.], [0., 1.]];
            if phase > 1. && phase < 3. {
                let amount = (phase - 1.).min(1.) * 18.;
                offset = vectors[first].map(|v| v * amount);
            } else if phase >= 3. {
                let a = ((phase - 3.).min(1.) * std::f32::consts::FRAC_PI_2).sin();
                let b = ((phase - 3.).min(1.) * std::f32::consts::FRAC_PI_2).cos();
                offset =
                    std::array::from_fn(|j| 18. * (vectors[first][j] * b + vectors[second][j] * a));
            }
        }
        return Some(Slot {
            animation_bit: i,
            glyph,
            offset,
            size: [64., 64.],
            pressed: phase >= 0.5 && !(2..=10).contains(&i),
        });
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_native_element_gestures_commit_once_until_centered() {
        let values = [Vec2::X, Vec2::Y, Vec2::NEG_X, Vec2::NEG_Y];
        for (element, (first, second)) in ELEMENT_GESTURES.into_iter().enumerate() {
            let mut gesture = Gesture::default();
            assert_eq!(gesture.update(values[first], false), None);
            assert_eq!(gesture.update(values[second], false), Some(element));
            assert_eq!(gesture.update(values[first], false), None);
            gesture.update(Vec2::ZERO, false);
            gesture.update(values[first], false);
            assert_eq!(gesture.update(values[second], false), Some(element));
        }
    }
    #[test]
    fn tutorial_slots_follow_native_schedule_and_placement() {
        assert_eq!(
            hint_layout(8, [300., 60.], 1, [1280., 720.]),
            ([1002., 654.], [428., 60.])
        );
        assert_eq!(tutorial_slot(1 | 2048, 0.75).unwrap().glyph, 2);
        assert_eq!(tutorial_slot(1 | 2048, 1.25).unwrap().glyph, 7);
        assert_eq!(tutorial_slot(1 | 2048, 2.5).unwrap().glyph, 2);
        assert_eq!(tutorial_slot(0, 0.), None);
        assert_eq!(tutorial_slot(8, 2.).unwrap().offset, [-18., 0.]);
        let water = tutorial_slot(8, 4.5).unwrap();
        assert!((water.offset[1] + 18.).abs() < 0.0001);
    }
}

#[derive(Default)]
pub struct Wheel {
    pub fade: [f32; 4],
    pub movement: [f32; 4],
}
impl Wheel {
    pub fn update(&mut self, value: Vec2, help: bool, dt: f32) {
        let direction = (value.length() >= 0.3).then(|| cardinal(value));
        for i in 0..4 {
            if help || direction == Some(i) {
                self.fade[i] = (self.fade[i] + dt).min(0.2);
                self.movement[i] = (self.movement[i] + dt).min(0.2);
            } else {
                self.fade[i] = (self.fade[i] - dt).max(0.);
                if self.fade[i] == 0. {
                    self.movement[i] = 0.;
                }
            }
        }
    }
}

/// KeyboardHUD uses the element bit order; gameplay input uses Q W E R A S D F.
pub const HUD_TO_INPUT: [usize; 8] = [6, 0, 3, 7, 4, 5, 1, 2];
pub fn element_glyphs(hud_index: usize, style: super::ElementStyle) -> Vec<usize> {
    let element = HUD_TO_INPUT[hud_index];
    if style == super::ElementStyle::RightStick {
        let (first, second) = ELEMENT_GESTURES[element];
        vec![17 + first, 21, 17 + second]
    } else {
        let direction = [13, 14, 15, 16][element % 4];
        if element >= 4 {
            vec![4, 22, direction]
        } else {
            vec![direction]
        }
    }
}

/// Element-guide glyph and key-background bounds around a key anchor.
/// A single control keeps the original KeyboardHUD key background; a
/// two-control sequence gets one smaller background per control. Pairs are
/// 44 pixels wide in the 50-pixel column pitch, so the 2-pixel gap inside a
/// pair stays narrower than the 6-pixel gap to the neighbouring pair.
pub fn element_layout(glyphs: &[usize], [x, y]: [f32; 2]) -> (Vec<[f32; 4]>, Vec<[f32; 4]>) {
    if glyphs.len() < 3 {
        // Original key background [x−14, y, x+11, y+25] is centred at x−1.5.
        let control = [x - 11.5, y + 2.5, x + 8.5, y + 22.5];
        return (
            glyphs.iter().map(|_| control).collect(),
            vec![[x - 14., y, x + 11., y + 25.]],
        );
    }
    let control = |cx: f32| [cx - 9., y + 3.5, cx + 9., y + 21.5];
    let bounds = glyphs
        .iter()
        .enumerate()
        .map(|(i, glyph)| match i {
            0 => control(x - 11.5),
            _ if *glyph >= 21 => [x - 3., y + 8.5, x + 3., y + 16.5],
            _ => control(x + 11.5),
        })
        .collect();
    let badges = vec![
        [x - 22., y + 2., x - 1., y + 23.],
        [x + 1., y + 2., x + 22., y + 23.],
    ];
    (bounds, badges)
}
