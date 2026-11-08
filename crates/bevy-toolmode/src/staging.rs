//! Staged (inactive) spawns: catalogue entries placed, oriented and given
//! properties without existing in the game yet. Each shows as a model ghost
//! with a marker; realizing one turns it into a [`crate::SpawnRequest`].
use crate::{
    catalogue::PropertyValues, gizmo::ToolGizmos, Selection, SpawnCatalogue, SpawnRequest, ToolMode,
};
use bevy::{gltf::GltfAssetLabel, prelude::*};

#[derive(Clone, Debug)]
pub struct Staged {
    pub id: u64,
    pub spawnable: String,
    pub transform: Transform,
    pub properties: PropertyValues,
    /// Free-form note for whoever realizes it later.
    pub note: String,
}

#[derive(Resource, Default)]
pub struct StagedSpawns {
    pub items: Vec<Staged>,
    next_id: u64,
    /// Show model ghosts (markers are always drawn while tool mode is on).
    pub hide_ghosts: bool,
}

impl StagedSpawns {
    pub fn add(
        &mut self,
        spawnable: String,
        transform: Transform,
        properties: PropertyValues,
    ) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.items.push(Staged {
            id,
            spawnable,
            transform,
            properties,
            note: String::new(),
        });
        id
    }

    pub fn get(&self, id: u64) -> Option<&Staged> {
        self.items.iter().find(|s| s.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Staged> {
        self.items.iter_mut().find(|s| s.id == id)
    }

    pub fn remove(&mut self, id: u64) -> Option<Staged> {
        let at = self.items.iter().position(|s| s.id == id)?;
        Some(self.items.remove(at))
    }

    /// Duplicate `id` beside the original.
    pub fn duplicate(&mut self, id: u64, offset: Vec3) -> Option<u64> {
        let s = self.get(id)?.clone();
        let mut transform = s.transform;
        transform.translation += offset;
        let copy = self.add(s.spawnable, transform, s.properties);
        self.get_mut(copy)?.note = s.note;
        Some(copy)
    }

    /// Remove `id` and produce its spawn request.
    pub fn realize(&mut self, id: u64) -> Option<SpawnRequest> {
        let s = self.remove(id)?;
        Some(SpawnRequest {
            spawnable: s.spawnable,
            transform: s.transform,
            properties: s.properties,
            from_stage: Some(s.id),
        })
    }

    /// Remove everything and produce spawn requests in placement order.
    pub fn realize_all(&mut self) -> Vec<SpawnRequest> {
        let ids: Vec<u64> = self.items.iter().map(|s| s.id).collect();
        ids.into_iter().filter_map(|id| self.realize(id)).collect()
    }
}

/// Root of any ghost model (staged entry or spawn cursor). Games whose
/// material pipelines rewrite glTF materials should leave its meshes alone
/// so the translucent ghost material sticks.
#[derive(Component, Default)]
pub struct Ghost;

/// Model ghost of a staged entry.
#[derive(Component)]
#[require(Ghost)]
pub struct StagedGhost {
    pub id: u64,
    spawnable: String,
}

/// Ghost of the selected spawnable following the reticle (spawn tool).
#[derive(Component)]
#[require(Ghost)]
pub(crate) struct CursorGhost(String);

pub(crate) fn sync_ghosts(
    mut commands: Commands,
    staged: Res<StagedSpawns>,
    catalogue: Res<SpawnCatalogue>,
    mode: Res<ToolMode>,
    reticle: Res<crate::Reticle>,
    assets: Res<AssetServer>,
    mut ghosts: Query<(Entity, &StagedGhost, &mut Transform), Without<CursorGhost>>,
    mut cursor: Query<(Entity, &CursorGhost, &mut Transform), Without<StagedGhost>>,
) {
    let load = |path: &str| {
        WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path.to_string())))
    };
    let mut present = Vec::new();
    for (entity, ghost, mut transform) in &mut ghosts {
        match staged
            .get(ghost.id)
            .filter(|s| s.spawnable == ghost.spawnable && !staged.hide_ghosts)
        {
            Some(s) => {
                if *transform != s.transform {
                    *transform = s.transform;
                }
                present.push(ghost.id);
            }
            None => commands.entity(entity).try_despawn(),
        }
    }
    if !staged.hide_ghosts {
        for s in staged.items.iter().filter(|s| !present.contains(&s.id)) {
            if let Some(model) = catalogue.get(&s.spawnable).and_then(|e| e.model.as_deref()) {
                commands.spawn((
                    StagedGhost {
                        id: s.id,
                        spawnable: s.spawnable.clone(),
                    },
                    load(model),
                    s.transform,
                ));
            }
        }
    }
    // Cursor ghost.
    let wanted = mode
        .spawnable
        .as_ref()
        .filter(|_| mode.active && mode.tool == crate::tools::SPAWN)
        .and_then(|id| catalogue.get(id))
        .and_then(|e| Some((e.id.clone(), e.model.clone()?)));
    let pose = reticle.hit.map(|h| {
        Transform::from_translation(h.point).with_rotation(Quat::from_rotation_y(mode.spawn_yaw))
    });
    let mut kept = false;
    for (entity, ghost, mut transform) in &mut cursor {
        match (&wanted, pose) {
            (Some((id, _)), Some(pose)) if *id == ghost.0 && !kept => {
                *transform = pose;
                kept = true;
            }
            _ => commands.entity(entity).try_despawn(),
        }
    }
    if let (false, Some((id, model)), Some(pose)) = (kept, wanted, pose) {
        commands.spawn((CursorGhost(id), load(&model), pose));
    }
}

