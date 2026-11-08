//! Decoded collision geometry adapter; local meshes, not complete LevelModel.
use magicka_content::level::TriMesh;
use magicka_sim::mesh_octree::{MeshError, MeshOctree};
pub fn collision_mesh(source: &TriMesh) -> Result<MeshOctree, MeshError> {
    let indices = source
        .triangles
        .iter()
        .map(|tri| {
            let mut out = [0; 3];
            for (i, &index) in tri.iter().enumerate() {
                out[i] = usize::try_from(index).map_err(|_| MeshError::InvalidIndex)?;
            }
            Ok(out)
        })
        .collect::<Result<Vec<_>, MeshError>>()?;
    MeshOctree::build(source.vertices.clone(), indices)
}

/// LevelModel's ten identity-transformed material primitives, including empties.
/// Force fields and animated parts are not members of this static skin.
#[derive(Clone, Debug)]
pub struct StaticLevelCollision {
    layers: [MeshOctree; 10],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LevelCollisionError {
    LayerCount(usize),
    Mesh { layer: usize, error: MeshError },
}
impl StaticLevelCollision {
    pub fn from_layers(source: &[TriMesh]) -> Result<Self, LevelCollisionError> {
        if source.len() != 10 {
            return Err(LevelCollisionError::LayerCount(source.len()));
        }
        let layers = source
            .iter()
            .enumerate()
            .map(|(layer, mesh)| {
                collision_mesh(mesh).map_err(|error| LevelCollisionError::Mesh { layer, error })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            layers: layers.try_into().unwrap(),
        })
    }
    /// The ten material primitives in skin order (index = CollisionMaterials).
    pub fn layers(&self) -> &[MeshOctree; 10] {
        &self.layers
    }
    pub fn segment_intersect(
        &self,
        segment: magicka_sim::spray_pulse::Segment,
        ignore_backfaces: bool,
    ) -> Option<magicka_sim::collision_skin::SkinHit> {
        magicka_sim::collision_skin::segment_intersect(
            10,
            segment,
            ignore_backfaces,
            |i, segment, backfaces| {
                self.layers[i]
                    .segment_intersect(segment, backfaces)
                    .map(|h| magicka_sim::collision_skin::PrimitiveHit {
                        fraction: h.hit.fraction,
                        position: h.position,
                        normal: h.normal,
                    })
            },
        )
    }
}

#[derive(Clone, Debug)]
pub struct TransformedCollisionMesh {
    pub mesh: MeshOctree,
    pub transform: magicka_sim::mesh_octree::MeshTransform,
}
impl TransformedCollisionMesh {
    fn query(
        &self,
        segment: magicka_sim::spray_pulse::Segment,
    ) -> Option<magicka_sim::collision_skin::PrimitiveHit> {
        self.mesh
            .segment_intersect_transformed(segment, false, self.transform)
            .map(|h| magicka_sim::collision_skin::PrimitiveHit {
                fraction: h.hit.fraction,
                position: h.position,
                normal: h.normal,
            })
    }
}
#[derive(Clone, Debug)]
pub struct AnimatedCollisionPart {
    pub id: i32,
    /// Scene-local object identity, independent of scoped name hashes.
    pub owner: u32,
    pub material: u8,
    pub collision: Option<TransformedCollisionMesh>,
    /// Caller supplies original dictionary enumeration order.
    pub children: Vec<AnimatedCollisionPart>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneHit {
    pub hit: magicka_sim::collision_skin::PrimitiveHit,
    pub primitive: i32,
    pub animated_part: Option<i32>,
    pub animated_owner: Option<u32>,
}
impl AnimatedCollisionPart {
    pub fn segment_intersect(
        &self,
        segment: magicka_sim::spray_pulse::Segment,
    ) -> Option<SceneHit> {
        let mut result = self
            .collision
            .as_ref()
            .and_then(|c| c.query(segment))
            .map(|hit| SceneHit {
                hit,
                primitive: i32::from(self.material),
                animated_part: Some(self.id),
                animated_owner: Some(self.owner),
            });
        for child in &self.children {
            if let Some(hit) = child.segment_intersect(segment) {
                if result
                    .as_ref()
                    .is_none_or(|old| hit.hit.fraction < old.hit.fraction)
                {
                    result = Some(hit);
                }
            }
        }
        result.filter(|r| r.hit.fraction <= 1.0)
    }
}
/// LevelModel query order; live animation transforms and dictionary order are
/// supplied by its owner. Dynamic mesh queries do not inherit backface filtering.
#[derive(Clone, Debug)]
pub struct SceneCollision {
    /// Retained top-level Water and Lava in original liquid enumeration order.
    /// The historical field name remains for existing water callers.
    pub water: Vec<crate::water_collision::WaterCollision>,
    pub static_skin: StaticLevelCollision,
    pub force_fields: Vec<TransformedCollisionMesh>,
    pub animated_parts: Vec<AnimatedCollisionPart>,
}
impl SceneCollision {
    /// Liquid.Freeze for retained top-level Water/Lava. Animated liquids remain
    /// pending. Host supplies shield clipping for each local arc.
    pub fn freeze_water(
        &mut self,
        origin: [f32; 3],
        direction: [f32; 3],
        spread: f32,
        multiplier: f32,
        damages: &[magicka_sim::damage::Damage],
        mut clip_shields: impl FnMut(&mut magicka_sim::spray_pulse::Segment),
    ) {
        for water in &mut self.water {
            let magnitude = magicka_sim::liquid_freeze::magnitude(damages, water.lava, multiplier);
            water.freeze_local(origin, direction, spread, magnitude, &mut clip_shields);
        }
    }

