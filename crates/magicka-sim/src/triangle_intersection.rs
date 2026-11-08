//! JigLibX Intersection.SegmentTriangleIntersection.
use crate::{impact_motion::Vector, spray_pulse::Segment};
#[derive(Clone, Copy, Debug)]
pub struct Triangle {
    pub origin: Vector,
    pub edge0: Vector,
    pub edge1: Vector,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriangleHit {
    pub fraction: f32,
    pub barycentric0: f32,
    pub barycentric1: f32,
}
fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
impl Triangle {
    pub fn from_vertices(a: Vector, b: Vector, c: Vector) -> Self {
        Self {
            origin: a,
            edge0: std::array::from_fn(|i| b[i] - a[i]),
            edge1: std::array::from_fn(|i| c[i] - a[i]),
        }
    }
    pub fn segment_intersect(
        &self,
        segment: Segment,
        ignore_backfaces: bool,
    ) -> Option<TriangleHit> {
        let mut normal = cross(self.edge0, self.edge1);
        let length_squared = dot(normal, normal);
        if length_squared != 0.0 {
            let inverse_length = 1.0 / (length_squared as f64).sqrt() as f32;
            for value in &mut normal {
                *value *= inverse_length;
            }
        }
        if ignore_backfaces && dot(segment.delta, normal) >= 0.0 {
            return None;
        }
        let p = cross(segment.delta, self.edge1);
        let determinant = dot(self.edge0, p);
        if determinant > -1e-6 && determinant < 1e-6 {
            return None;
        }
        let inverse = 1.0 / determinant;
        let offset = std::array::from_fn(|i| segment.origin[i] - self.origin[i]);
        let u = dot(offset, p) * inverse;
        if u < 0.0 || u > 1.0 {
            return None;
        }
        let q = cross(offset, self.edge0);
        let v = dot(segment.delta, q) * inverse;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = dot(self.edge1, q) * inverse;
        if t < 0.0 || t > 1.0 {
            return None;
        }
        Some(TriangleHit {
            fraction: t,
            barycentric0: u,
            barycentric1: v,
        })
    }
}
