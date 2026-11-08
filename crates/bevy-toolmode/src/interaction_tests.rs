use super::*;
use bevy::{
    camera::{
        primitives::Aabb,
        visibility::{InheritedVisibility, ViewVisibility},
        ComputedCameraValues, RenderTargetInfo,
    },
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
    time::TimeUpdateStrategy,
    window::{PrimaryWindow, WindowResolution},
};
use std::time::Duration;

const SIZE: Vec2 = Vec2::new(1280.0, 720.0);

#[derive(Resource, Default)]
struct Collected {
    spawns: Vec<SpawnRequest>,
    edits: Vec<LiveEdit>,
}

fn collect(
    mut spawns: MessageReader<SpawnRequest>,
    mut edits: MessageReader<LiveEdit>,
    mut out: ResMut<Collected>,
) {
    out.spawns.extend(spawns.read().cloned());
    out.edits.extend(edits.read().cloned());
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )))
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<AccumulatedMouseMotion>()
        .init_resource::<AccumulatedMouseScroll>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<ToolMode>()
        .init_resource::<ToolModeSettings>()
        .init_resource::<ToolRegistry>()
        .init_resource::<SpawnCatalogue>()
        .init_resource::<StagedSpawns>()
        .init_resource::<FreeCam>()
        .init_resource::<ToolRay>()
        .init_resource::<Reticle>()
        .init_resource::<ToolButtons>()
        .init_resource::<gizmo::GizmoDrag>()
        .init_resource::<Inspector>()
        .init_resource::<Collected>()
        .add_message::<SpawnRequest>()
        .add_message::<LiveEdit>()
        .add_message::<ToolAction>()
        .add_systems(
            Update,
            (
                buttons,
                toggles,
                freecam::drive,
                reticle::cast,
                gizmo::interact,
                tools::use_tools,
                inspector::pick,
                collect,
            )
                .chain(),
        );
    app.world_mut().spawn((
        Window {
            resolution: WindowResolution::new(SIZE.x as u32, SIZE.y as u32),
            ..default()
        },
        PrimaryWindow,
    ));
    // A camera 10 up and 10 back looking at the origin.
    let mut camera = Camera::default();
    camera.computed = ComputedCameraValues {
        clip_from_view: Mat4::perspective_infinite_reverse_rh(0.8, SIZE.x / SIZE.y, 0.1),
        target_info: Some(RenderTargetInfo {
            physical_size: SIZE.as_uvec2(),
            scale_factor: 1.0,
        }),
        ..default()
    };
    app.world_mut().spawn((
        camera,
        ToolCamera,
        Transform::from_xyz(0.0, 10.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    app.update();
    app
}

fn view(app: &mut App) -> (Camera, GlobalTransform) {
    let mut q = app
        .world_mut()
        .query_filtered::<(&Camera, &Transform), With<ToolCamera>>();
    let (c, t) = q.single(app.world()).unwrap();
    (c.clone(), GlobalTransform::from(*t))
}

fn screen(app: &mut App, p: Vec3) -> Vec2 {
    let (c, g) = view(app);
    c.world_to_viewport(&g, p).unwrap()
}

fn cursor(app: &mut App, at: Vec2) {
    let mut q = app.world_mut().query::<&mut Window>();
    q.single_mut(app.world_mut())
        .unwrap()
        .set_cursor_position(Some(at));
}

/// One frame; edge states of buttons and keys clear afterwards.
fn frame(app: &mut App) {
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
}

fn mouse(app: &mut App) -> Mut<'_, ButtonInput<MouseButton>> {
    app.world_mut().resource_mut::<ButtonInput<MouseButton>>()
}

fn mode(app: &mut App) -> Mut<'_, ToolMode> {
    app.world_mut().resource_mut::<ToolMode>()
}

fn goblin_catalogue(app: &mut App) {
    app.world_mut()
        .resource_mut::<SpawnCatalogue>()
        .replace(vec![Spawnable {
            id: "goblin".into(),
            label: "Goblin".into(),
            category: vec!["Characters".into()],
            properties: vec![PropertyDef {
                key: "order".into(),
                label: "Order".into(),
                kind: PropertyKind::Choice(vec!["attack".into(), "idle".into()]),
                default: PropertyValue::Text("attack".into()),
            }],
            ..default()
        }]);
}

