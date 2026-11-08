//! Filesystem loading for a play session: the `.lvl` Start element, scene XML
//! model/kill-plane fields and decoded LevelModel/CharacterTemplate XNBs.
//! See research/play-session.md.
use crate::{scene_loading::custom_hash, scene_script::xml};
use magicka_content::{character::CharacterTemplate, level::LevelModel, parse_primary, Content};
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub enum PlayContentError {
    Io(PathBuf, std::io::Error),
    Xml(PathBuf, String),
    Xnb(PathBuf, String),
    MissingStart(PathBuf),
    MissingModel(PathBuf),
    UnsupportedName(String),
    WrongContent(PathBuf),
}

/// Level.ctor `Start` attributes. Scene is hashed lowercase; Area locations
/// hash `Area + index` *without* lowercasing (case-sensitive, like the
/// locator dictionary keys), so they match only exact locator names.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelStart {
    pub scene: String,
    pub scene_hash: i32,
    pub area: Option<String>,
    pub locations: [i32; 4],
    pub spawn_players: bool,
    /// Level.SpawnFairy (`spawnFairy` on Start; the constructor defaults it
    /// to true).
    pub spawn_fairy: bool,
    /// Level.NoItems (`noItems` on Start, default false): the first scene of
    /// a level start leaves every avatar with weapon_unarmed in both slots.
    pub no_items: bool,
    /// `Scenes/Scene` paths relative to the level file directory.
    pub scenes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneFile {
    /// Lowercase file stem (GameScene.mName).
    pub name: String,
    pub path: PathBuf,
    /// Scene directory joined with the `Model` element text.
    pub model: PathBuf,
    /// `KillPlanePosition` (GameScene default -50).
    pub kill_plane: f32,
    /// `Filter` and `Bloom` RenderManager settings.
    pub post: ScenePost,
    /// `Indoor` (GameScene.Indoors; Rain refuses indoors).
    pub indoor: bool,
}

/// GameScene Filter (Brightness, Contrast, Saturation) and Bloom (Threshold,
/// Multiplier/Multiplyer, Blur) attributes, applied to RenderManager when
/// the scene starts (GameScene IL around 0x1539).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScenePost {
    pub fog: Option<SceneFog>,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub bloom_threshold: f32,
    pub bloom_multiplier: f32,
    pub blur_sigma: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct SceneFog {
    pub start: f32,
    pub end: f32,
    pub color: [f32; 4],
}
impl Default for ScenePost {
    fn default() -> Self {
        Self {
            fog: None,
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            bloom_threshold: 0.8,
            bloom_multiplier: 1.0,
            blur_sigma: 2.5,
        }
    }
}
impl ScenePost {
    /// Parse one `Filter` or `Bloom` element; other elements are ignored.
    /// Attribute names compare case-insensitively (OrdinalIgnoreCase).
    pub fn apply(&mut self, node: &crate::scene_script::XmlElement) -> Result<(), String> {
        if node.name.eq_ignore_ascii_case("fog") {
            let fog = self.fog.get_or_insert_with(SceneFog::default);
            for (name, value) in &node.attributes {
                match name.to_ascii_lowercase().as_str() {
                    "start" => fog.start = value.trim().parse().map_err(|_| "invalid fog start")?,
                    "end" => fog.end = value.trim().parse().map_err(|_| "invalid fog end")?,
                    "color" => {
                        let parts = value
                            .split(',')
                            .map(|v| v.trim().parse::<f32>())
                            .collect::<Result<Vec<_>, _>>()
                            .map_err(|_| "invalid fog color")?;
                        if parts.len() != 3 && parts.len() != 4 {
                            return Err("invalid fog color length".into());
                        }
                        fog.color[..3].copy_from_slice(&parts[..3]);
                        fog.color[3] = parts.get(3).copied().unwrap_or(1.);
                    }
                    _ => {}
                }
            }
            return Ok(());
        }
        let filter = node.name.eq_ignore_ascii_case("filter");
        let bloom = node.name.eq_ignore_ascii_case("bloom");
        for (name, value) in &node.attributes {
            let n = name.to_ascii_lowercase();
            let target = match (filter, bloom, n.as_str()) {
                (true, _, "brightness") => &mut self.brightness,
                (true, _, "contrast") => &mut self.contrast,
                (true, _, "saturation") => &mut self.saturation,
                (_, true, "threshold") => &mut self.bloom_threshold,
                (_, true, "multiplyer" | "multiplier") => &mut self.bloom_multiplier,
                (_, true, "blur") => &mut self.blur_sigma,
                _ => continue,
            };
            *target = value
                .trim()
                .parse()
                .map_err(|_| format!("invalid {} {name}={value}", node.name))?;
        }
        Ok(())
    }
}

/// Resolve a relative path component-wise without regard to case, as the
/// original's Windows file system does. Exact matches win.
pub fn resolve(root: &Path, relative: &Path) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            path.push(component);
            continue;
        };
        let exact = path.join(name);
        if exact.exists() {
            path = exact;
            continue;
        }
        let wanted = name.to_string_lossy().to_lowercase();
        let found = std::fs::read_dir(&path)
            .ok()?
            .flatten()
            .find(|entry| entry.file_name().to_string_lossy().to_lowercase() == wanted)?;
        path = found.path();
    }
    Some(path)
}

fn read(path: &Path) -> Result<Vec<u8>, PlayContentError> {
    std::fs::read(path).map_err(|e| PlayContentError::Io(path.into(), e))
}

fn lower_ascii(value: &str) -> Result<String, PlayContentError> {
    if !value.is_ascii() {
        return Err(PlayContentError::UnsupportedName(value.into()));
    }
    Ok(value.to_ascii_lowercase())
}

