//! Shared Water/Lava frozen and molten CPU mesh state. Animated transforms remain host work.
use magicka_sim::{
    mesh_octree::{Bounds, MeshError, MeshOctree},
    water_freeze::WaterFreeze,
};

#[derive(Clone, Debug)]
pub struct WaterCollision {
    pub mesh: MeshOctree,
    pub freeze: WaterFreeze,
    pub lava: bool,
    pub has_collision: bool,
    triangles: Vec<[usize; 3]>,
    vertices: Vec<[f32; 3]>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterSegmentHit {
    pub triangle: usize,
    pub fraction: f32,
    pub position: [f32; 3],
    pub normal: [f32; 3],
}
impl WaterCollision {
    /// Collision vertices (world space for retained top-level Water).
    pub fn vertices(&self) -> &[[f32; 3]] {
        &self.vertices
    }
    /// Water.Freeze after inverse-transforming inputs. The callback applies
    /// native shield intersections in list order to each local segment.
    pub fn freeze_local(
        &mut self,
        origin: [f32; 3],
        direction: [f32; 3],
        spread: f32,
        magnitude: f32,
        mut clip_shields: impl FnMut(&mut magicka_sim::spray_pulse::Segment),
    ) {
        if !self.freeze.freezable || !self.has_collision {
            return;
        }
        let count = (spread / (std::f32::consts::PI / 4.0)) as i32;
        let arc_spread = spread / (count * 2 + 1) as f32;
        let mut batches = Vec::new();
        let mut used = 0;
        let mut flattened_origin = origin;
        for index in -count..=count {
            let yaw = index as f32 * arc_spread;
            let (sin, cos) = ((yaw as f64).sin() as f32, (yaw as f64).cos() as f32);
            let mut segment = magicka_sim::spray_pulse::Segment {
                // Water.Freeze initializes seg.Origin once, before the arc
                // loop. IceMesh flattens only the separate query origin.
                origin,
                delta: [
                    direction[0] * cos + direction[2] * sin,
                    direction[1],
                    -direction[0] * sin + direction[2] * cos,
                ],
            };
            clip_shields(&mut segment);
            let batch = self.arc_vertices(flattened_origin, segment.delta, arc_spread, 2048 - used);
            used += batch.len();
            batches.push(batch);
            // IceMesh's query mutates its by-ref origin's Y to zero.
            flattened_origin[1] = 0.0;
        }
        if self.lava {
            // Lava.Freeze retains cross-arc duplicates; Water has a separate
            // duplicate-marking phase. Each individual query is still unique.
            for index in batches.into_iter().flatten() {
                self.freeze.vertices[index] += magnitude;
            }
        } else {
            self.freeze.freeze_arc_batches(&batches, magnitude);
        }
    }

    /// Unfiltered IceMesh octree candidates feed the recovered arc predicate.
    pub fn arc_vertices(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        spread: f32,
        capacity: usize,
    ) -> Vec<usize> {
        let bounds = magicka_sim::water_freeze::arc_bounds(origin, direction, spread);
        let candidates = self.mesh.candidates(bounds, 2048);
        magicka_sim::water_freeze::arc_vertices(
            &self.vertices,
            &self.triangles,
            &candidates,
            origin,
            direction,
            spread,
            capacity,
        )
    }

    /// Native Water query transforms only the input into local space.
    pub fn segment_intersect_transformed(
        &self,
        segment: magicka_sim::spray_pulse::Segment,
        inverse: &[f32; 16],
        ignore_backfaces: bool,
        ignore_water: bool,
        ignore_ice: bool,
    ) -> Option<WaterSegmentHit> {
        let vector = |v: [f32; 3]| {
            std::array::from_fn(|i| {
                v[0] * inverse[i] + v[1] * inverse[4 + i] + v[2] * inverse[8 + i]
            })
        };
        let direction: [f32; 3] = vector(segment.origin);
        self.segment_intersect(
            magicka_sim::spray_pulse::Segment {
                origin: std::array::from_fn(|i| direction[i] + inverse[12 + i]),
                delta: vector(segment.delta),
            },
            ignore_backfaces,
            ignore_water,
            ignore_ice,
        )
    }

    /// Water.SegmentIntersect local-space result. Native leaves the output
    /// position in local space; the scene caller owns coordinate routing.
    pub fn segment_intersect(
        &self,
        mut segment: magicka_sim::spray_pulse::Segment,
        ignore_backfaces: bool,
        ignore_water: bool,
        ignore_ice: bool,
    ) -> Option<WaterSegmentHit> {
        if !self.has_collision || (ignore_water && ignore_ice) {
            return None;
        }
        let end: [f32; 3] = std::array::from_fn(|i| segment.origin[i] + segment.delta[i]);
        let bounds = Bounds {
            min: std::array::from_fn(|i| segment.origin[i].min(end[i])),
            max: std::array::from_fn(|i| segment.origin[i].max(end[i])),
        };
        let mut candidates = self.mesh.candidates(bounds, 2048);
        if ignore_water || ignore_ice {
            self.freeze
                .filter_collision_candidates(&mut candidates, &self.triangles, ignore_water);
        }
        let mut closest = f32::MAX;
        let mut result = None;
        for id in candidates {
            let [a, b, c] = self.mesh.triangle(id);
            let triangle = magicka_sim::triangle_intersection::Triangle::from_vertices(a, b, c);
            if let Some(hit) = triangle.segment_intersect(segment, ignore_backfaces) {
                if hit.fraction < closest {
                    closest = hit.fraction;
                    segment.delta = segment.delta.map(|v| v * hit.fraction);
                    result = Some(WaterSegmentHit {
                        triangle: id,
                        fraction: closest,
                        position: std::array::from_fn(|i| segment.origin[i] + segment.delta[i]),
                        normal: {
                            let u = triangle.edge0;
                            let v = triangle.edge1;
                            let cross = [
                                u[1] * v[2] - u[2] * v[1],
                                u[2] * v[0] - u[0] * v[2],
                                u[0] * v[1] - u[1] * v[0],
                            ];
                            let length =
                                (cross.iter().map(|x| x * x).sum::<f32>() as f64).sqrt() as f32;
                            cross.map(|x| x / length)
                        },
                    });
                }
            }
        }
        result
    }

    pub fn from_geometry(
        source: &magicka_content::level::TriMesh,
        freezable: bool,
        auto_freeze: bool,
    ) -> Result<Self, MeshError> {
        let mesh = crate::level_collision::collision_mesh(source)?;
        let triangles = source
            .triangles
            .iter()
            .map(|tri| tri.map(|i| i as usize))
            .collect();
        Ok(Self {
            mesh,
            lava: false,
            has_collision: true,
            triangles,
            vertices: source.vertices.clone(),
            freeze: WaterFreeze::new(source.vertices.len(), freezable, auto_freeze),
        })
    }
    /// Shared octree traversal first, followed by native IceMesh/WaterMesh
    /// candidate filtering. Capacity applies before filtering.
    pub fn candidates(&self, bounds: Bounds, capacity: usize, ice: bool) -> Vec<usize> {
        if !self.has_collision {
            return vec![];
        }
        let mut candidates = self.mesh.candidates(bounds, capacity);
        self.freeze
            .filter_collision_candidates(&mut candidates, &self.triangles, ice);
        candidates
    }
}
