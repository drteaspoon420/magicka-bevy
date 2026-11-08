//! Rewrite controller adapter. Uses Bevy/gilrs normalized physical positions.
//! Steam Input gamepad emulation works without a Steamworks dependency.
use bevy::{input::InputSystems, prelude::*};
use magicka_runtime::{avatar::PlayerInput, play_session::PlaySession};
use std::collections::HashMap;

#[path = "controller_audit.rs"]
mod audit;
#[path = "controller_font.rs"]
mod font;
#[path = "controller_log.rs"]
mod input_log;
#[path = "controller_settings.rs"]
mod settings;
pub(crate) use settings::ElementStyle;
#[path = "controller_glyphs.rs"]
mod glyphs;
#[path = "controller_native.rs"]
pub(crate) mod native;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Family {
    #[default]
    Xbox,
    Ps5,
    Deck,
    Steam,
}
impl Family {
    pub const ALL: [Self; 4] = [Self::Xbox, Self::Ps5, Self::Deck, Self::Steam];
    pub fn label(self) -> &'static str {
        match self {
            Self::Xbox => "Xbox",
            Self::Ps5 => "PS5",
            Self::Deck => "Steam Deck",
            Self::Steam => "Steam Controller",
        }
    }
    pub fn face_names(self) -> [&'static str; 4] {
        if self == Self::Ps5 {
            ["Cross", "Circle", "Square", "Triangle"]
        } else {
            ["A", "B", "X", "Y"]
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.label() == s)
    }
    pub(crate) fn detect(name: &str, vendor: Option<u16>) -> Self {
        let name = name.to_lowercase();
        if name.contains("steam deck") || name.contains("neptune") {
            Self::Deck
        } else if name.contains("steam controller") {
            Self::Steam
        } else if vendor == Some(0x054c)
            || name.contains("dualsense")
            || name.contains("playstation")
            || name.contains("ps5")
        {
            Self::Ps5
        } else {
            Self::Xbox
        }
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ControllerPrompts;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ControllerCollect;

pub(crate) struct ControllerPlugin;
impl Plugin for ControllerPlugin {
    fn build(&self, app: &mut App) {
        audit::install(app);
        input_log::install(app);
        app.insert_resource(settings::Preferences::for_args(
            &std::env::args().collect::<Vec<_>>(),
        ));
        app.init_resource::<font::FontPrompts>()
            .add_systems(
                PostUpdate,
                font::update.after(crate::native_text::NativeTextSet::Mesh),
            )
            .init_resource::<NativePrompts>()
            .init_resource::<ControllerInput>()
            .init_resource::<LocalDevices>()
            .add_systems(
                PreUpdate,
                collect.after(InputSystems).in_set(ControllerCollect),
            )
            .add_systems(
                Update,
                (prompts.in_set(ControllerPrompts), cursor_visibility),
            );
    }
}

/// Session/lobby device ownership survives keyboard activity and neutral sticks.
#[derive(Resource, Default)]
pub(crate) struct LocalDevices {
    pub locked: bool,
    pub primary: Option<Entity>,
    pub awaiting_primary: bool,
    pub occupied: std::collections::HashSet<Entity>,
}
impl LocalDevices {
    /// Restore the chapter's owner after the shared menu temporarily uses input.
    pub(crate) fn restore_primary(&mut self, selection: &str, pads: impl Iterator<Item = Entity>) {
        self.primary = selection
            .parse::<u64>()
            .ok()
            .and_then(|bits| pads.into_iter().find(|p| p.to_bits() == bits));
        self.awaiting_primary = selection != "keyboard" && self.primary.is_none();
    }
}

#[derive(Resource, Default)]
pub(crate) struct ControllerInput {
    pub element_style: ElementStyle,
    style_changed: bool,
    pub in_game: bool,
    pub in_session: bool,
    pub show_legend: bool,
    pub hud_visible: bool,
    pub screen: [f32; 2],
    pub interaction: Option<&'static str>,
    pub inventory_open: bool,
    pub element_enabled: [bool; 8],
    pub player_slot: usize,
    pub active: bool,
    pub release_movement: bool,
    /// Player one is locked to a pad: keyboard and mouse never steer that
    /// player (each local player keeps one device in play).
    pub exclusive: bool,
    pub entity: Option<Entity>,
    pub family: Family,
    pub confirm: bool,
    pub dismiss_hint: bool,
    pub interact_pressed: bool,
    pub back: bool,
    pub pause: bool,
    pub direction: [bool; 4],
    movement: Vec2,
    aim: Vec2,
    cast: bool,
    pub self_cast: bool,
    area: bool,
    attack: bool,
    pub interact: bool,
    block: bool,
    pub help: bool,
    pub wheel_fade: [f32; 4],
    pub wheel_move: [f32; 4],
    boost: bool,
    elements: [bool; 8],
    prev: bool,
    next: bool,
    inventory: bool,
    override_family: Option<Family>,
    override_changed: bool,
}

/// Radial dead zone; preserve analog speed and clamp diagonal magnitude.
fn stick(v: Vec2) -> Vec2 {
    if !v.is_finite() || v.length() < 0.25 {
        Vec2::ZERO
    } else {
        v.clamp_length_max(1.0)
    }
}
fn axis(pad: &Gamepad, x: GamepadAxis, y: GamepadAxis) -> Vec2 {
    stick(Vec2::new(
        pad.get(x).unwrap_or(0.0),
        pad.get(y).unwrap_or(0.0),
    ))
}
fn trigger(pad: &Gamepad, button: GamepadButton) -> bool {
    pad.pressed(button) || pad.get(button).unwrap_or(0.0) > 0.5
}
fn collect(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    motion: Option<Res<bevy::input::mouse::AccumulatedMouseMotion>>,
    pads: Query<(Entity, &Gamepad, Option<&Name>)>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut state: ResMut<ControllerInput>,
    mut preferences: ResMut<settings::Preferences>,
    mut previous_stick: Local<Vec2>,
    mut profiles: Local<HashMap<Entity, Family>>,
    mut gesture: Local<native::Gesture>,
    mut wheel: Local<native::Wheel>,
    mut wait_neutral: Local<bool>,
    time: Option<Res<Time>>,
    menu: Option<Res<crate::menu_input::MenuInput>>,
    mut devices: ResMut<LocalDevices>,
    #[cfg(feature = "main-menu")] mut frontend: Option<ResMut<crate::frontend::Frontend>>,
) {
    let mut style = preferences.style;
    #[cfg(feature = "main-menu")]
    if let Some(frontend) = frontend.as_ref() {
        style = frontend.controller_element_style();
    }
    if state.style_changed {
        *wait_neutral = true;
        style = state.element_style;
        #[cfg(feature = "main-menu")]
        if let Some(frontend) = frontend.as_mut() {
            frontend.set_controller_element_style(style);
        }
        if style != preferences.style {
            preferences.save(style);
        }
        *gesture = default();
        *wheel = default();
    }
    let previous_device = state.entity;
    let owner = if devices.locked {
        devices.primary
    } else {
        state.entity
    };
    let old_aim = state.aim;
    let was_active = state.active;
    profiles.retain(|entity, _| pads.get(*entity).is_ok());
    let mut request = state.override_changed.then_some(state.override_family);
    #[cfg(feature = "main-menu")]
    if let Some(frontend) = frontend.as_mut() {
        request = frontend.controller_prompt_request.take().or(request);
    }
    if let (Some(entity), Some(family)) = (owner, request) {
        if let Some(family) = family {
            profiles.insert(entity, family);
        } else {
            profiles.remove(&entity);
        }
    }
    let cycle = keys.as_ref().is_some_and(|k| k.just_pressed(KeyCode::F8));
    let unfocused = windows.iter().next().is_some_and(|w| !w.focused);
    let keyboard_active = keys
        .as_ref()
        .is_some_and(|k| k.get_just_pressed().any(|k| *k != KeyCode::F8))
        || mouse
            .as_ref()
            .is_some_and(|m| m.get_just_pressed().next().is_some())
        || motion
            .as_ref()
            .is_some_and(|m| m.delta.length_squared() > 4.0);
    if devices.locked && devices.primary.is_some_and(|e| pads.get(e).is_err()) {
        devices.primary = None;
        devices.awaiting_primary = true;
    }
    if devices.awaiting_primary && !unfocused {
        let replacement = pads
            .iter()
            .filter(|(e, p, _)| {
                !devices.occupied.contains(e) && p.just_pressed(GamepadButton::South)
            })
            .map(|(e, ..)| e)
            .min_by_key(|e| e.to_bits());
        if let Some(e) = replacement {
            devices.primary = Some(e);
            devices.awaiting_primary = false;
        }
    }
    // Select one device, keeping ownership while held/neutral. Never blend pads.
    // Pads seated as extra local players drive their own avatars.
    let mut candidates: Vec<_> = pads
        .iter()
        .filter(|(e, ..)| {
            (!devices.locked || devices.primary == Some(*e))
                && !devices.occupied.contains(e)
                && menu.as_ref().is_none_or(|m| !m.seated.contains(e))
        })
        .collect();
    candidates.sort_by_key(|(e, ..)| (Some(*e) != owner, e.to_bits()));
    let changed = |p: &Gamepad| {
        trigger(p, GamepadButton::LeftTrigger2)
            || trigger(p, GamepadButton::RightTrigger2)
            || p.get_just_pressed().next().is_some()
            || p.get_just_released().next().is_some()
            || axis(p, GamepadAxis::LeftStickX, GamepadAxis::LeftStickY) != Vec2::ZERO
            || axis(p, GamepadAxis::RightStickX, GamepadAxis::RightStickY) != Vec2::ZERO
    };
    let selected = candidates
        .iter()
        .find(|(_, p, _)| changed(p))
        .or_else(|| candidates.iter().find(|(e, ..)| Some(*e) == owner));
    *state = ControllerInput {
        element_style: style,
        exclusive: devices.locked && devices.primary.is_some(),
        ..default()
    };
    let Some((entity, pad, name)) = selected else {
        #[cfg(feature = "main-menu")]
        if let Some(frontend) = frontend.as_mut() {
            frontend.controller_device_present = false;
            frontend.controller_prompt_override = None;
        }
        state.release_movement = was_active;
        *previous_stick = Vec2::ZERO;
        *gesture = default();
        *wheel = default();
        return;
    };
    let detected = Family::detect(name.map_or("", |n| n.as_str()), pad.vendor_id());
    state.entity = Some(*entity);
    state.override_family = profiles.get(entity).copied();
    if cycle {
        state.cycle_prompts();
        if let Some(family) = state.override_family {
            profiles.insert(*entity, family);
        } else {
            profiles.remove(entity);
        }
        state.override_changed = false;
    }
    state.family = state.override_family.unwrap_or(detected);
    #[cfg(feature = "main-menu")]
    if let Some(frontend) = frontend.as_mut() {
        frontend.controller_device_present = true;
        frontend.controller_prompt_override = state.override_family;
    }
    // A locked pad keeps player one through keyboard and mouse activity.
    state.active = !unfocused
        && (state.exclusive || !keyboard_active)
        && (changed(pad) || (was_active && previous_device == Some(*entity)));
    if !state.active {
        state.release_movement = was_active;
        *previous_stick = Vec2::ZERO;
        *gesture = default();
        *wheel = default();
        return;
    }
    if previous_device != Some(*entity) {
        *gesture = default();
        *wheel = default();
        *previous_stick = Vec2::ZERO;
    }
    use GamepadButton as B;
    state.movement = axis(pad, GamepadAxis::LeftStickX, GamepadAxis::LeftStickY);
    let aim = state.movement;
    state.aim = if aim != Vec2::ZERO {
        aim.normalize()
    } else if old_aim != Vec2::ZERO && previous_device == Some(*entity) {
        old_aim
    } else {
        Vec2::ZERO
    };
    state.confirm = pad.just_pressed(B::South);
    state.dismiss_hint = [B::South, B::East, B::West, B::North]
        .into_iter()
        .any(|b| pad.just_pressed(b));
    state.interact_pressed = pad.just_pressed(B::West);
    state.back = pad.just_pressed(B::East);
    state.pause = pad.just_pressed(B::Start);
    state.cast = trigger(pad, B::RightTrigger2);
    state.area = trigger(pad, B::LeftTrigger2);
    state.self_cast = pad.pressed(B::North);
    state.attack = pad.pressed(B::South);
    state.interact = pad.pressed(B::West);
    state.boost = pad.pressed(B::East);
    state.block = pad.pressed(B::RightTrigger);
    state.help = pad.pressed(B::RightThumb);
    state.inventory = pad.just_pressed(B::Select);
    state.prev = pad.just_pressed(B::DPadUp);
    state.next = pad.just_pressed(B::DPadDown);
    let held = [
        state.movement.y > 0.5,
        state.movement.y < -0.5,
        state.movement.x < -0.5,
        state.movement.x > 0.5,
    ];
    let old = [
        previous_stick.y > 0.5,
        previous_stick.y < -0.5,
        previous_stick.x < -0.5,
        previous_stick.x > 0.5,
    ];
    let dpad = [B::DPadUp, B::DPadDown, B::DPadLeft, B::DPadRight];
    state.direction = std::array::from_fn(|i| pad.just_pressed(dpad[i]) || (held[i] && !old[i]));
    *previous_stick = state.movement;
    let right = Vec2::new(
        pad.get(GamepadAxis::RightStickX).unwrap_or(0.),
        pad.get(GamepadAxis::RightStickY).unwrap_or(0.),
    );
    if style == ElementStyle::RightStick {
        if let Some(element) = gesture.update(right, state.help) {
            state.elements[element] = true;
        }
        wheel.update(
            right,
            state.help,
            time.as_ref().map_or(1. / 60., |t| t.delta_secs()),
        );
        state.wheel_fade = wheel.fade;
        state.wheel_move = wheel.movement;
    } else {
        let browsing = pad.pressed(B::RightTrigger);
        state.prev = browsing && pad.just_pressed(B::DPadLeft);
        state.next = browsing && pad.just_pressed(B::DPadRight);
        state.block = browsing && !pad.pressed(B::DPadLeft) && !pad.pressed(B::DPadRight);
        if !browsing {
            let offset = if pad.pressed(B::LeftTrigger) { 4 } else { 0 };
            for (index, button) in dpad.into_iter().enumerate() {
                state.elements[offset + index] = pad.pressed(button);
            }
        }
        *gesture = default();
        *wheel = default();
    }
    if *wait_neutral {
        *gesture = default();
        state.elements = [false; 8];
        if right.length() < 0.3
            && !dpad.into_iter().any(|b| pad.pressed(b))
            && !pad.pressed(B::LeftTrigger)
        {
            *wait_neutral = false;
        }
    }
}
impl ControllerInput {
    pub fn cycle_element_style(&mut self) {
        self.element_style = self.element_style.other();
        self.style_changed = true;
    }
    pub fn cycle_prompts(&mut self) {
        self.override_changed = true;
        self.override_family = match self.override_family {
            None => Some(Family::Xbox),
            Some(Family::Xbox) => Some(Family::Ps5),
            Some(Family::Ps5) => Some(Family::Deck),
            Some(Family::Deck) => Some(Family::Steam),
            Some(Family::Steam) => None,
        };
        if let Some(family) = self.override_family {
            self.family = family;
        }
    }
    pub fn prompt_style_label(&self) -> &'static str {
        self.override_family.map_or("Automatic", Family::label)
    }
    pub fn update_context(
        &mut self,
        session: &PlaySession,
        screen: [f32; 2],
        paused: bool,
        controls: bool,
    ) {
        self.screen = screen;
        self.in_session = true;
        self.in_game = !paused;
        self.show_legend = false;
        let _ = controls;
        self.hud_visible = session.world.ui_enabled && !paused;
        self.element_enabled = session.keyboard_hud.icons.map(|i| i.enabled);
        self.inventory_open = session.inventory.active();
        let dialog = !session.world.tutorial.dialog_done()
            || session
                .world
                .dialogs
                .as_ref()
                .is_some_and(|d| d.can_advance(Some(0)));
        let (cursor, reachable) = session.pad_cursor();
        use magicka_runtime::play_session::Cursor;
        self.interaction = if session.world.flags.cutscene {
            Some("Skip cutscene")
        } else if dialog {
            Some("Continue")
        } else if reachable {
            match cursor {
                Cursor::Talk => Some("Talk"),
                Cursor::PickUp => Some("Pick up"),
                Cursor::Examine => Some("Examine"),
                Cursor::Interact => Some("Interact"),
                _ => None,
            }
        } else {
            None
        };
    }
    pub fn player_input(&self, session: &PlaySession, aspect: f32) -> PlayerInput {
        let camera = session.camera();
        let movement = Vec2::new(self.movement.x, -self.movement.y);
        let aim = if self.aim == Vec2::ZERO {
            let direction = session.body().direction();
            Vec2::new(direction[0], direction[2])
        } else {
            Vec2::new(self.aim.x, -self.aim.y)
        };
        let position = session.body().position;
        // Project a unit facing vector through the existing casting API.
        // Controller interaction uses native directional queries, not this point.
        let point = [position[0] + aim.x, position[1], position[2] + aim.y, 1.0];
        let vp = magicka_runtime::camera::multiply(&camera.view(), &camera.projection(aspect));
        let clip: [f32; 4] =
            std::array::from_fn(|i| (0..4).map(|j| point[j] * vp[j * 4 + i]).sum());
        let cursor_ndc = (clip[3].abs() > 1e-5).then(|| [clip[0] / clip[3], clip[1] / clip[3]]);
        PlayerInput {
            cursor_ndc,
            pad_movement: Some(movement.to_array()),
            move_down: self.interact,
            pad_attack: self.attack,
            pad_block: self.block,
            cast_down: self.cast || self.area,
            cast_self_down: self.self_cast,
            shift_down: self.area,
            boost_down: self.boost,
            elements: self.elements,
            magick_prev: self.prev,
            magick_next: self.next,
            inventory: self.inventory,
        }
    }
}

