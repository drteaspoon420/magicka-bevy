//! The 3D menu Tome: the desk (`UI/ToM/background_mesh`), the camera rig
//! (`background`), the book (`tome`), its two resting pages (`page`) and the
//! riffled pages (`page_flip`), with every submenu drawn into one 2048-square
//! page render target as the original does. Timing and the menu stack live in
//! `magicka_runtime::frontend_tome`; this module poses the converted models.
use bevy::{
    asset::embedded_asset,
    camera::{visibility::RenderLayers, ImageRenderTarget, RenderTarget},
    core_pipeline::tonemapping::Tonemapping,
    gltf::{Gltf, GltfAssetLabel},
    image::{ImageSampler, ImageSamplerDescriptor},
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    render::render_resource::{TextureFormat, TextureUsages},
    shader::ShaderRef,
    window::PrimaryWindow,
    world_serialization::{WorldAsset, WorldAssetRoot},
};
use magicka_runtime::frontend_tome::{
    pick, PageHit, Pose, Tome, TomeClips, TomeEvent, TomeState, LIGHT_AMBIENT, LIGHT_DIFFUSE,
    LIGHT_SPECULAR, MAX_PAGES,
};
use std::collections::HashMap;

/// Render layer of everything drawn onto the page sheets.
pub(crate) const PAGE_LAYER: usize = 20;
/// The Light joint's bind translation in the converted rig.
const LIGHT_POSITION: Vec3 = Vec3::new(-1.438, 8.539, 3.036);

/// The Tome's shadow-casting light, shown with the Tome only.
#[derive(Component)]
struct TomeShadowLight;

fn sync_shadow_light(
    view: Res<TomeView>,
    mut lights: Query<&mut Visibility, With<TomeShadowLight>>,
) {
    for mut visibility in &mut lights {
        *visibility = if view.visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}
/// Size of the page render target (Tome: tome_pages_0's size).
const TARGET: u32 = 2048;

#[derive(Clone, Copy, Debug, Default, ShaderType)]
struct TomeParams {
    light_position: Vec4,
    ambient: Vec4,
    diffuse: Vec4,
    specular: Vec4,
    /// DiffuseColor, a: diffuseIsHDR.
    material: Vec4,
    /// SpecularAmount, SpecularPower, specular map enabled.
    spec: Vec4,
}
/// SkinnedModelBasicEffect's NonDeffered technique (frontend_tome.wgsl).
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
struct TomeExtension {
    #[uniform(100)]
    params: TomeParams,
    #[texture(101)]
    #[sampler(102)]
    specular_map: Option<Handle<Image>>,
}
impl MaterialExtension for TomeExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://magicka_viewer/frontend_tome.wgsl".into()
    }
}
type TomeMaterial = ExtendedMaterial<StandardMaterial, TomeExtension>;

/// The submenus the Tome stacks. `Main` is SubMenuMain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MenuId {
    Main,
    SaveSlot,
    Character,
    Online,
    Options,
    Sound,
    Intro,
    EndGame,
    /// SubMenuCutscene: a chapter's campaign map.
    Cutscene,
    GameOptions,
    Graphics,
    Controls,
    Keyboard,
    Dlc,
    /// Challenge level selection (SubMenuCharacterSelect's level list).
    Challenge,
    /// Versus mode and map selection (the lobby's mode box and levels).
    Versus,
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct TomeSet;

/// Pick the current screen cursor before any menu consumes its movement edge.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct TomePointerSet;

/// The open book under the cursor, in pixels of a 1024-square page sheet.
#[derive(Resource, Default)]
pub(crate) struct PagePointer(pub Option<PageHit>);

/// A page's content root: placed on whichever sheet the Tome draws that
/// menu's page on, hidden otherwise. Children must carry [`PAGE_LAYER`].
#[derive(Component, Clone, Copy)]
pub(crate) struct SheetRoot {
    pub menu: MenuId,
    pub right: bool,
    /// The menu's previous content, shown while a riffle within the menu
    /// turns it away ([`magicka_runtime::frontend_tome::SheetDraw::old`]).
    pub old: bool,
}

/// Pending submenu navigation, applied when the book accepts it (the
/// original's PushMenu/PopMenu are ignored while a page turns).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Nav {
    Push(MenuId, usize),
    Pop,
}