pub fn read_level(path: &Path) -> Result<LevelStart, PlayContentError> {
    let root =
        xml(&read(path)?).map_err(|e| PlayContentError::Xml(path.into(), format!("{e:?}")))?;
    let mut start = None;
    let mut scenes = Vec::new();
    for node in &root.children {
        if node.name.eq_ignore_ascii_case("start") {
            let mut scene = String::new();
            let mut area = None;
            // Level.ctor initialises SpawnPlayers = true before parsing.
            let mut spawn_players = true;
            let mut spawn_fairy = true;
            let mut no_items = false;
            for (key, value) in &node.attributes {
                match key.to_ascii_lowercase().as_str() {
                    "scene" => scene = value.clone(),
                    "area" => area = Some(value.clone()),
                    "spawnplayers" => {
                        spawn_players = value.trim().eq_ignore_ascii_case("true");
                    }
                    "spawnfairy" => {
                        spawn_fairy = value.trim().eq_ignore_ascii_case("true");
                    }
                    "noitems" => no_items = value.trim().eq_ignore_ascii_case("true"),
                    _ => {}
                }
            }
            start = Some((scene, area, spawn_players, spawn_fairy, no_items));
        } else if node.name.eq_ignore_ascii_case("scenes") {
            scenes.extend(
                node.children
                    .iter()
                    .map(|s| s.inner_text.trim().to_string()),
            );
        }
    }
    let (scene, area, spawn_players, spawn_fairy, no_items) =
        start.ok_or_else(|| PlayContentError::MissingStart(path.into()))?;
    let scene_hash = custom_hash(&lower_ascii(&scene)?);
    let locations = match &area {
        Some(area) => std::array::from_fn(|i| custom_hash(&format!("{area}{i}"))),
        None => [0; 4],
    };
    Ok(LevelStart {
        scene,
        scene_hash,
        area,
        locations,
        spawn_players,
        spawn_fairy,
        no_items,
        scenes,
    })
}

