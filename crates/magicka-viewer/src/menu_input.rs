//! Frontend actions shared by menus and cinematics. Physical bindings belong
//! here; page code never interprets branded A/B/Cross/Circle button names.
use bevy::{input::InputSystems, prelude::*};
use std::collections::HashMap;

pub(crate) struct MenuInputPlugin;
impl Plugin for MenuInputPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<crate::controller::ControllerPlugin>() {
            app.add_plugins(crate::controller::ControllerPlugin);
        }
        app.init_resource::<MenuInput>()
            .init_resource::<MenuBindings>()
            .add_systems(
                PreUpdate,
                collect
                    .after(InputSystems)
                    .after(crate::controller::ControllerCollect),
            );
    }
}

/// Indices match Up, Down, Left, Right throughout the frontend.
#[derive(Resource, Default)]
pub(crate) struct MenuInput {
    pub confirm: bool,
    pub back: bool,
    pub direction: [bool; 4],
    pub controller: Option<Entity>,
    /// Every pad's menu edges this frame, including seated ones (local
    /// co-op lobby/hot join read them per device).
    pub pads: HashMap<Entity, PadEdges>,
    /// Pads owned by extra local players; they never drive the shared menu.
    pub seated: std::collections::HashSet<Entity>,
}
/// One pad's menu edges for a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PadEdges {
    pub confirm: bool,
    pub back: bool,
    pub start: bool,
    pub select: bool,
    pub direction: [bool; 4],
}
#[derive(Clone)]
pub(crate) struct ControllerBindings {
    pub confirm: GamepadButton,
    pub back: GamepadButton,
    pub direction: [GamepadButton; 4],
    pub stick: [GamepadAxis; 2],
    pub stick_threshold: f32,
}
impl Default for ControllerBindings {
    fn default() -> Self {
        Self {
            confirm: GamepadButton::South,
            back: GamepadButton::East,
            direction: [
                GamepadButton::DPadUp,
                GamepadButton::DPadDown,
                GamepadButton::DPadLeft,
                GamepadButton::DPadRight,
            ],
            stick: [GamepadAxis::LeftStickX, GamepadAxis::LeftStickY],
            stick_threshold: 0.5,
        }
    }
}
/// Bevy's backend normalizes physical devices, including virtual Steam Input
/// pads. Per-device overrides allow confirm/back conventions to differ without
/// changing a menu or a cinematic. Glyph selection is a separate concern.
#[derive(Resource, Default)]
pub(crate) struct MenuBindings {
    pub default: ControllerBindings,
    pub devices: HashMap<Entity, ControllerBindings>,
}