#[test]
fn spawn_tool_stages_at_the_reticle_and_spawns_when_not_staging() {
    let mut app = app();
    goblin_catalogue(&mut app);
    {
        let mut m = mode(&mut app);
        m.active = true;
        m.tool = tools::SPAWN.into();
        m.stage_spawns = true;
        m.spawnable = Some("goblin".into());
    }
    let target = Vec3::new(2.0, 0.0, -1.0);
    let at = screen(&mut app, target);
    cursor(&mut app, at);
    mouse(&mut app).press(MouseButton::Left);
    frame(&mut app);
    mouse(&mut app).release(MouseButton::Left);
    frame(&mut app);
    let staged = &app.world().resource::<StagedSpawns>().items;
    assert_eq!(staged.len(), 1);
    assert!(staged[0].transform.translation.distance(target) < 0.01);
    assert!(app.world().resource::<Collected>().spawns.is_empty());

    mode(&mut app).stage_spawns = false;
    mode(&mut app)
        .spawn_properties
        .insert("order".into(), PropertyValue::Text("idle".into()));
    mouse(&mut app).press(MouseButton::Left);
    frame(&mut app);
    let spawns = &app.world().resource::<Collected>().spawns;
    assert_eq!(spawns.len(), 1);
    assert_eq!(spawns[0].spawnable, "goblin");
    assert_eq!(spawns[0].properties["order"].as_str(), Some("idle"));
    assert!(spawns[0].transform.translation.distance(target) < 0.01);
}

#[test]
fn dragging_a_gizmo_axis_moves_a_staged_entry_along_it() {
    let mut app = app();
    goblin_catalogue(&mut app);
    let id = app.world_mut().resource_mut::<StagedSpawns>().add(
        "goblin".into(),
        Transform::IDENTITY,
        PropertyValues::new(),
    );
    {
        let mut m = mode(&mut app);
        m.active = true;
        m.tool = tools::SELECT.into();
        m.selection = Some(Selection::Staged(id));
    }
    let (c, g) = view(&mut app);
    let size = gizmo::world_size(&c, &g, Vec3::ZERO, 90.0);
    let grab = screen(&mut app, Vec3::X * size * 0.7);
    let drop = screen(&mut app, Vec3::X * (size * 0.7 + 2.0));
    cursor(&mut app, grab);
    frame(&mut app);
    assert_eq!(
        app.world().resource::<gizmo::GizmoDrag>().hover,
        Some(gizmo::Handle::Axis(0))
    );
    mouse(&mut app).press(MouseButton::Left);
    frame(&mut app);
    cursor(&mut app, drop);
    frame(&mut app);
    mouse(&mut app).release(MouseButton::Left);
    frame(&mut app);
    let t = app
        .world()
        .resource::<StagedSpawns>()
        .get(id)
        .unwrap()
        .transform;
    assert!((t.translation.x - 2.0).abs() < 0.05, "{:?}", t.translation);
    assert!(t.translation.y.abs() < 1e-4 && t.translation.z.abs() < 1e-4);
    // The click went to the gizmo, not to re-selecting.
    assert_eq!(mode(&mut app).selection, Some(Selection::Staged(id)));
    assert!(!app.world().resource::<gizmo::GizmoDrag>().dragging());
}

#[test]
fn dragging_a_live_selection_sends_move_edits() {
    let mut app = app();
    let at = Vec3::new(3.0, 0.0, 0.0);
    app.world_mut().spawn((
        Selectable {
            id: 7,
            label: "npc".into(),
            radius: 0.9,
            center_height: 1.0,
        },
        GlobalTransform::from_translation(at),
    ));
    {
        let mut m = mode(&mut app);
        m.active = true;
        m.tool = tools::SELECT.into();
        m.selection = Some(Selection::Live(7));
    }
    let (c, g) = view(&mut app);
    let size = gizmo::world_size(&c, &g, at, 90.0);
    let grab = screen(&mut app, at + Vec3::Z * size * 0.7);
    let drop = screen(&mut app, at + Vec3::Z * (size * 0.7 + 1.5));
    cursor(&mut app, grab);
    mouse(&mut app).press(MouseButton::Left);
    frame(&mut app);
    cursor(&mut app, drop);
    frame(&mut app);
    mouse(&mut app).release(MouseButton::Left);
    frame(&mut app);
    let edits = &app.world().resource::<Collected>().edits;
    let Some(LiveEdit {
        id: 7,
        edit: LiveEditKind::Move(t),
    }) = edits.last()
    else {
        panic!("no move edit: {edits:?}");
    };
    assert!((t.translation.z - 1.5).abs() < 0.05, "{:?}", t.translation);
    assert!((t.translation.x - 3.0).abs() < 1e-4);
}

#[test]
fn freecam_looks_and_flies_while_right_mouse_is_held_and_clicks_still_reach_tools() {
    let mut app = app();
    goblin_catalogue(&mut app);
    {
        let mut m = mode(&mut app);
        m.active = true;
        m.freecam = true;
        m.tool = tools::SPAWN.into();
        m.spawnable = Some("goblin".into());
    }
    frame(&mut app);
    let start = app.world().resource::<FreeCam>().clone();
    assert!(start.initialised && start.yaw.abs() < 1e-4);
    // Hold right mouse and move: look, and W flies instead of picking slot 2.
    mouse(&mut app).press(MouseButton::Right);
    app.world_mut()
        .resource_mut::<AccumulatedMouseMotion>()
        .delta = Vec2::new(100.0, 0.0);
    for _ in 0..3 {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        frame(&mut app);
    }
    app.world_mut()
        .resource_mut::<AccumulatedMouseMotion>()
        .delta = Vec2::ZERO;
    mouse(&mut app).release(MouseButton::Right);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyW);
    frame(&mut app);
    let cam = app.world().resource::<FreeCam>().clone();
    assert!(
        (cam.yaw - (start.yaw - 0.9)).abs() < 1e-3,
        "yaw {}",
        cam.yaw
    );
    assert!(cam.position.distance(start.position) > 0.1, "did not fly");
    assert_eq!(
        mode(&mut app).tool,
        tools::SPAWN,
        "W switched tools while flying"
    );
    assert_eq!(
        mode(&mut app).spawn_yaw,
        0.0,
        "a look drag counted as a click"
    );
    // A short right click without travel is the tool's secondary action.
    mouse(&mut app).press(MouseButton::Right);
    frame(&mut app);
    mouse(&mut app).release(MouseButton::Right);
    frame(&mut app);
    assert!((mode(&mut app).spawn_yaw - std::f32::consts::FRAC_PI_4).abs() < 1e-5);
}

