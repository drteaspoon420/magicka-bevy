//! Pause and tutorial presentation using user-supplied retail bitmap assets.
use super::PlayAppExt;
use super::{step_session, Play};
use crate::{
    native_text::{geometry_mesh, upload_font},
    play_hud::HudMaterial,
    text_box_visual::{background_mesh, read_asset, upload_background, TextBoxMaterial},
};
use bevy::prelude::*;
use magicka_content::assets::Asset;
use magicka_runtime::{
    hud::HudQuad,
    scene_loading::custom_hash,
    text_geometry::{GeometryFont, TextAlign, TextGeometry, TextStyle},
};
use magicka_sim::spells::{MagickType, MAGICK_COMBOS};
use std::sync::Arc;

#[derive(Resource, Default)]
pub(super) struct PauseMenu {
    pub visible: bool,
    pub restart: bool,
    /// InGameMenuMain.TutorialSkipCallback: ExecuteTrigger("skip_tutorial").
    pub skip_tutorial: bool,
    /// Who can be removed from the game from this computer (seat, name); None
    /// when this computer cannot remove anyone.
    pub kickable: Option<Vec<(usize, String)>>,
    /// A removal confirmed on the Players page.
    pub kick: Option<usize>,
    /// The seat awaiting confirmation.
    kick_target: Option<usize>,
    /// The Banana item is enabled (scene wc_s3, not a network client).
    banana: bool,
    pub consume_frame: bool,
    page: u8,
    selected: usize,
    clock_paused: bool,
    magick: usize,
    scroll: usize,
    cursor: Option<Vec2>,
}
#[derive(Resource)]
struct MenuAssets {
    body: Arc<GeometryFont>,
    menu: Arc<GeometryFont>,
    sizes: [[f32; 2]; 6],
    atlases: [Handle<HudMaterial>; 6],
}
#[derive(Component, Clone, Copy)]
enum Visual {
    Shade,
    MenuBox,
    JoinBox,
    JoinText,
    Selection,
    Heading,
    Row(usize),
    Hint,
    Dialog,
    Icons,
    Keys,
    Art,
    Elements,
    Description,
    Notifier,
    NotifierKey,
    NotifierBackground,
    Wheel,
}
#[derive(Component, Clone, Copy)]
enum Panel {
    Hint,
    Dialog,
}
impl PauseMenu {
    /// QA: ask to remove the first player, as choosing them on the page does.
    pub(super) fn open_kick_confirmation(&mut self) {
        self.visible = true;
        self.kick_target = self
            .kickable
            .as_deref()
            .and_then(|everyone| everyone.first())
            .map(|(seat, _)| *seat)
            .or(self.kick_target);
        self.page = PAGE_KICK;
    }
    /// QA: answer Yes, as the confirmation's first row does, then close the menu.
    pub(super) fn confirm_kick(&mut self) {
        self.kick = self.kick_target.take();
        self.page = 0;
        self.visible = false;
    }
    /// QA: show the Players page.
    pub(super) fn open_players_page(&mut self) {
        self.visible = true;
        self.page = PAGE_PLAYERS;
    }
    pub(super) fn show_controller_controls(&self) -> bool {
        self.visible && self.page == 1
    }
}
const MAIN: &[&str] = &[
    "#menu_resume",
    "#add_menu_restartcheckpoint",
    "#menu_tome_02",
    "Controls",
    "#menu_main_07",
];
/// InGameMenuMain items: "#banana" first, drawn only while enabled.
fn main_items(menu: &PauseMenu) -> Vec<&'static str> {
    let mut items = Vec::with_capacity(MAIN.len() + 1);
    if menu.banana {
        items.push("#banana");
    }
    items.extend_from_slice(MAIN);
    // Rewrite extension: the host and dungeon masters can remove players.
    if menu.kickable.is_some() {
        items.insert(items.len() - 1, "Players");
    }
    items
}
/// Page numbers beyond the original's.
const PAGE_PLAYERS: u8 = 6;
const PAGE_KICK: u8 = 7;
/// Rows on the Players page before it scrolls, plus More and Back.
const PLAYER_ROWS: usize = 10;
/// The Players page's labels and what each does: a seat, or None for More/Back.
fn player_rows(menu: &PauseMenu) -> Vec<(String, Option<usize>)> {
    let everyone = menu.kickable.as_deref().unwrap_or_default();
    let mut rows: Vec<(String, Option<usize>)> = everyone
        .iter()
        .skip(menu.scroll)
        .take(PLAYER_ROWS)
        .map(|(seat, name)| (name.clone(), Some(*seat)))
        .collect();
    if everyone.len() > PLAYER_ROWS {
        rows.push(("More".into(), None));
    }
    rows.push(("Back".into(), None));
    rows
}
fn pool() -> Vec<MagickType> {
    (1..magicka_sim::spells::MAGICK_INDEX_COUNT)
        .filter_map(|i| MagickType::try_from(i).ok())
        .collect()
}
fn count(menu: &PauseMenu) -> usize {
    match menu.page {
        0 => main_items(menu).len(),
        1 => 3,
        4 => pool().len() + 1,
        PAGE_PLAYERS => player_rows(menu).len(),
        _ => 2,
    }
}
fn row_y(menu: &PauseMenu, index: usize) -> f32 {
    if menu.page == 4 {
        if index == pool().len() {
            670.0
        } else {
            150.0 + (index - menu.scroll) as f32 * 32.0
        }
    } else if menu.page == 1 {
        568.0 + index as f32 * 32.0
    } else if menu.page == PAGE_PLAYERS {
        150.0 + index as f32 * 32.0
    } else {
        290.0 + index as f32 * 32.0
    }
}
pub(super) fn plugin(app: &mut App) {
    app.init_play_resource::<PauseMenu>()
        .add_play_systems(Startup, setup)
        .add_play_systems(
            Update,
            (controls.run_if(super::frontend_loading::ready), pause_clock)
                .chain()
                .before(step_session),
        )
        .add_play_systems(
            Update,
            present
                .after(step_session)
                .before(crate::controller::ControllerPrompts),
        );
}
fn pause_clock(mut menu: ResMut<PauseMenu>, play: Res<Play>, mut time: ResMut<Time<Virtual>>) {
    let paused = menu.visible
        && play.session.world.role == magicka_sim::damage_delivery::NetworkRole::Offline;
    if paused && !menu.clock_paused {
        time.pause();
        menu.clock_paused = true;
    }
    if !paused && menu.clock_paused {
        time.unpause();
        menu.clock_paused = false;
    }
}
fn setup(
    mut commands: Commands,
    play: Res<Play>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut colors: ResMut<Assets<ColorMaterial>>,
    mut hud_materials: ResMut<Assets<HudMaterial>>,
    mut boxes: ResMut<Assets<TextBoxMaterial>>,
) {
    let result = (|| -> Result<(), String> {
        let mut font = |name| -> Result<_, String> {
            let Asset::BitmapFont(font) =
                read_asset(&play.content.join(format!("Languages/eng/Font/{name}.xnb")))?
            else {
                return Err("expected bitmap font".into());
            };
            let (font, material) = upload_font(&font, &mut images, &mut colors)?;
            // This overlay is drawn before the original final HDR decode.
            // Use the HUD's encoding and sRGB conversion for bitmap text too.
            let texture = colors.get(&material).unwrap().texture.clone().unwrap();
            Ok((
                font,
                hud_materials.add(HudMaterial {
                    texture,
                    saturation: Vec4::X,
                }),
            ))
        };
        let (body, body_material) = font("Maiandra14")?;
        let (menu, menu_material) = font("Maiandra18")?;
        let mut sizes = [[0.; 2]; 6];
        let mut atlases = Vec::new();
        for (i, name) in [
            "UI/Menu/FadeBox",
            "UI/HUD/Tutorial",
            "UI/HUD/hud",
            "UI/Menu/Magicks",
            "UI/Menu/Magicks_2",
            "UI/HUD/NotifierButton",
        ]
        .into_iter()
        .enumerate()
        {
            let Asset::Texture2D(texture) = read_asset(&play.content.join(format!("{name}.xnb")))?
            else {
                return Err(format!("expected texture {name}"));
            };
            sizes[i] = [texture.width as f32, texture.height as f32];
            let image = crate::play_hud::upload_texture(&texture, &mut images)?;
            atlases.push(hud_materials.add(HudMaterial {
                texture: image,
                saturation: Vec4::X,
            }));
        }
        // Dimming is untextured in the original. A solid white texel lets
        // this textured shader retain only the black vertex tint and alpha.
        let white = images.add(Image::new_fill(
            bevy::render::render_resource::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            bevy::render::render_resource::TextureDimension::D2,
            &[255; 4],
            bevy::render::render_resource::TextureFormat::Rgba8Unorm,
            bevy::asset::RenderAssetUsages::default(),
        ));
        let shade = hud_materials.add(HudMaterial {
            texture: white,
            saturation: Vec4::X,
        });
        let mut spawn = |visual, material, layer| {
            commands.spawn((
                visual,
                Mesh2d(meshes.add(Rectangle::new(1., 1.))),
                MeshMaterial2d(material),
                Transform::from_xyz(0., 0., layer),
                Visibility::Hidden,
            ));
        };
        spawn(Visual::Shade, shade, 999.1);
        spawn(Visual::MenuBox, atlases[0].clone(), 999.2);
        spawn(Visual::JoinBox, atlases[0].clone(), 999.7);
        spawn(Visual::JoinText, menu_material.clone(), 999.8);
        spawn(Visual::Selection, atlases[0].clone(), 999.3);
        spawn(Visual::Heading, menu_material.clone(), 999.5);
        for i in 0..13 {
            spawn(Visual::Row(i), menu_material.clone(), 999.5);
        }
        for visual in [
            Visual::Hint,
            Visual::Dialog,
            Visual::Keys,
            Visual::Description,
            Visual::Notifier,
            Visual::NotifierKey,
            Visual::Wheel,
        ] {
            spawn(visual, body_material.clone(), 999.5);
        }
        spawn(Visual::NotifierBackground, atlases[5].clone(), 999.4);
        spawn(Visual::Icons, atlases[1].clone(), 999.4);
        spawn(Visual::Art, atlases[3].clone(), 999.4);
        spawn(Visual::Elements, atlases[2].clone(), 999.4);
        let Asset::Texture2D(texture) = read_asset(&play.content.join("UI/HUD/Dialog_Say.xnb"))?
        else {
            return Err("expected dialog texture".into());
        };
        for panel in [Panel::Hint, Panel::Dialog] {
            let material = upload_background(&texture, &mut images, &mut boxes)?;
            commands.spawn((
                panel,
                Mesh2d(meshes.add(Rectangle::new(1., 1.))),
                MeshMaterial2d(material),
                Transform::from_xyz(0., 0., 999.2),
                Visibility::Hidden,
            ));
        }
        commands.insert_resource(MenuAssets {
            body,
            menu,
            sizes,
            atlases: atlases.try_into().unwrap(),
        });
        Ok(())
    })();
    if let Err(e) = result {
        error!("retail pause/tutorial assets unavailable: {e}");
    }
}
fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut controller: ResMut<crate::controller::ControllerInput>,
    mouse: Res<ButtonInput<MouseButton>>,
    wheel: Option<Res<bevy::input::mouse::AccumulatedMouseScroll>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut menu: ResMut<PauseMenu>,
    play: Res<Play>,
    mut exit: MessageWriter<super::LeaveSession>,
) {
    menu.consume_frame = false;
    // OnEnter: mMenuItems[0].Enabled = scene == wc_s3 && !client.
    menu.banana = play.session.scene_name().eq_ignore_ascii_case("wc_s3")
        && play.session.world.role != magicka_sim::damage_delivery::NetworkRole::Client;
    if keys.just_pressed(KeyCode::Escape) || controller.pause || (menu.visible && controller.back) {
        if menu.visible && menu.page != 0 {
            menu.page = 0;
        } else {
            menu.visible = !menu.visible;
        }
        menu.selected = 0;
        menu.consume_frame = true;
        return;
    }
    if !menu.visible {
        return;
    }
    let n = count(&menu);
    if keys.just_pressed(KeyCode::ArrowDown) || controller.direction[1] {
        menu.selected = (menu.selected + 1) % n;
    }
    if keys.just_pressed(KeyCode::ArrowUp) || controller.direction[0] {
        menu.selected = (menu.selected + n - 1) % n;
    }
    if menu.page == 4 {
        let wheel = wheel.as_ref().map_or(0., |w| w.delta.y);
        if wheel > 0. {
            menu.selected = menu.selected.saturating_sub(1);
        }
        if wheel < 0. {
            menu.selected = (menu.selected + 1).min(n - 1);
        }
        if keys.just_pressed(KeyCode::PageDown) {
            menu.selected = (menu.selected + 12).min(n - 1);
        }
        if keys.just_pressed(KeyCode::PageUp) {
            menu.selected = menu.selected.saturating_sub(12);
        }
        if menu.selected < n - 1 {
            if menu.selected < menu.scroll {
                menu.scroll = menu.selected;
            }
            if menu.selected >= menu.scroll + 12 {
                menu.scroll = menu.selected - 11;
            }
            menu.magick = menu.selected;
        }
    }
    let mut activate = keys.just_pressed(KeyCode::Enter)
        || keys.just_pressed(KeyCode::Space)
        || controller.confirm;
    if let Ok(window) = windows.single() {
        if let Some(cursor) = window.cursor_position() {
            let moved = menu.cursor != Some(cursor);
            menu.cursor = Some(cursor);
            let scale = window.height() / 720.;
            let x = if menu.page == 4 {
                window.width() * 0.5 - 350. * scale
            } else {
                window.width() * 0.5
            };
            for i in 0..n {
                if menu.page == 4 && i < n - 1 && (i < menu.scroll || i >= menu.scroll + 12) {
                    continue;
                }
                if (cursor.x - x).abs() < 150. * scale
                    && (cursor.y - (row_y(&menu, i) + 12.) * scale).abs() < 16. * scale
                    && (moved || mouse.just_pressed(MouseButton::Left))
                {
                    menu.selected = i;
                    if menu.page == 4 && i < n - 1 {
                        menu.magick = i;
                    }
                    activate |= mouse.just_pressed(MouseButton::Left);
                }
            }
        }
    }
    if !activate {
        return;
    }
    let main = main_items(&menu);
    match (menu.page, menu.selected) {
        (0, i) => match main.get(i).copied() {
            Some("#banana") => menu.page = 5,
            Some("#menu_resume") => menu.visible = false,
            Some("#add_menu_restartcheckpoint") => menu.page = 2,
            Some("#menu_tome_02") => {
                menu.page = 4;
                menu.scroll = 0;
                menu.magick = 0;
            }
            Some("Controls") => menu.page = 1,
            Some("Players") => {
                menu.page = PAGE_PLAYERS;
                menu.scroll = 0;
            }
            Some("#menu_main_07") => menu.page = 3,
            _ => return,
        },
        (1, 0) => {
            controller.cycle_prompts();
            return;
        }
        (1, 1) => {
            controller.cycle_element_style();
            return;
        }
        (1, _) | (2..=3 | 5, 1) => menu.page = 0,
        (5, 0) => {
            // InGameMenu.Hide, then the scene's skip_tutorial trigger.
            menu.skip_tutorial = true;
            menu.visible = false;
        }
        (2, 0) => {
            menu.restart = true;
            menu.visible = false;
        }
        (3, 0) => {
            exit.write(super::LeaveSession::default());
        }
        (4, i) if i == n - 1 => menu.page = 0,
        (PAGE_PLAYERS, i) => {
            let rows = player_rows(&menu);
            match rows.get(i) {
                Some((_, Some(seat))) => {
                    menu.kick_target = Some(*seat);
                    menu.page = PAGE_KICK;
                }
                Some((label, None)) if label == "More" => {
                    let everyone = menu.kickable.as_deref().map_or(0, <[_]>::len);
                    menu.scroll = if menu.scroll + PLAYER_ROWS < everyone {
                        menu.scroll + PLAYER_ROWS
                    } else {
                        0
                    };
                }
                _ => menu.page = 0,
            }
        }
        (PAGE_KICK, 0) => {
            menu.kick = menu.kick_target.take();
            menu.page = PAGE_PLAYERS;
        }
        (PAGE_KICK, _) => {
            menu.kick_target = None;
            menu.page = PAGE_PLAYERS;
        }
        _ => return,
    }
    menu.selected = 0;
    menu.consume_frame = true;
}
fn localized(play: &Play, key: &str) -> String {
    if !key.starts_with('#') {
        return key.into();
    }
    play.session
        .world
        .dialog_text
        .as_ref()
        .and_then(|h| h.strings.get_string(custom_hash(key)))
        .unwrap_or_default()
}
fn text_mesh(
    font: &GeometryFont,
    text: &str,
    center: bool,
    alpha: f32,
    black: bool,
    replacements: Option<(&mut crate::controller::NativePrompts, [f32; 2], f32)>,
) -> Result<Mesh, String> {
    let mut geometry = TextGeometry::build(
        font,
        &text.encode_utf16().collect::<Vec<_>>(),
        TextStyle {
            align: if center {
                TextAlign::Center
            } else {
                TextAlign::Left
            },
            ..default()
        },
    )
    .map_err(|e| format!("{e:?}"))?;
    if let Some((prompts, origin, scale)) = replacements {
        geometry.quads.retain(|q| {
            let glyph = match q.input {
                0x320 => 16,
                0x321 => 14,
                0x322 => 15,
                0x323 => 13,
                0x324 => 0,
                0x325 => 1,
                0x326 => 2,
                0x327 => 3,
                0x328 => 10,
                0x329 => 11,
                _ => return true,
            };
            let a = q.vertices[0].position;
            let b = q.vertices[2].position;
            prompts.0.push(crate::controller::NativePrompt {
                intensity: 1.,
                element: None,
                glyph,
                bounds: [
                    origin[0] + a[0] * scale,
                    origin[1] + a[1] * scale,
                    origin[0] + b[0] * scale,
                    origin[1] + b[1] * scale,
                ],
                alpha,
            });
            false
        });
    }
    for q in &mut geometry.quads {
        for v in &mut q.vertices {
            v.color[3] *= alpha;
            if black {
                v.color[..3].fill(0.);
            }
        }
    }
    geometry_mesh(&geometry, geometry.quads.len() as i32)
}
fn quad(bounds: [f32; 4], atlas: [f32; 4], alpha: f32) -> HudQuad {
    HudQuad {
        bounds,
        atlas,
        color: [1., 1., 1., alpha],
    }
}
fn measure(font: &GeometryFont, text: &str) -> [f32; 2] {
    font.metrics.measure(text, true).unwrap_or([300., 40.])
}
fn hint_text(
    hint: &magicka_runtime::tutorial::Hint,
    _assets: &MenuAssets,
    style: Option<crate::controller::ElementStyle>,
) -> String {
    let Some(style) = style else {
        return hint.text.clone();
    };
    let text = hint.controller_text.as_deref().unwrap_or(&hint.text);
    if style == crate::controller::ElementStyle::BumperDpad {
        let text = text
            .replace("Right Stick", "D-pad")
            .replace("Right\nStick", "D-pad");
        // The replacement is shorter; keep the retail line breaks and all words.
        text
    } else {
        text.to_owned()
    }
}
fn hint_layout(
    play: &Play,
    assets: &MenuAssets,
    style: Option<crate::controller::ElementStyle>,
) -> ([f32; 2], [f32; 2]) {
    let Some(hint) = &play.session.world.tutorial.hint else {
        return ([0.; 2], [0.; 2]);
    };
    let text = hint_text(hint, assets, style);
    crate::controller::native::hint_layout(
        hint.position,
        measure(&assets.body, &text),
        hint.animation,
        play.screen,
    )
}
fn dialog_layout(play: &Play, assets: &MenuAssets) -> ([f32; 2], [f32; 2], f32) {
    let Some(dialog) = &play.session.world.tutorial.dialog else {
        return ([0.; 2], [0.; 2], 0.);
    };
    let text = measure(&assets.body, &dialog.text);
    let scale = dialog.scale.unwrap_or(1.);
    let image = if dialog.magick != 0 {
        [400., 250.]
    } else if dialog.element != 0 {
        [50., 50.]
    } else {
        [0., 0.]
    };
    let offset = if image[0] > 0. {
        (image[0] + 6.) * scale
    } else {
        0.
    };
    (
        [play.screen[0] * 0.5 + 0.5, play.screen[1] * 0.4 + 0.5],
        dialog
            .size
            .unwrap_or([text[0] + offset, text[1].max(image[1] * scale)]),
        offset,
    )
}
// Keyboard atlas texels and phase thresholds decoded from HintRenderData.
fn control_icons(
    animation: i32,
    time: f32,
    origin: [f32; 2],
    alpha: f32,
) -> (Vec<HudQuad>, String) {
    let durations = [
        1., 1.5, 1.5, 2., 2., 2., 2., 2., 2., 2., 2., 1.5, 1.5, 1.5, 1.5, 1., 1.5,
    ];
    let total: f32 = durations
        .iter()
        .enumerate()
        .filter(|(i, _)| animation & (1 << i) != 0)
        .map(|(_, d)| d)
        .sum();
    if total <= 0. {
        return (vec![], String::new());
    }
    let mut phase = time.rem_euclid(total);
    let mut selected = 0;
    for (i, duration) in durations.into_iter().enumerate() {
        if animation & (1 << i) == 0 {
            continue;
        }
        if phase < duration {
            selected = i;
            break;
        }
        phase -= duration;
    }
    let mut quads = vec![];
    let mut push = |x: f32, y: f32, w: f32, h: f32, u: f32, v: f32| {
        quads.push(quad(
            [
                origin[0] + x,
                origin[1] + y,
                origin[0] + x + w,
                origin[1] + y + h,
            ],
            [u, v, u + w, v + h],
            alpha,
        ));
    };
    let mut key = String::new();
    match selected {
        0 => push(-40., 0., 80., 64., 432., if phase < 0.5 { 0. } else { 64. }),
        1 => push(-40., 0., 80., 64., 352., 192.),
        2 => {
            for x in [-56., -24., 8.] {
                push(x, 0., 48., 48., 384., 96.);
            }
            key = "A S F".into();
        }
        3..=10 => {
            push(
                -24.,
                0.,
                48.,
                48.,
                if phase < 1.333 { 384. } else { 336. },
                96.,
            );
            key.push(['Q', 'W', 'E', 'R', 'A', 'S', 'D', 'F'][selected - 3]);
        }
        11 | 13 => push(
            -40.,
            0.,
            80.,
            64.,
            432.,
            if phase < 1. {
                0.
            } else if selected == 11 {
                192.
            } else {
                128.
            },
        ),
        12 | 16 => {
            push(-80., 0., 64., 48., 368., if phase < 0.5 { 0. } else { 48. });
            push(-44., 0., 32., 32., 192., 0.);
            push(
                0.,
                0.,
                80.,
                64.,
                432.,
                if phase < 1. {
                    0.
                } else if selected == 12 {
                    192.
                } else {
                    64.
                },
            );
        }
        14 => push(-64., 0., 128., 48., 240., if phase < 1. { 0. } else { 48. }),
        15 => push(
            -36.,
            0.,
            72.,
            48.,
            if phase < 0.5 { 368. } else { 304. },
            144.,
        ),
        _ => {}
    }
    (quads, key)
}
fn magick_info(
    play: &Play,
    assets: &MenuAssets,
    kind: MagickType,
) -> (String, String, &'static [magicka_sim::spells::Elements]) {
    let key = magicka_runtime::tome_magick::name_key(kind).unwrap_or_default();
    let mut name = localized(play, &key);
    let mut description = localized(play, &format!("{key}d"));
    if kind as u8 >= 36 {
        if name.is_empty() {
            name = format!("{kind:?}");
        }
        if description.is_empty() {
            description = match kind {
                MagickType::Grow => "Make your target larger.",
                MagickType::Shrink => "Reduce your target to a smaller size.",
                MagickType::Etherealize => "Shift your target into an ethereal form.",
                MagickType::Polymorph => "Transform your target into another creature.",
                MagickType::Earthquake => "Shake the ground around you.",
                MagickType::KnockDownChance => {
                    "Enchant your weapon with a chance to knock enemies down."
                }
                MagickType::InflictBleedDamage => "Enchant your weapon to inflict bleeding.",
                MagickType::InflictPoisonDamage => "Enchant your weapon to poison enemies.",
                _ => "An unusual magick. Combine the elements shown below to cast it.",
            }
            .into();
        }
    }
    let description = assets
        .body
        .metrics
        .wrap(&description, 420, true)
        .map(|w| w.text)
        .unwrap_or(description);
    let elements = MAGICK_COMBOS
        .iter()
        .find(|(m, _)| *m == kind)
        .map_or(&[][..], |(_, c)| *c);
    (name, description, elements)
}
fn art(kind: MagickType) -> (usize, [f32; 4]) {
    // InGameMenuMagicks.MAGICK_TEXTURE_LOOKUP. Entries lacking art use its
    // native unknown panel; custom art can later replace that placeholder.
    let cells = [
        (0, 0),
        (1, 0),
        (2, 0),
        (3, 0),
        (4, 0),
        (0, 1),
        (1, 1),
        (2, 1),
        (3, 1),
        (4, 1),
        (0, 2),
        (1, 2),
        (2, 2),
        (3, 2),
        (4, 2),
        (0, 3),
        (1, 3),
        (2, 3),
        (3, 3),
        (4, 3),
        (0, 4),
        (1, 4),
        (0, 6),
        (1, 6),
        (4, 6),
        (1, 7),
        (0, 7),
        (2, 6),
        (3, 6),
        (1, 0),
        (2, 0),
        (4, 0),
        (3, 0),
    ];
    let index = kind as usize;
    let (atlas, (x, y)) = if index >= 36 || index == 34 {
        (3, cells[28])
    } else {
        (if index >= 30 { 4 } else { 3 }, cells[index - 1])
    };
    let x = x as f32 * 400.;
    let y = match index {
        6 => 256.,
        11 => 512.,
        16 => 768.,
        _ => y as f32 * 250.,
    };
    (atlas, [x, y, x + 400., y + 250.])
}
/// The native prompt (glyph, label, boostable): the grip and shield-boost
/// prompts dim their key glyph while Boosts > 0 (`hud::notifier_intensity`).
fn controller_notifier(
    play: &Play,
    controller: &crate::controller::ControllerInput,
) -> Option<(usize, String, bool)> {
    let avatar = play.session.avatar();
    let body = play.session.body();
    let gripped = avatar.character.grip.gripper.is_some();
    let shield = &avatar.character.combat.health.shield;
    if gripped {
        return Some((1, localized(play, "#interact_12"), true));
    }
    if !avatar.polymorphed {
        if let Some(action) = controller
            .interaction
            .filter(|a| !matches!(*a, "Continue" | "Skip cutscene"))
        {
            let key = match action {
                "Pick up" => "#interact_01",
                "Examine" => "#interact_02",
                "Talk" => "#interact_03",
                _ => "#interact_04",
            };
            return Some((2, localized(play, key), false));
        }
        if (shield.active() && !shield.solid())
            || magicka_sim::shield_boost::shield_to_boost(
                body.position,
                body.direction(),
                play.session.world.shields.iter().map(|s| &s.shield),
            )
            .is_some()
        {
            return Some((1, localized(play, "#interact_06"), true));
        }
    }
    None
}
fn avatar_screen(play: &Play) -> [f32; 2] {
    let p = play.session.body().position;
    let camera = play.session.camera();
    let matrix = magicka_runtime::camera::multiply(
        &camera.view(),
        &camera.projection(play.screen[0] / play.screen[1].max(1.)),
    );
    let point = [p[0], p[1], p[2], 1.];
    let clip: [f32; 4] =
        std::array::from_fn(|i| (0..4).map(|j| point[j] * matrix[j * 4 + i]).sum());
    [
        (clip[0] / clip[3] + 1.) * play.screen[0] * 0.5,
        (1. - clip[1] / clip[3]) * play.screen[1] * 0.5,
    ]
}
fn present(
    controller: Res<crate::controller::ControllerInput>,
    mut prompts: ResMut<crate::controller::NativePrompts>,
    time: Res<Time>,
    mut notifier_cache: Local<Option<(usize, String)>>,
    mut notifier_alpha: Local<f32>,
    mut notifier_intensity: Local<Option<f32>>,
    play: Res<Play>,
    menu: Res<PauseMenu>,
    coop: Res<super::local_coop::LocalCoop>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    assets: Option<Res<MenuAssets>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<TextBoxMaterial>>,
    mut visuals: Query<
        (
            &Visual,
            &Mesh2d,
            &mut MeshMaterial2d<HudMaterial>,
            &mut Transform,
            &mut Visibility,
        ),
        Without<Panel>,
    >,
    mut panels: Query<
        (
            &Panel,
            &Mesh2d,
            &MeshMaterial2d<TextBoxMaterial>,
            &mut Visibility,
        ),
        Without<Visual>,
    >,
) {
    let Some(assets) = assets else {
        return;
    };
    let screen = play.screen;
    let scale = screen[1] / 720.;
    let tutorial = &play.session.world.tutorial;
    let notifier = (controller.active
        && !menu.visible
        && play.session.world.ui_enabled
        && !play.session.input_limited()
        && tutorial.dialog.is_none())
    .then(|| controller_notifier(&play, &controller))
    .flatten();
    // SetButtonIntensity(0.5) precedes NotifierButton.Update's alpha step.
    let boost_available = notifier
        .as_ref()
        .is_some_and(|(_, _, boostable)| *boostable)
        && play.session.avatar().boost.count > 0;
    if let Some((glyph, label, _)) = notifier {
        *notifier_cache = Some((glyph, label));
        *notifier_alpha = (*notifier_alpha + time.delta_secs() * 3.).min(1.);
    } else {
        *notifier_alpha = (*notifier_alpha - time.delta_secs() * 3.).max(0.);
    }
    let key_intensity = magicka_runtime::hud::notifier_intensity(
        notifier_intensity.unwrap_or(1.),
        boost_available,
        *notifier_alpha,
        time.delta_secs(),
    );
    *notifier_intensity = Some(key_intensity);
    let [avatar_x, avatar_y] = avatar_screen(&play);
    let notifier_origin = [avatar_x - 42., avatar_y + 20.];

    prompts.0.clear();
    if controller.active && !menu.visible && play.session.world.ui_enabled {
        for index in 0..8 {
            let glyphs = crate::controller::native::element_glyphs(index, controller.element_style);
            let anchor = magicka_runtime::keyboard_hud::key_anchor(index, screen[1]);
            let (bounds, _) = crate::controller::native::element_layout(&glyphs, anchor);
            for (glyph, bounds) in glyphs.into_iter().zip(bounds) {
                prompts.0.push(crate::controller::NativePrompt {
                    intensity: 1.,
                    element: Some(index),
                    glyph,
                    bounds,
                    alpha: if play.session.keyboard_hud.icons[index].enabled {
                        1.
                    } else {
                        0.35
                    },
                });
            }
        }
    }

    let (hint_center, hint_size) = hint_layout(
        &play,
        &assets,
        controller.active.then_some(controller.element_style),
    );
    let (dialog_center, dialog_size, dialog_offset) = dialog_layout(&play, &assets);
    let list = pool();
    let kind = list[menu.magick.min(list.len() - 1)];
    let unlocked = play.session.magicks_allowed() & (1 << kind as u8) != 0;
    let (name, description, combo) = if unlocked {
        magick_info(&play, &assets, kind)
    } else {
        ("???".into(), String::new(), &[][..])
    };
    let icon_origin = [
        (hint_center[0] + hint_size[0] * 0.5).floor() - 64.,
        hint_center[1],
    ];
    let (icons, key) = tutorial
        .hint
        .as_ref()
        .map(|h| control_icons(h.animation, tutorial.time, icon_origin, tutorial.hint_alpha))
        .unwrap_or_default();
    if controller.active && !menu.visible {
        if let Some(hint) = &tutorial.hint {
            if let Some(slot) =
                crate::controller::native::tutorial_slot(hint.animation, tutorial.time)
            {
                if controller.element_style == crate::controller::ElementStyle::BumperDpad
                    && (1..=10).contains(&slot.animation_bit)
                {
                    let glyphs = match slot.animation_bit {
                        1 => vec![5, 22, if slot.glyph == 13 { 15 } else { 16 }],
                        2 => vec![12],
                        bit => {
                            let input = bit - 3;
                            let glyph = [13, 14, 15, 16][input % 4];
                            if input >= 4 {
                                vec![4, 22, glyph]
                            } else {
                                vec![glyph]
                            }
                        }
                    };
                    let size = if glyphs.len() == 1 { 64. } else { 42. };
                    let width = if glyphs.len() == 1 { 64. } else { 96. };
                    let mut left = icon_origin[0] - width * 0.5;
                    for glyph in glyphs {
                        let w = if glyph == 22 { 12. } else { size };
                        let h = if glyph == 22 { 12. } else { size };
                        prompts.0.push(crate::controller::NativePrompt {
                            intensity: 1.,
                            element: None,
                            glyph,
                            bounds: [
                                left,
                                icon_origin[1] - h * 0.5,
                                left + w,
                                icon_origin[1] + h * 0.5,
                            ],
                            alpha: tutorial.hint_alpha,
                        });
                        left += w;
                    }
                } else {
                    let x = icon_origin[0] + slot.offset[0];
                    let y = icon_origin[1] + slot.offset[1];
                    prompts.0.push(crate::controller::NativePrompt {
                        intensity: 1.,
                        element: None,
                        glyph: slot.glyph,
                        bounds: [
                            x - slot.size[0] * 0.5,
                            y - slot.size[1] * 0.5,
                            x + slot.size[0] * 0.5,
                            y + slot.size[1] * 0.5,
                        ],
                        alpha: tutorial.hint_alpha * if slot.pressed { 1. } else { 0.65 },
                    });
                }
            }
        }
    }
    for (visual, mesh, mut material, mut transform, mut visibility) in &mut visuals {
        *visibility = Visibility::Hidden;
        transform.translation.x = 0.;
        transform.translation.y = 0.;
        transform.scale = Vec3::ONE;
        let mut text: Option<(String, bool, bool, [f32; 2], f32)> = None;
        let mut quads = Vec::new();
        let mut atlas = None;
        match *visual {
            Visual::JoinBox if coop.join_text().is_some() => {
                let cx = screen[0] * 0.5;
                let cy = screen[1] * 0.5;
                quads.push(quad(
                    [
                        cx - 260. * scale,
                        cy - 215. * scale,
                        cx + 260. * scale,
                        cy + 215. * scale,
                    ],
                    [0., 0., assets.sizes[0][0], assets.sizes[0][1]],
                    0.95,
                ));
                if let Some(q) = quads.last_mut() {
                    q.color[..3].fill(0.);
                }
                atlas = Some(0);
            }
            Visual::JoinText if coop.join_text().is_some() => {
                text = Some((
                    coop.join_text().unwrap(),
                    true,
                    true,
                    [screen[0] * 0.5, screen[1] * 0.5 - 190. * scale],
                    1.,
                ));
            }
            Visual::Shade if menu.visible || tutorial.dark_alpha > 0. => {
                // Native GUIBasicEffect darkening uses an untextured black quad.
                let alpha = if menu.visible {
                    0.5
                } else {
                    tutorial.dark_alpha * 0.5
                };
                quads.push(HudQuad {
                    bounds: [0., 0., screen[0], screen[1]],
                    atlas: [0.; 4],
                    color: [0., 0., 0., alpha],
                });
            }
            Visual::MenuBox if menu.visible => {
                let w = if menu.page == 4 {
                    1100.
                } else if menu.page == 1 {
                    750.
                } else {
                    400.
                } * scale;
                let h = if menu.page == 4 || menu.page == 1 || menu.page == PAGE_PLAYERS {
                    600.
                } else {
                    400.
                } * scale;
                quads.push(quad(
                    [
                        screen[0] * 0.5 - w * 0.5,
                        screen[1] * 0.5 - h * 0.5,
                        screen[0] * 0.5 + w * 0.5,
                        screen[1] * 0.5 + h * 0.5,
                    ],
                    [0., 0., assets.sizes[0][0], assets.sizes[0][1]],
                    0.666,
                ));
                if let Some(panel) = quads.last_mut() {
                    panel.color[..3].fill(0.);
                }
                atlas = Some(0);
            }
            Visual::Selection if menu.visible => {
                let x = if menu.page == 4 {
                    screen[0] * 0.5 - 350. * scale
                } else {
                    screen[0] * 0.5
                };
                let y = row_y(&menu, menu.selected) * scale;
                quads.push(quad(
                    [x - 150. * scale, y, x + 150. * scale, y + 28. * scale],
                    [0., 0., assets.sizes[0][0], assets.sizes[0][1]],
                    0.8,
                ));
                atlas = Some(0);
            }
            Visual::Heading if menu.visible => {
                let heading = match menu.page {
                    1 if controller.active => { let [attack, boost, interact, special] = controller.family.face_names(); format!("{} controls\n\nLeft stick: move and face\n{}\n{interact}: interact / {attack}: weapon\n{special}: self cast / staff special\n{boost}: boost / execute magick\nRight trigger: cast / Left trigger: area cast\nRight bumper: block\nView or Create: inventory / Menu or Options: pause", controller.family.label(), controller.element_style.instructions()) },
                    1 => "Controls\n\nLeft mouse: move / interact / pick up\nRight mouse: cast\nShift + right mouse: area cast\nMiddle mouse: self cast\nShift + left mouse: melee\nQ W E R A S D F: elements\nSpace: magick / shield boost\nMouse wheel: choose magick".into(),
                    2 | 3 => localized(&play, "#add_menu_rus"),
                    5 => localized(&play, "#banana_sure"),
                    4 => name.clone(),
                    PAGE_PLAYERS => "Remove a player".to_string(),
                    PAGE_KICK => {
                        let target = menu
                            .kickable
                            .as_deref()
                            .unwrap_or_default()
                            .iter()
                            .find(|(seat, _)| Some(*seat) == menu.kick_target)
                            .map_or("this player", |(_, name)| name.as_str());
                        format!("Remove {target}?")
                    }
                    _ => String::new(),
                };
                text = Some((
                    heading,
                    true,
                    true,
                    [
                        screen[0] * 0.5,
                        if menu.page == 4 || menu.page == PAGE_PLAYERS {
                            100. * scale
                        } else {
                            180. * scale
                        },
                    ],
                    1.,
                ));
            }
            Visual::Row(slot) if menu.visible => {
                let (index, label) = if menu.page == 4 {
                    let index = if slot == 12 {
                        list.len()
                    } else {
                        menu.scroll + slot
                    };
                    if index > list.len() || (slot < 12 && index >= list.len()) {
                        continue;
                    }
                    let label = if index == list.len() {
                        localized(&play, "#menu_back")
                    } else if play.session.magicks_allowed() & (1 << list[index] as u8) == 0 {
                        "???".into()
                    } else {
                        magick_info(&play, &assets, list[index]).0
                    };
                    (index, label)
                } else {
                    let labels: Vec<String> = match menu.page {
                        0 => main_items(&menu)
                            .iter()
                            .map(|s| localized(&play, s))
                            .collect(),
                        1 => vec![
                            format!("Prompt style: {}", controller.prompt_style_label()),
                            format!("Element input: {}", controller.element_style.label()),
                            localized(&play, "#menu_back"),
                        ],
                        PAGE_PLAYERS => player_rows(&menu).into_iter().map(|(l, _)| l).collect(),
                        _ => ["#add_menu_yes", "#add_menu_no"]
                            .iter()
                            .map(|s| localized(&play, s))
                            .collect(),
                    };
                    let Some(label) = labels.get(slot) else {
                        continue;
                    };
                    (slot, label.clone())
                };
                text = Some((
                    label,
                    true,
                    true,
                    [
                        if menu.page == 4 {
                            screen[0] * 0.5 - 350. * scale
                        } else {
                            screen[0] * 0.5
                        },
                        row_y(&menu, index) * scale,
                    ],
                    1.,
                ));
            }
            Visual::Wheel
                if controller.element_style == crate::controller::ElementStyle::RightStick
                    && controller.active
                    && !menu.visible
                    && !play.session.input_limited()
                    && tutorial.dialog.is_none()
                    && play.session.world.ui_enabled =>
            {
                let center = avatar_screen(&play);
                let angles = [
                    4.31969,
                    std::f32::consts::PI * 7. / 8.,
                    std::f32::consts::PI / 8.,
                    5.8904862,
                    3.5342917,
                    std::f32::consts::PI * 3. / 8.,
                    std::f32::consts::PI * 5. / 8.,
                    5.105088,
                ];
                let pairs = [[2, 3], [5, 6], [4, 1], [0, 7]];
                for direction in 0..4 {
                    let visible_direction = if play.session.avatar().controls_inverted {
                        (direction + 2) % 4
                    } else {
                        direction
                    };
                    let pair = pairs[visible_direction];
                    let alpha = controller.wheel_fade[direction] / 0.2;
                    if alpha <= 0. {
                        continue;
                    }
                    let t = (controller.wheel_move[direction] / 0.2).min(1.);
                    let cardinal = visible_direction as f32 * std::f32::consts::FRAC_PI_2;
                    for index in pair {
                        let angle: f32 = angles[index];
                        let x = center[0] + 64. * (cardinal.cos() * (1. - t) + angle.cos() * t);
                        let y = center[1] - 64. * (cardinal.sin() * (1. - t) + angle.sin() * t);
                        let u = (index % 5) as f32 * 50.;
                        let v = 156. + (index / 5) as f32 * 50.;
                        quads.push(quad(
                            [x - 25., y - 25., x + 25., y + 25.],
                            [u, v, u + 50., v + 50.],
                            alpha,
                        ));
                    }
                }
                atlas = Some(2);
            }
            Visual::NotifierKey if controller.active && !menu.visible && *notifier_alpha > 0. => {
                if let Some((glyph, _)) = notifier_cache.as_ref() {
                    let code = [0x324, 0x325, 0x326, 0x327][*glyph];
                    text = Some((
                        char::from_u32(code).unwrap().to_string(),
                        false,
                        true,
                        [notifier_origin[0] + 16., notifier_origin[1] + 3.],
                        *notifier_alpha,
                    ));
                }
            }
            Visual::NotifierBackground
                if controller.active && !menu.visible && *notifier_alpha > 0. =>
            {
                if let Some((_, label)) = notifier_cache.as_ref() {
                    let width = measure(&assets.body, label)[0];
                    let [x, y] = notifier_origin;
                    let tex = assets.sizes[5];
                    // Native VS adds Width only to vertices with x >= 40.
                    for (left, right, u, v) in [
                        (0., 32., 0., 0.5),
                        (32., 48. + width, 0.5, 0.75),
                        (48. + width, 64. + width, 0.75, 1.),
                    ] {
                        quads.push(quad(
                            [x + left, y, x + right, y + 32.],
                            [u * tex[0], 0., v * tex[0], tex[1]],
                            *notifier_alpha,
                        ));
                    }
                    atlas = Some(5);
                }
            }
            Visual::Notifier
                if !menu.visible && !play.session.input_limited() && tutorial.dialog.is_none() =>
            {
                let avatar = play.session.avatar();
                let body = play.session.body();
                let shield = &avatar.character.combat.health.shield;
                let gripped = avatar.character.grip.gripper.is_some();
                let boost = (shield.active() && !shield.solid())
                    || magicka_sim::shield_boost::shield_to_boost(
                        body.position,
                        body.direction(),
                        play.session.world.shields.iter().map(|s| &s.shield),
                    )
                    .is_some();
                if controller.active {
                    if let Some((_, label)) =
                        notifier_cache.as_ref().filter(|_| *notifier_alpha > 0.)
                    {
                        text = Some((
                            label.clone(),
                            false,
                            false,
                            [notifier_origin[0] + 34., notifier_origin[1] + 8.],
                            *notifier_alpha,
                        ));
                    }
                } else if gripped || (boost && !avatar.polymorphed) {
                    let label = localized(
                        &play,
                        if gripped {
                            "#interact_12"
                        } else {
                            "#interact_06"
                        },
                    );
                    let cursor = windows
                        .single()
                        .ok()
                        .and_then(Window::cursor_position)
                        .unwrap_or(Vec2::new(screen[0] * 0.5, screen[1] * 0.5));
                    text = Some((
                        format!("\u{32a} {label}"),
                        false,
                        false,
                        [cursor.x + 18., cursor.y + 20.],
                        1.,
                    ));
                }
            }
            Visual::Hint if tutorial.hint.is_some() && !menu.visible => {
                text = Some((
                    hint_text(
                        tutorial.hint.as_ref().unwrap(),
                        &assets,
                        controller.active.then_some(controller.element_style),
                    ),
                    false,
                    false,
                    [
                        hint_center[0] - hint_size[0] * 0.5,
                        hint_center[1] - hint_size[1] * 0.5,
                    ],
                    tutorial.hint_alpha,
                ));
            }
            Visual::Dialog if tutorial.dialog.is_some() && !menu.visible => {
                text = Some((
                    tutorial.dialog.as_ref().unwrap().text.clone(),
                    false,
                    false,
                    [
                        dialog_center[0] - dialog_size[0] * 0.5 + dialog_offset,
                        dialog_center[1] - dialog_size[1] * 0.5,
                    ],
                    tutorial.dialog_alpha,
                ));
            }
            Visual::Icons if tutorial.hint.is_some() && !menu.visible && !controller.active => {
                quads = icons.clone();
                atlas = Some(1);
            }
            Visual::Keys if !key.is_empty() && !menu.visible && !controller.active => {
                text = Some((
                    key.clone(),
                    false,
                    true,
                    [icon_origin[0], icon_origin[1] + 14.],
                    tutorial.hint_alpha,
                ));
            }
            Visual::Description if menu.visible && menu.page == 4 => {
                text = Some((
                    description.clone(),
                    false,
                    true,
                    [screen[0] * 0.5 + 150. * scale, 410. * scale],
                    1.,
                ));
            }
            Visual::Art if menu.visible && menu.page == 4 => {
                let (a, cell) = art(if unlocked { kind } else { MagickType::Grow });
                quads.push(quad(
                    [
                        screen[0] * 0.5 - 50. * scale,
                        150. * scale,
                        screen[0] * 0.5 + 350. * scale,
                        400. * scale,
                    ],
                    cell,
                    1.,
                ));
                atlas = Some(a);
            }
            Visual::Art if tutorial.dialog.is_some() && !menu.visible => {
                let h = tutorial.dialog.as_ref().unwrap();
                let s = h.scale.unwrap_or(1.);
                let x = dialog_center[0] - dialog_size[0] * 0.5;
                let y = dialog_center[1] - dialog_size[1] * 0.5;
                if h.magick != 0 {
                    let i = if h.magick >= 37 {
                        h.magick - 37
                    } else {
                        h.magick
                    };
                    let u = (i % 5) as f32 * 400.;
                    let v = (i / 5) as f32 * 250.;
                    quads.push(quad(
                        [x, y, x + 400. * s, y + 250. * s],
                        [u, v, u + 400., v + 250.],
                        tutorial.dialog_alpha,
                    ));
                    atlas = Some(if h.magick >= 37 { 4 } else { 3 });
                } else if h.element != 0 {
                    let i = h.element.trailing_zeros();
                    let u = (i % 5) as f32 * 50.;
                    let v = 156. + (i / 5) as f32 * 50.;
                    quads.push(quad(
                        [x, y, x + 50. * s, y + 50. * s],
                        [u, v, u + 50., v + 50.],
                        tutorial.dialog_alpha,
                    ));
                    atlas = Some(2);
                }
            }
            Visual::Elements if menu.visible && menu.page == 4 && unlocked => {
                for (i, element) in combo.iter().enumerate() {
                    let index = element.0.trailing_zeros();
                    let u = (index % 5) as f32 * 50.;
                    let v = 156. + (index / 5) as f32 * 50.;
                    let x = screen[0] * 0.5
                        + 150. * scale
                        + (i as f32 - combo.len() as f32 * 0.5) * 50. * scale;
                    quads.push(quad(
                        [x, 580. * scale, x + 50. * scale, 630. * scale],
                        [u, v, u + 50., v + 50.],
                        1.,
                    ));
                }
                atlas = Some(2);
            }
            _ => continue,
        }
        let prompts_before = prompts.0.len();
        let result = if let Some((value, menu_font, centered, position, alpha)) = text {
            let font = if menu_font {
                &assets.menu
            } else {
                &assets.body
            };
            transform.translation.x = position[0] - screen[0] * 0.5;
            transform.translation.y = screen[1] * 0.5 - position[1];
            if menu_font || matches!(visual, Visual::Description) {
                transform.scale = Vec3::splat(scale);
            }
            text_mesh(
                font,
                &value,
                centered,
                alpha,
                matches!(visual, Visual::Keys)
                    || match *visual {
                        Visual::Row(slot) if menu.visible => {
                            let index = if menu.page == 4 {
                                if slot == 12 {
                                    list.len()
                                } else {
                                    menu.scroll + slot
                                }
                            } else {
                                slot
                            };
                            index == menu.selected
                        }
                        _ => false,
                    },
                (controller.active && !matches!(visual, Visual::JoinText)).then_some((
                    &mut *prompts,
                    position,
                    if menu_font || matches!(visual, Visual::Description) {
                        scale
                    } else {
                        1.
                    },
                )),
            )
        } else {
            if let Some(a) = atlas {
                material.0 = assets.atlases[a].clone();
            }
            Ok(crate::play_hud::geometry(
                &quads,
                screen,
                atlas.map_or([1., 1.], |i| assets.sizes[i]),
            ))
        };
        if matches!(visual, Visual::NotifierKey) {
            // RenderData.Draw: GUIBasicEffect.Color = ButtonIntensity for the key.
            for prompt in &mut prompts.0[prompts_before..] {
                prompt.intensity = key_intensity;
            }
        }
        match result {
            Ok(value) => {
                if let Some(mut target) = meshes.get_mut(&mesh.0) {
                    *target = value;
                }
                *visibility = Visibility::Visible;
            }
            Err(e) => warn!("retail menu geometry: {e}"),
        }
    }
    for (panel, mesh, material, mut visibility) in &mut panels {
        let (active, center, size, alpha) = match panel {
            Panel::Hint => (
                tutorial.hint.is_some(),
                hint_center,
                hint_size,
                tutorial.hint_alpha * 0.5,
            ),
            Panel::Dialog => (
                tutorial.dialog.is_some(),
                dialog_center,
                dialog_size,
                tutorial.dialog_alpha,
            ),
        };
        *visibility = if active && !menu.visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if active {
            if let Some(mut target) = meshes.get_mut(&mesh.0) {
                *target = background_mesh(center, size, 1., screen);
            }
            if let Some(mut target) = materials.get_mut(&material.0) {
                target.color = Vec4::new(1., 1., 1., alpha);
            }
        }
    }
}