fn collect(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<(Entity, &Gamepad)>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    bindings: Res<MenuBindings>,
    devices: Res<crate::controller::LocalDevices>,
    mut input: ResMut<MenuInput>,
    mut sticks: Local<HashMap<Entity, [bool; 4]>>,
) {
    if windows.iter().next().is_some_and(|w| !w.focused) {
        let seated = std::mem::take(&mut input.seated);
        *input = MenuInput {
            seated,
            ..default()
        };
        sticks.clear();
        return;
    }
    input.pads.clear();
    let seated = input.seated.clone();
    input.seated.retain(|entity| pads.get(*entity).is_ok());
    let keyboard = [
        KeyCode::ArrowUp,
        KeyCode::ArrowDown,
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
    ]
    .map(|key| keys.just_pressed(key));
    input.confirm = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space);
    input.back = keys.just_pressed(KeyCode::Escape);
    input.direction = keyboard;
    if input
        .controller
        .is_some_and(|entity| pads.get(entity).is_err())
    {
        input.controller = None;
    }
    sticks.retain(|entity, _| pads.get(*entity).is_ok());
    let keyboard_active = input.confirm || input.back || keyboard.into_iter().any(|value| value);
    if keyboard_active {
        input.controller = None;
    }
    let mut candidates = Vec::new();
    for (entity, pad) in &pads {
        let map = bindings.devices.get(&entity).unwrap_or(&bindings.default);
        let x = pad.get(map.stick[0]).unwrap_or(0.0);
        let y = pad.get(map.stick[1]).unwrap_or(0.0);
        let t = map.stick_threshold;
        let held = [y > t, y < -t, x < -t, x > t];
        let previous = sticks.entry(entity).or_default();
        let direction = std::array::from_fn(|i| {
            pad.just_pressed(map.direction[i]) || (held[i] && !previous[i])
        });
        *previous = held;
        let confirm = pad.just_pressed(map.confirm);
        let back = pad.just_pressed(map.back);
        input.pads.insert(
            entity,
            PadEdges {
                confirm,
                back,
                start: pad.just_pressed(GamepadButton::Start),
                select: pad.just_pressed(GamepadButton::Select),
                direction,
            },
        );
        if seated.contains(&entity)
            || devices.occupied.contains(&entity)
            || (devices.locked && devices.primary != Some(entity))
        {
            continue;
        }
        if confirm || back || direction.into_iter().any(|value| value) {
            candidates.push((entity, confirm, back, direction));
        }
    }
    // One device owns an action frame. Do not combine one player's direction
    // with another player's confirm. Keep the active controller on ties.
    candidates.sort_by_key(|(entity, ..)| (Some(*entity) != input.controller, entity.to_bits()));
    if !keyboard_active {
        if let Some((entity, confirm, back, direction)) = candidates.first() {
            input.controller = Some(*entity);
            input.confirm = *confirm;
            input.back = *back;
            input.direction = *direction;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .add_plugins(MenuInputPlugin);
        app
    }

    #[test]
    fn controllers_keep_separate_bindings_and_never_combine_actions() {
        let mut app = app();
        let mut pad = Gamepad::default();
        pad.digital_mut().press(GamepadButton::East);
        let first = app.world_mut().spawn(pad).id();
        let mut pad = Gamepad::default();
        pad.digital_mut().press(GamepadButton::DPadDown);
        let second = app.world_mut().spawn(pad).id();
        app.world_mut()
            .resource_mut::<MenuBindings>()
            .devices
            .insert(
                first,
                ControllerBindings {
                    confirm: GamepadButton::East,
                    back: GamepadButton::South,
                    ..default()
                },
            );
        app.world_mut().resource_mut::<MenuInput>().controller = Some(first);
        app.update();
        let input = app.world().resource::<MenuInput>();
        assert!(input.confirm);
        assert!(!input.back);
        assert_eq!(input.direction, [false; 4]);
        assert_eq!(input.controller, Some(first));
        app.world_mut().despawn(first);
        app.update();
        let input = app.world().resource::<MenuInput>();
        assert_eq!(input.controller, Some(second));
        assert_eq!(input.direction, [false, true, false, false]);
        assert!(!input.confirm);
    }

    #[test]
    fn stick_motion_produces_one_navigation_edge_until_released() {
        let mut app = app();
        let mut pad = Gamepad::default();
        pad.analog_mut().set(GamepadAxis::LeftStickX, 0.8);
        let entity = app.world_mut().spawn(pad).id();
        app.update();
        assert_eq!(
            app.world().resource::<MenuInput>().direction,
            [false, false, false, true]
        );
        app.update();
        assert_eq!(app.world().resource::<MenuInput>().direction, [false; 4]);
        app.world_mut()
            .get_mut::<Gamepad>(entity)
            .unwrap()
            .analog_mut()
            .set(GamepadAxis::LeftStickX, 0.0);
        app.update();
        app.world_mut()
            .get_mut::<Gamepad>(entity)
            .unwrap()
            .analog_mut()
            .set(GamepadAxis::LeftStickX, -0.8);
        app.update();
        assert_eq!(
            app.world().resource::<MenuInput>().direction,
            [false, false, true, false]
        );
    }
}
