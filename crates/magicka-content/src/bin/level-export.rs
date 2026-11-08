//! Export a level scene's static geometry.
//!
//!     level-export <content-dir> <level.xnb> <out-dir> [textures-dir]
//!
//! Writes `<name>.obj` + `<name>.mtl` (BiTree geometry, one object per root,
//! textures referenced from `textures-dir` as produced by `tex-export`),
//! `<name>.top.png` (top-down shaded render, highest surface wins) and
//! `<name>.nav.png` (nav mesh, colored by movement properties). Output is
//! derived from proprietary data: keep it in an untracked location.

use magicka_content::level::LevelModel;
use magicka_content::mesh;
use magicka_content::texture::encode_png;
use magicka_content::xna::{BiTreeNode, XnaObject};
use magicka_content::{parse_primary, Content};
use magicka_xnb::xnb;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn leaves(n: &BiTreeNode, out: &mut Vec<(usize, usize)>) {
    if n.child_a.is_none() && n.child_b.is_none() {
        out.push((
            n.start_index.max(0) as usize,
            n.primitive_count.max(0) as usize,
        ));
    }
    for c in [&n.child_a, &n.child_b].into_iter().flatten() {
        leaves(c, out);
    }
}

fn index_files(dir: &Path, root: &Path, map: &mut HashMap<String, PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            index_files(&p, root, map);
        } else if let Ok(rel) = p.strip_prefix(root) {
            let key = rel
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/")
                .to_lowercase();
            map.insert(key, p.clone());
        }
    }
}

struct Mesh {
    name: String,
    texture: Option<String>,
    color: [f32; 3],
    positions: Vec<[f32; 3]>,
    uvs: Option<Vec<[f32; 2]>>,
    triangles: Vec<[u32; 3]>,
}

fn collect(level: &LevelModel, asset: &str, report: &mut String) -> Vec<Mesh> {
    let mut meshes = Vec::new();
    let Some(model) = &level.model else {
        return meshes;
    };
    for (ri, root) in model.roots.iter().enumerate() {
        let (Some(decl), Some(vb), Some((sixteen, ib))) = (
            &root.vertex_declaration,
            &root.vertex_buffer,
            &root.index_buffer,
        ) else {
            continue;
        };
        let stride = root.vertex_stride.max(0) as usize;
        let Some(positions) = mesh::positions(decl, vb, stride) else {
            let _ = writeln!(report, "root {ri}: no Vector3 position element");
            continue;
        };
        let uvs = mesh::texcoords(decl, vb, stride);
        let idx = mesh::indices(*sixteen, ib);
        let mut ranges = Vec::new();
        leaves(&root.node, &mut ranges);
        let covered: usize = ranges.iter().map(|r| r.1 * 3).sum();
        let mut triangles = Vec::new();
        for (start, prims) in ranges {
            for t in 0..prims {
                let b = start + 3 * t;
                if let Some(s) = idx.get(b..b + 3) {
                    triangles.push([s[0], s[1], s[2]]);
                }
            }
        }
        let _ = writeln!(
            report,
            "root {ri}: {} vertices (declared {}), {} indices, leaf ranges cover {covered}, visible={}",
            positions.len(),
            root.num_vertices,
            idx.len(),
            root.visible
        );
        let (texture, color) = match &root.effect {
            Some(XnaObject::RenderDeferredEffect(e)) => (
                e.layer0
                    .diffuse_texture
                    .as_ref()
                    .map(|t| mesh::resolve_reference(asset, t)),
                e.layer0.diffuse_color,
            ),
            _ => (None, [0.7, 0.7, 0.7]),
        };
        meshes.push(Mesh {
            name: format!("root_{ri}"),
            texture,
            color,
            positions,
            uvs,
            triangles,
        });
    }
    meshes
}