pub fn read_scene(path: &Path) -> Result<SceneFile, PlayContentError> {
    let root =
        xml(&read(path)?).map_err(|e| PlayContentError::Xml(path.into(), format!("{e:?}")))?;
    let directory = path.parent().unwrap_or(Path::new(""));
    let mut model = None;
    let mut kill_plane = -50.0;
    let mut indoor = false;
    let mut post = ScenePost::default();
    for node in &root.children {
        post.apply(node)
            .map_err(|e| PlayContentError::Xml(path.into(), e))?;
        if node.name.eq_ignore_ascii_case("model") {
            model = Some(directory.join(node.inner_text.trim().replace('\\', "/")));
        } else if node.name.eq_ignore_ascii_case("indoor") {
            // bool.Parse: "true"/"false" in any case, surrounding whitespace.
            indoor = match node.inner_text.trim().to_ascii_lowercase().as_str() {
                "true" => true,
                "false" => false,
                _ => {
                    return Err(PlayContentError::Xml(path.into(), "invalid Indoor".into()));
                }
            };
        } else if node.name.eq_ignore_ascii_case("killplaneposition") {
            kill_plane = node.inner_text.trim().parse().map_err(|_| {
                PlayContentError::Xml(path.into(), "invalid KillPlanePosition".into())
            })?;
        }
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(SceneFile {
        name: lower_ascii(&stem)?,
        path: path.into(),
        model: model.ok_or_else(|| PlayContentError::MissingModel(path.into()))?,
        kill_plane,
        post,
        indoor,
    })
}

fn parse(path: &Path) -> Result<Content, PlayContentError> {
    let bytes = read(path)?;
    let xnb = magicka_xnb::xnb::parse(&bytes)
        .map_err(|e| PlayContentError::Xnb(path.into(), format!("{e:?}")))?;
    let parsed = parse_primary(&xnb)
        .ok_or_else(|| PlayContentError::Xnb(path.into(), "null primary".into()))?
        .map_err(|e| PlayContentError::Xnb(path.into(), format!("{e}")))?;
    Ok(parsed.content)
}

/// Physics template keys with this prefix name Data/AnimatedProps templates.
pub const ANIMATED_PROPS: &str = "animatedprops/";

pub fn load_physics_template(
    content: &Path,
    name: &str,
) -> Result<magicka_content::physics::PhysicsEntityTemplate, PlayContentError> {
    let (dir, name) = match name.strip_prefix(ANIMATED_PROPS) {
        Some(rest) => ("Data/AnimatedProps", rest),
        None => ("Data/PhysicsEntities", name),
    };
    let relative = Path::new(dir)
        .join(name.replace('\\', "/"))
        .with_extension("xnb");
    let path = resolve(content, &relative).unwrap_or_else(|| content.join(&relative));
    match parse(&path)? {
        Content::PhysicsEntity(t) => Ok(*t),
        _ => Err(PlayContentError::Xnb(
            path,
            "expected physics entity template".into(),
        )),
    }
}

/// Pickable collision bounds from mesh 0 / part 0 position data. The native
/// constructors use those model vertices, without model-bone transforms.
pub fn pickup_model_bounds(content: &Path, model: &Path) -> Option<crate::pickups::PickupBounds> {
    let path = resolve(content, &model.with_extension("xnb"))?;
    let Content::Xna(object) = parse(&path).ok()? else {
        return None;
    };
    let magicka_content::xna::XnaObject::Model(model) = *object else {
        return None;
    };
    let mesh = model.meshes.first()?;
    let part = mesh.parts.first()?;
    let decl = model
        .vertex_declarations
        .get(usize::try_from(part.vertex_declaration).ok()?)?;
    let stride = magicka_content::scene::decl_stride(decl);
    let positions = magicka_content::mesh::positions(decl, mesh.vertex_buffer.as_ref()?, stride)?;
    let count = usize::try_from(part.num_vertices).ok()?;
    let points = positions.get(..count)?;
    if points.is_empty() {
        return None;
    }
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for point in points {
        for i in 0..3 {
            if !point[i].is_finite() {
                return None;
            }
            min[i] = min[i].min(point[i]);
            max[i] = max[i].max(point[i]);
        }
    }
    Some(crate::pickups::PickupBounds { min, max })
}

/// `path` without extension, like ContentManager.Load names.
pub fn load_level_model(content: &Path, model: &Path) -> Result<LevelModel, PlayContentError> {
    let relative = model.with_extension("xnb");
    let path = resolve(content, &relative).unwrap_or_else(|| content.join(&relative));
    match parse(&path)? {
        Content::Level(level) => Ok(*level),
        _ => Err(PlayContentError::WrongContent(path)),
    }
}

pub fn load_character(content: &Path, name: &str) -> Result<CharacterTemplate, PlayContentError> {
    let relative = Path::new("Data/Characters").join(format!("{name}.xnb"));
    let path = resolve(content, &relative).unwrap_or_else(|| content.join(&relative));
    match parse(&path)? {
        Content::Character(template) => Ok(*template),
        _ => Err(PlayContentError::WrongContent(path)),
    }
}

/// A level's start scene with its decoded model.
pub struct StartScene {
    pub start: LevelStart,
    pub scene: SceneFile,
    /// Model path relative to the content root, case-resolved, no extension
    /// (e.g. `Levels/WizardCastle/wc_s1`); exported glTF mirrors it.
    pub model_path: PathBuf,
    pub model: LevelModel,
}

/// Load a level's start scene. Scene paths are relative to `Content/Levels`.
pub fn load_start_scene(content: &Path, level_file: &str) -> Result<StartScene, PlayContentError> {
    load_level_scene(content, level_file, None)
}

/// Developer catalogue, including nested DLC, challenges and versus scenarios.
/// Keep the relative path: different directories can contain identical stems.
pub fn level_files(content: &Path) -> Result<Vec<String>, PlayContentError> {
    level_files_in(&content.join("Levels"))
}
/// Author-owned scenario catalogue, returned as absolute paths for the loader.
pub fn authored_level_files(directory: &Path) -> Result<Vec<String>, PlayContentError> {
    let root =
        std::fs::canonicalize(directory).map_err(|e| PlayContentError::Io(directory.into(), e))?;
    Ok(level_files_in(&root)?
        .into_iter()
        .map(|name| root.join(name).to_string_lossy().into_owned())
        .collect())
}
fn level_files_in(root: &Path) -> Result<Vec<String>, PlayContentError> {
    fn walk(
        root: &Path,
        directory: &Path,
        files: &mut Vec<String>,
    ) -> Result<(), PlayContentError> {
        for entry in
            std::fs::read_dir(directory).map_err(|e| PlayContentError::Io(directory.into(), e))?
        {
            let entry = entry.map_err(|e| PlayContentError::Io(directory.into(), e))?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|e| PlayContentError::Io(path.clone(), e))?;
            if kind.is_dir() {
                walk(root, &path, files)?;
            } else if kind.is_file()
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("lvl"))
            {
                files.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

/// Load one of a level's scenes by GameScene ID (lowercase file-stem hash,
/// Level.mScenes key); `None` loads the `Start` scene. `start` is always the
/// level's own Start element.
pub fn load_level_scene(
    content: &Path,
    level_file: &str,
    scene_hash: Option<i32>,
) -> Result<StartScene, PlayContentError> {
    let levels = content.join("Levels");
    let level_path = resolve(&levels, Path::new(level_file)).ok_or_else(|| {
        PlayContentError::Io(levels.join(level_file), std::io::ErrorKind::NotFound.into())
    })?;
    let start = read_level(&level_path)?;
    let wanted = scene_hash.unwrap_or(start.scene_hash);
    let mut found = None;
    let directory = level_path.parent().unwrap_or(&levels);
    for relative in &start.scenes {
        let relative = relative.replace('\\', "/");
        let Some(path) = resolve(directory, Path::new(&relative)) else {
            continue;
        };
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase());
        if stem.is_some_and(|s| custom_hash(&s) == wanted) {
            found = Some(path);
            break;
        }
    }
    let scene_path = found.ok_or_else(|| {
        PlayContentError::Io(
            directory.join(format!("{}#{wanted}", start.scene)),
            std::io::ErrorKind::NotFound.into(),
        )
    })?;
    let scene = read_scene(&scene_path)?;
    let relative_model = scene
        .model
        .strip_prefix(&levels)
        .map(|p| Path::new("Levels").join(p))
        .unwrap_or_else(|_| scene.model.clone());
    if relative_model.extension().is_some_and(|e| e == "arena") {
        let model = crate::authored_arena::decode(&read(&scene.model)?)
            .map_err(|e| PlayContentError::Xml(scene.model.clone(), e))?;
        return Ok(StartScene {
            start,
            model_path: scene.model.clone(),
            scene,
            model,
        });
    }
    let xnb = relative_model.with_extension("xnb");
    let resolved = resolve(content, &xnb).ok_or_else(|| {
        PlayContentError::Io(content.join(&xnb), std::io::ErrorKind::NotFound.into())
    })?;
    let model_path = resolved
        .strip_prefix(content)
        .map(|p| p.with_extension(""))
        .unwrap_or_else(|_| relative_model.clone());
    let model = load_level_model(content, &relative_model)?;
    Ok(StartScene {
        start,
        scene,
        model_path,
        model,
    })
}

/// The template's `skeleton_model` skeleton and XNAnimation clips.
pub fn load_skeleton(
    content: &Path,
    template: &CharacterTemplate,
) -> Result<Option<std::sync::Arc<crate::character_pose::CharacterSkeleton>>, PlayContentError> {
    /// Parsed character models, shared. A respawn, a revive or a player joining rebuilds
    /// an avatar during a simulation tick, and re-reading a 2.7 MB model there stalled
    /// the tick for 70 ms, and on a host the whole session (`research/performance.md`).
    /// Character models never change while the game runs.
    static SKELETONS: std::sync::OnceLock<
        std::sync::Mutex<
            std::collections::HashMap<
                std::path::PathBuf,
                std::sync::Arc<crate::character_pose::CharacterSkeleton>,
            >,
        >,
    > = std::sync::OnceLock::new();
    let Some(skeleton) = &template.skeleton_model else {
        return Ok(None);
    };
    // ContentManager resolves the reference from the template's directory.
    let relative = Path::new("Data/Characters")
        .join(skeleton.replace('\\', "/"))
        .with_extension("xnb");
    let key = resolve(content, &relative).unwrap_or_else(|| content.join(&relative));
    let cache = SKELETONS.get_or_init(Default::default);
    if let Some(found) = cache.lock().expect("skeleton cache").get(&key) {
        return Ok(Some(found.clone()));
    }
    // Parsed outside the lock, so other threads are not held up by the disk.
    let resolved = load_skinned_model(content, &relative)?;
    let parsed = std::sync::Arc::new(crate::character_pose::CharacterSkeleton::new(
        resolved.skeleton,
        resolved.clips,
    ));
    cache
        .lock()
        .expect("skeleton cache")
        .insert(key, parsed.clone());
    Ok(Some(parsed))
}

/// Dedicated entity models use the same XNB/shared-resource loader as NPC rigs.
pub fn load_skinned_model(
    content: &Path,
    relative: &Path,
) -> Result<magicka_content::skinned::Resolved, PlayContentError> {
    let path = resolve(content, relative).unwrap_or_else(|| content.join(relative));
    let bytes = read(&path)?;
    let xnb = magicka_xnb::xnb::parse(&bytes)
        .map_err(|e| PlayContentError::Xnb(path.clone(), format!("{e:?}")))?;
    let parsed = parse_primary(&xnb)
        .ok_or_else(|| PlayContentError::Xnb(path.clone(), "null primary".into()))?
        .map_err(|e| PlayContentError::Xnb(path.clone(), format!("{e}")))?;
    let Content::Xna(object) = parsed.content else {
        return Err(PlayContentError::WrongContent(path));
    };
    let magicka_content::xna::XnaObject::Asset(asset) = *object else {
        return Err(PlayContentError::WrongContent(path));
    };
    let magicka_content::assets::Asset::SkinnedModel(model) = *asset else {
        return Err(PlayContentError::WrongContent(path));
    };
    magicka_content::skinned::resolve(&model, parsed.shared)
        .map_err(|e| PlayContentError::Xnb(path, e))
}

/// SpellMine's `Models/Effects/mine` "Take 001" clip duration (the
/// spawn animation gates arming and removal).
pub fn mine_clip_duration(content: &Path) -> Result<f32, PlayContentError> {
    skinned_clips(content, "Models/Effects/mine.xnb")?
        .into_iter()
        .find(|(name, _)| name == "Take 001")
        .map(|(_, d)| d)
        .ok_or_else(|| PlayContentError::WrongContent(content.join("Models/Effects/mine.xnb")))
}

/// Barrier appear/death clip durations (the static Barrier ctor loads):
/// the first clip of each animation model.
pub fn barrier_clips(
    content: &Path,
) -> Result<magicka_sim::barrier::BarrierClips, PlayContentError> {
    let first = |model: &str| -> Result<f32, PlayContentError> {
        let relative = format!("Models/Effects/{model}.xnb");
        skinned_clips(content, &relative)?
            .first()
            .map(|(_, d)| *d)
            .ok_or_else(|| PlayContentError::WrongContent(content.join(&relative)))
    };
    Ok(magicka_sim::barrier::BarrierClips {
        ice_appear: first("iceBarrier_animation")?,
        ice_geyser: first("iceBarrier_geyser")?,
        ice_tesla: first("iceBarrier_tesla")?,
        ice_earth: first("iceBarrier_Earth")?,
        earth_appear: first("earthBarrier_animation")?,
        earth_volcano: first("earthBarrier_volcano")?,
    })
}

/// WaveEntity's mAnimationTimer: the first clip of
/// `Models/Effects/rockpillar_animation` (sWaveAppearClips[0]).
pub fn wave_pillar_clip(content: &Path) -> Result<f32, PlayContentError> {
    let relative = "Models/Effects/rockpillar_animation.xnb";
    skinned_clips(content, relative)?
        .first()
        .map(|(_, d)| *d)
        .ok_or_else(|| PlayContentError::WrongContent(content.join(relative)))
}

/// (name, duration) of every clip of a SkinnedModel asset, in file order.
fn skinned_clips(content: &Path, relative: &str) -> Result<Vec<(String, f32)>, PlayContentError> {
    let relative = Path::new(relative);
    let path = resolve(content, relative).unwrap_or_else(|| content.join(relative));
    let bytes = read(&path)?;
    let xnb = magicka_xnb::xnb::parse(&bytes)
        .map_err(|e| PlayContentError::Xnb(path.clone(), format!("{e:?}")))?;
    let parsed = parse_primary(&xnb)
        .ok_or_else(|| PlayContentError::Xnb(path.clone(), "null primary".into()))?
        .map_err(|e| PlayContentError::Xnb(path.clone(), format!("{e}")))?;
    let Content::Xna(object) = parsed.content else {
        return Err(PlayContentError::WrongContent(path));
    };
    let magicka_content::xna::XnaObject::Asset(asset) = *object else {
        return Err(PlayContentError::WrongContent(path));
    };
    let magicka_content::assets::Asset::SkinnedModel(model) = *asset else {
        return Err(PlayContentError::WrongContent(path));
    };
    let resolved = magicka_content::skinned::resolve(&model, parsed.shared)
        .map_err(|e| PlayContentError::Xnb(path.clone(), e))?;
    Ok(resolved
        .clips
        .iter()
        .map(|c| (c.name.clone(), c.duration))
        .collect())
}

/// Clip durations of a template's `skeleton_model` (XNAnimation clips).
pub fn clip_durations(
    content: &Path,
    template: &CharacterTemplate,
) -> Result<std::collections::HashMap<String, f32>, PlayContentError> {
    Ok(load_skeleton(content, template)?
        .map(|s| s.clip_durations())
        .unwrap_or_default())
}

/// Skeleton, equipment attach points and spell source of a template
/// (Character.ApplyTemplate with model 0).
#[derive(Clone, Debug)]
pub struct CharacterRig {
    pub skeleton: std::sync::Arc<crate::character_pose::CharacterSkeleton>,
    pub attachments: Vec<crate::character_pose::EquipmentAttachment>,
    /// ModelProperties.Transform = CreateScale(scale).
    pub static_transform: magicka_content::skinned::Mat,
    /// Index into `attachments` of the first Staff item (mSourceOfSpellIndex).
    pub spell_source: Option<usize>,
    /// Character.GetLeft/RightAttachOrientation, independent of equipped items.
    pub hand_joints: [Option<crate::character_pose::EquipmentAttachment>; 2],
    /// Character.GetHipAttachOrientation (CharacterTemplate.HipJoint).
    pub hip_joint: Option<crate::character_pose::EquipmentAttachment>,
    /// Render bounding radius: Meshes[0].BoundingSphere.Radius ·
    /// mBoundingScale (1.333) · Transform.M11 (None without a readable model).
    pub bounding_radius: Option<f32>,
}

fn scale_matrix(s: f32) -> magicka_content::skinned::Mat {
    [
        s, 0.0, 0.0, 0.0, 0.0, s, 0.0, 0.0, 0.0, 0.0, s, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

/// The item template itself (melee data) with its class and attach0.
fn load_item_full(
    content: &Path,
    from_dir: &Path,
    item: &str,
) -> Option<(
    u8,
    crate::character_pose::ItemPoints,
    magicka_content::item::Item,
)> {
    let relative = from_dir.join(item.replace('\\', "/")).with_extension("xnb");
    let path = resolve(content, &relative)?;
    let Ok(Content::Item(item)) = parse(&path) else {
        return None;
    };
    let attach0 = item
        .model
        .as_ref()
        .and_then(|m| {
            let dir = path.parent()?.strip_prefix(content).ok()?.to_path_buf();
            let model = dir.join(m.replace('\\', "/")).with_extension("xnb");
            let model_path = resolve(content, &model)?;
            match parse(&model_path).ok()? {
                Content::Xna(object) => match *object {
                    magicka_content::xna::XnaObject::Model(model) => {
                        Some(crate::character_pose::ItemPoints::from_bones(&model.bones))
                    }
                    _ => None,
                },
                _ => None,
            }
        })
        .unwrap_or(crate::character_pose::ItemPoints::NONE);
    Some((item.weapon_class, attach0, *item))
}

/// CharacterTemplate gibs with loadable models (Data/Characters-relative
/// external references) and their first mesh's bounding radius. A null or
/// unreadable model is skipped (Gib.Initialize would dereference null).
pub fn gib_definitions(
    content: &Path,
    template: &magicka_content::character::CharacterTemplate,
) -> Vec<crate::gibs::GibDefinition> {
    gib_definitions_in(content, "Data/Characters", &template.gibs)
}

/// PhysicsEntityTemplate gibs (Data/PhysicsEntities-relative references).
pub fn physics_gib_definitions(
    content: &Path,
    template: &magicka_content::physics::PhysicsEntityTemplate,
) -> Vec<crate::gibs::GibDefinition> {
    gib_definitions_in(content, "Data/PhysicsEntities", &template.gibs)
}

fn gib_definitions_in(
    content: &Path,
    base: &str,
    gibs: &[magicka_content::character::Gib],
) -> Vec<crate::gibs::GibDefinition> {
    gibs.iter()
        .filter_map(|gib| {
            let reference = gib.model.as_ref()?.replace('\\', "/");
            let mut relative = PathBuf::from(base);
            for component in Path::new(&reference).components() {
                match component {
                    Component::ParentDir => {
                        relative.pop();
                    }
                    Component::Normal(name) => relative.push(name),
                    _ => {}
                }
            }
            let path = resolve(content, &relative.with_extension("xnb"))?;
            let Content::Xna(object) = parse(&path).ok()? else {
                return None;
            };
            let magicka_content::xna::XnaObject::Model(model) = *object else {
                return None;
            };
            let radius = model.meshes.first()?.bounding_sphere.1;
            let model = path
                .strip_prefix(content)
                .ok()?
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/");
            Some(crate::gibs::GibDefinition {
                model,
                mass: gib.mass,
                scale: gib.scale,
                radius,
            })
        })
        .collect()
}

/// WeaponClass of `Data/Items/Wizard/<name>` (Item.GetCachedWeapon).
pub fn item_weapon_class(content: &Path, name: &str) -> Option<u8> {
    load_item_full(content, Path::new("Data/Items/Wizard"), name).map(|(class, _, _)| class)
}

/// Live avatar equipment, retaining the original item definitions.
pub fn avatar_items(
    content: &Path,
    template: &CharacterTemplate,
) -> Vec<(i32, crate::avatar::AvatarItem)> {
    template
        .equipment
        .iter()
        .filter_map(|(slot, attachment)| {
            let (_, _, item) = load_item_full(
                content,
                Path::new("Data/Characters"),
                attachment.item.as_deref()?,
            )?;
            let ranged = ranged_item(
                content,
                Path::new("Data/Characters"),
                attachment.item.as_deref()?,
                &item,
            );
            let mut held = crate::avatar::AvatarItem::new(item);
            held.melee.ranged = ranged;
            Some((*slot, held))
        })
        .collect()
}

/// Resolve projectile geometry from the same source item for NPC and player equipment.
pub fn ranged_item(
    content: &Path,
    from_dir: &Path,
    item_name: &str,
    item: &magicka_content::item::Item,
) -> Option<std::sync::Arc<crate::npc_combat::RangedItem>> {
    if item.ranged_conditions.0.is_empty() {
        return None;
    }
    // Item.mProjectileModel: its file and the on-disk path
    // relative to Content (the glTF cache mirrors that case).
    let projectile = item.projectile_model.as_deref().and_then(|m| {
        let item_dir = from_dir.join(item_name.replace('\\', "/"));
        let model = item_dir
            .parent()?
            .join(m.replace('\\', "/"))
            .with_extension("xnb");
        let file = std::fs::canonicalize(resolve(content, &model)?).ok()?;
        let root = std::fs::canonicalize(content).ok()?;
        let relative = file
            .strip_prefix(&root)
            .ok()?
            .with_extension("")
            .to_string_lossy()
            .replace('\\', "/");
        Some((file, relative))
    });
    Some(std::sync::Arc::new(crate::npc_combat::RangedItem {
        range: item.ranged_range,
        elevation: item.ranged_elevation_deg.to_radians(),
        homing: item.homing,
        facing: item.facing,
        danger: item.ranged_danger,
        // Model bounding radius unless Heavy (17), else 0.25.
        radius: if item.passive_ability == 14 {
            0.5
        } else {
            projectile
                .as_ref()
                .filter(|_| item.weapon_class != 17)
                .and_then(|(file, _)| {
                    let Content::Xna(object) = parse(file).ok()? else {
                        return None;
                    };
                    let magicka_content::xna::XnaObject::Model(model) = *object else {
                        return None;
                    };
                    Some(model.meshes.first()?.bounding_sphere.1)
                })
                .unwrap_or(0.25)
        },
        model: projectile.map(|(_, relative)| relative),
        conditions: item.ranged_conditions.0.clone(),
    }))
}

/// Melee data of a template's equipped items: (slot, weapon).
/// Equipped item effect frames, including items with only melee follow effects.
pub fn held_item_effects(
    content: &Path,
    template: &CharacterTemplate,
) -> Vec<crate::npc::HeldItemEffects> {
    let from_dir = Path::new("Data/Characters");
    template
        .equipment
        .iter()
        .filter_map(|(slot, attachment)| {
            let (_, _, item) = load_item_full(content, from_dir, attachment.item.as_deref()?)?;
            Some(crate::npc::HeldItemEffects {
                slot: *slot,
                effects: std::sync::Arc::new(item.effects.clone()),
                hide: item.hide_effect,
                frame: None,
            })
        })
        .collect()
}

pub fn melee_weapons(
    content: &Path,
    template: &CharacterTemplate,
) -> Vec<(i32, crate::npc_combat::MeleeWeapon)> {
    let from_dir = Path::new("Data/Characters");
    template
        .equipment
        .iter()
        .filter_map(|(slot, attachment)| {
            let item_name = attachment.item.as_deref()?;
            let (_, _, item) = load_item_full(content, from_dir, item_name)?;
            let mut weapon = crate::npc_combat::MeleeWeapon::new(
                item.melee_range,
                item.melee_multi_hit,
                std::sync::Arc::new(item.melee_conditions.0.clone()),
            );
            weapon.block_value = item.block_value;
            weapon.auras = item
                .auras
                .iter()
                .map(crate::generic_aura::GenericAura::from_content)
                .collect();
            weapon.special = item.special_ability.as_ref().map(|(recharge, ability)| {
                crate::item_special::ItemSpecial::new(*recharge, ability.clone())
            });
            // Attachment.Release: pickable items become pickups (the rewrite
            // spawns pickups from Data/Items/Wizard only).
            let wizard = item_name
                .replace('\\', "/")
                .to_ascii_lowercase()
                .contains("items/wizard/");
            weapon.release = Some((
                (item.pickable && wizard).then(|| {
                    item_name
                        .rsplit(['\\', '/'])
                        .next()
                        .unwrap_or("")
                        .to_owned()
                }),
                item.model.is_some(),
            ));
            weapon.ranged = ranged_item(content, from_dir, item_name, &item);
            Some((*slot, weapon))
        })
        .collect()
}

pub fn load_rig(
    content: &Path,
    template: &CharacterTemplate,
) -> Result<Option<CharacterRig>, PlayContentError> {
    let Some(skeleton) = load_skeleton(content, template)? else {
        return Ok(None);
    };
    let scale = template.models.first().map_or(1.0, |m| m.scale);
    let static_transform = scale_matrix(scale);
    let inverse_static = crate::live_collision::inverse(&static_transform)
        .unwrap_or(magicka_content::skinned::IDENTITY);
    let from_dir = Path::new("Data/Characters");
    let mut attachments = Vec::new();
    for (slot, attachment) in &template.equipment {
        let Some(bone) = skeleton.bone(&attachment.bone) else {
            continue;
        };
        let (weapon_class, points) = attachment
            .item
            .as_deref()
            .and_then(|item| load_item_full(content, from_dir, item))
            .map_or(
                (None, crate::character_pose::ItemPoints::NONE),
                |(c, p, _)| (Some(c), p),
            );
        if let Some(mut a) = crate::character_pose::EquipmentAttachment::new(
            *slot,
            bone,
            &skeleton.skeleton.bones[bone].inverse_bind,
            attachment.rotation,
            &inverse_static,
            points.attach0,
            weapon_class,
        ) {
            a.attach1 = points.attach1;
            a.effect0 = points.effect0;
            a.effect1 = points.effect1;
            attachments.push(a);
        }
    }
    let spell_source = attachments.iter().position(|a| {
        a.has_item && a.weapon_class == crate::animation_controller::WEAPON_CLASS_STAFF
    });
    let bounding_radius =
        model_bounding_radius(content, template).map(|r| r * BOUNDING_SCALE * scale);
    let joint = |bone: usize| {
        crate::character_pose::EquipmentAttachment::new(
            -1,
            bone,
            &skeleton.skeleton.bones[bone].inverse_bind,
            [0.; 3],
            &inverse_static,
            magicka_content::skinned::IDENTITY,
            None,
        )
    };
    let hand_joints = ["LeftAttach", "RightAttach"].map(|name| joint(skeleton.bone(name)?));
    // CharacterTemplate takes the hip by position, not name: bone index 1.
    let hip_joint = (skeleton.skeleton.bones.len() > 1)
        .then(|| joint(1))
        .flatten();
    Ok(Some(CharacterRig {
        skeleton,
        attachments,
        static_transform,
        spell_source,
        hand_joints,
        hip_joint,
        bounding_radius,
    }))
}

/// Character.mBoundingScale outside ForceDraw.
pub const BOUNDING_SCALE: f32 = 1.333;

/// Meshes[0].BoundingSphere.Radius of the template's first model.
pub fn model_bounding_radius(content: &Path, template: &CharacterTemplate) -> Option<f32> {
    let reference = template.models.first()?.model.as_deref()?;
    let relative = Path::new("Data/Characters")
        .join(reference.replace('\\', "/"))
        .with_extension("xnb");
    let path = resolve(content, &relative)?;
    let bytes = read(&path).ok()?;
    let xnb = magicka_xnb::xnb::parse(&bytes).ok()?;
    let parsed = parse_primary(&xnb)?.ok()?;
    let Content::Xna(object) = parsed.content else {
        return None;
    };
    let magicka_content::xna::XnaObject::Asset(asset) = *object else {
        return None;
    };
    let magicka_content::assets::Asset::SkinnedModel(model) = *asset else {
        return None;
    };
    let resolved = magicka_content::skinned::resolve(&model, parsed.shared).ok()?;
    Some(resolved.model?.meshes.first()?.bounding_sphere.1)
}

/// WeaponClass of each occupied equipment slot, in mEquipment index order
/// (Character.GoToAnimation walks the slots in that order).
pub fn weapon_classes(rig: Option<&CharacterRig>) -> Vec<u8> {
    let Some(rig) = rig else {
        return Vec::new();
    };
    let mut slots: Vec<_> = rig.attachments.iter().filter(|a| a.has_item).collect();
    slots.sort_by_key(|a| a.slot);
    slots.iter().map(|a| a.weapon_class).collect()
}

/// MovementProperties of a template: OR of its move-animation keys.
pub fn move_abilities(template: &CharacterTemplate) -> u8 {
    template.move_animations.iter().fold(0, |a, (k, _)| a | k)
}

fn stems(root: &Path, ext: &str) -> std::collections::HashMap<String, PathBuf> {
    fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, ext, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)) {
                out.push(p);
            }
        }
    }
    let mut files = vec![];
    walk(root, ext, &mut files);
    files
        .into_iter()
        .filter_map(|p| Some((p.file_stem()?.to_string_lossy().to_ascii_lowercase(), p)))
        .collect()
}

