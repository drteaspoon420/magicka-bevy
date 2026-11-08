//! JigLibX Box.SegmentIntersect with world-space center and orientation axes.
use crate::{impact_motion::Vector, spray_pulse::Segment};
#[derive(Clone, Copy, Debug)]
pub struct OrientedBox {
    pub center: Vector,
    /// XNA Right, Up, Backward, respectively.
    pub axes: [Vector; 3],
    pub side_lengths: Vector,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxHit {
    pub fraction: f32,
    pub position: Vector,
    pub normal: Vector,
}
fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
impl OrientedBox {
    pub fn segment_intersect(&self, segment: Segment) -> Option<BoxHit> {
        let offset = std::array::from_fn(|i| self.center[i] - segment.origin[i]);
        let mut entry = f32::MIN;
        let mut exit = f32::MAX;
        let mut entry_axis = 0;
        let mut exit_axis = 0;
        for i in 0..3 {
            let e = dot(self.axes[i], offset);
            let f = dot(self.axes[i], segment.delta);
            let half = self.side_lengths[i] * 0.5;
            if f.abs() > 1e-6 {
                let mut near = (e + half) / f;
                let mut far = (e - half) / f;
                if near > far {
                    std::mem::swap(&mut near, &mut far);
                }
                if near > entry {
                    entry = near;
                    entry_axis = i;
                }
                if far < exit {
                    exit = far;
                    exit_axis = i;
                }
                if entry > exit || exit < 0.0 {
                    return None;
                }
            } else if 0.0 - e - half > 0.0 || 0.0 - e + half < 0.0 {
                return None;
            }
        }
        let (fraction, axis) = if (0.0..=1.0).contains(&entry) {
            (entry, entry_axis)
        } else if exit <= 1.0 {
            (exit, exit_axis)
        } else {
            return None;
        };
        let fraction = fraction.clamp(0.0, 1.0);
        let normal = if dot(self.axes[axis], segment.delta) > 0.0 {
            self.axes[axis].map(|v| -v)
        } else {
            self.axes[axis]
        };
        Some(BoxHit {
            fraction,
            position: std::array::from_fn(|i| segment.origin[i] + segment.delta[i] * fraction),
            normal,
        })
    }
}