fn write_obj(
    out: &Path,
    base: &str,
    meshes: &[Mesh],
    textures: &HashMap<String, PathBuf>,
) -> std::io::Result<()> {
    let mut obj = format!("mtllib {base}.mtl\n");
    let mut mtl = String::new();
    let (mut vbase, mut tbase) = (1usize, 1usize);
    for m in meshes {
        let _ = writeln!(
            mtl,
            "newmtl {}\nKd {} {} {}",
            m.name, m.color[0], m.color[1], m.color[2]
        );
        if let Some(png) = m
            .texture
            .as_ref()
            .and_then(|t| textures.get(&t.to_lowercase()))
        {
            let _ = writeln!(mtl, "map_Kd {}", png.display());
        }
        let _ = writeln!(obj, "o {}\nusemtl {}", m.name, m.name);
        for p in &m.positions {
            let _ = writeln!(obj, "v {} {} {}", p[0], p[1], p[2]);
        }
        if let Some(uvs) = &m.uvs {
            for t in uvs {
                let _ = writeln!(obj, "vt {} {}", t[0], 1.0 - t[1]);
            }
        }
        for t in &m.triangles {
            let [a, b, c] = t.map(|i| i as usize);
            if m.uvs.is_some() {
                let _ = writeln!(
                    obj,
                    "f {0}/{3} {1}/{4} {2}/{5}",
                    a + vbase,
                    b + vbase,
                    c + vbase,
                    a + tbase,
                    b + tbase,
                    c + tbase
                );
            } else {
                let _ = writeln!(obj, "f {} {} {}", a + vbase, b + vbase, c + vbase);
            }
        }
        vbase += m.positions.len();
        tbase += m.uvs.as_ref().map_or(0, |u| u.len());
    }
    std::fs::write(out.join(format!("{base}.obj")), obj)?;
    std::fs::write(out.join(format!("{base}.mtl")), mtl)
}

/// Top-down orthographic raster over X/Z; the highest surface wins.
struct Raster {
    w: usize,
    h: usize,
    min: [f32; 2],
    scale: f32,
    rgba: Vec<u8>,
    depth: Vec<f32>,
}

impl Raster {
    fn new(points: impl Iterator<Item = [f32; 3]>, size: usize) -> Option<Self> {
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in points {
            lo = [lo[0].min(p[0]), lo[1].min(p[2])];
            hi = [hi[0].max(p[0]), hi[1].max(p[2])];
        }
        if lo[0] >= hi[0] || lo[1] >= hi[1] {
            return None;
        }
        let scale = (size as f32 - 1.0) / (hi[0] - lo[0]).max(hi[1] - lo[1]);
        let (w, h) = (
            ((hi[0] - lo[0]) * scale) as usize + 1,
            ((hi[1] - lo[1]) * scale) as usize + 1,
        );
        Some(Raster {
            w,
            h,
            min: lo,
            scale,
            rgba: vec![0; w * h * 4],
            depth: vec![f32::MIN; w * h],
        })
    }