/// SceneContent over the user's Content directory (lazy, cached).
pub struct FsSceneContent {
    characters: std::collections::HashMap<String, PathBuf>,
    physics: std::collections::HashMap<String, PathBuf>,
    character_cache: std::collections::HashMap<String, Option<crate::scene_world::NpcTemplate>>,
    physics_cache: std::collections::HashMap<String, Option<i32>>,
    actor_cache: std::collections::HashMap<String, Option<crate::npc::NpcActorTemplate>>,
    content: PathBuf,
    effects: std::collections::HashSet<i32>,
    cues: std::collections::HashSet<i32>,
}
impl FsSceneContent {
    pub fn new(content: &Path) -> Self {
        let cues = (|| {
            let Ok(Content::Xna(list)) = parse(&content.join("Audio/SoundList.xnb")) else {
                return None;
            };
            let magicka_content::xna::XnaObject::Asset(asset) = *list else {
                return None;
            };
            let magicka_content::assets::Asset::StringList(names) = *asset else {
                return None;
            };
            crate::audio_action::sound_dictionary(names.iter().flatten().map(String::as_str))
                .ok()
                .map(|d| d.into_keys().collect())
        })()
        .unwrap_or_default();
        Self {
            characters: stems(&content.join("Data/Characters"), "xnb"),
            physics: {
                // SpawnAnimProp templates live under Data/AnimatedProps.
                let mut m = stems(&content.join("Data/PhysicsEntities"), "xnb");
                m.extend(
                    stems(&content.join("Data/AnimatedProps"), "xnb")
                        .into_iter()
                        .map(|(k, v)| (format!("{ANIMATED_PROPS}{k}"), v)),
                );
                m
            },
            character_cache: Default::default(),
            physics_cache: Default::default(),
            actor_cache: Default::default(),
            content: content.to_path_buf(),
            effects: stems(&content.join("Effects"), "xml")
                .into_keys()
                .map(|s| custom_hash(&s))
                .collect(),
            cues,
        }
    }
}
impl crate::scene_world::SceneContent for FsSceneContent {
    fn projectile_item(&mut self, name: &str) -> Option<crate::npc_combat::RangedItem> {
        let from = Path::new("Data/Items/NPC");
        // SpawnMissileEvent resolves Item.Name in the pickable item cache;
        // file stems need not match that name or follow a numeric suffix.
        let mut paths: Vec<_> = stems(&resolve(&self.content, from)?, "xnb")
            .into_values()
            .collect();
        paths.sort();
        let item = paths.into_iter().find_map(|path| {
            let Content::Item(item) = parse(&path).ok()? else {
                return None;
            };
            item.name.eq_ignore_ascii_case(name).then_some(*item)
        })?;
        let model = from
            .join(item.projectile_model.as_deref()?.replace('\\', "/"))
            .with_extension("xnb");
        let path = std::fs::canonicalize(resolve(&self.content, &model)?).ok()?;
        let root = std::fs::canonicalize(&self.content).ok()?;
        let relative = path
            .strip_prefix(root)
            .ok()?
            .with_extension("")
            .to_string_lossy()
            .into_owned();
        let Content::Xna(object) = parse(&path).ok()? else {
            return None;
        };
        let magicka_content::xna::XnaObject::Model(model) = *object else {
            return None;
        };
        Some(crate::npc_combat::RangedItem {
            range: item.ranged_range,
            elevation: item.ranged_elevation_deg.to_radians(),
            homing: item.homing,
            facing: item.facing,
            danger: item.ranged_danger,
            radius: model.meshes.first()?.bounding_sphere.1,
            model: Some(relative),
            conditions: item.ranged_conditions.0,
        })
    }
    fn player_ranged_item(
        &mut self,
        name: &str,
    ) -> Option<std::sync::Arc<crate::npc_combat::RangedItem>> {
        let item = self.item(name)?;
        ranged_item(&self.content, Path::new("Data/Items/Wizard"), name, &item)
    }
    fn item(&mut self, name: &str) -> Option<magicka_content::item::Item> {
        let path = resolve(
            &self.content,
            &Path::new("Data/Items/Wizard")
                .join(name)
                .with_extension("xnb"),
        )?;
        match parse(&path).ok()? {
            Content::Item(item) => Some(*item),
            _ => None,
        }
    }

