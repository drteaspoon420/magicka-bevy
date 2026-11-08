//! Magicka NavMesh A* + funnel on synthetic grids.
use magicka_sim::navmesh::{movement as M, NavMesh, NavTriangle, PathNode};
use std::collections::HashMap;

/// Square grid of `n` x `n` cells (2 triangles each) on y = 0, skipping cells
/// for which `solid(x, z)` is true. Neighbours come from shared edges and the
/// costs between edge pairs are distances between their midpoints:
/// CostAB = AB↔BC, CostBC = BC↔CA, CostCA = CA↔AB (Triangle.GetCostFrom).
fn grid(
    n: usize,
    solid: impl Fn(usize, usize) -> bool,
    props: impl Fn(usize, usize) -> u8,
) -> NavMesh {
    let v = |x: usize, z: usize| (z * (n + 1) + x) as u16;
    let vertices: Vec<[f32; 3]> = (0..=n)
        .flat_map(|z| (0..=n).map(move |x| [x as f32, 0.0, z as f32]))
        .collect();
    let mut tris: Vec<[u16; 3]> = Vec::new();
    let mut tprops = Vec::new();
    for z in 0..n {
        for x in 0..n {
            if solid(x, z) {
                continue;
            }
            // Retail navmeshes wind clockwise seen from above (normals -Y).
            tris.push([v(x, z), v(x + 1, z + 1), v(x, z + 1)]);
            tris.push([v(x, z), v(x + 1, z), v(x + 1, z + 1)]);
            tprops.push(props(x, z));
            tprops.push(props(x, z));
        }
    }
    let mut edges: HashMap<(u16, u16), Vec<usize>> = HashMap::new();
    for (i, t) in tris.iter().enumerate() {
        for e in 0..3 {
            let (a, b) = (t[e], t[(e + 1) % 3]);
            edges.entry((a.min(b), a.max(b))).or_default().push(i);
        }
    }
    let mid = |a: u16, b: u16| {
        let (p, q) = (vertices[a as usize], vertices[b as usize]);
        [
            (p[0] + q[0]) / 2.0,
            (p[1] + q[1]) / 2.0,
            (p[2] + q[2]) / 2.0,
        ]
    };
    let dist = |p: [f32; 3], q: [f32; 3]| ((p[0] - q[0]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
    let triangles = tris
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let neighbours = std::array::from_fn(|e| {
                let (a, b) = (t[e], t[(e + 1) % 3]);
                edges[&(a.min(b), a.max(b))]
                    .iter()
                    .copied()
                    .find(|&j| j != i)
                    .map_or(u16::MAX, |j| j as u16)
            });
            let (ab, bc, ca) = (mid(t[0], t[1]), mid(t[1], t[2]), mid(t[2], t[0]));
            NavTriangle {
                vertices: *t,
                neighbours,
                costs: [dist(ab, bc), dist(bc, ca), dist(ca, ab)],
                properties: tprops[i],
            }
        })
        .collect();
    NavMesh::new(vertices, triangles).unwrap()
}

fn no_ice(_: [f32; 3]) -> bool {
    false
}

