//! Content-to-collision scene loading; owner supplies live animated transforms.
use crate::{
    force_field::{force_field_collision, ForceFieldError},
    level_collision::{
        collision_mesh, AnimatedCollisionPart, LevelCollisionError, SceneCollision,
        StaticLevelCollision, TransformedCollisionMesh,
    },
};
use magicka_content::level::{AnimatedLevelPart, LevelModel};
/// Helper.GetHashCodeCustom (moved to `magicka_sim::hash` so the mod
/// script host can hash names the same way).
pub use magicka_sim::hash::custom_hash;
use magicka_sim::mesh_octree::{MeshError, MeshTransform};
use std::collections::HashSet;
#[derive(Debug, PartialEq, Eq)]
pub enum SceneLoadError {
    Liquid {
        index: usize,
        error: String,
    },
    Static(LevelCollisionError),
    ForceField {
        index: usize,
        error: ForceFieldError,
    },
    UnsupportedName(String),
    DuplicatePartId(i32),
    AnimatedMesh {
        path: Vec<String>,
        error: MeshError,
    },
    MissingTransform(Vec<String>),
}
fn load_parts(
    source: &[AnimatedLevelPart],
    path: &mut Vec<String>,
    transform: &mut impl FnMut(&[String]) -> Option<MeshTransform>,
    next_owner: &mut u32,
) -> Result<Vec<AnimatedCollisionPart>, SceneLoadError> {
    let mut ids = HashSet::new();
    let mut result = Vec::with_capacity(source.len());
    for part in source {
        let owner = *next_owner;
        *next_owner += 1;
        // CLR ToLowerInvariant beyond ASCII is not recovered; report explicitly.
        if !part.name.is_ascii() {
            return Err(SceneLoadError::UnsupportedName(part.name.clone()));
        }
        let name = part.name.to_ascii_lowercase();
        let id = custom_hash(&name);
        if !ids.insert(id) {
            return Err(SceneLoadError::DuplicatePartId(id));
        }
        path.push(name);
        let (material, collision) = if let Some((material, source)) = &part.collision {
            let mesh = collision_mesh(source).map_err(|error| SceneLoadError::AnimatedMesh {
                path: path.clone(),
                error,
            })?;
            let transform =
                transform(path).ok_or_else(|| SceneLoadError::MissingTransform(path.clone()))?;
            (
                *material,
                Some(TransformedCollisionMesh { mesh, transform }),
            )
        } else {
            (0, None)
        };
        let children = load_parts(&part.children, path, transform, next_owner)?;
        path.pop();
        result.push(AnimatedCollisionPart {
            id,
            owner,
            material,
            collision,
            children,
        });
    }
    Ok(result)
}
impl SceneCollision {
    /// Returns a complete collision hierarchy for the decoded scene. Vector order
    /// follows decoded order; original Dictionary mutation/enumeration is pending.
    pub fn from_content(
        source: &LevelModel,
        mut transforms: impl FnMut(&[String]) -> Option<MeshTransform>,
    ) -> Result<Self, SceneLoadError> {
        let static_skin = StaticLevelCollision::from_layers(&source.collision_layers)
            .map_err(SceneLoadError::Static)?;
        let force_fields = source
            .force_fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                force_field_collision(field)
                    .map_err(|error| SceneLoadError::ForceField { index, error })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let animated_parts =
            load_parts(&source.animated_parts, &mut vec![], &mut transforms, &mut 0)?;
        let mut water = Vec::new();
        for (index, liquid) in source.liquids.iter().enumerate() {
            let geometry = liquid
                .collision_geometry()
                .map_err(|error| SceneLoadError::Liquid {
                    index,
                    error: error.to_string(),
                })?;
            let mut retained = crate::water_collision::WaterCollision::from_geometry(
                &geometry,
                liquid.freezable,
                liquid.auto_freeze,
            )
            .map_err(|error| SceneLoadError::Liquid {
                index,
                error: format!("{error:?}"),
            })?;
            retained.lava = matches!(
                liquid.material,
                magicka_content::level::LiquidMaterial::Lava(_)
            );
            retained.has_collision = liquid.has_collision;
            water.push(retained);
        }
        Ok(Self {
            water,
            static_skin,
            force_fields,
            animated_parts,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn empty(name: &str) -> AnimatedLevelPart {
        AnimatedLevelPart {
            name: name.into(),
            affect_shields: false,
            model: None,
            mesh_settings: vec![],
            liquids: vec![],
            locators: vec![],
            animation_duration: 0.0,
            animation: vec![],
            effects: vec![],
            lights: vec![],
            collision: None,
            nav_mesh: None,
            children: vec![],
        }
    }
    #[test]
    fn names_lowercase_hash_and_child_scope_match_constructor() {
        let mut parent = empty("ROOT");
        parent.children = vec![empty("Child")];
        let parts = load_parts(
            &[parent, empty("Child")],
            &mut vec![],
            &mut |_| panic!(),
            &mut 0,
        )
        .unwrap();
        assert_eq!(parts[0].id, custom_hash("root"));
        assert_eq!(parts[0].children[0].id, parts[1].id);
        assert_ne!(parts[0].children[0].owner, parts[1].owner);
        assert!(matches!(
            load_parts(
                &[empty("ROOT"), empty("root")],
                &mut vec![],
                &mut |_| panic!(),
                &mut 0
            ),
            Err(SceneLoadError::DuplicatePartId(_))
        ));
    }
    #[test]
    fn animated_mesh_requires_transform_and_retains_material() {
        let mut part = empty("Door");
        part.collision = Some((
            7,
            magicka_content::level::TriMesh {
                vertices: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                triangles: vec![[0, 1, 2]],
            },
        ));
        assert!(matches!(
            load_parts(&[part.clone()], &mut vec![], &mut |_| None, &mut 0),
            Err(SceneLoadError::MissingTransform(_))
        ));
        let identity = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let parts = load_parts(
            &[part],
            &mut vec![],
            &mut |p| {
                assert_eq!(p, ["door"]);
                Some(MeshTransform {
                    forward: identity,
                    inverse: identity,
                })
            },
            &mut 0,
        )
        .unwrap();
        assert_eq!(parts[0].material, 7);
        assert!(parts[0].collision.is_some());
    }
    #[test]
    fn empty_hash_uses_final_wrapping_sum_and_unicode_casing_is_rejected() {
        assert_eq!(
            custom_hash(""),
            352654597i32.wrapping_add(352654597i32.wrapping_mul(1566083941))
        );
        assert!(matches!(
            load_parts(&[empty("Dörr")], &mut vec![], &mut |_| None, &mut 0),
            Err(SceneLoadError::UnsupportedName(_))
        ));
    }
}