    fn character(&mut self, name: &str) -> Option<crate::scene_world::NpcTemplate> {
        let path = self.characters.get(&name.to_ascii_lowercase()).cloned();
        self.character_cache
            .entry(name.into())
            .or_insert_with(|| match parse(&path?).ok()? {
                Content::Character(t) => Some(crate::scene_world::NpcTemplate {
                    type_id: custom_hash(&t.id.to_ascii_lowercase()),
                    faction: t.faction,
                    max_hit_points: t.max_hitpoints,
                    radius: t.radius,
                    length: t.length,
                    undying: t.undying,
                }),
                _ => None,
            })
            .clone()
    }
    fn physics_max_hit_points(&mut self, name: &str) -> Option<i32> {
        let path = self.physics.get(&name.to_ascii_lowercase()).cloned();
        *self
            .physics_cache
            .entry(name.into())
            .or_insert_with(|| match parse(&path?).ok()? {
                Content::PhysicsEntity(t) => Some(t.max_hitpoints),
                _ => None,
            })
    }
    fn physics_template(
        &mut self,
        name: &str,
    ) -> Option<magicka_content::physics::PhysicsEntityTemplate> {
        let path = self.physics.get(&name.to_ascii_lowercase())?;
        match parse(path).ok()? {
            Content::PhysicsEntity(t) => Some(*t),
            _ => None,
        }
    }
    fn effect_exists(&mut self, effect: i32) -> bool {
        self.effects.contains(&effect)
    }
    fn cue_exists(&mut self, cue: i32) -> bool {
        self.cues.contains(&cue)
    }
    fn physics_gibs(&mut self, name: &str) -> Vec<crate::gibs::GibDefinition> {
        load_physics_template(&self.content, name)
            .map(|t| physics_gib_definitions(&self.content, &t))
            .unwrap_or_default()
    }
    fn model_radius(&mut self, path: &str) -> Option<f32> {
        let path = resolve(&self.content, &Path::new(path).with_extension("xnb"))?;
        let Content::Xna(object) = parse(&path).ok()? else {
            return None;
        };
        let magicka_content::xna::XnaObject::Model(model) = *object else {
            return None;
        };
        Some(model.meshes.first()?.bounding_sphere.1)
    }
    fn clip_duration(&mut self, path: &str, clip: &str) -> Option<f32> {
        let relative = Path::new(path).with_extension("xnb");
        skinned_clips(&self.content, &relative.to_string_lossy())
            .ok()?
            .into_iter()
            .find(|(name, _)| name == clip)
            .map(|(_, d)| d)
    }
    fn skinned_model(&mut self, path: &str) -> Option<magicka_content::skinned::Resolved> {
        load_skinned_model(&self.content, &Path::new(path).with_extension("xnb")).ok()
    }
    fn model_bounds(&mut self, path: &str) -> Option<crate::pickups::PickupBounds> {
        pickup_model_bounds(&self.content, Path::new(path))
    }
    fn elemental_egg_geometry(&mut self) -> Option<crate::elemental_egg_actor::EggGeometry> {
        let fire = load_character(&self.content, "Elemental_Fire").ok()?;
        crate::elemental_egg_actor::EggGeometry::from_fire_template(&fire).ok()
    }
    fn actor(&mut self, name: &str) -> Option<crate::npc::NpcActorTemplate> {
        let content = self.content.clone();
        self.actor_cache
            .entry(name.to_ascii_lowercase())
            .or_insert_with(|| {
                let template = load_character(&content, name).ok()?;
                let mut definition = crate::CharacterDefinition::from_content(&template, 1).ok()?;
                definition.gibs = gib_definitions(&content, &template);
                let durations = clip_durations(&content, &template).unwrap_or_default();
                let rig = load_rig(&content, &template).ok().flatten();
                // GoToAnimation consults the equipped weapons' sets first.
                let animations =
                    std::sync::Arc::new(crate::animation_controller::AnimationSet::with_weapons(
                        &template.animation_sets,
                        &weapon_classes(rig.as_ref()),
                        durations,
                    ));
                let abilities = template
                    .abilities
                    .iter()
                    .map(crate::npc_combat::CombatAbility::from_content)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()?;
                let weapons = melee_weapons(&content, &template);
                let held_effects = held_item_effects(&content, &template);
                Some(crate::npc::NpcActorTemplate {
                    definition,
                    animations,
                    move_abilities: move_abilities(&template),
                    weights: crate::npc::AgentWeights {
                        alert_radius: template.alert_radius,
                        friendly_avoidance: template.friendly_avoidance,
                        enemy_avoidance: template.enemy_avoidance,
                        sight_avoidance: template.sight_avoidance,
                        danger_avoidance: template.danger_avoidance,
                        group: magicka_sim::agent_steering::GroupWeights {
                            chase: template.group_chase,
                            separation: template.group_separation,
                            cohesion: template.group_cohesion,
                            alignment: template.group_alignment,
                            wander: template.group_wander,
                        },
                    },
                    abilities: std::sync::Arc::new(abilities),
                    target_weights: crate::npc_combat::TargetWeights {
                        alert_radius: template.alert_radius,
                        anger: template.anger_weight,
                        distance: template.distance_weight,
                        health: template.health_weight,
                        can_see_invisible: template.can_see_invisible,
                    },
                    rig,
                    weapons,
                    held_effects,
                    summon_element_cue: (!template.summon_element_cue.is_empty()).then(|| {
                        (
                            template.summon_element_bank as u16,
                            template.summon_element_cue.clone(),
                        )
                    }),
                })
            })
            .clone()
    }
}

