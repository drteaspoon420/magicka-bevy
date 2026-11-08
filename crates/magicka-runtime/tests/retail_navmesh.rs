//! Retail navmesh pathfinding (skips without user content): every level
//! model's navmesh builds; seeded random queries return funnel paths whose
//! nodes lie on the mesh and whose segments stay inside the A* corridor.
use magicka_content::{parse_primary, Content};
use magicka_runtime::navigation::nav_mesh;
use magicka_sim::navmesh::{movement as M, NavMesh};
use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xnb")) {
            out.push(p);
        }
    }
}

fn inside_2d(p: [f32; 3], t: [[f32; 3]; 3], eps: f32) -> bool {
    let s =
        |a: [f32; 3], b: [f32; 3]| (b[0] - a[0]) * (p[2] - a[2]) - (b[2] - a[2]) * (p[0] - a[0]);
    let (d1, d2, d3) = (s(t[0], t[1]), s(t[1], t[2]), s(t[2], t[0]));
    let scale = [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])]
        .iter()
        .map(|(a, b)| ((b[0] - a[0]).powi(2) + (b[2] - a[2]).powi(2)).sqrt())
        .fold(0.0f32, f32::max);
    let e = eps * scale;
    !((d1 < -e || d2 < -e || d3 < -e) && (d1 > e || d2 > e || d3 > e))
}

fn check_level(mesh: &NavMesh, seed: &mut u64, stats: &mut [usize; 3]) {
    let tris = mesh.triangles();
    let walkable: Vec<usize> = (0..tris.len())
        .filter(|&i| tris[i].properties == M::DEFAULT)
        .collect();
    if walkable.len() < 2 {
        return;
    }
    let mut rand = || {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    };
    let corner = |i: usize| tris[i].vertices.map(|v| mesh.vertices()[v as usize]);
    let centre = |i: usize| {
        let c = corner(i);
        std::array::from_fn(|k| (c[0][k] + c[1][k] + c[2][k]) / 3.0)
    };
    for _ in 0..40 {
        let a = walkable[(rand() % walkable.len() as u64) as usize];
        let b = walkable[(rand() % walkable.len() as u64) as usize];
        let (start, end) = (centre(a), centre(b));
        let (found, corridor) = mesh
            .find_path(start, end, M::DEFAULT, &|_| false)
            .expect("no inconsistent edges");
        let path = mesh
            .straighten(start, end, &corridor)
            .expect("valid portals");
        stats[0] += 1;
        stats[1] += usize::from(found);
        for node in &path {
            let (d, _, _) = mesh.nearest(node.position, M::ALL).unwrap();
            assert!(d < 1e-3, "node {:?} off mesh ({d})", node.position);
        }
        let corridor_tris: Vec<[[f32; 3]; 3]> = corridor
            .iter()
            .map(|&t| corner((t & 0xFFFF) as usize))
            .collect();
        for w in path.windows(2) {
            for k in 0..=16 {
                let t = k as f32 / 16.0;
                let p: [f32; 3] = std::array::from_fn(|i| {
                    w[0].position[i] + (w[1].position[i] - w[0].position[i]) * t
                });
                assert!(
                    corridor_tris.iter().any(|tri| inside_2d(p, *tri, 1e-3)),
                    "segment point {p:?} leaves its corridor"
                );
            }
            stats[2] += 1;
        }
    }
}

#[test]
fn retail_navmeshes_path_inside_their_corridors() {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    if !root.is_dir() {
        eprintln!("SKIP: retail content absent");
        return;
    }
    let mut files = vec![];
    walk(&root.join("Levels"), &mut files);
    files.sort();
    let (mut levels, mut seed, mut stats) = (0, 0x5eed_u64, [0usize; 3]);
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        let Ok(xnb) = magicka_xnb::xnb::parse(&bytes) else {
            continue;
        };
        let Some(Ok(parsed)) = parse_primary(&xnb) else {
            continue;
        };
        let Content::Level(level) = parsed.content else {
            continue;
        };
        let mesh =
            nav_mesh(&level.nav_mesh).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
        levels += 1;
        check_level(&mesh, &mut seed, &mut stats);
    }
    eprintln!(
        "levels {levels}, queries {}, goal reached {}, segments checked {}",
        stats[0], stats[1], stats[2]
    );
    assert!(levels >= 130);
}
