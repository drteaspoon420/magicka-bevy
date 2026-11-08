//! Translate/rotate handles for the selection: world-aligned axis arrows and
//! an XZ plane square, or three rotation rings. Handles keep a constant
//! screen size and are hit-tested in screen space; drags project the cursor
//! ray back onto the axis line, plane or ring.
use crate::{
    LiveEdit, LiveEditKind, Selectable, Selection, StagedSpawns, ToolButtons, ToolCamera, ToolMode,
    ToolRay,
};
use bevy::prelude::*;

/// Tool mode gizmos (drawn over geometry). Games pick the render layers
/// through `GizmoConfigStore::config_mut::<ToolGizmos>()`.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct ToolGizmos;

/// Tool gizmos draw over geometry by default.
pub(crate) fn configure(mut store: ResMut<GizmoConfigStore>) {
    store.config_mut::<ToolGizmos>().0.depth_bias = -1.0;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GizmoMode {
    /// Move arrows and rotation rings together (rings drawn larger).
    #[default]
    Universal,
    Translate,
    Rotate,
}

impl GizmoMode {
    fn moves(self) -> bool {
        self != GizmoMode::Rotate
    }
    fn rotates(self) -> bool {
        self != GizmoMode::Translate
    }
    /// Ring radius relative to the handle size.
    fn ring_scale(self) -> f32 {
        if self == GizmoMode::Universal {
            1.35
        } else {
            1.0
        }
    }

    /// The next mode for the select tool's secondary button.
    pub fn next(self) -> Self {
        match self {
            GizmoMode::Universal => GizmoMode::Translate,
            GizmoMode::Translate => GizmoMode::Rotate,
            GizmoMode::Rotate => GizmoMode::Universal,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GizmoMode::Universal => "move + rotate",
            GizmoMode::Translate => "move",
            GizmoMode::Rotate => "rotate",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Axis(usize),
    PlaneXZ,
    Ring(usize),
}

#[derive(Clone, Copy, Debug)]
struct Active {
    handle: Handle,
    target: Selection,
    start: Transform,
    /// Axis parameter, plane hit or ring angle at grab time.
    grab: Vec3,
}

#[derive(Resource, Default)]
pub struct GizmoDrag {
    pub hover: Option<Handle>,
    active: Option<Active>,
    last_sent: Option<Transform>,
}

impl GizmoDrag {
    pub fn dragging(&self) -> bool {
        self.active.is_some()
    }
}

const AXES: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];
/// Handle length in logical pixels.
const HANDLE_PIXELS: f32 = 90.0;
/// Hover distance in logical pixels.
const PICK_PIXELS: f32 = 10.0;
const RING_SEGMENTS: usize = 48;

/// Parameter along `axis` (unit) through `origin` of the point closest to
/// the ray; `None` when the ray is parallel to the axis.
pub fn axis_param(origin: Vec3, axis: Vec3, ray: Ray3d) -> Option<f32> {
    let d = *ray.direction;
    let w0 = origin - ray.origin;
    let b = axis.dot(d);
    let c = d.dot(d);
    let dd = axis.dot(w0);
    let e = d.dot(w0);
    let denom = c - b * b;
    if denom.abs() < 1e-6 {
        return None;
    }
    Some((b * e - c * dd) / denom)
}

/// Ray/plane intersection in front of the ray origin.
pub fn plane_hit(point: Vec3, normal: Vec3, ray: Ray3d) -> Option<Vec3> {
    let t = ray.intersect_plane(point, InfinitePlane3d::new(normal))?;
    Some(ray.get_point(t))
}

fn ring_basis(axis: Vec3) -> (Vec3, Vec3) {
    let u = axis.any_orthonormal_vector();
    (u, axis.cross(u))
}

/// Angle of the ray's hit on the plane of a ring around `axis`.
pub fn ring_angle(center: Vec3, axis: Vec3, ray: Ray3d) -> Option<f32> {
    let v = plane_hit(center, axis, ray)? - center;
    let (u, w) = ring_basis(axis);
    Some(v.dot(w).atan2(v.dot(u)))
}

/// Round to a multiple of `step` (no-op for step <= 0).
pub fn snap(value: f32, step: f32) -> f32 {
    if step > 0.0 {
        (value / step).round() * step
    } else {
        value
    }
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

struct View<'a> {
    camera: &'a Camera,
    transform: GlobalTransform,
}

impl View<'_> {
    fn screen(&self, p: Vec3) -> Option<Vec2> {
        self.camera.world_to_viewport(&self.transform, p).ok()
    }
    fn size(&self, origin: Vec3) -> f32 {
        world_size(self.camera, &self.transform, origin, HANDLE_PIXELS)
    }
}

/// World length at `point` that spans `pixels` on screen (any projection
/// or field of view); falls back to a distance-based size off screen.
pub fn world_size(camera: &Camera, view: &GlobalTransform, point: Vec3, pixels: f32) -> f32 {
    let distance = (view.translation() - point).length().max(0.5);
    let a = camera.world_to_viewport(view, point);
    let b = camera.world_to_viewport(view, point + view.right() * distance * 0.1);
    match (a, b) {
        (Ok(a), Ok(b)) if a.distance(b) > 1e-3 => pixels * distance * 0.1 / a.distance(b),
        _ => distance * 0.1,
    }
}

fn ring_points(center: Vec3, axis: Vec3, radius: f32) -> impl Iterator<Item = Vec3> {
    let (u, w) = ring_basis(axis);
    (0..=RING_SEGMENTS).map(move |i| {
        let a = i as f32 / RING_SEGMENTS as f32 * std::f32::consts::TAU;
        center + (u * a.cos() + w * a.sin()) * radius
    })
}

fn plane_corners(origin: Vec3, size: f32) -> [Vec3; 4] {
    let (a, b) = (size * 0.25, size * 0.5);
    [
        origin + Vec3::new(a, 0.0, a),
        origin + Vec3::new(b, 0.0, a),
        origin + Vec3::new(b, 0.0, b),
        origin + Vec3::new(a, 0.0, b),
    ]
}

fn hover(view: &View, mode: GizmoMode, origin: Vec3, cursor: Vec2) -> Option<Handle> {
    let size = view.size(origin);
    let o = view.screen(origin)?;
    let mut best: Option<(f32, Handle)> = None;
    let mut consider = |d: f32, h: Handle| {
        if d < PICK_PIXELS && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, h));
        }
    };
    if mode.moves() {
        for (i, axis) in AXES.iter().enumerate() {
            if let Some(end) = view.screen(origin + *axis * size) {
                consider(segment_distance(cursor, o, end), Handle::Axis(i));
            }
        }
        let corners: Vec<Vec2> = plane_corners(origin, size)
            .iter()
            .filter_map(|c| view.screen(*c))
            .collect();
        if corners.len() == 4 {
            let center = corners.iter().sum::<Vec2>() / 4.0;
            let radius = corners
                .iter()
                .map(|c| c.distance(center))
                .fold(0.0, f32::max);
            if cursor.distance(center) < radius {
                consider(PICK_PIXELS * 0.5, Handle::PlaneXZ);
            }
        }
    }
    if mode.rotates() {
        for (i, axis) in AXES.iter().enumerate() {
            let points: Vec<Vec2> = ring_points(origin, *axis, size * mode.ring_scale())
                .filter_map(|p| view.screen(p))
                .collect();
            let d = points
                .windows(2)
                .map(|w| segment_distance(cursor, w[0], w[1]))
                .fold(f32::MAX, f32::min);
            consider(d, Handle::Ring(i));
        }
    }
    best.map(|(_, h)| h)
}

