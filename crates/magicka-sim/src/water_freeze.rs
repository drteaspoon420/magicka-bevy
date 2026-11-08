//! Water freeze-vertex state; spatial arc selection remains a host query.

/// IceMesh arc candidate box, including its reused negative-Z test vector.
/// Yaw endpoints are mathematically equivalent to XNA quaternion rotation;
/// external-framework rounding parity is not established.
pub fn arc_bounds(
    mut origin: [f32; 3],
    mut direction: [f32; 3],
    spread: f32,
) -> crate::mesh_octree::Bounds {
    origin[1] = 0.0;
    direction[1] = 0.0;
    let range = ((direction[0] * direction[0] + direction[2] * direction[2]) as f64).sqrt() as f32;
    let normalized = direction.map(|v| v / range);
    let angle = |v: [f32; 3]| {
        ((normalized[0] * v[0] + normalized[2] * v[2]).clamp(-1.0, 1.0) as f64).acos() as f32
    };
    let mut bounds = crate::mesh_octree::Bounds {
        min: [origin[0], -10000.0, origin[2]],
        max: [origin[0], 10000.0, origin[2]],
    };
    let mut include = |p: [f32; 3]| {
        for i in [0, 2] {
            bounds.min[i] = bounds.min[i].min(p[i]);
            bounds.max[i] = bounds.max[i].max(p[i]);
        }
    };
    if angle([-1.0, 0.0, 0.0]) <= spread {
        let mut p = origin;
        p[0] -= range;
        include(p);
    }
    let mut temporary = [1.0, 0.0, 0.0];
    if angle(temporary) <= spread {
        temporary = origin;
        temporary[0] += range;
        include(temporary);
    }
    temporary[2] = -1.0;
    if angle(temporary) <= spread {
        let mut p = origin;
        p[2] -= range;
        include(p);
    }
    if angle([0.0, 0.0, 1.0]) <= spread {
        let mut p = origin;
        p[2] += range;
        include(p);
    }
    for yaw in [spread, -spread] {
        let (sin, cos) = ((yaw as f64).sin() as f32, (yaw as f64).cos() as f32);
        include([
            origin[0] + direction[0] * cos + direction[2] * sin,
            0.0,
            origin[2] - direction[0] * sin + direction[2] * cos,
        ]);
    }
    bounds
}

/// IceMesh.GetVerticesIntersectingArc after octree triangle selection.
/// Candidates are supplied in octree order; each contributes its vertices
/// in triangle order. Geometry and Y are local-space, and Y is discarded.
pub fn arc_vertices(
    vertices: &[[f32; 3]],
    triangles: &[[usize; 3]],
    candidates: &[usize],
    origin: [f32; 3],
    direction: [f32; 3],
    spread: f32,
    capacity: usize,
) -> Vec<usize> {
    let range_sq = direction[0] * direction[0] + direction[2] * direction[2];
    let range = (range_sq as f64).sqrt() as f32;
    let normalized = [direction[0] / range, direction[2] / range];
    let mut output = Vec::new();
    for &triangle in candidates {
        for &index in &triangles[triangle] {
            if output.len() >= capacity {
                return output;
            }
            let delta = [
                vertices[index][0] - origin[0],
                vertices[index][2] - origin[2],
            ];
            let distance_sq = delta[0] * delta[0] + delta[1] * delta[1];
            if distance_sq > range_sq || output.contains(&index) {
                continue;
            }
            let distance = (distance_sq as f64).sqrt() as f32;
            let dot = normalized[0] * (delta[0] / distance) + normalized[1] * (delta[1] / distance);
            let angle = (dot.clamp(-1.0, 1.0) as f64).acos() as f32;
            if angle <= spread {
                output.push(index);
            }
        }
    }
    output
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaterFreeze {
    pub vertices: Vec<f32>,
    pub freezable: bool,
    pub auto_freeze: bool,
}
impl WaterFreeze {
    /// Water.Freeze's post-query accumulation. Each batch is the result of
    /// one shield-clipped local-space arc query, already unique within that
    /// batch and truncated by the shared 2048-entry query capacity.
    pub fn freeze_arc_batches(&mut self, batches: &[Vec<usize>], magnitude: f32) {
        if !self.freezable {
            return;
        }
        let mut indices: Vec<Option<usize>> = Vec::new();
        for batch in batches {
            let old_len = indices.len();
            let remaining = 2048usize.saturating_sub(old_len);
            indices.extend(batch.iter().take(remaining).copied().map(Some));
            // Native begins at old_len + 1, leaving the first new vertex
            // unchecked. Marked entries still consume buffer capacity.
            for earlier in 0..old_len {
                for later in old_len + 1..indices.len() {
                    if indices[earlier] == indices[later] {
                        indices[later] = None;
                    }
                }
            }
        }
        for index in indices.into_iter().flatten() {
            self.vertices[index] += magnitude;
        }
    }

    pub fn new(count: usize, freezable: bool, auto_freeze: bool) -> Self {
        Self {
            vertices: vec![if auto_freeze { 1.0 } else { 0.0 }; count],
            freezable,
            auto_freeze,
        }
    }
    pub fn freeze_all(&mut self, magnitude: f32) {
        if self.freezable {
            for vertex in &mut self.vertices {
                *vertex += magnitude;
            }
        }
    }
    /// DataChannel.None skips vertex evolution in the original.
    pub fn update(&mut self, dt: f32, data_channel_present: bool) {
        if !data_channel_present {
            return;
        }
        let amount = dt * 0.1;
        for vertex in &mut self.vertices {
            *vertex = if self.auto_freeze {
                (*vertex + amount).clamp(-1.0, 1.0)
            } else {
                (*vertex - amount).clamp(0.0, 2.0)
            };
        }
    }
    /// Water.SegmentIntersect classifies a triangle as ice if any vertex
    /// is at least .5, rather than averaging or requiring all three.
    pub fn triangle_is_ice(&self, indices: [usize; 3]) -> bool {
        indices.iter().any(|&i| self.vertices[i] >= 0.5)
    }
    /// IceMesh/WaterMesh filter the existing octree candidates in place,
    /// preserving order. The candidate-query capacity is applied before
    /// filtering; removed candidates are not replaced by later triangles.
    pub fn filter_collision_candidates(
        &self,
        candidates: &mut Vec<usize>,
        triangles: &[[usize; 3]],
        ice: bool,
    ) {
        candidates.retain(|&id| {
            let indices = triangles[id];
            if ice {
                // Literal IceMesh removal condition, also preserving NaN
                // comparison behavior rather than treating this as !water.
                !indices.iter().all(|&i| self.vertices[i] < 0.5)
            } else {
                !self.triangle_is_ice(indices)
            }
        });
    }
}