/// `.lvl` Dialogs element resolved next to the level file.
pub fn level_dialogs(content: &Path, level_file: &str) -> Option<PathBuf> {
    let levels = content.join("Levels");
    let path = resolve(&levels, Path::new(level_file))?;
    let root = xml(&std::fs::read(&path).ok()?).ok()?;
    let name = root
        .children
        .iter()
        .find(|n| n.name.eq_ignore_ascii_case("dialogs"))?
        .inner_text
        .trim()
        .to_string();
    resolve(path.parent()?, Path::new(&name))
}

/// A BitmapFont asset (e.g. `Languages/eng/Font/Maiandra14`).
pub fn bitmap_font(
    content: &Path,
    relative: &str,
) -> Result<magicka_content::assets::BitmapFont, PlayContentError> {
    let rel = Path::new(relative).with_extension("xnb");
    let path = resolve(content, &rel).unwrap_or_else(|| content.join(&rel));
    match parse(&path)? {
        Content::Xna(object) => match *object {
            magicka_content::xna::XnaObject::Asset(asset) => match *asset {
                magicka_content::assets::Asset::BitmapFont(font) => Ok(font),
                _ => Err(PlayContentError::WrongContent(path)),
            },
            _ => Err(PlayContentError::WrongContent(path)),
        },
        _ => Err(PlayContentError::WrongContent(path)),
    }
}

