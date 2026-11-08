//! Tool registry and the built-in tools: spawn (directly or staged), select
//! and move (gizmo), remove and realize. Games add [`ToolKind::Custom`]
//! tools and handle their [`ToolAction`]s.
use crate::{
    LiveEdit, LiveEditKind, Reticle, Selectable, Selection, SpawnCatalogue, SpawnRequest,
    StagedSpawns, ToolAction, ToolButton, ToolButtons, ToolMode, ToolRay,
};
use bevy::prelude::*;

pub const SPAWN: &str = "spawn";
pub const SELECT: &str = "select";
pub const REMOVE: &str = "remove";
pub const REALIZE: &str = "realize";
pub const INSPECT: &str = "inspect";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Spawn,
    Select,
    Remove,
    Realize,
    /// Pick an entity for the reflection [`crate::Inspector`].
    Inspect,
    /// Game-defined: uses become [`ToolAction`] messages.
    Custom,
}

#[derive(Clone, Debug)]
pub struct ToolInfo {
    pub id: String,
    pub label: String,
    pub category: String,
    /// One line per control, shown in the tool panel and HUD.
    pub help: String,
    pub kind: ToolKind,
    /// Changes the world (refused while [`ToolMode::read_only`] is set).
    pub edits_world: bool,
}

#[derive(Resource)]
pub struct ToolRegistry {
    pub tools: Vec<ToolInfo>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        let tool = |id: &str, label: &str, category: &str, help: &str, kind| ToolInfo {
            id: id.into(),
            label: label.into(),
            category: category.into(),
            help: help.into(),
            kind,
            edits_world: true,
        };
        Self {
            tools: vec![
                tool(
                    SPAWN,
                    "Spawner",
                    "Construction",
                    "Primary: place the selected spawnable at the reticle\nSecondary: turn the next spawn 45 degrees\nStage option: place an inactive entry to realize later",
                    ToolKind::Spawn,
                ),
                tool(
                    SELECT,
                    "Select / move",
                    "Construction",
                    "Primary: select, drag arrows to move and rings to rotate (Ctrl: no snapping)\nSecondary: cycle move + rotate / move / rotate\nDelete: remove selection; Ctrl+D: duplicate staged entry",
                    ToolKind::Select,
                ),
                tool(
                    REMOVE,
                    "Remover",
                    "Construction",
                    "Primary: remove the staged entry or live entity under the cursor",
                    ToolKind::Remove,
                ),
                tool(
                    REALIZE,
                    "Realize",
                    "Staging",
                    "Primary: make the staged entry under the cursor real\nSecondary: realize every staged entry",
                    ToolKind::Realize,
                ),
                ToolInfo {
                    edits_world: false,
                    ..tool(
                        INSPECT,
                        "Inspector",
                        "Utilities",
                        "Primary: open the entity under the cursor in the component inspector",
                        ToolKind::Inspect,
                    )
                },
            ],
        }
    }
}

impl ToolRegistry {
    pub fn get(&self, id: &str) -> Option<&ToolInfo> {
        self.tools.iter().find(|t| t.id == id)
    }

    /// Add or replace a tool by id.
    pub fn register(&mut self, tool: ToolInfo) {
        match self.tools.iter_mut().find(|t| t.id == tool.id) {
            Some(slot) => *slot = tool,
            None => self.tools.push(tool),
        }
    }

    /// Categories in first-appearance order.
    pub fn categories(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for t in &self.tools {
            if !out.contains(&t.category.as_str()) {
                out.push(&t.category);
            }
        }
        out
    }
}

/// Pick radius and centre height for staged entries.
const STAGED_RADIUS: f32 = 0.8;
const STAGED_CENTER: f32 = 1.0;

/// Distance along `ray` to a sphere, if hit.
pub fn ray_sphere(ray: Ray3d, center: Vec3, radius: f32) -> Option<f32> {
    let oc = ray.origin - center;
    let b = oc.dot(*ray.direction);
    let c = oc.length_squared() - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    let t = if t >= 0.0 { t } else { -b + disc.sqrt() };
    (t >= 0.0).then_some(t)
}

/// The staged entry or live selectable nearest along the ray.
pub fn pick(
    ray: Ray3d,
    staged: &StagedSpawns,
    live: &Query<(&Selectable, &GlobalTransform)>,
) -> Option<Selection> {
    let staged = staged.items.iter().filter_map(|s| {
        let center = s.transform.translation + Vec3::Y * STAGED_CENTER;
        ray_sphere(ray, center, STAGED_RADIUS).map(|t| (t, Selection::Staged(s.id)))
    });
    let live = live.iter().filter_map(|(s, g)| {
        let center = g.translation() + Vec3::Y * s.center_height;
        ray_sphere(ray, center, s.radius).map(|t| (t, Selection::Live(s.id)))
    });
    staged
        .chain(live)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, s)| s)
}