fn cursor_visibility(
    controller: Res<ControllerInput>,
    mut windows: Query<&mut bevy::window::CursorOptions, With<bevy::window::PrimaryWindow>>,
) {
    for mut cursor in &mut windows {
        cursor.visible = !controller.active;
    }
}

#[derive(Component)]
struct PromptRoot;
/// Glyph replacements supplied by the original UI's actual draw paths.
#[derive(Resource, Default)]
pub(crate) struct NativePrompts(pub Vec<NativePrompt>);
#[derive(Clone, Debug)]
pub(crate) struct NativePrompt {
    pub element: Option<usize>,
    pub glyph: usize,
    pub bounds: [f32; 4],
    pub alpha: f32,
    /// NotifierButton.ButtonIntensity: the key glyph's RGB multiplier
    /// (GUIBasicEffect.Color), 1 except while a boost is available.
    pub intensity: f32,
}
#[derive(Component, Debug)]
struct ElementPrompt(usize);
#[derive(Component, Debug)]
struct PromptIcon {
    family: Family,
    glyph: usize,
    controller: Option<Entity>,
    player_slot: usize,
}
#[derive(Resource)]
struct PromptImages([[Handle<Image>; 23]; 4]);
fn prompts(
    mut commands: Commands,
    state: Res<ControllerInput>,
    placements: Option<Res<NativePrompts>>,
    images: Option<ResMut<Assets<Image>>>,
    loaded: Option<Res<PromptImages>>,
    roots: Query<Entity, With<PromptRoot>>,
) {
    let Some(mut images) = images else { return };
    let Some(loaded) = loaded else {
        use bevy::image::{ImageSampler, ImageType};
        let handles = glyphs::PNGS.map(|row| {
            row.map(|png| {
                images.add(
                    Image::from_buffer(
                        png,
                        ImageType::Extension("png"),
                        bevy::image::CompressedImageFormats::NONE,
                        true,
                        ImageSampler::default(),
                        default(),
                    )
                    .expect("checked-in Kenney PNG"),
                )
            })
        });
        commands.insert_resource(PromptImages(handles));
        return;
    };
    for root in &roots {
        commands.entity(root).despawn();
    }
    if !state.active {
        return;
    }
    let family = Family::ALL.iter().position(|f| *f == state.family).unwrap();
    if !state.show_legend {
        if !state.in_session {
            return;
        }
        if let Some(placements) = placements {
            for prompt in &placements.0 {
                let [x, y, r, b] = prompt.bounds;
                let mut node = commands.spawn((
                    PromptRoot,
                    PromptIcon {
                        family: state.family,
                        glyph: prompt.glyph,
                        controller: state.entity,
                        player_slot: state.player_slot,
                    },
                    ImageNode {
                        color: Color::srgba(
                            prompt.intensity,
                            prompt.intensity,
                            prompt.intensity,
                            prompt.alpha,
                        ),
                        ..ImageNode::new(loaded.0[family][prompt.glyph].clone())
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(x),
                        top: px(y),
                        width: px(r - x),
                        height: px(b - y),
                        ..default()
                    },
                    GlobalZIndex(103),
                ));
                if let Some(element) = prompt.element {
                    node.insert(ElementPrompt(element));
                }
            }
        }
        return;
    }
    commands
        .spawn((
            PromptRoot,
            Node {
                position_type: PositionType::Absolute,
                top: if state.in_game { px(6) } else { Val::Auto },
                bottom: if state.in_game { Val::Auto } else { px(6) },
                left: px(8),
                right: px(8),
                flex_wrap: FlexWrap::Wrap,
                align_items: AlignItems::Center,
                column_gap: px(6),
                row_gap: px(2),
                padding: UiRect::all(px(4)),
                ..default()
            },
            GlobalZIndex(100),
            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.88)),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(format!("{}  ", state.family.label())),
                TextFont {
                    font_size: bevy::text::FontSize::Px(14.0),
                    ..default()
                },
            ));
            let game = [
                (10, "Move / Navigate"),
                (11, "Conjure elements"),
                (0, "Weapon / Confirm"),
                (1, "Boost / Magick / Back"),
                (2, "Interact"),
                (3, "Self cast / Staff"),
                (7, "Cast"),
                (6, "Area cast"),
                (12, "Up / Down: Magick"),
                (11, "Element gestures"),
                (5, "Block"),
                (9, "Inventory"),
                (8, "Pause"),
            ];
            // Only the explicit --controller-preview diagnostic reaches this legend.
            for &(glyph, label) in &game {
                parent
                    .spawn(Node {
                        flex_shrink: 0.0,
                        align_items: AlignItems::Center,
                        column_gap: px(3),
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn((
                            PromptIcon {
                                family: state.family,
                                glyph,
                                controller: state.entity,
                                player_slot: state.player_slot,
                            },
                            ImageNode::new(loaded.0[family][glyph].clone()),
                            Node {
                                width: Val::Auto,
                                height: px(26),
                                flex_shrink: 0.0,
                                ..default()
                            },
                        ));
                        row.spawn((
                            Text::new(label),
                            TextLayout::no_wrap(),
                            TextFont {
                                font_size: bevy::text::FontSize::Px(14.0),
                                ..default()
                            },
                        ));
                    });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(ControllerPlugin);
        app
    }
    #[test]
    fn restoring_primary_does_not_finish_another_pads_gesture_or_inherit_its_aim() {
        let mut app = app();
        let mut menu_pad = Gamepad::default();
        menu_pad.analog_mut().set(GamepadAxis::RightStickY, 1.);
        menu_pad.analog_mut().set(GamepadAxis::LeftStickY, 1.);
        app.world_mut().spawn(menu_pad);
        app.update();
        assert_eq!(app.world().resource::<ControllerInput>().aim, Vec2::Y);
        let mut playing_pad = Gamepad::default();
        playing_pad.analog_mut().set(GamepadAxis::RightStickX, 1.);
        let playing_pad = app.world_mut().spawn(playing_pad).id();
        {
            let mut devices = app.world_mut().resource_mut::<LocalDevices>();
            devices.locked = true;
            devices.restore_primary(
                &playing_pad.to_bits().to_string(),
                [playing_pad].into_iter(),
            );
        }
        app.update();
        let input = app.world().resource::<ControllerInput>();
        assert_eq!(input.entity, Some(playing_pad));
        assert_eq!(
            input.elements, [false; 8],
            "two pads cannot supply halves of one gesture"
        );
        assert_eq!(
            input.aim,
            Vec2::ZERO,
            "aim belongs to the actual previous device"
        );
        // The restored pad can still finish its own Right -> Up gesture.
        {
            let mut pad = app.world_mut().get_mut::<Gamepad>(playing_pad).unwrap();
            pad.analog_mut().set(GamepadAxis::RightStickX, 0.);
            pad.analog_mut().set(GamepadAxis::RightStickY, 1.);
        }
        app.update();
        assert!(app.world().resource::<ControllerInput>().elements[3]);
    }
    #[test]
    fn chapter_primary_survives_menu_input_and_missing_pad_can_be_reclaimed() {
        let mut world = World::new();
        let primary = world.spawn_empty().id();
        let other = world.spawn_empty().id();
        let mut devices = LocalDevices {
            primary: Some(other),
            ..default()
        };
        devices.restore_primary(&primary.to_bits().to_string(), [primary, other].into_iter());
        assert_eq!(devices.primary, Some(primary));
        assert!(!devices.awaiting_primary);
        devices.restore_primary(&primary.to_bits().to_string(), [other].into_iter());
        assert_eq!(devices.primary, None);
        assert!(devices.awaiting_primary);
        devices.restore_primary("keyboard", [other].into_iter());
        assert_eq!(devices.primary, None);
        assert!(!devices.awaiting_primary);
    }
    #[test]
    fn locked_primary_survives_other_pads_and_can_reclaim_after_disconnect() {
        let mut app = app();
        let primary = app.world_mut().spawn(Gamepad::default()).id();
        {
            let mut devices = app.world_mut().resource_mut::<LocalDevices>();
            devices.locked = true;
            devices.primary = Some(primary);
        }
        let mut other = Gamepad::default();
        other.digital_mut().press(GamepadButton::Start);
        let other = app.world_mut().spawn(other).id();
        app.update();
        assert_eq!(
            app.world().resource::<ControllerInput>().entity,
            Some(primary)
        );
        assert!(!app.world().resource::<ControllerInput>().pause);
        app.world_mut().despawn(primary);
        app.update();
        assert!(app.world().resource::<LocalDevices>().awaiting_primary);
        app.world_mut()
            .get_mut::<Gamepad>(other)
            .unwrap()
            .digital_mut()
            .clear();
        app.world_mut()
            .get_mut::<Gamepad>(other)
            .unwrap()
            .digital_mut()
            .press(GamepadButton::South);
        app.update();
        assert_eq!(app.world().resource::<LocalDevices>().primary, Some(other));
    }
    #[test]
    fn keyboard_activity_never_takes_player_one_from_a_locked_pad() {
        let mut app = app();
        app.init_resource::<ButtonInput<KeyCode>>();
        let mut pad = Gamepad::default();
        pad.analog_mut().set(GamepadAxis::LeftStickX, 1.);
        let pad = app.world_mut().spawn(pad).id();
        {
            let mut devices = app.world_mut().resource_mut::<LocalDevices>();
            devices.locked = true;
            devices.primary = Some(pad);
        }
        app.update();
        let state = app.world().resource::<ControllerInput>();
        assert!(state.active && state.exclusive);
        // Typing on the keyboard while the pad is held: still the pad's.
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        app.update();
        let state = app.world().resource::<ControllerInput>();
        assert!(state.active, "keyboard took over a locked pad");
        assert_eq!(state.entity, Some(pad));
        // Player one on keyboard and mouse: a pad is never exclusive.
        app.world_mut().resource_mut::<LocalDevices>().primary = None;
        app.update();
        assert!(!app.world().resource::<ControllerInput>().exclusive);
    }
    #[test]
    fn modern_families_and_steam_emulation_fallback() {
        assert_eq!(
            Family::detect("DualSense Wireless Controller", Some(0x054c)),
            Family::Ps5
        );
        assert_eq!(Family::detect("Steam Deck", None), Family::Deck);
        assert_eq!(Family::detect("Steam Controller", None), Family::Steam);
        assert_eq!(
            Family::detect("Xbox Series X Controller", None),
            Family::Xbox
        );
        assert_eq!(
            Family::detect("Microsoft X-Box 360 pad", None),
            Family::Xbox
        );
    }
    #[test]
    fn native_movement_deadzone_and_diagonal_clamp() {
        assert_eq!(stick(Vec2::splat(0.1)), Vec2::ZERO);
        assert!((stick(Vec2::new(0.6, 0.0)).length() - 0.6).abs() < 1e-5);
        assert!((stick(Vec2::ONE).length() - 1.0).abs() < 1e-5);
        assert_eq!(stick(Vec2::new(f32::NAN, 1.0)), Vec2::ZERO);
    }
    #[test]
    fn gesture_conjures_once_and_disconnect_releases() {
        let mut app = app();
        let mut pad = Gamepad::default();
        pad.analog_mut().set(GamepadAxis::RightStickX, 1.);
        let entity = app.world_mut().spawn(pad).id();
        app.update();
        {
            let mut pad = app.world_mut().get_mut::<Gamepad>(entity).unwrap();
            pad.analog_mut().set(GamepadAxis::RightStickX, 0.);
            pad.analog_mut().set(GamepadAxis::RightStickY, -1.);
            pad.analog_mut().set(GamepadButton::RightTrigger2, 0.8);
        }
        app.update();
        let state = app.world().resource::<ControllerInput>();
        assert!(state.active && state.cast && state.elements[7]);
        app.update();
        assert_eq!(
            app.world().resource::<ControllerInput>().elements,
            [false; 8]
        );
        app.world_mut().despawn(entity);
        app.update();
        let state = app.world().resource::<ControllerInput>();
        assert!(!state.active && !state.cast && !state.elements.iter().any(|v| *v));
        assert!(state.release_movement);
        assert_eq!(state.entity, None);
    }
    #[test]
    fn ownership_never_combines_pads_and_keyboard_takes_over() {
        let mut app = app();
        app.init_resource::<ButtonInput<KeyCode>>();
        let mut first = Gamepad::default();
        first.digital_mut().press(GamepadButton::South);
        let first = app.world_mut().spawn(first).id();
        let mut second = Gamepad::default();
        second.digital_mut().press(GamepadButton::West);
        let second = app.world_mut().spawn(second).id();
        app.update();
        let owner = app.world().resource::<ControllerInput>().entity.unwrap();
        let other = if owner == first { second } else { first };
        let state = app.world().resource::<ControllerInput>();
        assert_ne!(state.confirm, state.interact);
        for entity in [first, second] {
            app.world_mut()
                .get_mut::<Gamepad>(entity)
                .unwrap()
                .digital_mut()
                .clear();
        }
        app.world_mut()
            .get_mut::<Gamepad>(other)
            .unwrap()
            .digital_mut()
            .press(GamepadButton::East);
        app.update();
        let state = app.world().resource::<ControllerInput>();
        assert_eq!(state.entity, Some(other));
        assert!(state.boost);
        assert!(!state.confirm);
        assert_eq!(state.interact, other == second);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyQ);
        app.update();
        let state = app.world().resource::<ControllerInput>();
        assert!(!state.active);
        assert!(state.release_movement);
        assert!(!state.boost);
    }
    #[test]
    fn all_families_build_gameplay_and_menu_prompts_and_hide_on_keyboard() {
        let mut app = App::new();
        app.init_resource::<Assets<Image>>()
            .init_resource::<ControllerInput>()
            .init_resource::<LocalDevices>()
            .add_systems(Update, prompts);
        app.update(); // Decode the embedded images.
        for family in Family::ALL {
            for (in_game, expected) in [(true, 13), (false, 0)] {
                *app.world_mut().resource_mut::<ControllerInput>() = ControllerInput {
                    active: true,
                    family,
                    in_game,
                    show_legend: in_game,
                    ..default()
                };
                app.update();
                let count = app
                    .world_mut()
                    .query::<&ImageNode>()
                    .iter(app.world())
                    .count();
                assert_eq!(count, expected, "{family:?} gameplay={in_game}");
            }
        }
        app.world_mut().resource_mut::<ControllerInput>().active = false;
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ImageNode>()
                .iter(app.world())
                .count(),
            0
        );
    }
    #[test]
    fn native_face_buttons_and_magick_browsing_are_independent() {
        use GamepadButton as B;
        let mut app = app();
        let mut pad = Gamepad::default();
        for button in [
            B::South,
            B::West,
            B::East,
            B::North,
            B::DPadUp,
            B::RightTrigger,
        ] {
            pad.digital_mut().press(button);
        }
        app.world_mut().spawn(pad);
        app.update();
        let state = app.world().resource::<ControllerInput>();
        assert!(
            state.attack
                && state.interact
                && state.self_cast
                && state.boost
                && state.block
                && state.prev
        );
        assert_eq!(state.elements, [false; 8]);
    }
    #[test]
    fn both_schemes_emit_the_element_shown_by_each_live_hud_binding() {
        use GamepadButton as B;
        let directions = [Vec2::X, Vec2::Y, Vec2::NEG_X, Vec2::NEG_Y];
        for style in [ElementStyle::RightStick, ElementStyle::BumperDpad] {
            let mut app = app();
            app.world_mut()
                .resource_mut::<settings::Preferences>()
                .style = style;
            let entity = app.world_mut().spawn(Gamepad::default()).id();
            for hud in 0..8 {
                let element = native::HUD_TO_INPUT[hud];
                {
                    let mut pad = app.world_mut().get_mut::<Gamepad>(entity).unwrap();
                    pad.digital_mut().release_all();
                    pad.digital_mut().clear();
                    pad.analog_mut().set(GamepadAxis::RightStickX, 0.);
                    pad.analog_mut().set(GamepadAxis::RightStickY, 0.);
                }
                app.update();
                let glyphs = native::element_glyphs(hud, style);
                if style == ElementStyle::RightStick {
                    for glyph in [glyphs[0], glyphs[2]] {
                        let direction = directions[glyph - 17];
                        {
                            let mut pad = app.world_mut().get_mut::<Gamepad>(entity).unwrap();
                            pad.analog_mut().set(GamepadAxis::RightStickX, direction.x);
                            pad.analog_mut().set(GamepadAxis::RightStickY, direction.y);
                        }
                        app.update();
                    }
                } else {
                    let button = [B::DPadUp, B::DPadDown, B::DPadLeft, B::DPadRight]
                        [glyphs.last().unwrap() - 13];
                    {
                        let mut pad = app.world_mut().get_mut::<Gamepad>(entity).unwrap();
                        if glyphs.len() == 3 {
                            pad.digital_mut().press(B::LeftTrigger);
                        }
                        pad.digital_mut().press(button);
                    }
                    app.update();
                }
                let state = app.world().resource::<ControllerInput>();
                assert!(
                    state.elements[element],
                    "{style:?}, HUD element {hud}, glyphs {glyphs:?}"
                );
                assert_eq!(state.elements.iter().filter(|down| **down).count(), 1);
            }
        }
    }
    #[test]
    fn element_guide_draws_one_key_background_per_control() {
        let screen = 720.;
        for style in [ElementStyle::RightStick, ElementStyle::BumperDpad] {
            for hud in 0..8 {
                let glyphs = native::element_glyphs(hud, style);
                let anchor = magicka_runtime::keyboard_hud::key_anchor(hud, screen);
                let (bounds, badges) = native::element_layout(&glyphs, anchor);
                let controls: Vec<_> = glyphs
                    .iter()
                    .zip(&bounds)
                    .filter(|(g, _)| **g < 21)
                    .map(|(_, b)| *b)
                    .collect();
                assert_eq!(badges.len(), controls.len(), "{style:?} {hud}");
                for (c, b) in controls.iter().zip(&badges) {
                    assert!(b[0] <= c[0] && c[2] <= b[2] && b[1] <= c[1] && c[3] <= b[3]);
                }
                let (left, right) = (badges[0][0], badges.last().unwrap()[2]);
                assert!(
                    right - left <= 44.,
                    "pairs stay visibly apart in the 50px pitch"
                );
                if badges.len() == 1 {
                    let original =
                        magicka_runtime::keyboard_hud::quads(hud, screen, Default::default())[2]
                            .bounds;
                    assert_eq!(badges[0], original);
                } else {
                    assert!(badges[0][2] < badges[1][0], "two separate backgrounds");
                    let connector = bounds[1];
                    assert!(connector[0] < badges[0][2] + 2. && badges[1][0] - 2. < connector[2]);
                }
            }
        }
    }
    #[test]
    fn bumper_dpad_reserves_magick_browsing_and_ignores_right_stick_gestures() {
        use GamepadButton as B;
        let mut app = app();
        app.world_mut()
            .resource_mut::<settings::Preferences>()
            .style = ElementStyle::BumperDpad;
        let mut pad = Gamepad::default();
        pad.digital_mut().press(B::RightTrigger);
        pad.digital_mut().press(B::DPadLeft);
        pad.analog_mut().set(GamepadAxis::RightStickX, 1.);
        let entity = app.world_mut().spawn(pad).id();
        app.update();
        assert!(app.world().resource::<ControllerInput>().prev);
        {
            let mut pad = app.world_mut().get_mut::<Gamepad>(entity).unwrap();
            pad.analog_mut().set(GamepadAxis::RightStickX, 0.);
            pad.analog_mut().set(GamepadAxis::RightStickY, 1.);
        }
        app.update();
        let state = app.world().resource::<ControllerInput>();
        assert_eq!(state.elements, [false; 8]);
        assert!(!state.block);
    }
    #[test]
    fn face_and_system_actions_and_analog_triggers_survive_focus_changes() {
        use GamepadButton as B;
        let mut app = app();
        let mut pad = Gamepad::default();
        for button in [B::South, B::East, B::West, B::North, B::Select, B::Start] {
            pad.digital_mut().press(button);
        }
        pad.analog_mut().set(B::LeftTrigger2, 0.8);
        pad.analog_mut().set(B::RightTrigger2, 0.4);
        let entity = app.world_mut().spawn(pad).id();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    focused: true,
                    ..default()
                },
                bevy::window::PrimaryWindow,
            ))
            .id();
        app.update();
        let state = app.world().resource::<ControllerInput>();
        assert!(
            state.confirm
                && state.back
                && state.self_cast
                && state.attack
                && state.boost
                && state.inventory
                && state.pause
        );
        assert!(state.area && !state.cast);
        app.world_mut()
            .get_mut::<Gamepad>(entity)
            .unwrap()
            .analog_mut()
            .set(B::RightTrigger2, 0.8);
        app.update();
        assert!(app.world().resource::<ControllerInput>().cast);
        app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
        app.update();
        let state = app.world().resource::<ControllerInput>();
        assert!(!state.active && !state.cast && !state.area && state.release_movement);
        app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
        app.update();
        assert!(app.world().resource::<ControllerInput>().cast);
    }
    #[test]
    fn projected_pad_aim_round_trips_through_live_camera_and_runtime() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let content = root.join("../../../../target/Magicka/Content");
        if !content.is_dir() {
            eprintln!("live camera check requires user-supplied Magicka content");
            return;
        }
        let level = root
            .join("../../scenarios/feature-lab.lvl")
            .canonicalize()
            .unwrap();
        let mut session = PlaySession::load(&content, level.to_str().unwrap(), "Wizard").unwrap();
        session.disable_scripts();
        session.step(1.0 / 60.0);
        let state = ControllerInput {
            active: true,
            movement: Vec2::X,
            aim: Vec2::Y,
            ..default()
        };
        let input = state.player_input(&session, 16.0 / 9.0);
        let position = Vec3::from(session.body().position);
        let aim = Vec3::from(session.camera().mouse_pick(
            input.cursor_ndc.unwrap(),
            16.0 / 9.0,
            position.y,
        ));
        let direction = (aim - position).normalize();
        let movement = input.pad_movement.unwrap();
        let movement = Vec3::new(movement[0], 0.0, movement[1]);
        assert!((aim.distance(position) - 1.0).abs() < 0.002);
        assert!(direction.dot(movement).abs() < 0.002);
        assert!((movement.length() - 1.0).abs() < 1e-5);
        session.handle_input(input, 16.0 / 9.0);
        assert!(Vec3::from(session.body().movement()).distance(movement) < 1e-5);
        assert!(Vec3::from(session.body().desired_direction()).dot(direction) > 0.999);
    }
    #[test]
    fn live_hud_tracks_the_owning_pad_and_never_inherits_another_pads_override() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let content = root.join("../../../../target/Magicka/Content");
        if !content.is_dir() {
            return;
        }
        let level = root
            .join("../../scenarios/feature-lab.lvl")
            .canonicalize()
            .unwrap();
        let mut session = PlaySession::load(&content, level.to_str().unwrap(), "Wizard").unwrap();
        session.disable_scripts();
        session.step(1.0 / 60.0);
        #[derive(Resource)]
        struct Session(PlaySession);
        let mut app = app();
        app.init_resource::<Assets<Image>>()
            .insert_resource(Session(session))
            .add_systems(
                Update,
                (|mut session: ResMut<Session>, mut input: ResMut<ControllerInput>| {
                    if input.active {
                        let frame = input.player_input(&session.0, 16.0 / 9.0);
                        session.0.handle_input(frame, 16.0 / 9.0);
                    }
                    input.update_context(&session.0, [1280.0, 720.0], false, false);
                })
                .before(ControllerPrompts),
            );
        let xbox = app
            .world_mut()
            .spawn((Gamepad::default(), Name::new("Xbox Series Controller")))
            .id();
        let ps5 = app
            .world_mut()
            .spawn((
                Gamepad::default(),
                Name::new("DualSense Wireless Controller"),
            ))
            .id();
        app.world_mut()
            .get_mut::<Gamepad>(xbox)
            .unwrap()
            .digital_mut()
            .press(GamepadButton::DPadUp);
        app.insert_resource(NativePrompts(vec![NativePrompt {
            intensity: 1.,
            element: None,
            glyph: 2,
            bounds: [10., 10., 34., 34.],
            alpha: 1.,
        }]));
        app.update();
        app.update();
        let check = |app: &mut App, expected: Family| {
            assert_eq!(app.world().resource::<ControllerInput>().family, expected);
            assert_eq!(
                app.world_mut()
                    .query::<&PromptIcon>()
                    .iter(app.world())
                    .count(),
                1
            );
            let mut query = app.world_mut().query::<(&PromptIcon, &ImageNode)>();
            let images = app.world().resource::<PromptImages>();
            for (icon, image) in query.iter(app.world()) {
                assert_eq!(icon.family, expected);
                assert_eq!(
                    icon.controller,
                    app.world().resource::<ControllerInput>().entity
                );
                assert_eq!(icon.player_slot, 0);
                let family = Family::ALL.iter().position(|f| *f == expected).unwrap();
                assert_eq!(image.image, images.0[family][icon.glyph]);
            }
            assert!(app
                .world_mut()
                .query::<&Text>()
                .iter(app.world())
                .all(|text| !text.0.contains("Move / Navigate")));
        };
        check(&mut app, Family::Xbox);
        // A user override belongs to Xbox's entity only.
        app.world_mut()
            .resource_mut::<ControllerInput>()
            .cycle_prompts(); // Xbox override
        app.update();
        app.world_mut()
            .resource_mut::<ControllerInput>()
            .cycle_prompts(); // PS5 override
        app.update();
        app.world_mut()
            .resource_mut::<ControllerInput>()
            .cycle_prompts(); // Deck override
        app.update();
        check(&mut app, Family::Deck);
        app.world_mut()
            .get_mut::<Gamepad>(xbox)
            .unwrap()
            .digital_mut()
            .clear();
        app.world_mut()
            .get_mut::<Gamepad>(ps5)
            .unwrap()
            .digital_mut()
            .press(GamepadButton::DPadDown);
        app.update();
        assert_eq!(app.world().resource::<ControllerInput>().entity, Some(ps5));
        check(&mut app, Family::Ps5);
        app.world_mut()
            .get_mut::<Gamepad>(ps5)
            .unwrap()
            .digital_mut()
            .clear();
        app.world_mut()
            .get_mut::<Gamepad>(xbox)
            .unwrap()
            .digital_mut()
            .press(GamepadButton::DPadRight);
        app.update();
        assert_eq!(app.world().resource::<ControllerInput>().entity, Some(xbox));
        check(&mut app, Family::Deck); // Xbox keeps its own explicit override.
    }
    #[test]
    fn every_family_has_decodable_prompt_images() {
        for row in glyphs::PNGS {
            for png in row {
                assert!(Image::from_buffer(
                    png,
                    bevy::image::ImageType::Extension("png"),
                    bevy::image::CompressedImageFormats::NONE,
                    true,
                    default(),
                    default()
                )
                .is_ok());
            }
        }
    }
}

