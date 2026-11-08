//! Magicka.PathFinding: navmesh nearest-position queries, A* over triangles and
//! funnel path straightening. See research/navmesh.md.
//!
//! World-space triangle groups support animated navmeshes and dynamic links.
//! Two original crash bugs are fixed: the A* open heap grows instead of
//! overflowing at 512 entries, and an inconsistent parent edge yields an error
//! instead of an exception.
use crate::{
    geometry::{point_triangle_distance_sq, tri_point},
    impact_motion::Vector,
    mesh_octree::{MeshError, MeshOctree},
};
use std::collections::HashMap;

/// MovementProperties flags.
pub mod movement {
    pub const DEFAULT: u8 = 0;
    pub const WATER: u8 = 1;
    pub const JUMP: u8 = 2;
    pub const FLY: u8 = 4;
    pub const DYNAMIC: u8 = 0x80;
    pub const ALL: u8 = 0xFF;
}
use movement::*;

const NONE: u16 = u16::MAX;
/// Mesh id of the static scene navmesh in TriangleNode.LongId.
pub const STATIC_MESH: u16 = u16::MAX;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavTriangle {
    /// VertexA, VertexB, VertexC.
    pub vertices: [u16; 3],
    /// NeighbourA (edge AB), NeighbourB (BC), NeighbourC (CA); 0xFFFF = none.
    pub neighbours: [u16; 3],
    /// CostAB, CostBC, CostCA.
    pub costs: [f32; 3],
    pub properties: u8,
}