/// Every node and every segment sample lies on the navmesh (XZ within 1e-3).
fn assert_on_mesh(mesh: &NavMesh, path: &[PathNode]) {
    for w in path.windows(2) {
        for k in 0..=8 {
            let t = k as f32 / 8.0;
            let p: [f32; 3] = std::array::from_fn(|i| {
                w[0].position[i] + (w[1].position[i] - w[0].position[i]) * t
            });
            let (_, q, _) = mesh.nearest(p, M::ALL).unwrap();
            let d = ((p[0] - q[0]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
            assert!(d < 1e-3, "{p:?} off mesh by {d}");
        }
    }
}

fn length(path: &[PathNode]) -> f32 {
    path.windows(2)
        .map(|w| {
            let (a, b) = (w[0].position, w[1].position);
            ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        })
        .sum()
}

#[test]
fn open_grid_path_is_taut_within_the_astar_corridor() {
    let mesh = grid(10, |_, _| false, |_, _| M::DEFAULT);
    let (start, end) = ([0.5, 0.0, 0.5], [9.5, 0.0, 8.5]);
    let (found, path) = mesh
        .find_shortest_path(start, end, M::DEFAULT, &no_ice)
        .unwrap();
    assert!(found);
    assert_eq!(path[0].position, start);
    assert_eq!(path[path.len() - 1].position, end);
    assert_on_mesh(&mesh, &path);
    // The |dx| + 1.75|dz| heuristic yields a diagonal staircase corridor the
    // straight line leaves at once; the funnel bends once at its corner.
    assert_eq!(path[1].position, [8.0, 0.0, 8.0], "{path:?}");
    let straight = ((9.0f32).powi(2) + (8.0f32).powi(2)).sqrt();
    assert!(length(&path) < straight * 1.02);
    // Original quirk: an apex move on the final portal re-adds the end point.
    assert_eq!(path.len(), 4);
    assert_eq!(path[2].position, end);
}

#[test]
fn wall_with_gap_routes_through_the_gap() {
    // Wall at x = 5 except a gap at z = 8.
    let mesh = grid(10, |x, z| x == 5 && z != 8, |_, _| M::DEFAULT);
    let (found, path) = mesh
        .find_shortest_path([1.5, 0.0, 1.5], [8.5, 0.0, 1.5], M::DEFAULT, &no_ice)
        .unwrap();
    assert!(found);
    assert!(path.len() >= 4, "{path:?}");
    assert_on_mesh(&mesh, &path);
    // Turning points are the gap's corners.
    let corners: Vec<_> = path[1..path.len() - 1].iter().map(|n| n.position).collect();
    assert!(
        corners.contains(&[5.0, 0.0, 8.0]) && corners.contains(&[6.0, 0.0, 8.0]),
        "{corners:?}"
    );
}

#[test]
fn unreachable_goal_keeps_the_start_triangle() {
    // Full wall at x = 5. The best partial node is chosen by strictly lower
    // TotalCost (g + h); straight progress keeps g + h constant, so no node
    // beats the start and the path collapses to the start triangle.
    let mesh = grid(10, |x, _| x == 5, |_, _| M::DEFAULT);
    let (found, triangles) = mesh
        .find_path([1.5, 0.0, 4.5], [8.5, 0.0, 4.5], M::DEFAULT, &no_ice)
        .unwrap();
    assert!(!found);
    assert_eq!(triangles.len(), 1);
    let (_, path) = mesh
        .find_shortest_path([1.5, 0.0, 4.5], [8.5, 0.0, 4.5], M::DEFAULT, &no_ice)
        .unwrap();
    assert!(path.iter().all(|n| n.position[0] < 5.0), "{path:?}");
}

#[test]
fn movement_properties_gate_traversal() {
    // Column x = 5 is Jump-only.
    let mesh = grid(
        10,
        |_, _| false,
        |x, _| if x == 5 { M::JUMP } else { M::DEFAULT },
    );
    let (found, _) = mesh
        .find_shortest_path([1.5, 0.0, 4.5], [8.5, 0.0, 4.5], M::DEFAULT, &no_ice)
        .unwrap();
    assert!(!found);
    let (found, path) = mesh
        .find_shortest_path([1.5, 0.0, 4.5], [8.5, 0.0, 4.5], M::JUMP, &no_ice)
        .unwrap();
    assert!(found);
    assert!(path
        .iter()
        .all(|n| n.properties == M::DEFAULT || n.properties == M::JUMP));
    // Water is only passable over ice.
    let water = grid(
        10,
        |_, _| false,
        |x, _| if x == 5 { M::WATER } else { M::DEFAULT },
    );
    let (found, _) = water
        .find_shortest_path([1.5, 0.0, 4.5], [8.5, 0.0, 4.5], M::DEFAULT, &no_ice)
        .unwrap();
    assert!(!found);
    let (found, _) = water
        .find_shortest_path([1.5, 0.0, 4.5], [8.5, 0.0, 4.5], M::DEFAULT, &|_| true)
        .unwrap();
    assert!(found);
}

#[test]
fn large_search_exceeds_original_heap_capacity_without_failing() {
    // Serpentine walls force exploring most of a 60x60 grid (7,000+ triangles).
    let mesh = grid(
        60,
        |x, z| x % 4 == 2 && if (x / 4) % 2 == 0 { z != 59 } else { z != 0 },
        |_, _| M::DEFAULT,
    );
    let (found, path) = mesh
        .find_shortest_path([0.5, 0.0, 0.5], [59.5, 0.0, 0.5], M::DEFAULT, &no_ice)
        .unwrap();
    assert!(found);
    assert_on_mesh(&mesh, &path);
    assert!(path.len() > 20);
}

#[test]
fn nearest_position_respects_filters() {
    let mesh = grid(
        4,
        |_, _| false,
        |x, _| if x == 0 { M::FLY } else { M::DEFAULT },
    );
    let (d, p, _) = mesh.nearest([0.5, 2.0, 0.5], M::DEFAULT).unwrap();
    assert!(
        (p[0] - 1.0).abs() < 1e-6 && (d - 4.25).abs() < 1e-4,
        "{p:?} {d}"
    );
    let (d, p, _) = mesh.nearest([0.5, 2.0, 0.5], M::FLY).unwrap();
    assert!(
        (p[0] - 0.5).abs() < 1e-6 && (d - 4.0).abs() < 1e-4,
        "{p:?} {d}"
    );
}

#[test]
fn animated_mesh_links_require_dynamic_triangles_within_native_tolerance() {
    for (gap, dynamic, expected) in [(0.2, true, true), (0.3, true, false), (0.2, false, false)] {
        let vertices = vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 0., 1.],
            [1. + gap, 0., 0.],
            [2. + gap, 0., 0.],
            [1. + gap, 0., 1.],
        ];
        let triangles = vec![
            NavTriangle {
                vertices: [0, 1, 2],
                neighbours: [u16::MAX; 3],
                costs: [1.; 3],
                properties: M::DYNAMIC,
            },
            NavTriangle {
                vertices: [3, 4, 5],
                neighbours: [u16::MAX; 3],
                costs: [1.; 3],
                properties: if dynamic { M::DYNAMIC } else { M::DEFAULT },
            },
        ];
        let mesh = NavMesh::new(vertices, triangles)
            .unwrap()
            .with_groups(vec![0, 1]);
        let (found, path) = mesh
            .find_shortest_path([0.2, 0., 0.2], [1.2 + gap, 0., 0.2], M::ALL, &no_ice)
            .unwrap();
        assert_eq!(found, expected, "gap={gap}, dynamic={dynamic}, {path:?}");
        // The native transition deliberately spans a gap of up to 0.25 m;
        // nodes lie on meshes, but the connecting segment can leave both.
        for node in &path {
            assert!(mesh.nearest(node.position, M::ALL).unwrap().0 < 1e-6);
        }
    }
}