fn grab(handle: Handle, start: Vec3, ray: Ray3d) -> Option<Vec3> {
    match handle {
        Handle::Axis(i) => axis_param(start, AXES[i], ray).map(|t| Vec3::splat(t)),
        Handle::PlaneXZ => plane_hit(start, Vec3::Y, ray),
        Handle::Ring(i) => ring_angle(start, AXES[i], ray).map(Vec3::splat),
    }
}

/// The transform a drag produces, given the grab-time state and the ray now.
fn dragged(active: &Active, ray: Ray3d, snap_step: f32, angle_snap_deg: f32) -> Option<Transform> {
    let mut t = active.start;
    let origin = active.start.translation;
    match active.handle {
        Handle::Axis(i) => {
            let now = axis_param(origin, AXES[i], ray)?;
            t.translation = origin + AXES[i] * snap(now - active.grab.x, snap_step);
        }
        Handle::PlaneXZ => {
            let now = plane_hit(origin, Vec3::Y, ray)?;
            let delta = now - active.grab;
            t.translation =
                origin + Vec3::new(snap(delta.x, snap_step), 0.0, snap(delta.z, snap_step));
        }
        Handle::Ring(i) => {
            let now = ring_angle(origin, AXES[i], ray)?;
            let delta = snap((now - active.grab.x).to_degrees(), angle_snap_deg).to_radians();
            t.rotation = Quat::from_axis_angle(AXES[i], delta) * active.start.rotation;
        }
    }
    Some(t)
}

