//! JigLibX octree construction/query ordering for finite local triangle meshes.
use crate::{
    impact_motion::Vector,
    spray_pulse::Segment,
    triangle_intersection::{Triangle, TriangleHit},
};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: Vector,
    pub max: Vector,
}
impl Bounds {
    fn empty() -> Self {
        Self {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        }
    }
    fn include(&mut self, p: Vector) {
        for (i, v) in p.into_iter().enumerate() {
            self.min[i] = self.min[i].min(v);
            self.max[i] = self.max[i].max(v);
        }
    }
    fn contains(self, b: Self) -> bool {
        (0..3).all(|i| self.min[i] <= b.min[i] && self.max[i] >= b.max[i])
    }
    fn intersects(self, b: Self) -> bool {
        (0..3).all(|i| self.max[i] >= b.min[i] && self.min[i] <= b.max[i])
    }
    fn child(self, kind: u8) -> Self {
        let half: Vector = std::array::from_fn(|i| 0.5 * (self.max[i] - self.min[i]));
        let min: Vector =
            std::array::from_fn(|i| self.min[i] + f32::from((kind >> i) & 1) * half[i]);
        let max: Vector = std::array::from_fn(|i| min[i] + half[i]);
        Self {
            min: std::array::from_fn(|i| min[i] - 1e-5 * half[i]),
            max: std::array::from_fn(|i| max[i] + 1e-5 * half[i]),
        }
    }
}
/// Distance.SqrDistance(point, BoundingBox): centre/half-extent clamping.
fn box_distance_sq(p: Vector, b: Bounds) -> f32 {
    let mut d = 0.0;
    for (i, coordinate) in p.into_iter().enumerate() {
        let centre = (b.min[i] + b.max[i]) * 0.5;
        let half = (b.max[i] - b.min[i]) * 0.5;
        let v = coordinate - centre;
        if v < -half {
            d += (v + half) * (v + half);
        } else if v > half {
            d += (v - half) * (v - half);
        }
    }
    d
}
#[derive(Clone, Debug)]
struct Node {
    kind: u8,
    bounds: Bounds,
    children: Vec<usize>,
    triangles: Vec<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeshError {
    InvalidIndex,
    NonFiniteVertex,
    NonShrinkingCell,
    NodeIndexOverflow,
}
#[derive(Clone, Debug)]
pub struct MeshOctree {
    vertices: Vec<Vector>,
    indices: Vec<[usize; 3]>,
    boxes: Vec<Bounds>,
    nodes: Vec<Node>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshHit {
    pub triangle: usize,
    pub hit: TriangleHit,
    pub position: Vector,
    pub normal: Vector,
}
impl MeshOctree {
    pub fn build(vertices: Vec<Vector>, indices: Vec<[usize; 3]>) -> Result<Self, MeshError> {
        if vertices.iter().flatten().any(|v| !v.is_finite()) {
            return Err(MeshError::NonFiniteVertex);
        }
        let mut boxes = Vec::with_capacity(indices.len());
        let mut root = Bounds::empty();
        for tri in &indices {
            let mut bounds = Bounds::empty();
            for &i in tri {
                let p = *vertices.get(i).ok_or(MeshError::InvalidIndex)?;
                bounds.include(p);
            }
            root.include(bounds.min);
            root.include(bounds.max);
            boxes.push(bounds);
        }
        let mut nodes = vec![Node {
            kind: 0,
            bounds: root,
            children: vec![],
            triangles: vec![],
        }];
        for (triangle, &bounds) in boxes.iter().enumerate() {
            let mut node = 0;
            loop {
                let parent = nodes[node].bounds;
                let child = (0..8).find_map(|kind| {
                    let b = parent.child(kind);
                    b.contains(bounds).then_some((kind, b))
                });
                let Some((kind, b)) = child else {
                    nodes[node].triangles.push(triangle);
                    break;
                };
                // Fail explicitly on inputs that would make original construction
                // unbounded or wrap ushort node indices; do not invent a leaf rule.
                if b == parent {
                    return Err(MeshError::NonShrinkingCell);
                }
                let existing = nodes[node]
                    .children
                    .iter()
                    .copied()
                    .find(|&i| nodes[i].kind == kind);
                let next = if let Some(i) = existing {
                    i
                } else {
                    if nodes.len() > u16::MAX as usize {
                        return Err(MeshError::NodeIndexOverflow);
                    }
                    let i = nodes.len();
                    nodes.push(Node {
                        kind,
                        bounds: b,
                        children: vec![],
                        triangles: vec![],
                    });
                    nodes[node].children.push(i);
                    i
                };
                node = next;
            }
        }
        Ok(Self {
            vertices,
            indices,
            boxes,
            nodes,
        })
    }
    pub fn triangle_count(&self) -> usize {
        self.indices.len()
    }
    /// Local-space vertices of triangle `i` in decoded index order.
    pub fn triangle(&self, i: usize) -> [Vector; 3] {
        self.indices[i].map(|v| self.vertices[v])
    }
    /// Octree.FindClosestPoint (NavMeshOctree variant): breadth-first from the
    /// root, testing each node's triangles that pass `include` and queuing
    /// children whose box lies strictly closer than the best distance so far.
    /// Returns (distance², triangle, closest point); strict improvement keeps
    /// the first minimum.
    pub fn closest_point(
        &self,
        point: Vector,
        start_distance_sq: f32,
        include: impl Fn(usize) -> bool,
    ) -> (f32, Option<(usize, Vector)>) {
        let mut best = start_distance_sq;
        let mut found = None;
        if self.nodes.is_empty() || self.indices.is_empty() {
            return (f32::INFINITY, None);
        }
        let mut queue = vec![0usize];
        let mut cursor = 0;
        while cursor < queue.len() {
            let node = &self.nodes[queue[cursor]];
            cursor += 1;
            for &triangle in &node.triangles {
                if !include(triangle) {
                    continue;
                }
                let tri = self.triangle(triangle);
                let (d, s, t) = crate::geometry::point_triangle_distance_sq(point, &tri);
                if d < best {
                    best = d;
                    found = Some((triangle, crate::geometry::tri_point(&tri, s, t)));
                }
            }
            for &child in &node.children {
                if box_distance_sq(point, self.nodes[child].bounds) < best {
                    queue.push(child);
                }
            }
        }
        (best, found)
    }
    pub fn candidates(&self, bounds: Bounds, max_triangles: usize) -> Vec<usize> {
        if self.indices.is_empty() {
            return vec![];
        }
        let mut queue = vec![0];
        let mut cursor = 0;
        let mut out = vec![];
        while cursor < queue.len() {
            let node = &self.nodes[queue[cursor]];
            cursor += 1;
            if !bounds.intersects(node.bounds) {
                continue;
            }
            for &triangle in &node.triangles {
                if bounds.intersects(self.boxes[triangle]) && out.len() < max_triangles {
                    out.push(triangle);
                }
            }
            queue.extend_from_slice(&node.children);
        }
        out
    }
    /// Identity-transform TriangleMesh query. Full world transform is caller-owned.
    pub fn segment_intersect(&self, segment: Segment, ignore_backfaces: bool) -> Option<MeshHit> {
        let mut bounds = Bounds::empty();
        bounds.include(segment.origin);
        bounds.include(std::array::from_fn(|i| {
            segment.origin[i] + segment.delta[i]
        }));
        self.intersect_candidates(segment, ignore_backfaces, bounds)
    }
    fn intersect_candidates(
        &self,
        segment: Segment,
        ignore_backfaces: bool,
        bounds: Bounds,
    ) -> Option<MeshHit> {
        let mut closest = f32::MAX;
        let mut result = None;
        for i in self.candidates(bounds, 2048) {
            let [a, b, c] = self.indices[i];
            let tri = Triangle::from_vertices(self.vertices[a], self.vertices[b], self.vertices[c]);
            if let Some(hit) = tri.segment_intersect(segment, ignore_backfaces) {
                if hit.fraction < closest {
                    closest = hit.fraction;
                    result = Some(MeshHit {
                        triangle: i,
                        hit,
                        normal: plane_normal(tri),
                        position: std::array::from_fn(|j| {
                            segment.origin[j] + segment.delta[j] * hit.fraction
                        }),
                    });
                }
            }
        }
        result
    }
}

/// XNA row-major matrices with row-vector point convention. Inverse is supplied
/// from the owner's transform calculation; singular matrices are not supported.
#[derive(Clone, Copy, Debug)]
pub struct MeshTransform {
    pub forward: [f32; 16],
    pub inverse: [f32; 16],
}
impl MeshTransform {
    /// Local mesh point to world (forward matrix).
    pub fn point(&self, p: Vector) -> Vector {
        transform_point(p, &self.forward)
    }
    /// TriangleMesh.GetTrianglesIntersectingtAABox: the local query box spans
    /// the eight inverse-transformed corners of the world box.
    pub fn local_bounds(&self, world: Bounds) -> Bounds {
        let mut local = Bounds::empty();
        // Original corner order: MMM, MMP, MPM, MPP, PMM, PMP, PPM, PPP.
        for corner in 0..8 {
            let point = std::array::from_fn(|i| {
                if corner & (1 << (2 - i)) == 0 {
                    world.min[i]
                } else {
                    world.max[i]
                }
            });
            local.include(transform_point(point, &self.inverse));
        }
        local
    }
}
fn transform_vector(v: Vector, m: &[f32; 16]) -> Vector {
    std::array::from_fn(|i| v[0] * m[i] + v[1] * m[4 + i] + v[2] * m[8 + i])
}
fn transform_point(v: Vector, m: &[f32; 16]) -> Vector {
    let direction = transform_vector(v, m);
    std::array::from_fn(|i| direction[i] + m[12 + i])
}
fn plane_normal(triangle: Triangle) -> Vector {
    let a = triangle.edge0;
    let b = triangle.edge1;
    let normal = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]) as f64;
    let inverse = 1.0 / length.sqrt() as f32;
    normal.map(|v| v * inverse)
}
impl MeshOctree {
    /// TriangleMesh transforms the world segment AABB's eight corners for its
    /// candidate query, rather than querying only the local segment's AABB.
    pub fn segment_intersect_transformed(
        &self,
        segment: Segment,
        ignore_backfaces: bool,
        transform: MeshTransform,
    ) -> Option<MeshHit> {
        let mut world = Bounds::empty();
        world.include(segment.origin);
        world.include(std::array::from_fn(|i| {
            segment.origin[i] + segment.delta[i]
        }));
        let local_bounds = transform.local_bounds(world);
        let local = Segment {
            origin: transform_point(segment.origin, &transform.inverse),
            delta: transform_vector(segment.delta, &transform.inverse),
        };
        self.intersect_candidates(local, ignore_backfaces, local_bounds)
            .map(|mut hit| {
                hit.position = transform_point(hit.position, &transform.forward);
                // Original transforms the plane normal directly, without inverse
                // transpose or renormalizing. Preserve that behavior for scale.
                hit.normal = transform_vector(hit.normal, &transform.forward);
                hit
            })
    }
}
