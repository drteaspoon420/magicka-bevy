//! Decoded LevelModel navmeshes as `magicka_sim::navmesh::NavMesh`.
use magicka_sim::{
    mesh_octree::MeshError,
    navmesh::{NavMesh, NavTriangle},
};

pub fn nav_mesh(source: &magicka_content::level::NavMesh) -> Result<NavMesh, MeshError> {
    let triangles = source
        .triangles
        .iter()
        .map(|t| NavTriangle {
            vertices: t.vertices,
            neighbours: t.neighbours,
            costs: t.costs,
            properties: t.properties,
        })
        .collect();
    NavMesh::new(source.vertices.clone(), triangles)
}

/// Animated navigation uses the same world matrices as collision and locators.
pub fn part_transforms(
    sources: &[magicka_content::level::AnimatedLevelPart],
    parts: &[crate::animated_hierarchy::AnimatedPartState],
) -> Vec<[f32; 16]> {
    fn visit(
        sources: &[magicka_content::level::AnimatedLevelPart],
        parts: &[crate::animated_hierarchy::AnimatedPartState],
        out: &mut Vec<[f32; 16]>,
    ) {
        for (source, part) in sources.iter().zip(parts) {
            if source.nav_mesh.is_some() {
                out.push(part.world);
            }
            visit(&source.children, &part.children, out);
        }
    }
    let mut out = Vec::new();
    visit(sources, parts, &mut out);
    out
}

pub fn scene_nav_mesh(
    level: &magicka_content::level::LevelModel,
    parts: &[crate::animated_hierarchy::AnimatedPartState],
) -> Result<NavMesh, MeshError> {
    let mut vertices = level.nav_mesh.vertices.clone();
    let mut triangles: Vec<_> = level
        .nav_mesh
        .triangles
        .iter()
        .map(|t| NavTriangle {
            vertices: t.vertices,
            neighbours: t.neighbours,
            costs: t.costs,
            properties: t.properties,
        })
        .collect();
    let mut groups = vec![0u16; triangles.len()];
    fn append(
        sources: &[magicka_content::level::AnimatedLevelPart],
        parts: &[crate::animated_hierarchy::AnimatedPartState],
        vertices: &mut Vec<[f32; 3]>,
        triangles: &mut Vec<NavTriangle>,
        groups: &mut Vec<u16>,
        group: &mut u16,
    ) -> Result<(), MeshError> {
        for (source, part) in sources.iter().zip(parts) {
            if let Some(mesh) = &source.nav_mesh {
                *group += 1;
                let vo = vertices.len();
                let to = triangles.len();
                if vo + mesh.vertices.len() >= u16::MAX as usize
                    || to + mesh.triangles.len() >= u16::MAX as usize
                {
                    return Err(MeshError::NodeIndexOverflow);
                }
                vertices.extend(mesh.vertices.iter().map(|p| {
                    std::array::from_fn(|i| {
                        p[0] * part.world[i]
                            + p[1] * part.world[4 + i]
                            + p[2] * part.world[8 + i]
                            + part.world[12 + i]
                    })
                }));
                triangles.extend(mesh.triangles.iter().map(|t| {
                    NavTriangle {
                        vertices: t.vertices.map(|v| v + vo as u16),
                        neighbours: t
                            .neighbours
                            .map(|n| if n == u16::MAX { n } else { n + to as u16 }),
                        costs: t.costs,
                        properties: t.properties,
                    }
                }));
                groups.extend(std::iter::repeat_n(*group, mesh.triangles.len()));
            }
            append(
                &source.children,
                &part.children,
                vertices,
                triangles,
                groups,
                group,
            )?;
        }
        Ok(())
    }
    append(
        &level.animated_parts,
        parts,
        &mut vertices,
        &mut triangles,
        &mut groups,
        &mut 0,
    )?;
    Ok(NavMesh::new(vertices, triangles)?.with_groups(groups))
}