/// Current transform of a selection.
pub fn selection_transform(
    selection: Selection,
    staged: &StagedSpawns,
    live: &Query<(&Selectable, &GlobalTransform)>,
) -> Option<Transform> {
    match selection {
        Selection::Staged(id) => staged.get(id).map(|s| s.transform),
        Selection::Live(id) => live
            .iter()
            .find(|(s, _)| s.id == id)
            .map(|(_, g)| g.compute_transform()),
    }
}

pub(crate) fn interact(
    mut mode: ResMut<ToolMode>,
    mut drag: ResMut<GizmoDrag>,
    mut buttons: ResMut<ToolButtons>,
    ray: Res<ToolRay>,
    keys: Res<ButtonInput<KeyCode>>,
    mut staged: ResMut<StagedSpawns>,
    live: Query<(&Selectable, &GlobalTransform)>,
    cameras: Query<(&Camera, &Transform), With<ToolCamera>>,
    mut edits: MessageWriter<LiveEdit>,
) {
    let usable = mode.active && mode.tool == crate::tools::SELECT;
    let target = mode.selection.filter(|_| usable);
    let Some(((camera, cam_t), target)) = cameras.iter().next().zip(target) else {
        drag.hover = None;
        drag.active = None;
        return;
    };
    let Some(current) = selection_transform(target, &staged, &live) else {
        mode.selection = None;
        drag.active = None;
        return;
    };
    let view = View {
        camera,
        transform: GlobalTransform::from(*cam_t),
    };
    let (Some(cursor), Some(r)) = (ray.cursor, ray.ray) else {
        return;
    };
    if drag.active.is_none() {
        drag.hover = hover(&view, mode.gizmo, current.translation, cursor);
        if let Some(handle) = drag.hover.filter(|_| buttons.primary_pressed) {
            if let Some(g) = grab(handle, current.translation, r) {
                drag.active = Some(Active {
                    handle,
                    target,
                    start: current,
                    grab: g,
                });
                drag.last_sent = None;
                buttons.primary_pressed = false;
            }
        }
        return;
    }
    let active = drag.active.unwrap();
    buttons.primary_pressed = false;
    // Ctrl drags without snapping.
    let free = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let (step, angle) = if free {
        (0.0, 0.0)
    } else {
        (mode.snap, mode.angle_snap)
    };
    if let Some(t) = dragged(&active, r, step, angle) {
        match active.target {
            Selection::Staged(id) => {
                if let Some(s) = staged.get_mut(id) {
                    s.transform = t;
                }
            }
            Selection::Live(id) => {
                if drag.last_sent != Some(t) {
                    edits.write(LiveEdit {
                        id,
                        edit: LiveEditKind::Move(t),
                    });
                    drag.last_sent = Some(t);
                }
            }
        }
    }
    if !buttons.primary_held {
        drag.active = None;
    }
}