    fn tri(&mut self, p: [[f32; 3]; 3], color: [u8; 4]) {
        let s = p.map(|v| {
            [
                (v[0] - self.min[0]) * self.scale,
                (v[2] - self.min[1]) * self.scale,
                v[1],
            ]
        });
        let area =
            (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
        if area.abs() < 1e-9 {
            return;
        }
        let x0 = s
            .iter()
            .map(|v| v[0])
            .fold(f32::MAX, f32::min)
            .floor()
            .max(0.0) as usize;
        let x1 = (s.iter().map(|v| v[0]).fold(f32::MIN, f32::max).ceil() as usize).min(self.w - 1);
        let y0 = s
            .iter()
            .map(|v| v[1])
            .fold(f32::MAX, f32::min)
            .floor()
            .max(0.0) as usize;
        let y1 = (s.iter().map(|v| v[1]).fold(f32::MIN, f32::max).ceil() as usize).min(self.h - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((s[1][0] - px) * (s[2][1] - py) - (s[2][0] - px) * (s[1][1] - py)) / area;
                let w1 = ((s[2][0] - px) * (s[0][1] - py) - (s[0][0] - px) * (s[2][1] - py)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * s[0][2] + w1 * s[1][2] + w2 * s[2][2];
                let i = y * self.w + x;
                if z > self.depth[i] {
                    self.depth[i] = z;
                    self.rgba[i * 4..i * 4 + 4].copy_from_slice(&color);
                }
            }
        }
    }

    fn png(&self) -> Vec<u8> {
        encode_png(self.w, self.h, &self.rgba)
    }
}

fn shade(p: [[f32; 3]; 3], base: [f32; 3]) -> [u8; 4] {
    let u = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
    let v = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
    let l = [0.35f32, 0.85, 0.4];
    let ll = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
    let d = ((n[0] * l[0] + n[1] * l[1] + n[2] * l[2]) / (len * ll)).abs();
    let k = 0.25 + 0.75 * d;
    let c = |x: f32| ((x.clamp(0.0, 1.0) * k) * 255.0) as u8;
    [c(base[0]), c(base[1]), c(base[2]), 255]
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !(3..=4).contains(&args.len()) {
        eprintln!("usage: level-export <content-dir> <level.xnb> <out-dir> [textures-dir]");
        return ExitCode::from(2);
    }
    let (content, file, out) = (
        Path::new(&args[0]),
        Path::new(&args[1]),
        Path::new(&args[2]),
    );
    let asset = file
        .strip_prefix(content)
        .unwrap_or(file)
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/");
    let x = match xnb::parse(&std::fs::read(file).expect("read level")) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let level = match parse_primary(&x) {
        Some(Ok(p)) => match p.content {
            Content::Level(l) => l,
            _ => {
                eprintln!("not a LevelModel");
                return ExitCode::FAILURE;
            }
        },
        Some(Err(e)) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
        None => return ExitCode::FAILURE,
    };
    let mut textures = HashMap::new();
    if let Some(t) = args.get(3) {
        // Absolute paths, so the MTL works wherever it is opened from.
        let t = std::fs::canonicalize(t).unwrap_or_else(|_| PathBuf::from(t));
        index_files(&t, &t, &mut textures);
    }
    let _ = std::fs::create_dir_all(out);
    let base = asset.rsplit('/').next().unwrap_or("level").to_string();
    let mut report = String::new();
    let meshes = collect(&level, &asset, &mut report);
    if let Err(e) = write_obj(out, &base, &meshes, &textures) {
        eprintln!("write obj: {e}");
        return ExitCode::FAILURE;
    }
    let all = meshes.iter().flat_map(|m| m.positions.iter().copied());
    if let Some(mut r) = Raster::new(all, 1024) {
        for m in &meshes {
            for t in &m.triangles {
                let p = t.map(|i| m.positions.get(i as usize).copied().unwrap_or([0.0; 3]));
                r.tri(p, shade(p, m.color));
            }
        }
        let _ = std::fs::write(out.join(format!("{base}.top.png")), r.png());
    }
    let nav = &level.nav_mesh;
    if let Some(mut r) = Raster::new(nav.vertices.iter().copied(), 1024) {
        for t in &nav.triangles {
            let p = t
                .vertices
                .map(|i| nav.vertices.get(i as usize).copied().unwrap_or([0.0; 3]));
            let c = t.properties;
            let color = [
                80 + (c & 7) * 25,
                160u8.wrapping_add(c.wrapping_mul(37)),
                200u8.wrapping_sub(c.wrapping_mul(53)),
                255,
            ];
            r.tri(p, color);
        }
        let _ = std::fs::write(out.join(format!("{base}.nav.png")), r.png());
    }
    let tris: usize = meshes.iter().map(|m| m.triangles.len()).sum();
    eprint!("{report}");
    eprintln!(
        "{asset}: {} roots exported, {tris} triangles; nav mesh {} vertices / {} triangles",
        meshes.len(),
        nav.vertices.len(),
        nav.triangles.len()
    );
    ExitCode::SUCCESS
}
