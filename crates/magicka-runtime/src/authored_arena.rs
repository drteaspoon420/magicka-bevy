//! Clean authored geometry; no retail XNB serialization or target writes.
use crate::scene_script::{xml, XmlElement};
use magicka_content::{
    level::{LevelLight, LevelModel, Locator, NavMesh, NavTriangle, TriMesh, TriggerArea},
    skinned::IDENTITY,
};
use std::collections::BTreeMap;
pub mod terrain;

/// Rewrite authoring metadata. Visuals never supply collision or game rules.
#[derive(Clone, Debug, Default)]
pub struct Presentation {
    pub hidden_boxes: Vec<usize>,
    pub terrain: terrain::Terrain,
    pub surfaces: Vec<Surface>,
    pub scenery: Vec<Scenery>,
}
#[derive(Clone, Debug)]
pub struct Surface {
    pub edge: f32,
    pub center: [f32; 3],
    pub size: [f32; 2],
    pub yaw: f32,
    pub color: [f32; 3],
    pub texture: Option<String>,
    pub tile: f32,
}
#[derive(Clone, Debug)]
pub struct Scenery {
    pub asset: String,
    pub position: [f32; 3],
    pub scale: [f32; 3],
    pub yaw: f32,
}
fn scalar(n: &XmlElement, key: &str) -> Result<f32, String> {
    let v = attr(n, key)?
        .parse::<f32>()
        .map_err(|_| format!("invalid {key}"))?;
    if !v.is_finite() {
        return Err(format!("non-finite {key}"));
    }
    Ok(v)
}
fn cache_path(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.contains(['\\', ':'])
        || value
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err("visual asset must be a relative cache path".into());
    }
    Ok(value.into())
}
fn presentation_root(root: &XmlElement) -> Result<Presentation, String> {
    let mut out = Presentation {
        terrain: terrain::Terrain::read(root)?,
        ..Default::default()
    };
    let mut box_index = 0;
    for n in &root.children {
        match n.name.as_str() {
            "Box" => {
                if let Some((_, v)) = n.attributes.iter().find(|(k, _)| k == "visible") {
                    match v.as_str() {
                        "false" => out.hidden_boxes.push(box_index),
                        "true" => {}
                        _ => return Err("invalid Box visible".into()),
                    }
                }
                box_index += 1;
            }
            "Surface" => {
                let size = vector::<2>(n, "size")?;
                let color = vector::<3>(n, "color")?;
                let tile = scalar(n, "tile")?;
                if size.iter().any(|v| *v <= 0.)
                    || tile <= 0.
                    || color.iter().any(|v| !(0.0..=1.0).contains(v))
                {
                    return Err("invalid surface dimensions/color/tile".into());
                }
                let texture = n
                    .attributes
                    .iter()
                    .find(|(k, _)| k == "texture")
                    .map(|(_, v)| cache_path(v))
                    .transpose()?;
                let edge = n
                    .attributes
                    .iter()
                    .find(|(k, _)| k == "edge")
                    .map(|_| scalar(n, "edge"))
                    .transpose()?
                    .unwrap_or(0.);
                if edge < 0. || edge >= size[0] * 0.4 {
                    return Err("invalid surface edge variation".into());
                }
                out.surfaces.push(Surface {
                    edge,
                    center: vector(n, "center")?,
                    size,
                    yaw: scalar(n, "yaw")?,
                    color,
                    texture,
                    tile,
                });
            }
            "Scenery" => {
                let scale = vector::<3>(n, "scale")?;
                if scale.iter().any(|v| *v <= 0.) {
                    return Err("invalid scenery scale".into());
                }
                out.scenery.push(Scenery {
                    asset: cache_path(attr(n, "asset")?)?,
                    position: vector(n, "position")?,
                    scale,
                    yaw: scalar(n, "yaw")?,
                });
            }
            _ => {}
        }
    }
    Ok(out)
}
pub fn presentation(bytes: &[u8]) -> Result<Presentation, String> {
    let root = xml(bytes).map_err(|e| format!("{e:?}"))?;
    if root.name != "Arena" || attr(&root, "version")? != "1" {
        return Err("expected Arena version=1".into());
    }
    presentation_root(&root)
}
fn attr<'a>(n: &'a XmlElement, key: &str) -> Result<&'a str, String> {
    n.attributes
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .ok_or_else(|| format!("{} requires {key}", n.name))
}
fn vector<const N: usize>(n: &XmlElement, key: &str) -> Result<[f32; N], String> {
    let v = attr(n, key)?
        .split(',')
        .map(|v| {
            v.trim()
                .parse::<f32>()
                .map_err(|_| format!("invalid {key}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let v: [f32; N] = v
        .try_into()
        .map_err(|_| format!("{key} requires {N} components"))?;
    if v.iter().any(|v| !v.is_finite()) {
        return Err(format!("non-finite {key}"));
    }
    Ok(v)
}
fn mesh_box(mesh: &mut TriMesh, p: [f32; 3], size: [f32; 3]) {
    let a = std::array::from_fn::<_, 3, _>(|i| p[i] - size[i] * 0.5);
    let b = std::array::from_fn::<_, 3, _>(|i| p[i] + size[i] * 0.5);
    let first = mesh.vertices.len() as i32;
    mesh.vertices.extend([
        [a[0], a[1], a[2]],
        [a[0], a[1], b[2]],
        [b[0], a[1], b[2]],
        [b[0], a[1], a[2]],
        [a[0], b[1], a[2]],
        [a[0], b[1], b[2]],
        [b[0], b[1], b[2]],
        [b[0], b[1], a[2]],
    ]);
    for t in [
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 4, 7],
        [0, 7, 3],
        [1, 2, 6],
        [1, 6, 5],
        [0, 1, 5],
        [0, 5, 4],
        [3, 7, 6],
        [3, 6, 2],
    ] {
        mesh.triangles.push(t.map(|i| i + first));
    }
}
pub fn decode(bytes: &[u8]) -> Result<LevelModel, String> {
    let root = xml(bytes).map_err(|e| format!("{e:?}"))?;
    if root.name != "Arena" || attr(&root, "version")? != "1" {
        return Err("expected Arena version=1".into());
    }
    let presentation = presentation_root(&root)?;
    let floor = root
        .children
        .iter()
        .find(|n| n.name == "Floor")
        .ok_or("missing Floor")?;
    if root.children.iter().filter(|n| n.name == "Floor").count() != 1 {
        return Err("requires exactly one Floor".into());
    }
    let center = vector::<3>(floor, "center")?;
    let size = vector::<2>(floor, "size")?;
    let cell = attr(floor, "cell")?
        .parse::<f32>()
        .map_err(|_| "invalid cell")?;
    let clearance = attr(floor, "clearance")?
        .parse::<f32>()
        .map_err(|_| "invalid clearance")?;
    if size.iter().any(|v| *v <= 0.)
        || !cell.is_finite()
        || cell <= 0.
        || !clearance.is_finite()
        || clearance < 0.
    {
        return Err("invalid floor dimensions".into());
    }
    let height = attr(floor, "height")?
        .parse::<f32>()
        .map_err(|_| "invalid height")?;
    if !height.is_finite() || height <= 0. {
        return Err("invalid navigation height".into());
    }
    let nx = (size[0] / cell).ceil() as usize;
    let nz = (size[1] / cell).ceil() as usize;
    if nx == 0 || nz == 0 || nx > 100 || nz > 100 {
        return Err("navigation grid exceeds 100 cells per axis".into());
    }
    let mut layers = vec![TriMesh::default(); 10];
    let mut walls = Vec::new();
    let mut locators = Vec::new();
    let mut areas = Vec::new();
    let mut lights = Vec::new();
    let mut camera_bounds = None;
    let x0 = center[0] - size[0] * 0.5;
    let z0 = center[2] - size[1] * 0.5;
    layers[0].vertices = vec![
        [x0, center[1], z0],
        [x0, center[1], z0 + size[1]],
        [x0 + size[0], center[1], z0 + size[1]],
        [x0 + size[0], center[1], z0],
    ];
    layers[0].triangles = vec![[0, 1, 2], [0, 2, 3]];
    for n in &root.children {
        match n.name.as_str() {
            "Floor" | "Surface" | "Scenery" | "River" | "Rise" => {}
            "Box" => {
                let p = vector::<3>(n, "center")?;
                let s = vector::<3>(n, "size")?;
                if s.iter().any(|v| *v <= 0.) {
                    return Err("invalid box size".into());
                }
                mesh_box(&mut layers[1], p, s);
                walls.push((p, s));
            }
            "CameraBounds" => {
                if camera_bounds.is_some() {
                    return Err("only one CameraBounds supported".into());
                }
                let center = vector::<2>(n, "center")?;
                let size = vector::<2>(n, "size")?;
                if size.iter().any(|v| *v <= 0.) {
                    return Err("invalid camera bounds".into());
                }
                camera_bounds = Some((center, size));
            }
            "DirectionalLight" => {
                let direction = vector::<3>(n, "direction")?;
                let diffuse = vector::<3>(n, "diffuse")?;
                let ambient = vector::<3>(n, "ambient")?;
                if direction.iter().map(|v| v * v).sum::<f32>() <= f32::EPSILON
                    || diffuse.iter().chain(ambient.iter()).any(|v| *v < 0.)
                {
                    return Err("invalid directional light".into());
                }
                lights.push(LevelLight {
                    name: attr(n, "name")?.into(),
                    position: [0.; 3],
                    direction,
                    light_type: 1,
                    variation_type: 0,
                    range: 0.,
                    use_attenuation: false,
                    cutoff_angle: 0.,
                    sharpness: 0.,
                    diffuse,
                    ambient,
                    specular: 0.,
                    variation_speed: 0.,
                    variation_amount: 0.,
                    shadow_map_size: 1024,
                    cast_shadows: true,
                });
            }
            "Locator" => {
                let mut t = IDENTITY;
                t[12..15].copy_from_slice(&vector::<3>(n, "position")?);
                let name = attr(n, "name")?.to_string();
                if locators.iter().any(|l: &Locator| l.name == name) {
                    return Err(format!("duplicate locator {name}"));
                }
                locators.push(Locator {
                    name,
                    transform: t,
                    radius: 0.,
                });
            }
            "Area" => {
                let sides = vector::<3>(n, "size")?;
                if sides.iter().any(|v| *v <= 0.) {
                    return Err("invalid area size".into());
                }
                areas.push(TriggerArea {
                    name: attr(n, "name")?.to_string(),
                    position: {
                        let center = vector::<3>(n, "center")?;
                        std::array::from_fn(|i| center[i] - sides[i] * 0.5)
                    },
                    sides,
                    orientation: [0., 0., 0., 1.],
                });
            }
            _ => return Err(format!("unsupported arena element {}", n.name)),
        }
    }
    let dx = size[0] / nx as f32;
    let dz = size[1] / nz as f32;
    if presentation.terrain.river.is_some()
        || !presentation.terrain.rises.is_empty()
        || camera_bounds.is_some()
    {
        let mut floor = TriMesh::default();
        for z in 0..=nz {
            for x in 0..=nx {
                let px = x0 + x as f32 * dx;
                let pz = z0 + z as f32 * dz;
                floor
                    .vertices
                    .push([px, presentation.terrain.height(px, pz, center[1]), pz]);
            }
        }
        for z in 0..nz {
            for x in 0..nx {
                let a = (z * (nx + 1) + x) as i32;
                let b = a + (nx + 1) as i32;
                let c = b + 1;
                let d = a + 1;
                floor.triangles.extend([[a, b, c], [a, c, d]]);
            }
        }
        layers[0] = floor;
    }
    let camera_mesh = camera_bounds.map(|(center, size)| {
        let mut mesh = layers[0].clone();
        for v in &mut mesh.vertices {
            v[1] = v[1].max(0.);
        }
        mesh.triangles.retain(|t| {
            t.iter().all(|i| {
                let v = mesh.vertices[*i as usize];
                (v[0] - center[0]).abs() <= size[0] * 0.5
                    && (v[2] - center[1]).abs() <= size[1] * 0.5
            })
        });
        mesh
    });
    if camera_mesh.as_ref().is_some_and(|m| m.triangles.is_empty()) {
        return Err("camera bounds do not cover terrain triangles".into());
    }
    let liquids = presentation
        .terrain
        .liquid(&layers[0])
        .into_iter()
        .collect();
    let mut nav = NavMesh::default();
    for z in 0..=nz {
        for x in 0..=nx {
            nav.vertices.push({
                let px = x0 + x as f32 * dx;
                let pz = z0 + z as f32 * dz;
                [
                    px,
                    if presentation.terrain.deep(px, pz) {
                        presentation.terrain.river.as_ref().unwrap().surface
                    } else {
                        presentation.terrain.height(px, pz, center[1])
                    },
                    pz,
                ]
            });
        }
    }
    for z in 0..nz {
        for x in 0..nx {
            let xa = x0 + x as f32 * dx;
            let za = z0 + z as f32 * dz;
            let ground = presentation
                .terrain
                .height(xa + dx * 0.5, za + dz * 0.5, center[1]);
            if walls.iter().any(|(p, s)| {
                p[1] - s[1] * 0.5 <= ground + height
                    && p[1] + s[1] * 0.5 >= ground
                    && xa + dx > p[0] - s[0] * 0.5 - clearance
                    && xa < p[0] + s[0] * 0.5 + clearance
                    && za + dz > p[2] - s[2] * 0.5 - clearance
                    && za < p[2] + s[2] * 0.5 + clearance
            }) {
                continue;
            }
            let a = (z * (nx + 1) + x) as u16;
            let b = a + (nx + 1) as u16;
            let c = b + 1;
            let d = a + 1;
            for vertices in [[a, b, c], [a, c, d]] {
                nav.triangles.push(NavTriangle {
                    vertices,
                    neighbours: [u16::MAX; 3],
                    costs: [0.; 3],
                    properties: if presentation.terrain.deep(xa + dx * 0.5, za + dz * 0.5) {
                        magicka_sim::navmesh::movement::WATER
                    } else {
                        0
                    },
                });
            }
        }
    }
    let mut edges = BTreeMap::new();
    let centers: Vec<[f32; 3]> = nav
        .triangles
        .iter()
        .map(|t| {
            std::array::from_fn(|i| {
                t.vertices
                    .iter()
                    .map(|v| nav.vertices[*v as usize][i] / 3.)
                    .sum()
            })
        })
        .collect();
    for ti in 0..nav.triangles.len() {
        for edge in 0..3 {
            let t = &nav.triangles[ti];
            let mut pair = [t.vertices[edge], t.vertices[(edge + 1) % 3]];
            pair.sort();
            if let Some((other, oe)) = edges.remove(&pair) {
                let other: usize = other;
                let oe: usize = oe;
                let cost = (0..3)
                    .map(|i| (centers[ti][i] - centers[other][i]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                nav.triangles[ti].neighbours[edge] = other as u16;
                nav.triangles[ti].costs[edge] = cost;
                nav.triangles[other].neighbours[oe] = ti as u16;
                nav.triangles[other].costs[oe] = cost;
            } else {
                edges.insert(pair, (ti, edge));
            }
        }
    }
    if nav.triangles.is_empty() {
        return Err("arena has no walkable navigation triangles".into());
    }
    Ok(LevelModel {
        model: None,
        animated_parts: vec![],
        lights,
        effects: vec![],
        physics_entities: vec![],
        liquids,
        force_fields: vec![],
        collision_layers: layers,
        camera_mesh,
        trigger_areas: areas,
        locators,
        nav_mesh: nav,
    })
}