#[test]
fn inspector_prefers_selectables_then_mesh_triangles_over_large_bounds() {
    let mut app = app();
    mode(&mut app).active = true;
    mode(&mut app).tool = tools::INSPECT.into();
    // A level slab whose bounds cover everything, as a named glTF node with
    // an unnamed primitive.
    let slab = Mesh::from(Cuboid::new(40.0, 0.2, 40.0));
    let aabb = Aabb::from_min_max(Vec3::new(-20.0, -0.1, -20.0), Vec3::new(20.0, 0.1, 20.0));
    let handle = app.world_mut().resource_mut::<Assets<Mesh>>().add(slab);
    let node = app
        .world_mut()
        .spawn((Name::new("level node"), GlobalTransform::IDENTITY))
        .id();
    app.world_mut().spawn((
        Mesh3d(handle),
        aabb,
        GlobalTransform::from_translation(Vec3::Y * -0.1),
        InheritedVisibility::VISIBLE,
        ViewVisibility::VISIBLE,
        ChildOf(node),
    ));
    let npc = app
        .world_mut()
        .spawn((
            Selectable {
                id: 1,
                label: "npc".into(),
                radius: 0.9,
                center_height: 1.0,
            },
            GlobalTransform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
        ))
        .id();
    let at = screen(&mut app, Vec3::new(1.0, 1.0, 0.0));
    cursor(&mut app, at);
    mouse(&mut app).press(MouseButton::Left);
    frame(&mut app);
    assert_eq!(app.world().resource::<Inspector>().entity, Some(npc));
    assert!(app.world().resource::<Inspector>().open);
    mouse(&mut app).release(MouseButton::Left);
    frame(&mut app);
    // Off the character: the slab's named node.
    let at = screen(&mut app, Vec3::new(-4.0, 0.0, 2.0));
    cursor(&mut app, at);
    mouse(&mut app).press(MouseButton::Left);
    frame(&mut app);
    assert_eq!(app.world().resource::<Inspector>().entity, Some(node));
}

#[test]
fn the_default_gizmo_rotates_with_its_ring_and_moves_with_its_arrows() {
    let mut app = app();
    goblin_catalogue(&mut app);
    let id = app.world_mut().resource_mut::<StagedSpawns>().add(
        "goblin".into(),
        Transform::IDENTITY,
        PropertyValues::new(),
    );
    {
        let mut m = mode(&mut app);
        m.active = true;
        m.tool = tools::SELECT.into();
        m.selection = Some(Selection::Staged(id));
        m.angle_snap = 0.0;
    }
    assert_eq!(mode(&mut app).gizmo, GizmoMode::Universal);
    let (c, g) = view(&mut app);
    let radius = gizmo::world_size(&c, &g, Vec3::ZERO, 90.0) * 1.35;
    // Grab the Y ring (in the XZ plane) at +X and drag a quarter turn round.
    let at = |deg: f32| {
        let a = deg.to_radians();
        Vec3::new(a.cos(), 0.0, -a.sin()) * radius
    };
    let grab = screen(&mut app, at(0.0));
    cursor(&mut app, grab);
    frame(&mut app);
    assert_eq!(
        app.world().resource::<gizmo::GizmoDrag>().hover,
        Some(gizmo::Handle::Ring(1))
    );
    mouse(&mut app).press(MouseButton::Left);
    frame(&mut app);
    for deg in [30.0, 60.0, 90.0] {
        let p = screen(&mut app, at(deg));
        cursor(&mut app, p);
        frame(&mut app);
    }
    mouse(&mut app).release(MouseButton::Left);
    frame(&mut app);
    let t = app
        .world()
        .resource::<StagedSpawns>()
        .get(id)
        .unwrap()
        .transform;
    let (axis, angle) = t.rotation.to_axis_angle();
    assert!(
        (angle.to_degrees() - 90.0).abs() < 1.0,
        "angle {}",
        angle.to_degrees()
    );
    assert!(axis.abs().distance(Vec3::Y) < 1e-3, "axis {axis:?}");
    assert!(t.translation.length() < 1e-4, "rotation moved it");
}