pub(crate) fn use_tools(
    mut mode: ResMut<ToolMode>,
    buttons: Res<ToolButtons>,
    keys: Res<ButtonInput<KeyCode>>,
    ray: Res<ToolRay>,
    reticle: Res<Reticle>,
    registry: Res<ToolRegistry>,
    catalogue: Res<SpawnCatalogue>,
    mut staged: ResMut<StagedSpawns>,
    live: Query<(&Selectable, &GlobalTransform)>,
    mut spawns: MessageWriter<SpawnRequest>,
    mut edits: MessageWriter<LiveEdit>,
    mut actions: MessageWriter<ToolAction>,
) {
    if !mode.active {
        return;
    }
    let Some(tool) = registry.get(&mode.tool).cloned() else {
        return;
    };
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if tool.kind == ToolKind::Select {
        if keys.just_pressed(KeyCode::Delete) {
            let selection = mode.selection;
            remove(&mut mode, &mut staged, &mut edits, selection);
        }
        if ctrl && keys.just_pressed(KeyCode::KeyD) {
            if let Some(Selection::Staged(id)) = mode.selection {
                if let Some(copy) = staged.duplicate(id, Vec3::new(1.0, 0.0, 0.0)) {
                    mode.selection = Some(Selection::Staged(copy));
                }
            }
        }
    }
    let pressed = if buttons.primary_pressed {
        Some(ToolButton::Primary)
    } else if buttons.secondary_clicked {
        Some(ToolButton::Secondary)
    } else {
        None
    };
    let Some(button) = pressed else {
        return;
    };
    let under = ray.ray.and_then(|r| pick(r, &staged, &live));
    // Staging edits never touch the game, so read-only seats may still plan.
    let touches_game = match (tool.kind, button) {
        (ToolKind::Spawn, ToolButton::Primary) => !mode.stage_spawns,
        (ToolKind::Remove, _) => matches!(under, Some(Selection::Live(_))),
        (ToolKind::Realize, _) => true,
        (ToolKind::Custom, _) => tool.edits_world,
        _ => false,
    };
    if touches_game {
        if let Some(reason) = mode.read_only.clone() {
            mode.message = reason;
            return;
        }
    }
    match (tool.kind, button) {
        (ToolKind::Spawn, ToolButton::Primary) => {
            let Some(entry) = mode.spawnable.as_ref().and_then(|id| catalogue.get(id)) else {
                mode.message = "Pick something to spawn in the menu first".into();
                return;
            };
            let Some(hit) = reticle.hit else {
                return;
            };
            let transform = Transform::from_translation(hit.point)
                .with_rotation(Quat::from_rotation_y(mode.spawn_yaw));
            let properties = entry.resolve(&mode.spawn_properties);
            if mode.stage_spawns {
                let id = staged.add(entry.id.clone(), transform, properties);
                mode.message = format!("Staged {} (inactive #{id})", entry.label);
            } else {
                spawns.write(SpawnRequest {
                    spawnable: entry.id.clone(),
                    transform,
                    properties,
                    from_stage: None,
                });
                mode.message = format!("Spawned {}", entry.label);
            }
        }
        (ToolKind::Spawn, ToolButton::Secondary) => {
            mode.spawn_yaw = (mode.spawn_yaw + std::f32::consts::FRAC_PI_4) % std::f32::consts::TAU;
        }
        (ToolKind::Select, ToolButton::Primary) => {
            mode.selection = under;
        }
        (ToolKind::Select, ToolButton::Secondary) => {
            mode.gizmo = mode.gizmo.next();
        }
        (ToolKind::Remove, _) => remove(&mut mode, &mut staged, &mut edits, under),
        (ToolKind::Realize, ToolButton::Primary) => match under {
            Some(Selection::Staged(id)) => {
                if let Some(request) = staged.realize(id) {
                    mode.message = format!("Realized #{id}");
                    spawns.write(request);
                }
            }
            _ => mode.message = "Point at a staged (inactive) entry".into(),
        },
        (ToolKind::Realize, ToolButton::Secondary) => {
            let all = staged.realize_all();
            mode.message = format!("Realized {} staged entries", all.len());
            spawns.write_batch(all);
        }
        // Picked by `inspector::pick`.
        (ToolKind::Inspect, _) => {}
        (ToolKind::Custom, _) => {
            actions.write(ToolAction {
                tool: tool.id.clone(),
                button,
                reticle: reticle.hit,
                target: match under {
                    Some(Selection::Live(id)) => Some(id),
                    _ => None,
                },
            });
        }
    }
}

fn remove(
    mode: &mut ToolMode,
    staged: &mut StagedSpawns,
    edits: &mut MessageWriter<LiveEdit>,
    target: Option<Selection>,
) {
    match target {
        Some(Selection::Staged(id)) => {
            staged.remove(id);
            mode.message = format!("Removed staged #{id}");
        }
        Some(Selection::Live(id)) => {
            if let Some(reason) = mode.read_only.clone() {
                mode.message = reason;
                return;
            }
            edits.write(LiveEdit {
                id,
                edit: LiveEditKind::Remove,
            });
            mode.message = format!("Removed #{id}");
        }
        None => return,
    }
    if mode.selection == target {
        mode.selection = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_sphere_hits_front_surface_and_misses_behind() {
        let ray = Ray3d::new(Vec3::ZERO, Dir3::X);
        assert!((ray_sphere(ray, Vec3::new(5.0, 0.0, 0.0), 1.0).unwrap() - 4.0).abs() < 1e-5);
        assert!(ray_sphere(ray, Vec3::new(-5.0, 0.0, 0.0), 1.0).is_none());
        assert!(ray_sphere(ray, Vec3::new(5.0, 2.0, 0.0), 1.0).is_none());
        // From inside: the exit point.
        assert!((ray_sphere(ray, Vec3::ZERO, 1.0).unwrap() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn register_replaces_by_id_and_categories_keep_order() {
        let mut r = ToolRegistry::default();
        let n = r.tools.len();
        r.register(ToolInfo {
            id: "smite".into(),
            label: "Smite".into(),
            category: "Dungeon master".into(),
            help: String::new(),
            kind: ToolKind::Custom,
            edits_world: true,
        });
        r.register(ToolInfo {
            label: "Smite!".into(),
            ..r.get("smite").unwrap().clone()
        });
        assert_eq!(r.tools.len(), n + 1);
        assert_eq!(r.get("smite").unwrap().label, "Smite!");
        assert_eq!(
            r.categories(),
            ["Construction", "Staging", "Utilities", "Dungeon master"]
        );
    }
}
