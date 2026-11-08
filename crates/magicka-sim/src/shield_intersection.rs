//! HollowSphere segment fractions and non-wall Shield.ArcIntersect.
use crate::{arc_intersection::ArcQuery, impact_motion::Vector, spray_pulse::Segment};
#[derive(Clone, Copy, Debug)]
pub struct HollowSphere {
    pub center: Vector,
    pub radius: f32,
    /// Original transform.Orientation.Forward, in world coordinates.
    pub forward: Vector,
    pub max_angle: f32,
}
fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
impl HollowSphere {
    pub fn segment_fraction(&self, segment: Segment) -> Option<f32> {
        let offset = std::array::from_fn(|i| self.center[i] - segment.origin[i]);
        let a = dot(segment.delta, segment.delta);
        let b = dot(segment.delta, offset);
        let c = dot(offset, offset) - self.radius * self.radius;
        let root = ((b * b - a * c) as f64).sqrt() as f32;
        let inverse = 1.0 / a;
        let far = (b + root) * inverse;
        let near = (b - root) * inverse;
        let mut far_valid = far > 0.0 && far < 1.0 && !far.is_nan();
        let mut near_valid = near > 0.0 && near < 1.0 && !near.is_nan();
        if !far_valid && !near_valid {
            return None;
        }
        let angle = |t: f32| {
            let mut point: Vector =
                std::array::from_fn(|i| segment.origin[i] + segment.delta[i] * t - self.center[i]);
            let inv_length = 1.0 / (dot(point, point) as f64).sqrt() as f32;
            for value in &mut point {
                *value *= inv_length;
            }
            (dot(point, self.forward) as f64).acos() as f32
        };
        // NaN angular results do not satisfy > and therefore are not rejected.
        if angle(far) > self.max_angle {
            far_valid = false;
        }
        if angle(near) > self.max_angle {
            near_valid = false;
        }
        if far_valid && (far < near || !near_valid) {
            Some(far)
        } else if near_valid && (near < far || !far_valid) {
            Some(near)
        } else {
            None
        }
    }
    pub fn arc_intersect(&self, query: ArcQuery) -> Option<Vector> {
        let segment = Segment {
            origin: query.origin,
            delta: query.direction.map(|v| v * query.range),
        };
        self.segment_fraction(segment)
            .map(|t| std::array::from_fn(|i| segment.origin[i] + segment.delta[i] * t))
    }
}

/// World-transformed shield primitive. Wall uses collision primitive index 1;
/// other shield types use the hollow sphere at index 0.
#[derive(Clone, Copy, Debug)]
pub enum ShieldGeometry {
    Wall(crate::box_intersection::OrientedBox),
    Sphere(HollowSphere),
}
impl ShieldGeometry {
    pub fn segment_fraction(&self, segment: Segment) -> Option<f32> {
        match self {
            Self::Wall(wall) => wall.segment_intersect(segment).map(|h| h.fraction),
            Self::Sphere(sphere) => sphere.segment_fraction(segment),
        }
    }
    pub fn arc_intersect(&self, query: ArcQuery) -> Option<Vector> {
        let segment = Segment {
            origin: query.origin,
            delta: query.direction.map(|v| v * query.range),
        };
        self.segment_fraction(segment)
            .map(|t| std::array::from_fn(|i| segment.origin[i] + segment.delta[i] * t))
    }
}