pub(crate) fn draw(
    mode: Res<ToolMode>,
    drag: Res<GizmoDrag>,
    staged: Res<StagedSpawns>,
    live: Query<(&Selectable, &GlobalTransform)>,
    cameras: Query<(&Camera, &Transform), With<ToolCamera>>,
    mut gizmos: Gizmos<ToolGizmos>,
) {
    if !mode.active || mode.tool != crate::tools::SELECT {
        return;
    }
    let (Some(selection), Some((camera, cam))) = (mode.selection, cameras.iter().next()) else {
        return;
    };
    let Some(t) = selection_transform(selection, &staged, &live) else {
        return;
    };
    let origin = t.translation;
    let size = world_size(camera, &GlobalTransform::from(*cam), origin, HANDLE_PIXELS);
    let lit = drag.active.map(|a| a.handle).or(drag.hover);
    let colour = |h: Handle, base: Color| {
        if lit == Some(h) {
            Color::srgb(1.0, 0.95, 0.2)
        } else {
            base
        }
    };
    let bases = [
        Color::srgb(0.95, 0.2, 0.2),
        Color::srgb(0.3, 0.95, 0.3),
        Color::srgb(0.3, 0.5, 1.0),
    ];
    if mode.gizmo.moves() {
        for (i, axis) in AXES.iter().enumerate() {
            gizmos
                .arrow(
                    origin,
                    origin + *axis * size,
                    colour(Handle::Axis(i), bases[i]),
                )
                .with_tip_length(size * 0.2);
        }
        let c = plane_corners(origin, size);
        let pc = colour(Handle::PlaneXZ, Color::srgb(0.9, 0.9, 0.9));
        for i in 0..4 {
            gizmos.line(c[i], c[(i + 1) % 4], pc);
        }
    }
    if mode.gizmo.rotates() {
        let radius = size * mode.gizmo.ring_scale();
        for (i, axis) in AXES.iter().enumerate() {
            let points: Vec<Vec3> = ring_points(origin, *axis, radius).collect();
            gizmos.linestrip(points, colour(Handle::Ring(i), bases[i]));
        }
        // Facing indicator.
        gizmos.line(
            origin,
            origin + t.rotation * Vec3::NEG_Z * radius * 1.15,
            Color::WHITE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ray(origin: Vec3, toward: Vec3) -> Ray3d {
        Ray3d::new(origin, Dir3::new(toward - origin).unwrap())
    }

    #[test]
    fn axis_param_finds_the_closest_point_on_the_axis() {
        // Looking straight down at x = 3 from above.
        let r = ray(Vec3::new(3.0, 10.0, 1.0), Vec3::new(3.0, 0.0, 1.0));
        let t = axis_param(Vec3::ZERO, Vec3::X, r).unwrap();
        assert!((t - 3.0).abs() < 1e-4);
        // Parallel ray has no answer.
        let parallel = ray(Vec3::new(0.0, 1.0, 0.0), Vec3::new(5.0, 1.0, 0.0));
        assert!(axis_param(Vec3::ZERO, Vec3::X, parallel).is_none());
    }

    #[test]
    fn ring_angle_measures_around_the_axis() {
        let center = Vec3::new(1.0, 0.0, 1.0);
        let (u, w) = ring_basis(Vec3::Y);
        let above = |p: Vec3| ray(p + Vec3::Y * 5.0, p);
        let a0 = ring_angle(center, Vec3::Y, above(center + u)).unwrap();
        let a1 = ring_angle(center, Vec3::Y, above(center + w)).unwrap();
        assert!(a0.abs() < 1e-4);
        assert!((a1 - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
    }

    #[test]
    fn axis_drag_moves_by_the_snapped_cursor_delta() {
        let start = Transform::from_xyz(2.0, 0.0, 0.0);
        let r0 = ray(Vec3::new(2.4, 10.0, 0.0), Vec3::new(2.4, 0.0, 0.0));
        let active = Active {
            handle: Handle::Axis(0),
            target: Selection::Staged(1),
            start,
            grab: grab(Handle::Axis(0), start.translation, r0).unwrap(),
        };
        let r1 = ray(Vec3::new(3.7, 10.0, 0.0), Vec3::new(3.7, 0.0, 0.0));
        let free = dragged(&active, r1, 0.0, 0.0).unwrap();
        assert!((free.translation.x - 3.3).abs() < 1e-4);
        let snapped = dragged(&active, r1, 0.5, 0.0).unwrap();
        assert!((snapped.translation.x - 3.5).abs() < 1e-4);
    }

    #[test]
    fn ring_drag_rotates_by_snapped_degrees() {
        let start = Transform::IDENTITY;
        let (u, w) = ring_basis(Vec3::Y);
        let above = |p: Vec3| ray(p + Vec3::Y * 5.0, p);
        let active = Active {
            handle: Handle::Ring(1),
            target: Selection::Staged(1),
            start,
            grab: grab(Handle::Ring(1), Vec3::ZERO, above(u)).unwrap(),
        };
        // 50 degrees snaps to 45 at a 15 degree step.
        let a = 50f32.to_radians();
        let t = dragged(&active, above(u * a.cos() + w * a.sin()), 0.0, 15.0).unwrap();
        let (axis, angle) = t.rotation.to_axis_angle();
        assert!((angle.to_degrees() - 45.0).abs() < 1e-3);
        assert!(axis.abs().distance(Vec3::Y) < 1e-4);
    }

    #[test]
    fn snap_rounds_to_steps() {
        assert_eq!(snap(1.26, 0.25), 1.25);
        assert_eq!(snap(1.26, 0.0), 1.26);
    }
}