impl NavTriangle {
    /// Triangle.GetCostFrom: cost from the parent's edge to `target_edge`.
    /// The original throws when the parent is not across another edge.
    fn cost_from(&self, parent: u16, target_edge: usize) -> Option<f32> {
        let [a, b, c] = self.neighbours;
        let [ab, bc, ca] = self.costs;
        match target_edge {
            0 if parent == b => Some(ab),
            0 if parent == c => Some(ca),
            1 if parent == a => Some(ab),
            1 if parent == c => Some(bc),
            2 if parent == a => Some(ca),
            2 if parent == b => Some(bc),
            _ => None,
        }
    }
    /// Triangle.GetPortalPoints: (left, right) of the edge shared with `n`.
    fn portal(&self, vertices: &[Vector], n: u16) -> Option<(Vector, Vector)> {
        let [va, vb, vc] = self.vertices.map(|v| vertices[v as usize]);
        if n == self.neighbours[0] {
            Some((vb, va))
        } else if n == self.neighbours[1] {
            Some((vc, vb))
        } else if n == self.neighbours[2] {
            Some((va, vc))
        } else {
            None
        }
    }
    /// Triangle.GetCenter (multiplies by 0.333333f, not 1/3).
    fn center(&self, vertices: &[Vector]) -> Vector {
        let [a, b, c] = self.vertices.map(|v| vertices[v as usize]);
        std::array::from_fn(|i| (a[i] + b[i] + c[i]) * 0.333333)
    }
    /// StraightenPath's inline centroid (multiplies by 1f/3f).
    fn centroid(&self, vertices: &[Vector]) -> Vector {
        let [a, b, c] = self.vertices.map(|v| vertices[v as usize]);
        std::array::from_fn(|i| (a[i] + b[i] + c[i]) * (1.0 / 3.0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathNode {
    pub position: Vector,
    pub properties: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathError {
    /// Triangle.GetCostFrom found no matching edge (original exception).
    InconsistentEdge { triangle: u16, parent: u16 },
    /// Triangle.GetPortalPoints with a non-neighbour (original exception).
    InvalidPortal { triangle: u16, neighbour: u16 },
}

#[derive(Clone, Copy, Debug, Default)]
struct TriangleNode {
    cost: f32,
    total_cost: f32,
    mesh: u16,
    id: u16,
    parent: u32,
}
impl TriangleNode {
    fn long_id(&self) -> u32 {
        (u32::from(self.mesh) << 16) | u32::from(self.id)
    }
}

/// TriangleHeap: 1-based binary min-heap on TotalCost with strict `<`
/// comparisons (tie order matters for paths). Grows instead of overflowing.
#[derive(Default)]
struct TriangleHeap {
    heap: Vec<TriangleNode>,
}
impl TriangleHeap {
    fn clear(&mut self) {
        self.heap.clear();
        self.heap.push(TriangleNode::default());
    }
    fn len(&self) -> usize {
        self.heap.len() - 1
    }
    fn push(&mut self, value: TriangleNode) {
        self.heap.push(value);
        let mut num = self.len();
        let mut parent = num / 2;
        while num > 1 && value.total_cost < self.heap[parent].total_cost {
            self.heap[num] = self.heap[parent];
            num = parent;
            parent = num / 2;
        }
        self.heap[num] = value;
    }
    fn pop(&mut self) -> Option<TriangleNode> {
        let last_index = self.len();
        if last_index < 1 {
            return None;
        }
        let result = self.heap[1];
        let last = self.heap[last_index];
        self.heap.pop();
        let last_index = last_index - 1;
        let mut num = 1;
        loop {
            let (left, right) = (num * 2, num * 2 + 1);
            let mut next = 0;
            if left <= last_index {
                let left_cost = self.heap[left].total_cost;
                if left_cost < last.total_cost {
                    next = left;
                }
                if right <= last_index {
                    let right_cost = self.heap[right].total_cost;
                    if right_cost < last.total_cost && right_cost < left_cost {
                        next = right;
                    }
                }
            }
            if next < 1 {
                break;
            }
            self.heap[num] = self.heap[next];
            num = next;
        }
        if num <= last_index {
            self.heap[num] = last;
        }
        Some(result)
    }
}

/// NavMesh.ManhattanDistance: |dx| + 1.75 |dz|.
pub fn manhattan(a: Vector, b: Vector) -> f32 {
    (a[0] - b[0]).abs() + (a[2] - b[2]).abs() * 1.75
}
/// NavMesh.TriArea2D.
fn tri_area_2d(a: Vector, b: Vector, c: Vector) -> f32 {
    (b[0] * a[2] - a[0] * b[2] + (c[0] * b[2] - b[0] * c[2]) + (a[0] * c[2] - c[0] * a[2])) * 0.5
}
fn distance_sq(a: Vector, b: Vector) -> f32 {
    (0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum()
}
fn distance(a: Vector, b: Vector) -> f32 {
    distance_sq(a, b).sqrt()
}

pub struct NavMesh {
    vertices: Vec<Vector>,
    triangles: Vec<NavTriangle>,
    octree: MeshOctree,
    groups: Vec<u16>,
    dynamic: Vec<Vec<(u16, f32, Vector)>>,
}

impl NavMesh {
    pub fn new(vertices: Vec<Vector>, triangles: Vec<NavTriangle>) -> Result<Self, MeshError> {
        let indices = triangles
            .iter()
            .map(|t| t.vertices.map(usize::from))
            .collect();
        let octree = MeshOctree::build(vertices.clone(), indices)?;
        Ok(Self {
            groups: vec![0; triangles.len()],
            dynamic: vec![Vec::new(); triangles.len()],
            vertices,
            triangles,
            octree,
        })
    }
    /// World-space meshes retain separate triangle groups. Dynamic triangles
    /// from different meshes connect within native AnimatedNavMesh's 0.25 m
    /// tolerance; ordinary static edges remain authored.
    pub fn with_groups(mut self, groups: Vec<u16>) -> Self {
        assert_eq!(groups.len(), self.triangles.len());
        self.groups = groups;
        for a in 0..self.triangles.len() {
            if self.triangles[a].properties & DYNAMIC == 0 {
                continue;
            }
            let av = self.triangles[a]
                .vertices
                .map(|i| self.vertices[i as usize]);
            for b in a + 1..self.triangles.len() {
                if self.groups[a] == self.groups[b] || self.triangles[b].properties & DYNAMIC == 0 {
                    continue;
                }
                let bv = self.triangles[b]
                    .vertices
                    .map(|i| self.vertices[i as usize]);
                if (0..3).any(|i| {
                    let lo = av.iter().map(|v| v[i]).fold(f32::INFINITY, f32::min);
                    let hi = av.iter().map(|v| v[i]).fold(f32::NEG_INFINITY, f32::max);
                    let blo = bv.iter().map(|v| v[i]).fold(f32::INFINITY, f32::min);
                    let bhi = bv.iter().map(|v| v[i]).fold(f32::NEG_INFINITY, f32::max);
                    lo > bhi + 0.25 || blo > hi + 0.25
                }) {
                    continue;
                }
                let mut dist = f32::INFINITY;
                for (x, y) in [(av, bv), (bv, av)] {
                    for i in 0..3 {
                        let delta = std::array::from_fn(|k| x[(i + 1) % 3][k] - x[i][k]);
                        dist = dist
                            .min(crate::geometry::segment_triangle_distance_sq(x[i], delta, &y).0);
                    }
                }
                if dist <= 0.0625 {
                    let ac = self.triangles[a].centroid(&self.vertices);
                    let bc = self.triangles[b].centroid(&self.vertices);
                    let cost = distance(ac, bc);
                    self.dynamic[a].push((b as u16, cost, bc));
                    self.dynamic[b].push((a as u16, cost, ac));
                }
            }
        }
        self
    }
    pub fn triangles(&self) -> &[NavTriangle] {
        &self.triangles
    }
    pub fn vertices(&self) -> &[Vector] {
        &self.vertices
    }
    fn allowed(properties: u8, abilities: u8) -> bool {
        properties == DEFAULT || properties & abilities != 0
    }

    /// No triangles at all (e.g. a hosted scene without authored navigation).
    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }
    /// GetNearestPosition over the composed world-space meshes: (distance², point, triangle).
    /// Triangles with properties are skipped unless they share a bit with
    /// `abilities`.
    pub fn nearest(&self, point: Vector, abilities: u8) -> Option<(f32, Vector, u16)> {
        let (d, found) = self.octree.closest_point(point, f32::MAX, |t| {
            Self::allowed(self.triangles[t].properties, abilities)
        });
        found.map(|(t, p)| (d, p, t as u16))
    }

    /// NavMesh.FindPath over authored edges and dynamic cross-mesh links. Returns whether the goal triangle
    /// was reached and the triangle LongIds from the best node back to the
    /// start (goal first), like the original `iPath`. `ice_below(centre)`
    /// answers the Water rule: water triangles are passable only where a
    /// 4 m downward segment from 1 m above their centre hits frozen liquid.
    pub fn find_path(
        &self,
        start: Vector,
        end: Vector,
        mut abilities: u8,
        ice_below: &dyn Fn(Vector) -> bool,
    ) -> Result<(bool, Vec<u32>), PathError> {
        if (start[0] + start[1] + start[2]).is_nan() {
            return Ok((false, Vec::new()));
        }
        abilities |= DYNAMIC | WATER;
        let Some((_, start_point, start_tri)) = self.nearest(start, abilities) else {
            return Ok((false, Vec::new()));
        };
        let Some((_, goal_point, goal_tri)) = self.nearest(end, abilities) else {
            return Ok((false, Vec::new()));
        };
        let mut closed: HashMap<u32, TriangleNode> = HashMap::new();
        let mut costs: HashMap<u32, f32> = HashMap::new();
        let mut open = TriangleHeap::default();
        open.clear();
        let start_node = TriangleNode {
            cost: 0.0,
            total_cost: manhattan(start_point, goal_point),
            mesh: STATIC_MESH,
            id: start_tri,
            parent: (u32::from(STATIC_MESH) << 16) | 0xFFFF,
        };
        open.push(start_node);
        let mut best_total = start_node.total_cost;
        let mut best = start_node;
        let mut found = false;
        while let Some(node) = open.pop() {
            let key = node.long_id();
            if closed.contains_key(&key) {
                continue;
            }
            closed.insert(key, node);
            if node.id == goal_tri && node.mesh == STATIC_MESH {
                best = node;
                found = true;
                break;
            }
            let triangle = self.triangles[node.id as usize];
            for edge in 0..3 {
                let n = triangle.neighbours[edge];
                let nkey = (u32::from(node.mesh) << 16) | u32::from(n);
                if n == NONE || closed.contains_key(&nkey) {
                    continue;
                }
                let neighbour = &self.triangles[n as usize];
                if !Self::allowed(neighbour.properties, abilities) {
                    continue;
                }
                if neighbour.properties & WATER == WATER {
                    let mut centre = neighbour.center(&self.vertices);
                    centre[1] += 1.0;
                    if !ice_below(centre) {
                        continue;
                    }
                }
                let a = self.vertices[triangle.vertices[edge] as usize];
                let b = self.vertices[triangle.vertices[(edge + 1) % 3] as usize];
                let midpoint: Vector = std::array::from_fn(|i| (a[i] + b[i]) * 0.5);
                let mut cost = if node.parent & 0xFFFF != 0xFFFF {
                    if self.groups[node.parent as u16 as usize] != self.groups[node.id as usize] {
                        0.5
                    } else {
                        let parent = node.parent as u16;
                        triangle
                            .cost_from(parent, edge)
                            .ok_or(PathError::InconsistentEdge {
                                triangle: node.id,
                                parent,
                            })?
                    }
                } else {
                    distance(start_point, midpoint)
                };
                if n == goal_tri && node.mesh == STATIC_MESH {
                    cost += distance(midpoint, goal_point);
                }
                cost += node.cost;
                if costs.get(&nkey).is_none_or(|&old| old > cost) {
                    costs.insert(nkey, cost);
                    let next = TriangleNode {
                        mesh: node.mesh,
                        id: n,
                        parent: key,
                        cost,
                        total_cost: cost + manhattan(midpoint, goal_point),
                    };
                    open.push(next);
                    if next.total_cost < best_total {
                        best_total = next.total_cost;
                        best = next;
                    }
                }
            }
            for &(n, bridge_cost, midpoint) in &self.dynamic[node.id as usize] {
                let nkey = (u32::from(STATIC_MESH) << 16) | u32::from(n);
                if closed.contains_key(&nkey)
                    || !Self::allowed(self.triangles[n as usize].properties, abilities)
                {
                    continue;
                }
                // Preserve native dynamic-edge cost accumulation, including
                // its independent cost when the parent is already assigned.
                let mut cost = if node.parent as u16 == NONE {
                    distance(start_point, midpoint)
                } else {
                    bridge_cost
                };
                if n == goal_tri {
                    cost += distance(midpoint, goal_point);
                }
                cost += bridge_cost;
                if costs.get(&nkey).is_none_or(|&old| old > cost) {
                    costs.insert(nkey, cost);
                    let next = TriangleNode {
                        mesh: STATIC_MESH,
                        id: n,
                        parent: key,
                        cost,
                        total_cost: cost + manhattan(midpoint, goal_point),
                    };
                    open.push(next);
                    if next.total_cost < best_total {
                        best_total = next.total_cost;
                        best = next;
                    }
                }
            }
        }
        let mut path = Vec::new();
        let mut id = best.long_id();
        while id as u16 != 0xFFFF {
            path.push(id);
            id = match closed.get(&id) {
                Some(node) => node.parent,
                // Every pushed key is closed once the open list drains, so
                // this only guards impossible states instead of panicking.
                None => break,
            };
        }
        Ok((found, path))
    }

    /// NavMesh.StraightenPath: funnel over the triangle portals from the
    /// start triangle (last entry) to the goal (first entry).
    pub fn straighten(
        &self,
        start: Vector,
        end: Vector,
        path: &[u32],
    ) -> Result<Vec<PathNode>, PathError> {
        let mut out = Vec::new();
        if path.is_empty() || path[0] == 0xFFFF {
            return Ok(out);
        }
        let tri = |id: u32| &self.triangles[(id & 0xFFFF) as usize];
        let corners = |t: &NavTriangle| t.vertices.map(|v| self.vertices[v as usize]);
        let snap = |p: Vector, t: &NavTriangle| {
            let c = corners(t);
            let (_, s, u) = point_triangle_distance_sq(p, &c);
            tri_point(&c, s, u)
        };
        let first = path[path.len() - 1];
        let start_point = snap(start, tri(first));
        let last = path[0];
        let end_point = snap(end, tri(last));
        out.push(PathNode {
            position: start_point,
            properties: tri(first).properties,
        });
        let (mut apex, mut left, mut right) = (start_point, start_point, start_point);
        let (mut left_i, mut right_i) = (0usize, 0usize);
        let mut i = path.len() as isize - 1;
        while i >= 0 {
            let index = i as usize;
            let current = path[index];
            let (l, r) = if index > 0 {
                let previous = path[index - 1];
                if self.groups[current as u16 as usize] == self.groups[previous as u16 as usize] {
                    tri(current).portal(&self.vertices, previous as u16).ok_or(
                        PathError::InvalidPortal {
                            triangle: current as u16,
                            neighbour: previous as u16,
                        },
                    )?
                } else {
                    let c = tri(current).centroid(&self.vertices);
                    (c, c)
                }
            } else {
                (end_point, end_point)
            };
            let restart = |out: &mut Vec<PathNode>, apex: Vector, apex_i: usize| {
                let previous = out[out.len() - 1].position;
                if distance_sq(previous, apex) >= 0.001 {
                    out.push(PathNode {
                        position: apex,
                        properties: tri(path[apex_i]).properties,
                    });
                }
            };
            // Right side of the funnel.
            if distance_sq(apex, right) < 0.001 {
                right = r;
                right_i = index;
            } else if tri_area_2d(apex, right, r) <= 0.0 {
                if tri_area_2d(apex, left, r) <= 0.0 || tri_area_2d(apex, left, r).is_nan() {
                    apex = left;
                    let apex_i = left_i;
                    restart(&mut out, apex, apex_i);
                    left = apex;
                    right = apex;
                    left_i = apex_i;
                    right_i = apex_i;
                    i = apex_i as isize - 1;
                    continue;
                }
                right = r;
                right_i = index;
            }
            // Left side of the funnel.
            if distance_sq(apex, left) < 0.001 {
                left = l;
                left_i = index;
            } else {
                let area = tri_area_2d(apex, left, l);
                if area < 0.0 || area.is_nan() {
                    i -= 1;
                    continue;
                }
                if tri_area_2d(apex, right, l) < 0.0 {
                    left = l;
                    left_i = index;
                    i -= 1;
                    continue;
                }
                apex = right;
                let apex_i = right_i;
                restart(&mut out, apex, apex_i);
                left = apex;
                right = apex;
                left_i = apex_i;
                right_i = apex_i;
                i = apex_i as isize - 1;
                continue;
            }
            i -= 1;
        }
        out.push(PathNode {
            position: end_point,
            properties: tri(last).properties,
        });
        // The original's property-change insertion pass compares each path
        // triangle with itself and therefore never inserts a node.
        Ok(out)
    }

    /// NavMesh.FindShortestPath.
    pub fn find_shortest_path(
        &self,
        start: Vector,
        end: Vector,
        abilities: u8,
        ice_below: &dyn Fn(Vector) -> bool,
    ) -> Result<(bool, Vec<PathNode>), PathError> {
        let (found, triangles) = self.find_path(start, end, abilities, ice_below)?;
        let nodes = self.straighten(start, end, &triangles)?;
        Ok((found, nodes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(id: u16, total_cost: f32) -> TriangleNode {
        TriangleNode {
            id,
            total_cost,
            ..Default::default()
        }
    }
    #[test]
    fn heap_ties_follow_original_sift_order() {
        let mut heap = TriangleHeap::default();
        heap.clear();
        for id in [1, 2, 3] {
            heap.push(node(id, 1.0));
        }
        let order: Vec<u16> = std::iter::from_fn(|| heap.pop().map(|n| n.id)).collect();
        // Pop moves the last element to the root; equal children never win.
        assert_eq!(order, [1, 3, 2]);
    }
    #[test]
    fn heap_orders_by_total_cost_and_grows_past_512() {
        let mut heap = TriangleHeap::default();
        heap.clear();
        for i in 0..2000u16 {
            heap.push(node(i, ((u32::from(i) * 7919) % 2000) as f32));
        }
        let mut previous = f32::MIN;
        let mut count = 0;
        while let Some(n) = heap.pop() {
            assert!(n.total_cost >= previous);
            previous = n.total_cost;
            count += 1;
        }
        assert_eq!(count, 2000);
    }
}
