//! Bevy input producer for the recovered keyboard/mouse message-box branch.
use bevy::{
    input::{
        keyboard::{Key, KeyboardInput},
        mouse::MouseWheel,
        ButtonState, InputSystems,
    },
    prelude::*,
    window::PrimaryWindow,
};
use magicka_runtime::message_box_device::BoxDeviceSample;

#[derive(Resource, Default)]
pub struct BoxDevices {
    pub key_downs: Vec<i32>,
    pub sample: BoxDeviceSample,
}
pub struct BoxDevicesPlugin;
impl Plugin for BoxDevicesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BoxDevices>()
            .add_systems(PreUpdate, collect.after(InputSystems));
    }
}
fn collect(
    mut devices: ResMut<BoxDevices>,
    mut keyboard: MessageReader<KeyboardInput>,
    mut wheel: MessageReader<MouseWheel>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
) {
    devices.key_downs.clear();
    let Some((entity, window)) = windows.iter().next() else {
        keyboard.clear();
        wheel.clear();
        devices.sample.focused = false;
        return;
    };
    for event in keyboard.read() {
        if event.window != entity || event.state != ButtonState::Pressed {
            continue;
        }
        let vk = match event.logical_key {
            Key::Enter => 13,
            Key::ArrowUp => 38,
            Key::ArrowDown => 40,
            Key::ArrowLeft => 37,
            Key::ArrowRight => 39,
            _ => continue,
        };
        devices.key_downs.push(vk); // Do not discard OS auto-repeat.
    }
    let vertical_delta: f64 = wheel
        .read()
        .filter(|event| event.window == entity)
        .map(|event| f64::from(event.y))
        .sum();
    if let Some(point) = window.physical_cursor_position() {
        devices.sample.point = [point.x.floor() as i32, point.y.floor() as i32];
    }
    devices.sample.focused = window.focused;
    devices.sample.escape_down = keys.pressed(KeyCode::Escape);
    devices.sample.left_down = buttons.pressed(MouseButton::Left);
    devices.sample.wheel_changed = vertical_delta != 0.;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        input::{mouse::MouseScrollUnit, InputPlugin},
        math::DVec2,
    };
    fn app() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin, BoxDevicesPlugin));
        let mut window = Window {
            focused: true,
            ..default()
        };
        window.set_physical_cursor_position(Some(DVec2::new(400., 364.)));
        let window = app.world_mut().spawn((window, PrimaryWindow)).id();
        (app, window)
    }
    #[test]
    fn logical_key_messages_preserve_repeat_and_ignore_other_windows_and_releases() {
        let (mut app, window) = app();
        for (repeat, state, entity) in [
            (false, ButtonState::Pressed, window),
            (true, ButtonState::Pressed, window),
            (false, ButtonState::Released, window),
            (false, ButtonState::Pressed, Entity::PLACEHOLDER),
        ] {
            app.world_mut().write_message(KeyboardInput {
                key_code: KeyCode::Enter,
                logical_key: Key::Enter,
                state,
                text: None,
                repeat,
                window: entity,
            });
        }
        app.update();
        let devices = app.world().resource::<BoxDevices>();
        assert_eq!(devices.key_downs, [13, 13]);
        assert_eq!(devices.sample.point, [400, 364]);
        assert!(devices.sample.focused);
        app.update();
        assert!(app.world().resource::<BoxDevices>().key_downs.is_empty());
    }
    #[test]
    fn vertical_wheel_deltas_cancel_per_poll_and_horizontal_wheel_is_ignored() {
        let (mut app, window) = app();
        for y in [1., -1.] {
            app.world_mut().write_message(MouseWheel {
                unit: MouseScrollUnit::Line,
                phase: bevy::input::touch::TouchPhase::Moved,
                x: 10.,
                y,
                window,
            });
        }
        app.update();
        assert!(!app.world().resource::<BoxDevices>().sample.wheel_changed);
        app.world_mut().write_message(MouseWheel {
            unit: MouseScrollUnit::Line,
            phase: bevy::input::touch::TouchPhase::Moved,
            x: 0.,
            y: 1.,
            window,
        });
        app.update();
        assert!(app.world().resource::<BoxDevices>().sample.wheel_changed);
        app.update();
        assert!(!app.world().resource::<BoxDevices>().sample.wheel_changed);
    }
}