    /// Water/Lava.Update freeze values; a None data channel leaves state unchanged.
    pub fn update_water(&mut self, dt: f32, data_channel_present: bool) {
        for water in &mut self.water {
            water.freeze.update(dt, data_channel_present);
        }
    }

    /// GameScene.LiquidSegmentIntersect returns the first matching liquid.
    /// Top-level Water/Lava have identity transforms. Animated liquids are pending.
    pub fn liquid_segment_intersect(
        &self,
        segment: magicka_sim::spray_pulse::Segment,
        ignore_backfaces: bool,
        ignore_water: bool,
        ignore_ice: bool,
    ) -> Option<crate::water_collision::WaterSegmentHit> {
        self.water.iter().find_map(|water| {
            water.segment_intersect(segment, ignore_backfaces, ignore_water, ignore_ice)
        })
    }

    pub fn segment_intersect(
        &self,
        segment: magicka_sim::spray_pulse::Segment,
        ignore_backfaces: bool,
    ) -> Option<SceneHit> {
        let mut result = self
            .static_skin
            .segment_intersect(segment, ignore_backfaces)
            .map(|skin| SceneHit {
                hit: skin.hit,
                primitive: skin.primitive as i32,
                animated_part: None,
                animated_owner: None,
            });
        for field in &self.force_fields {
            if let Some(hit) = field.query(segment) {
                if result
                    .as_ref()
                    .is_none_or(|old| hit.fraction < old.hit.fraction)
                {
                    result = Some(SceneHit {
                        hit,
                        primitive: 0,
                        animated_part: None,
                        animated_owner: None,
                    });
                }
            }
        }
        for part in &self.animated_parts {
            if let Some(hit) = part.segment_intersect(segment) {
                if result
                    .as_ref()
                    .is_none_or(|old| hit.hit.fraction < old.hit.fraction)
                {
                    result = Some(hit);
                }
            }
        }
        result.filter(|r| r.hit.fraction <= 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use magicka_sim::spray_pulse::Segment;
    #[test]
    fn decoded_indices_feed_mesh_query_and_invalid_negative_index_is_rejected() {
        let mut content = TriMesh {
            vertices: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            triangles: vec![[0, 1, 2]],
        };
        let mesh = collision_mesh(&content).unwrap();
        assert_eq!(
            mesh.segment_intersect(
                Segment {
                    origin: [0.25, 0.25, 1.0],
                    delta: [0.0, 0.0, -2.0]
                },
                false
            )
            .unwrap()
            .hit
            .fraction,
            0.5
        );
        content.triangles[0][0] = -1;
        assert!(matches!(
            collision_mesh(&content),
            Err(MeshError::InvalidIndex)
        ));
    }
    #[test]
    fn static_layers_keep_empty_material_slots_and_skin_arithmetic() {
        let mut layers = vec![TriMesh::default(); 10];
        for (i, z) in [1.0, 2.0, 4.0].into_iter().enumerate() {
            layers[i] = TriMesh {
                vertices: vec![[-1.0, -1.0, z], [1.0, -1.0, z], [0.0, 1.0, z]],
                triangles: vec![[0, 1, 2]],
            };
        }
        let collision = StaticLevelCollision::from_layers(&layers).unwrap();
        let hit = collision
            .segment_intersect(
                Segment {
                    origin: [0.0; 3],
                    delta: [0.0, 0.0, 8.0],
                },
                false,
            )
            .unwrap();
        assert_eq!(hit.primitive, 0);
        assert_eq!(hit.hit.fraction, 0.0625);
        assert_eq!(hit.hit.position, [0.0, 0.0, 1.0]);
        assert!(collision
            .segment_intersect(
                Segment {
                    origin: [0.0; 3],
                    delta: [0.0, 0.0, 8.0]
                },
                true
            )
            .is_none());
        assert!(matches!(
            StaticLevelCollision::from_layers(&layers[..9]),
            Err(LevelCollisionError::LayerCount(9))
        ));
    }
}
