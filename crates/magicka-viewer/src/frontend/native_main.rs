//! Retained original-atlas main page. The flat page host will also serve the
//! original 3D tome once its render-target/page geometry is connected.
use super::{activate_tile, Frontend};
use crate::frontend_tome::{MenuId, PagePointer, SheetRoot, TomeView, PAGE_LAYER};
use crate::{
    gui_basic::{GuiBasicMaterial, GuiBasicPlugin},
    native_text::{geometry_mesh, upload_atlas},
    text_box_visual::read_asset,
};
use bevy::camera::visibility::RenderLayers;
use bevy::{
    asset::RenderAssetUsages,
    image::{ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use magicka_content::{
    assets::{Asset, BitmapFont, Texture2D},
    texture::decode_rgba,
};
use magicka_runtime::{
    main_menu::{self, Atlas, Direction, TileId},
    play_content::{bitmap_font, resolve},
    scene_loading::custom_hash,
    text_geometry::{TextAlign, TextGeometry, TextStyle},
};
use std::path::Path;

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct NativeMenuSet;
#[derive(Resource)]
struct Loaded {
    pages: Texture2D,
    new_pages: Texture2D,
    store: Texture2D,
    paper: Texture2D,
    font: BitmapFont,
}
struct TileView {
    id: TileId,
    materials: Vec<Handle<GuiBasicMaterial>>,
    hovered: bool,
    pressed: bool,
    animation: [f32; 3],
}
#[derive(Resource)]
struct MenuView {
    tiles: Vec<main_menu::Tile>,
    views: Vec<TileView>,
    pointer: Option<Vec2>,
    pressed: Option<TileId>,
    /// The main page was the settled page last frame: a key that popped a
    /// submenu instantly this frame belongs to that submenu, not to us.
    settled: bool,
}

fn texture(root: &Path, name: &str) -> Result<Texture2D, String> {
    let path =
        resolve(root, Path::new(name)).ok_or_else(|| format!("missing menu texture {name}"))?;
    match read_asset(&path)? {
        Asset::Texture2D(t) => Ok(t),
        _ => Err(format!("expected menu texture {name}")),
    }
}
pub(super) fn install(app: &mut App, root: &str) -> Result<(), String> {
    let root = Path::new(root);
    let loaded = Loaded {
        pages: texture(root, "UI/Menu/pages.xnb")?,
        new_pages: texture(root, "UI/Spritesheets/pagesNew.xnb")?,
        store: texture(root, "UI/Store/Store_MainMenu.xnb")?,
        paper: texture(root, "UI/ToM/tome_pages_0.xnb")?,
        font: bitmap_font(root, "Languages/eng/Font/MenuOption")
            .map_err(|e| format!("menu font: {e:?}"))?,
    };
    if !app.is_plugin_added::<crate::menu_input::MenuInputPlugin>() {
        app.add_plugins(crate::menu_input::MenuInputPlugin);
    }
    app.add_plugins(GuiBasicPlugin)
        .insert_resource(loaded)
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (drive, animate)
                .chain()
                .in_set(NativeMenuSet)
                .after(crate::frontend_tome::TomeSet),
        );
    Ok(())
}
fn upload(texture: &Texture2D, images: &mut Assets<Image>) -> Handle<Image> {
    let data = decode_rgba(
        texture.format,
        texture.width as usize,
        texture.height as usize,
        &texture.mips[0],
    )
    .expect("menu texture decoding");
    let mut image = Image::new(
        Extent3d {
            width: texture.width as u32,
            height: texture.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    images.add(image)
}
fn quad(size: [f32; 2], uv: [f32; 4]) -> Mesh {
    let [w, h] = size;
    let [u, v, uw, vh] = uv;
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[0.0, 0.0, 0.0], [w, 0.0, 0.0], [w, -h, 0.0], [0.0, -h, 0.0]],
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[u, v], [u + uw, v], [u + uw, v + vh], [u, v + vh]],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0; 4]; 4]);
    mesh.insert_indices(Indices::U16(vec![0, 2, 1, 0, 3, 2]));
    mesh
}
fn material(texture: Handle<Image>) -> GuiBasicMaterial {
    super::book::page_material(texture, [1.0; 4], false)
}
fn setup(
    mut commands: Commands,
    loaded: Res<Loaded>,
    state: Res<Frontend>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<GuiBasicMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let pages = upload(&loaded.pages, &mut images);
    let new_pages = upload(&loaded.new_pages, &mut images);
    let store = upload(&loaded.store, &mut images);
    let paper = upload(&loaded.paper, &mut images);
    let (mut font, atlas) = upload_atlas(&loaded.font, &mut images).expect("menu font atlas");
    std::sync::Arc::make_mut(&mut font).metrics.spacing = 0;
    // The paper comes from the Tome's page target; tiles draw over it.
    let _ = paper;
    let root = commands
        .spawn((
            SheetRoot {
                menu: MenuId::Main,
                right: true,
                old: false,
            },
            Transform::default(),
            Visibility::Hidden,
        ))
        .id();
    let tiles = main_menu::tiles([loaded.store.width as f32, loaded.store.height as f32]);
    let mut views = Vec::new();
    for (order, tile) in tiles.iter().enumerate() {
        let (texture, size) = match tile.atlas {
            Atlas::Store => (
                store.clone(),
                [loaded.store.width as f32, loaded.store.height as f32],
            ),
            Atlas::Pages => (
                pages.clone(),
                [loaded.pages.width as f32, loaded.pages.height as f32],
            ),
            Atlas::PagesNew => (
                new_pages.clone(),
                [
                    loaded.new_pages.width as f32,
                    loaded.new_pages.height as f32,
                ],
            ),
        };
        let [x, y, w, h] = tile.source;
        let image_material = materials.add(material(texture));
        let parent = commands
            .spawn((
                Transform::from_xyz(
                    tile.position[0] - 512.0,
                    512.0 - tile.position[1],
                    1.0 + order as f32 * 0.1,
                ),
                Visibility::Inherited,
                ChildOf(root),
            ))
            .id();
        commands.spawn((
            Mesh2d(meshes.add(quad(
                [w, h],
                [x / size[0], y / size[1], w / size[0], h / size[1]],
            ))),
            MeshMaterial2d(image_material.clone()),
            Transform::default(),
            RenderLayers::layer(PAGE_LAYER),
            ChildOf(parent),
        ));
        let mut handles = vec![image_material];
        if let Some(key) = tile.label {
            let text = state
                .strings
                .get_string(custom_hash(key))
                .expect("localized menu label");
            let height = font
                .metrics
                .measure(&text, true)
                .expect("menu label metrics")[1];
            for (offset, color) in [
                (Vec3::new(2.0, -2.0, 0.01), [0.0, 0.0, 0.0, 0.85]),
                (Vec3::new(0.0, 0.0, 0.02), [0.8963, 0.8424, 0.9798, 1.0]),
            ] {
                let geometry = TextGeometry::build(
                    &font,
                    &text.encode_utf16().collect::<Vec<_>>(),
                    TextStyle {
                        align: TextAlign::Center,
                        default_color: color,
                        ..default()
                    },
                )
                .expect("menu text geometry");
                let mut font_material = material(atlas.clone());
                font_material.flags.y = 1;
                let font_material = materials.add(font_material);
                handles.push(font_material.clone());
                commands.spawn((
                    Mesh2d(
                        meshes.add(geometry_mesh(&geometry, geometry.quads.len() as i32).unwrap()),
                    ),
                    MeshMaterial2d(font_material),
                    RenderLayers::layer(PAGE_LAYER),
                    Transform::from_translation(
                        Vec3::new(
                            w * 0.5 + tile.label_offset[0],
                            -(h + tile.label_offset[1]) + height * 0.5,
                            0.0,
                        ) + offset,
                    ),
                    ChildOf(parent),
                ));
            }
        }
        views.push(TileView {
            id: tile.id,
            materials: handles,
            hovered: false,
            pressed: false,
            animation: [1.0, 1.0, 0.0],
        });
    }
    commands.insert_resource(MenuView {
        tiles,
        views,
        pointer: None,
        pressed: None,
        settled: false,
    });
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut view: ResMut<MenuView>,
    mut state: ResMut<Frontend>,
    mut tome: ResMut<TomeView>,
    input: Res<crate::menu_input::MenuInput>,
    mouse: Res<ButtonInput<MouseButton>>,
    pointer: Res<PagePointer>,
    page_input: Res<super::pages::PageInput>,
    mut cues: ResMut<super::sound::MenuCues>,
) {
    // Input belongs to the main page only while it is the settled page.
    let settled = tome.interactive()
        && tome.current() == Some(MenuId::Main)
        && matches!(state.session, super::Session::Idle);
    if !std::mem::replace(&mut view.settled, settled) || !settled {
        view.pressed = None;
        return;
    }
    if state.popup.is_some() {
        view.pressed = None;
        return;
    }
    let select = input.confirm;
    let back = input.back;
    {
        // Tome.MousePickTome on the right page, in its 1024-pixel sheet.
        let point = pointer
            .0
            .filter(|hit| hit.right_page)
            .map(|hit| Vec2::from(hit.position));
        let hovered = point.and_then(|p| main_menu::hit_test(&view.tiles, p.to_array()));
        // Mouse-move events come from the screen cursor: the picked page
        // point also drifts while the camera clip plays.
        if page_input.screen_moved
            || mouse.just_pressed(MouseButton::Left)
            || mouse.just_released(MouseButton::Left)
        {
            let before = state.cursor.selected;
            state.cursor.pointer(hovered);
            if hovered.is_some() && state.cursor.selected != before {
                cues.0.push("ui_menu_mouseover");
            }
        }
        view.pointer = point;
        if mouse.just_pressed(MouseButton::Left) {
            view.pressed = hovered;
        }
        let mut action = None;
        if mouse.just_released(MouseButton::Left) {
            if view.pressed.is_some() && hovered == view.pressed {
                action = hovered;
            }
            view.pressed = None;
        }
        for (pressed, direction) in input.direction.into_iter().zip([
            Direction::Up,
            Direction::Down,
            Direction::Left,
            Direction::Right,
        ]) {
            if pressed {
                state.cursor.navigate(direction);
            }
        }
        if select {
            action = state.cursor.select();
        }
        if back && state.cursor.back() {
            super::show_quit(&mut state);
        }
        if let Some(tile) = action {
            cues.0.push("ui_menu_select");
            activate_tile(&mut state, &mut tome, tile);
        }
    }
}

/// The authored menu_main.json colour clips, independent of input focus.
fn animate(
    mut view: ResMut<MenuView>,
    state: Res<Frontend>,
    mut materials: ResMut<Assets<GuiBasicMaterial>>,
    time: Res<Time>,
) {
    let pressed = view.pressed;
    for tile in &mut view.views {
        let hovered = state.cursor.selected == Some(tile.id);
        let down = pressed == Some(tile.id);
        // The four authored menu_main.json colour clips last 0.1 s.
        if down != tile.pressed {
            tile.animation = if down {
                [1.0, 200.0 / 255.0, 0.0]
            } else {
                [200.0 / 255.0, 1.0, 0.0]
            };
        } else if hovered != tile.hovered {
            tile.animation = if hovered {
                [1.0, 350.0 / 255.0, 0.0]
            } else {
                [350.0 / 255.0, 1.0, 0.0]
            };
        }
        tile.hovered = hovered;
        tile.pressed = down;
        tile.animation[2] += time.delta_secs();
        let [from, to, age] = tile.animation;
        let power = from + (to - from) * (age / 0.1).min(1.0);
        for handle in &tile.materials {
            materials.get_mut(handle).unwrap().color = Vec4::new(power, power, power, 1.0);
        }
    }
}