/// A content-independent visual harness for all four prompt sets.
pub(crate) fn preview(args: &[String]) {
    let value = |flag: &str| args.windows(2).find(|a| a[0] == flag).map(|a| a[1].clone());
    let family = value("--controller-preview")
        .as_deref()
        .and_then(Family::parse)
        .unwrap_or_default();
    let snapshot = value("--snapshot");
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: format!("{} controller prompts", family.label()),
            resolution: (1280, 720).into(), ..default()
        }), ..default()
    }))
    .insert_resource(ClearColor(Color::srgb(0.08, 0.10, 0.13)))
    .insert_resource(ControllerInput { active: true, family, show_legend: true, in_game: true,
        ..default() })
    .add_systems(Startup, move |mut commands: Commands| {
        commands.spawn(Camera2d);
        commands.spawn((
            Text::new(format!("{}\nController prompt preview\n\nLeft stick: independent analog movement\nRight stick / right trackpad: original element gestures\nD-pad Up / Down: browse magicks\nRight bumper: block\n\nUse a Gamepad Steam Input layout; right trackpad as right joystick.\nIn-game F8 cycles prompt styles. Options > Controls sets the active device style.", family.label())),
            TextFont { font_size: bevy::text::FontSize::Px(24.0), ..default() },
            Node { position_type: PositionType::Absolute, left: px(40), top: px(112), ..default() },
        ));
    })
    .add_systems(Update, prompts)
    .add_systems(Update, move |mut commands: Commands, mut frame: Local<u32>| {
        *frame += 1;
        if *frame == 40 {
            if let Some(path) = &snapshot {
                use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path.clone()))
                    .observe(|_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| { exit.write(AppExit::Success); });
            }
        }
    });
    app.run();
}