/// A ghost mesh already given its translucent material.
#[derive(Component)]
pub(crate) struct GhostMaterial;

/// Staged and cursor ghosts draw translucent and tinted, so inactive
/// entries do not read as real ones. Materials appear after the scene
/// spawns and load; each source material gets one ghost copy.
pub(crate) fn ghost_materials(
    mut commands: Commands,
    roots: Query<Entity, With<Ghost>>,
    children: Query<&Children>,
    meshes: Query<&MeshMaterial3d<StandardMaterial>, Without<GhostMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut copies: Local<
        std::collections::HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>,
    >,
) {
    for root in &roots {
        for e in children.iter_descendants(root) {
            let Ok(source) = meshes.get(e) else {
                continue;
            };
            let ghost = match copies.get(&source.id()) {
                Some(ghost) => ghost.clone(),
                None => {
                    let Some(ghost) = materials.get(&source.0).map(ghost_of) else {
                        continue;
                    };
                    let ghost = materials.add(ghost);
                    copies.insert(source.id(), ghost.clone());
                    ghost
                }
            };
            commands
                .entity(e)
                .try_insert((MeshMaterial3d(ghost), GhostMaterial));
        }
    }
}

/// The translucent, cyan-tinted copy of a material.
pub fn ghost_of(base: &StandardMaterial) -> StandardMaterial {
    let mut ghost = base.clone();
    ghost.alpha_mode = AlphaMode::Blend;
    ghost.base_color = base.base_color.with_alpha(GHOST_ALPHA);
    ghost.emissive = LinearRgba::rgb(0.05, 0.3, 0.45);
    ghost
}

const GHOST_ALPHA: f32 = 0.45;

/// Markers for staged entries (always) and the selection.
pub(crate) fn draw_markers(
    mode: Res<ToolMode>,
    staged: Res<StagedSpawns>,
    mut gizmos: Gizmos<ToolGizmos>,
) {
    if !mode.active && !mode.menu_open {
        return;
    }
    for s in &staged.items {
        let selected = mode.selection == Some(Selection::Staged(s.id));
        let colour = if selected {
            Color::srgb(1.0, 0.95, 0.2)
        } else {
            Color::srgb(0.3, 0.85, 1.0)
        };
        let p = s.transform.translation;
        let iso = Isometry3d::new(
            p + Vec3::Y * 0.02,
            Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
        );
        gizmos.circle(iso, 0.6, colour);
        gizmos.line(p, p + Vec3::Y * 2.2, colour);
        let facing = s.transform.rotation * Vec3::NEG_Z;
        gizmos.line(
            p + Vec3::Y * 0.05,
            p + Vec3::Y * 0.05 + facing * 0.9,
            colour,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ghost_materials_are_translucent_copies() {
        let base = StandardMaterial {
            base_color: Color::srgb(0.2, 0.4, 0.6),
            ..default()
        };
        let ghost = ghost_of(&base);
        assert_eq!(ghost.alpha_mode, AlphaMode::Blend);
        assert!((ghost.base_color.alpha() - GHOST_ALPHA).abs() < 1e-6);
        assert_eq!(ghost.base_color.with_alpha(1.0), base.base_color);
    }

    #[test]
    fn realize_removes_entries_and_keeps_their_data() {
        let mut staged = StagedSpawns::default();
        let a = staged.add(
            "goblin".into(),
            Transform::from_xyz(1.0, 0.0, 0.0),
            PropertyValues::new(),
        );
        let b = staged.add(
            "troll".into(),
            Transform::from_xyz(2.0, 0.0, 0.0),
            PropertyValues::new(),
        );
        let copy = staged.duplicate(a, Vec3::Z).unwrap();
        assert_eq!(
            staged.get(copy).unwrap().transform.translation,
            Vec3::new(1.0, 0.0, 1.0)
        );
        let request = staged.realize(b).unwrap();
        assert_eq!(request.spawnable, "troll");
        assert_eq!(request.from_stage, Some(b));
        assert!(staged.get(b).is_none());
        let rest: Vec<_> = staged
            .realize_all()
            .into_iter()
            .map(|r| r.from_stage.unwrap())
            .collect();
        assert_eq!(rest, [a, copy]);
        assert!(staged.items.is_empty());
    }
}
