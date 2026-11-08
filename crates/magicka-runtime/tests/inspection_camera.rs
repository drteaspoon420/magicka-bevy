use magicka_runtime::camera::{CameraView, MagickCamera};

#[test]
fn inspection_view_drives_projection_and_mouse_pick_then_resets() {
    let mut camera = MagickCamera::new(None);
    let native = (
        camera.position(),
        camera.direction(),
        camera.up(),
        camera.fov(),
    );
    let original_view = camera.view();
    let original_projection = camera.projection(1.5);
    camera.set_inspection_view(Some(CameraView {
        position: [10.0, 200.0, 20.0],
        direction: [0.0, -1.0, 0.0],
        up: [0.0, 0.0, -1.0],
        fov: 0.3,
    }));
    assert_ne!(camera.view(), original_view);
    assert_ne!(camera.projection(1.5), original_projection);
    let hit = camera.mouse_pick([0.0, 0.0], 1.5, 0.0);
    for (value, expected) in hit.into_iter().zip([10.0, 0.0, 20.0]) {
        assert!((value - expected).abs() < 0.001, "{hit:?}");
    }
    camera.set_inspection_view(None);
    assert_eq!(
        (
            camera.position(),
            camera.direction(),
            camera.up(),
            camera.fov()
        ),
        native
    );
    assert_eq!(camera.view(), original_view);
    assert_eq!(camera.projection(1.5), original_projection);
}

#[test]
fn inspection_view_does_not_change_native_camera_update() {
    let mut native = MagickCamera::new(None);
    let mut inspected = native.clone();
    inspected.set_inspection_view(Some(CameraView {
        position: [1000.0, 2000.0, 3000.0],
        direction: [0.0, -1.0, 0.0],
        up: [0.0, 0.0, -1.0],
        fov: 0.3,
    }));
    for camera in [&mut native, &mut inspected] {
        camera.move_to([15.0, 2.0, -30.0], 1.0);
        for _ in 0..60 {
            camera.update(&[], &[], 1.0 / 60.0);
        }
    }
    inspected.set_inspection_view(None);
    assert_eq!(inspected.ground_position(), native.ground_position());
    assert_eq!(inspected.view(), native.view());
    assert_eq!(inspected.fov(), native.fov());
}