#[derive(Resource, Default)]
pub(crate) struct TomeView {
    pub tome: Option<Tome<MenuId>>,
    pub nav: Vec<Nav>,
    pub events: Vec<TomeEvent>,
    /// Hidden while a play session owns the window.
    pub visible: bool,
    /// `--menu-auto-open`: audit instrumentation that presses confirm once the
    /// book accepts it.
    pub auto_open: bool,
    rng: u32,
}
impl TomeView {
    pub(crate) fn push(&mut self, menu: MenuId, pages: usize) {
        self.nav.push(Nav::Push(menu, pages));
    }
    pub(crate) fn pop(&mut self) {
        self.nav.push(Nav::Pop);
    }
    pub(crate) fn current(&self) -> Option<MenuId> {
        self.tome.as_ref().and_then(Tome::current)
    }
    /// The book is open on a settled page that accepts input.
    pub(crate) fn interactive(&self) -> bool {
        self.tome
            .as_ref()
            .is_some_and(|t| t.state == TomeState::Open && !t.input_locked())
            && self.nav.is_empty()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Model {
    Desk,
    Rig,
    Book,
    Page,
    Flip,
}
const MODELS: [(Model, &str); 5] = [
    (Model::Desk, "gltf/UI/ToM/background_mesh.glb"),
    (Model::Rig, "gltf/UI/ToM/background.glb"),
    (Model::Book, "gltf/UI/ToM/tome.glb"),
    (Model::Page, "gltf/UI/ToM/page.glb"),
    (Model::Flip, "gltf/UI/ToM/page_flip.glb"),
];

/// Which controller poses a spawned model instance.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
enum Part {
    Desk,
    Rig,
    Book,
    LeftPage,
    RightPage,
    Flip(usize),
}
impl Part {
    /// Model whose clips pose this part: the resting pages use page_flip's
    /// open/close clips on the `page` mesh (same skeleton), as the original.
    fn clips(self) -> Model {
        match self {
            Part::Desk => Model::Desk,
            Part::Rig => Model::Rig,
            Part::Book => Model::Book,
            Part::LeftPage | Part::RightPage | Part::Flip(_) => Model::Flip,
        }
    }
}

#[derive(Component)]
struct PartAnimator(Entity);
#[derive(Component)]
struct TomeCamera;
#[derive(Component)]
struct PageCamera;

type Graph = (Handle<AnimationGraph>, HashMap<String, AnimationNodeIndex>);
#[derive(Resource)]
struct Assets3d {
    gltf: HashMap<Model, Handle<Gltf>>,
    graphs: HashMap<Model, Graph>,
    camera_bind: Option<(Entity, Mat4)>,
    prepared: Vec<Entity>,
    page_texture: Handle<Image>,
    /// One material per converted glTF material (and the page target's).
    materials: HashMap<AssetId<StandardMaterial>, Handle<TomeMaterial>>,
}

pub(crate) fn install(app: &mut App, content: &str, args: &[String]) -> Result<(), String> {
    let paper = crate::text_box_visual::read_asset(
        &magicka_runtime::play_content::resolve(
            std::path::Path::new(content),
            std::path::Path::new("UI/ToM/tome_pages_0.xnb"),
        )
        .ok_or("UI/ToM/tome_pages_0 missing")?,
    )?;
    let magicka_content::assets::Asset::Texture2D(paper) = paper else {
        return Err("expected UI/ToM/tome_pages_0 texture".into());
    };
    embedded_asset!(app, "frontend_tome.wgsl");
    app.add_plugins(MaterialPlugin::<TomeMaterial>::default());
    app.insert_resource(Paper(paper))
        .insert_resource(TomeView {
            tome: None,
            nav: Vec::new(),
            events: Vec::new(),
            visible: true,
            auto_open: args.iter().any(|a| a == "--menu-auto-open"),
            rng: 0x2545_f491,
        })
        .init_resource::<PagePointer>()
        // GlobalTransform is the camera from the last rendered frame, matching
        // the book the user sees. Pick before collecting this frame's input;
        // otherwise screen_moved selects the previous page point for one frame.
        .add_systems(Update, point.in_set(TomePointerSet).before(TomeSet))
        .add_systems(Startup, setup)
        .add_systems(Update, sync_shadow_light)
        .add_systems(
            Update,
            (
                prepare.before(crate::lighting::apply_lit_materials),
                drive,
                place_sheets,
                pose,
            )
                .chain()
                .in_set(TomeSet),
        );
    Ok(())
}

#[derive(Resource)]
struct Paper(magicka_content::assets::Texture2D);

// The camera X coordinate -3.14 is a placement, not an approximation of π.
#[allow(clippy::approx_constant)]
fn setup(
    mut commands: Commands,
    assets: Res<AssetServer>,
    paper: Res<Paper>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut gui: ResMut<Assets<crate::gui_basic::GuiBasicMaterial>>,
) {
    let mut target = Image::new_target_texture(TARGET, TARGET, TextureFormat::Rgba8UnormSrgb, None);
    target.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;
    target.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    let target = images.add(target);
    // PreRenderUpdate: the paper fills the target, then the menus draw.
    commands.spawn((
        PageCamera,
        Camera2d,
        RenderLayers::layer(PAGE_LAYER),
        Camera {
            order: -10,
            // The paper's alpha (its torn edges) must survive into the
            // target: the page meshes blend with it.
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(ImageRenderTarget::from(target.clone())),
    ));
    // Paper with ScaleToHDR, like every GUI draw into the target.
    let paper = images.add(crate::frontend::book::upload(&paper.0).expect("tome paper"));
    commands.spawn((
        Mesh2d(meshes.add(crate::frontend::book::quad(
            [TARGET as f32; 2],
            [0.0, 0.0, 1.0, 1.0],
        ))),
        MeshMaterial2d(gui.add(crate::frontend::book::page_material(paper, [1.0; 4], false))),
        Transform::from_xyz(-(TARGET as f32) / 2.0, TARGET as f32 / 2.0, -100.0),
        RenderLayers::layer(PAGE_LAYER),
    ));
    let mut gltf = HashMap::new();
    for (model, path) in MODELS {
        gltf.insert(model, assets.load(path));
    }
    let mut spawn = |part: Part, path: &str| {
        commands.spawn((
            part,
            Transform::default(),
            Visibility::Hidden,
            WorldAssetRoot(
                assets.load::<WorldAsset>(GltfAssetLabel::Scene(0).from_asset(path.to_string())),
            ),
        ));
    };
    spawn(Part::Desk, MODELS[0].1);
    spawn(Part::Rig, MODELS[1].1);
    spawn(Part::Book, MODELS[2].1);
    spawn(Part::LeftPage, MODELS[3].1);
    spawn(Part::RightPage, MODELS[3].1);
    for i in 0..MAX_PAGES {
        spawn(Part::Flip(i), MODELS[4].1);
    }
    // RenderData.DrawShadows: a shadow map from the Light joint toward
    // (-2, 0, 2.5), CreatePerspectiveFieldOfView(1.885, 1, 1, 15). The
    // Tome shader samples it as the NonDeffered shadow term; the light adds
    // no lighting of its own (the material lights itself).
    commands.spawn((
        TomeShadowLight,
        SpotLight {
            intensity: 1.0,
            range: 15.0,
            radius: 0.0,
            shadow_maps_enabled: true,
            shadow_map_near_z: 1.0,
            outer_angle: 1.885 * 0.5,
            inner_angle: 1.885 * 0.5,
            ..default()
        },
        Transform::from_translation(LIGHT_POSITION).looking_at(Vec3::new(-2.0, 0.0, 2.5), Vec3::Y),
    ));
    commands.spawn((
        TomeCamera,
        Camera3d::default(),
        Camera {
            order: 0,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        // Tome.Update: CreatePerspectiveFieldOfView(0.75, aspect, 2, 200).
        Projection::Perspective(PerspectiveProjection {
            fov: 0.75,
            near: 2.0,
            far: 200.0,
            ..default()
        }),
        // RenderManager: the scene buffer holds 1 - 2^-x; FilterEffect here,
        // bloom and the decode on the menu overlay camera.
        Tonemapping::None,
        crate::post::MagickaFilterPass,
        crate::post::MagickaPost::default(),
        Transform::from_xyz(-3.14, 8.57, -0.31).looking_to(Vec3::NEG_Y, Vec3::Z),
    ));
    commands.insert_resource(Assets3d {
        gltf,
        graphs: HashMap::new(),
        camera_bind: None,
        prepared: Vec::new(),
        page_texture: target,
        materials: HashMap::new(),
    });
}

/// Build clip graphs, the Tome state once durations are known, page
/// materials, animators and the camera joint's bind pose.
#[allow(clippy::too_many_arguments)]
fn prepare(
    mut commands: Commands,
    mut store: ResMut<Assets3d>,
    mut view: ResMut<TomeView>,
    gltfs: Res<Assets<Gltf>>,
    clips: Res<Assets<AnimationClip>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    standard: Res<Assets<StandardMaterial>>,
    mut materials: ResMut<Assets<TomeMaterial>>,
    server: Res<AssetServer>,
    bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    parts: Query<(Entity, &Part)>,
    children: Query<&Children>,
    meshes: Query<
        (
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&SkinnedMesh>,
            Option<&bevy::gltf::GltfMaterialExtras>,
        ),
        With<Mesh3d>,
    >,
    players: Query<(), With<AnimationPlayer>>,
    names: Query<&Name>,
) {
    for (model, handle) in store.gltf.clone() {
        if store.graphs.contains_key(&model) {
            continue;
        }
        let Some(gltf) = gltfs.get(&handle) else {
            continue;
        };
        let mut graph = AnimationGraph::new();
        let nodes = gltf
            .named_animations
            .iter()
            .map(|(name, clip)| {
                (
                    name.to_string(),
                    graph.add_clip(clip.clone(), 1.0, graph.root),
                )
            })
            .collect();
        store.graphs.insert(model, (graphs.add(graph), nodes));
    }
    if view.tome.is_none() {
        let clips = &*clips;
        let duration = |model: Model| {
            let gltf = store.gltf.get(&model).and_then(|h| gltfs.get(h));
            move |name: &str| {
                gltf?
                    .named_animations
                    .get(name)
                    .and_then(|h| clips.get(h))
                    .map(AnimationClip::duration)
            }
        };
        if [Model::Rig, Model::Book, Model::Flip]
            .iter()
            .all(|m| store.gltf.get(m).is_some_and(|h| gltfs.get(h).is_some()))
        {
            match TomeClips::from_lookup(
                duration(Model::Rig),
                duration(Model::Book),
                duration(Model::Flip),
            ) {
                Ok(c) => view.tome = Some(Tome::new(c, MenuId::Main)),
                Err(e) => error!("menu tome: {e}"),
            }
        }
    }
    for (root, part) in &parts {
        if store.prepared.contains(&root) {
            continue;
        }
        let Some(graph) = store.graphs.get(&part.clips()).map(|g| g.0.clone()) else {
            continue;
        };
        let descendants: Vec<Entity> = children.iter_descendants(root).collect();
        if descendants.iter().all(|e| meshes.get(*e).is_err()) {
            continue;
        }
        for &entity in &descendants {
            if players.contains(entity) {
                commands
                    .entity(entity)
                    .insert((PartAnimator(root), AnimationGraphHandle(graph.clone())));
            }
            let Ok((material, skin, extras)) = meshes.get(entity) else {
                continue;
            };
            if *part == Part::Rig {
                // The rig only supplies the camera and light joints.
                commands.entity(entity).insert(Visibility::Hidden);
                if let Some(skin) = skin {
                    let bind = bindposes.get(&skin.inverse_bindposes);
                    for (i, joint) in skin.joints.iter().enumerate() {
                        if names.get(*joint).is_ok_and(|n| n.as_str() == "camera") {
                            if let Some(ibm) = bind.and_then(|b| b.get(i)) {
                                store.camera_bind = Some((*joint, *ibm));
                            }
                        }
                    }
                }
                continue;
            }
            let Some(material) = material else { continue };
            let page = matches!(part, Part::LeftPage | Part::RightPage | Part::Flip(_));
            let key = if page {
                store.page_texture.id().untyped().typed_unchecked()
            } else {
                material.0.id()
            };
            let handle = if let Some(handle) = store.materials.get(&key) {
                handle.clone()
            } else {
                let Some(source) = standard.get(&material.0) else {
                    continue;
                };
                let skinned = extras
                    .and_then(|e| serde_json::from_str::<serde_json::Value>(&e.value).ok())
                    .and_then(|j| j.get("magicka_skinned").cloned())
                    .unwrap_or_default();
                let f = |v: &serde_json::Value, i: usize, d: f32| {
                    v.get(i).and_then(|x| x.as_f64()).map_or(d, |x| x as f32)
                };
                let diffuse = &skinned["diffuse"];
                let spec = &skinned["spec"];
                let specular_map = skinned["material_enabled"]
                    .as_bool()
                    .unwrap_or(false)
                    .then(|| skinned["material"].as_str())
                    .flatten()
                    .map(|uri| {
                        let path = MODELS
                            .iter()
                            .find(|(m, _)| *m == if page { Model::Page } else { part.clips() })
                            .map_or("gltf/UI/ToM/tome.glb", |(_, p)| p);
                        server.load(crate::terrain::resolve(path, uri))
                    });
                let mut base = source.clone();
                base.base_color = Color::WHITE;
                if page {
                    // Every resting/riffled page samples the page target.
                    base.base_color_texture = Some(store.page_texture.clone());
                    // XNA culls counter-clockwise faces: each page is two
                    // single-sided faces (TOMELEFT up, TOMERIGHT down);
                    // drawing both shows the hidden sheet mirrored.
                    base.cull_mode = Some(bevy::render::render_resource::Face::Back);
                    // RenderData.Draw blends the background and riffled
                    // pages (with depth writes, in draw order) so the torn
                    // paper edges show the book. A cutout keeps the depth
                    // writes: the skinned pages share one origin, so a
                    // sorted blend pass cannot order them.
                    base.alpha_mode = AlphaMode::Mask(0.5);
                }
                let handle = materials.add(TomeMaterial {
                    base,
                    extension: TomeExtension {
                        params: TomeParams {
                            material: Vec4::new(
                                f(diffuse, 0, 1.0),
                                f(diffuse, 1, 1.0),
                                f(diffuse, 2, 1.0),
                                // RenderData.Draw sets DiffuseIsHDR on the
                                // page models: the target holds the GUI's
                                // 1 - 2^-x output (book::page_material).
                                if page { 1.0 } else { 0.0 },
                            ),
                            spec: Vec4::new(
                                f(spec, 0, 0.0),
                                f(spec, 1, 1.0),
                                if specular_map.is_some() { 1.0 } else { 0.0 },
                                0.0,
                            ),
                            ..default()
                        },
                        specular_map,
                    },
                });
                store.materials.insert(key, handle.clone());
                handle
            };
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert(MeshMaterial3d(handle));
        }
        if *part == Part::Rig && store.camera_bind.is_none() {
            continue;
        }
        store.prepared.push(root);
    }
}

fn drive(
    mut view: ResMut<TomeView>,
    time: Res<Time>,
    menu_input: Res<crate::menu_input::MenuInput>,
    mouse: Res<ButtonInput<MouseButton>>,
) {
    let view = &mut *view;
    let Some(tome) = view.tome.as_mut() else {
        return;
    };
    if !view.visible {
        return;
    }
    // Tome.ControllerA while closed (any confirm, or a click on the desk).
    if tome.state == TomeState::Closed || tome.state == TomeState::ClosedBack {
        let pressed = menu_input.confirm || mouse.just_released(MouseButton::Left);
        if pressed || (view.auto_open && tome.intro_time() > 1.1) {
            tome.press_open();
        }
    }
    let mut remaining = Vec::new();
    for nav in view.nav.drain(..) {
        let accepted = remaining.is_empty()
            && match nav {
                Nav::Push(menu, pages) => tome.push(menu, pages),
                Nav::Pop => tome.pop(),
            };
        if !accepted {
            remaining.push(nav);
        }
    }
    view.nav = remaining;
    view.rng ^= view.rng << 13;
    view.rng ^= view.rng >> 17;
    view.rng ^= view.rng << 5;
    let random = (view.rng >> 8) as f32 / (1u32 << 24) as f32;
    tome.update(time.delta_secs(), random);
    let events = tome.take_events();
    // Tome.OnBackClose (SubMenuEndGame after the credits): PopMenuInstant
    // down to SubMenuMain.
    if events.contains(&TomeEvent::BackClosed) {
        while tome.depth() > 1 {
            tome.pop_instant();
        }
    }
    view.events.extend(events);
}

/// Put each menu's page roots on the sheets the Tome draws them on.
fn place_sheets(
    view: Res<TomeView>,
    mut roots: Query<(&SheetRoot, &mut Transform, &mut Visibility)>,
) {
    let draws = view.tome.as_ref().map(Tome::draws).unwrap_or_default();
    for (root, mut transform, mut visibility) in &mut roots {
        let sheet = draws
            .iter()
            .find(|d| d.menu == root.menu && d.old == root.old)
            .map(|d| if root.right { d.right } else { d.left });
        match sheet.filter(|_| view.visible) {
            Some(sheet) => {
                let [x, y] = sheet.origin();
                // Sheet pixel (0, 0) is its top-left; the page camera's
                // world origin is the target's centre with y up.
                transform.translation.x = x + 512.0 - TARGET as f32 / 2.0;
                transform.translation.y = TARGET as f32 / 2.0 - (y + 512.0);
                *visibility = Visibility::Inherited;
            }
            None => *visibility = Visibility::Hidden,
        }
    }
}

fn apply(
    player: &mut AnimationPlayer,
    nodes: &HashMap<String, AnimationNodeIndex>,
    pose: Option<Pose>,
) {
    let mut want: Vec<(AnimationNodeIndex, f32, f32)> = Vec::new();
    if let Some(pose) = pose {
        let blend = pose.toward.map_or(0.0, |(_, a)| a);
        if let Some(&node) = nodes.get(pose.clip) {
            want.push((node, pose.time, 1.0 - blend));
        }
        if let Some((clip, amount)) = pose.toward {
            if let Some(&node) = nodes.get(clip) {
                want.push((node, 0.0, amount));
            }
        }
    }
    let stale: Vec<AnimationNodeIndex> = player
        .playing_animations()
        .map(|(node, _)| *node)
        .filter(|node| !want.iter().any(|(n, _, _)| n == node))
        .collect();
    for node in stale {
        player.stop(node);
    }
    for (node, time, weight) in want {
        player.play(node).seek_to(time).set_weight(weight).pause();
    }
}

#[allow(clippy::too_many_arguments)]
fn pose(
    view: Res<TomeView>,
    store: Res<Assets3d>,
    mut animators: Query<(&PartAnimator, &mut AnimationPlayer)>,
    mut parts: Query<(&Part, &mut Visibility)>,
    globals: Query<&GlobalTransform>,
    mut camera: Query<(&mut Transform, &mut Camera), With<TomeCamera>>,
    mut page_camera: Query<&mut Camera, (With<PageCamera>, Without<TomeCamera>)>,
    mut materials: ResMut<Assets<TomeMaterial>>,
) {
    let Some(tome) = view.tome.as_ref() else {
        return;
    };
    for (animator, mut player) in &mut animators {
        let Ok((part, _)) = parts.get(animator.0) else {
            continue;
        };
        let Some((_, nodes)) = store.graphs.get(&part.clips()) else {
            continue;
        };
        let pose = match *part {
            Part::Rig => tome.camera.pose(),
            Part::Book => tome.book.pose(),
            Part::LeftPage => tome.left_page.pose(),
            Part::RightPage => tome.right_page.pose(),
            Part::Flip(i) => tome.riffle.pages().get(i).and_then(|c| c.pose()),
            Part::Desk => None,
        };
        apply(&mut player, nodes, pose);
    }
    for (part, mut visibility) in &mut parts {
        let shown = view.visible
            && store.prepared.len() >= 5 + MAX_PAGES
            && match part {
                Part::Flip(i) => tome.riffling && *i < tome.riffle.pages().len(),
                _ => true,
            };
        *visibility = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for mut camera in &mut page_camera {
        camera.is_active = view.visible;
    }
    let Ok((mut transform, mut camera)) = camera.single_mut() else {
        return;
    };
    camera.is_active = view.visible;
    // Tome.Update: view = inverse(cameraBindPose * skinned camera bone),
    // where cameraBindPose = inverse(IBP) * RotationY(pi).
    if let Some((joint, ibm)) = store.camera_bind {
        if let Ok(global) = globals.get(joint) {
            let rotate = Mat4::from_rotation_y(std::f32::consts::PI);
            let world = global.to_matrix() * ibm * rotate * ibm.inverse();
            *transform = Transform::from_matrix(world);
        }
    }
    // RenderData light colours: base colour * (intensity + flicker) * 100,
    // at sLightPosition (the rig Light joint's bind translation).
    let factor = tome.light.factor();
    let colour = |c: [f32; 3]| (Vec3::from_array(c) * factor).extend(0.0);
    for handle in store.materials.values() {
        if let Some(mut material) = materials.get_mut(handle) {
            let params = &mut material.extension.params;
            params.light_position = LIGHT_POSITION.extend(1.0);
            params.ambient = colour(LIGHT_AMBIENT);
            params.diffuse = colour(LIGHT_DIFFUSE);
            params.specular = colour(LIGHT_SPECULAR);
        }
    }
}

/// MousePickTome: unproject the cursor at the far plane and intersect the
/// two page planes.
fn point(
    mut pointer: ResMut<PagePointer>,
    view: Res<TomeView>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<TomeCamera>>,
) {
    pointer.0 = None;
    if !view.visible
        || view
            .tome
            .as_ref()
            .is_none_or(|t| t.state != TomeState::Open)
    {
        return;
    }
    let (Ok(window), Ok((camera, global))) = (windows.single(), camera.single()) else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(global, cursor) else {
        return;
    };
    let far = ray.origin + *ray.direction * 200.0;
    pointer.0 = pick(ray.origin.to_array(), far.to_array());
}