/// English `*.loctable.xml` tables in file-name order (LanguageManager load).
pub fn english_strings(
    content: &Path,
    player_names: &[&str],
) -> crate::localization::LocalizationTable {
    let mut table = crate::localization::LocalizationTable::default();
    table.set_player_names(player_names);
    let dir = resolve(content, Path::new("Languages/eng"))
        .unwrap_or_else(|| content.join("Languages/eng"));
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.to_string_lossy().ends_with(".loctable.xml"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    for file in files {
        if let Ok(bytes) = std::fs::read(&file) {
            let _ = table.add_xml(&bytes);
        }
    }
    table
}

#[cfg(test)]
mod scene_post_tests {
    use super::*;

    #[test]
    fn filter_and_bloom_attributes_parse_case_insensitively() {
        let root = xml(
            br#"<Scene><Model>m</Model><bloom threshold="5" Multiplyer=".5" blur="2"/>
            <Filter Brightness="1.0" Contrast="1.2" Saturation="0.0" /><fog start="1"/></Scene>"#,
        )
        .unwrap();
        let mut post = ScenePost::default();
        for node in &root.children {
            post.apply(node).unwrap();
        }
        assert_eq!(
            post,
            ScenePost {
                fog: Some(SceneFog {
                    start: 1.,
                    ..Default::default()
                }),
                brightness: 1.0,
                contrast: 1.2,
                saturation: 0.0,
                bloom_threshold: 5.0,
                bloom_multiplier: 0.5,
                blur_sigma: 2.0,
            }
        );
    }
}
