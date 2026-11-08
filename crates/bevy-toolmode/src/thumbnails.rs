//! Generated catalogue thumbnails. Requested entries (the menu requests the
//! ones it shows) come from a PNG disk cache when present and newer than
//! their model; otherwise the model is loaded into an isolated "studio" (own
//! render layer, far from the world), framed from its bounds, rendered to an
//! offscreen image, read back and written to the cache. Several studio
//! slots (own camera and target, far apart) render in parallel while the
//! next models load in the background.
//!
//! With [`ThumbnailSettings::pregenerate`], every catalogue revision queues
//! the entries whose cache is missing or stale (first run, new or
//! re-exported content) as a batch with a progress screen;
//! [`ThumbnailSettings::force_regenerate`] wipes the cache first.
use crate::SpawnCatalogue;
use bevy::{
    asset::{LoadState, RenderAssetUsages},
    camera::{primitives::Aabb, visibility::RenderLayers, RenderTarget},
    gltf::GltfAssetLabel,
    image::{CompressedImageFormats, ImageSampler, ImageType},
    prelude::*,
    render::{
        render_resource::{TextureFormat, TextureUsages},
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
    world_serialization::WorldAsset,
};
use bevy_egui::{egui, EguiContexts, EguiTextureHandle};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Resource, Clone, Debug)]
pub struct ThumbnailSettings {
    pub enabled: bool,
    /// Square size in pixels.
    pub size: u32,
    /// Render layer reserved for the studio.
    pub layer: usize,
    /// PNG cache directory (None: in-memory only).
    pub cache_dir: Option<PathBuf>,
    /// Where models are placed while rendering.
    pub studio_origin: Vec3,
    /// Direction from a model toward the camera, in model space. The
    /// default views the glTF front (+Z) from above and to the side; games
    /// whose models face -Z flip z.
    pub view_direction: Vec3,
    /// Frames before a model that never loads is given up on.
    pub timeout_frames: u32,
    /// Models rendered at the same time (studio slots).
    pub parallel: usize,
    /// Render every missing or stale thumbnail when the catalogue changes,
    /// with a progress screen, instead of only those the menu shows.
    pub pregenerate: bool,
    /// Delete the cache and render everything again (once, on the first
    /// catalogue).
    pub force_regenerate: bool,
    /// File system root of model asset paths, to detect re-exported models
    /// (a model newer than its cached thumbnail is rendered again).
    pub asset_root: Option<PathBuf>,
}

impl Default for ThumbnailSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            size: 96,
            layer: 27,
            cache_dir: None,
            studio_origin: Vec3::new(0.0, -10_000.0, 0.0),
            view_direction: Vec3::new(0.55, 0.45, 1.0),
            timeout_frames: 900,
            parallel: 8,
            pregenerate: false,
            force_regenerate: false,
            asset_root: None,
        }
    }
}

#[derive(Clone, Debug)]
enum State {
    Queued,
    Rendering,
    Ready(Handle<Image>),
    Failed,
}

enum Stage {
    Loading,
    Settling(u32),
    Rendering(u32),
    Capturing,
}

/// A pre-generation run shown on the progress screen.
#[derive(Clone, Debug)]
pub struct Batch {
    pub total: usize,
    pub done: usize,
    /// The progress screen was dismissed (rendering continues).
    pub hidden: bool,
    pub current: String,
    ids: Vec<String>,
}

struct Job {
    id: String,
    model: String,
    root: Entity,
    scene: Handle<WorldAsset>,
    stage: Stage,
    frames: u32,
    /// Blank captures retried (pipelines still compiling, textures late).
    retries: u32,
    captured: Arc<Mutex<Option<Image>>>,
}

/// One studio position with its own camera and render target.
struct Slot {
    origin: Vec3,
    camera: Entity,
    target: Handle<Image>,
    job: Option<Job>,
}

/// Distance between studio slots, far beyond any framed model's view.
const SLOT_SPACING: f32 = 2000.0;

#[derive(Resource)]
pub struct Thumbnails {
    states: HashMap<String, State>,
    queue: VecDeque<String>,
    slots: Vec<Slot>,
    textures: HashMap<String, egui::TextureId>,
    /// Count of thumbnails rendered (not loaded from cache) this run.
    pub rendered: usize,
    pub batch: Option<Batch>,
    seen_revision: Option<u64>,
    /// Models loading ahead of their turn.
    prefetch: HashMap<String, Handle<WorldAsset>>,
}

impl Thumbnails {
    /// Ask for `id`'s thumbnail; cheap to call every frame.
    pub fn request(&mut self, id: &str) {
        if !self.states.contains_key(id) {
            self.states.insert(id.to_string(), State::Queued);
            self.queue.push_back(id.to_string());
        }
    }

    /// The egui texture once the thumbnail is ready.
    pub fn texture(&self, id: &str) -> Option<egui::TextureId> {
        self.textures.get(id).copied()
    }

    pub fn image(&self, id: &str) -> Option<&Handle<Image>> {
        match self.states.get(id) {
            Some(State::Ready(h)) => Some(h),
            _ => None,
        }
    }

    pub fn failed(&self, id: &str) -> bool {
        matches!(self.states.get(id), Some(State::Failed))
    }

    /// (queued, ready, failed) counts.
    pub fn progress(&self) -> (usize, usize, usize) {
        let mut out = (0, 0, 0);
        for s in self.states.values() {
            match s {
                State::Queued | State::Rendering => out.0 += 1,
                State::Ready(_) => out.1 += 1,
                State::Failed => out.2 += 1,
            }
        }
        out
    }

    /// Forget everything (and delete cached files) so thumbnails regenerate.
    pub fn regenerate(&mut self, settings: &ThumbnailSettings) {
        if let Some(dir) = &settings.cache_dir {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                if entry.path().extension().is_some_and(|e| e == "png") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        self.states.clear();
        self.queue.clear();
        self.textures.clear();
        self.prefetch.clear();
        self.batch = None;
        // Pre-generation (if enabled) queues everything again.
        self.seen_revision = None;
    }

    /// Stop the current batch; thumbnails still render when shown.
    pub fn skip_batch(&mut self) {
        if let Some(batch) = self.batch.take() {
            self.queue.retain(|id| !batch.ids.contains(id));
            for id in batch.ids {
                if matches!(self.states.get(&id), Some(State::Queued)) {
                    self.states.remove(&id);
                }
            }
        }
    }
}

/// Bump when studio lighting or framing changes so caches regenerate.
const STYLE: u32 = 2;

/// Cache file name: readable prefix plus a hash of id, model, size, view
/// direction and studio style.
pub fn cache_name(id: &str, model: &str, size: u32, view: Vec3) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let view = view.to_array().map(f32::to_bits);
    for b in id
        .bytes()
        .chain([0])
        .chain(model.bytes())
        .chain(size.to_le_bytes())
        .chain(view.iter().flat_map(|v| v.to_le_bytes()))
        .chain(STYLE.to_le_bytes())
    {
        hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    let prefix: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(40)
        .collect();
    format!("{prefix}-{hash:016x}.png")
}

/// Camera placement framing a bounding sphere for a vertical fov, looking
/// back along `view` (center to camera).
pub fn frame(center: Vec3, radius: f32, fov: f32, view: Vec3) -> Transform {
    let distance = radius.max(0.05) / (fov * 0.5).sin() * 1.05;
    let direction = view.normalize_or(Vec3::Z);
    Transform::from_translation(center + direction * distance).looking_at(center, Vec3::Y)
}

const FOV: f32 = 0.5;

pub(crate) fn setup(
    mut commands: Commands,
    settings: Res<ThumbnailSettings>,
    mut images: ResMut<Assets<Image>>,
) {
    let size = settings.size.max(16);
    let layer = RenderLayers::layer(settings.layer);
    let slots = (0..settings.parallel.max(1))
        .map(|i| {
            let mut image =
                Image::new_target_texture(size, size, TextureFormat::Rgba8UnormSrgb, None);
            image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
            let target = images.add(image);
            let origin = settings.studio_origin + Vec3::X * SLOT_SPACING * i as f32;
            let camera = commands
                .spawn((
                    Name::new(format!("Thumbnail studio camera {i}")),
                    Camera3d::default(),
                    Camera {
                        is_active: false,
                        order: -50 - i as isize,
                        clear_color: ClearColorConfig::Custom(Color::NONE),
                        ..default()
                    },
                    RenderTarget::Image(target.clone().into()),
                    Projection::Perspective(PerspectiveProjection {
                        fov: FOV,
                        near: 0.01,
                        ..default()
                    }),
                    AmbientLight {
                        color: Color::WHITE,
                        brightness: 1500.0,
                        ..default()
                    },
                    layer.clone(),
                    Transform::from_translation(origin + Vec3::Z * 5.0),
                ))
                .id();
            Slot {
                origin,
                camera,
                target,
                job: None,
            }
        })
        .collect();
    // One directional light serves every slot on the studio layer.
    commands.spawn((
        Name::new("Thumbnail studio light"),
        DirectionalLight {
            illuminance: 9000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        layer,
        Transform::from_translation(settings.studio_origin)
            .looking_to(Vec3::new(-0.4, -1.0, -0.6), Vec3::Y),
    ));
    commands.insert_resource(Thumbnails {
        states: HashMap::new(),
        queue: VecDeque::new(),
        slots,
        textures: HashMap::new(),
        rendered: 0,
        batch: None,
        seen_revision: None,
        prefetch: HashMap::new(),
    });
}

/// The cached file for `id`, if present and not older than its model.
fn fresh_cache(settings: &ThumbnailSettings, id: &str, model: &str) -> Option<PathBuf> {
    let path = settings.cache_dir.as_ref()?.join(cache_name(
        id,
        model,
        settings.size,
        settings.view_direction,
    ));
    let cached = std::fs::metadata(&path).ok()?.modified().ok()?;
    if let Some(root) = &settings.asset_root {
        let source = std::fs::metadata(root.join(model)).and_then(|m| m.modified());
        if source.is_ok_and(|source| source > cached) {
            return None;
        }
    }
    Some(path)
}

/// Delete cached PNGs no current catalogue entry would use (removed
/// content, or an older size/view/style).
fn prune_cache(settings: &ThumbnailSettings, catalogue: &SpawnCatalogue) {
    let Some(dir) = &settings.cache_dir else {
        return;
    };
    let wanted: std::collections::HashSet<String> = catalogue
        .entries
        .iter()
        .filter_map(|e| {
            Some(cache_name(
                &e.id,
                e.model.as_deref()?,
                settings.size,
                settings.view_direction,
            ))
        })
        .collect();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".png") && !wanted.contains(&name) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Re-renders of a blank capture before giving up.
const BLANK_RETRIES: u32 = 3;

/// Models loaded ahead per studio slot.
const PREFETCH_PER_SLOT: usize = 2;

fn load_cached(path: &std::path::Path) -> Option<Image> {
    let bytes = std::fs::read(path).ok()?;
    Image::from_buffer(
        &bytes,
        ImageType::Extension("png"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::Default,
        RenderAssetUsages::default(),
    )
    .ok()
}

/// Loads per frame from the disk cache.
const CACHE_LOADS_PER_FRAME: usize = 16;

pub(crate) fn process(
    mut commands: Commands,
    thumbs: Option<ResMut<Thumbnails>>,
    settings: Res<ThumbnailSettings>,
    catalogue: Res<SpawnCatalogue>,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    children: Query<&Children>,
    meshes: Query<(&Aabb, &GlobalTransform)>,
    unlayered: Query<(), Without<RenderLayers>>,
    mut cameras: Query<(&mut Camera, &mut Transform)>,
) {
    let Some(mut thumbs) = thumbs else {
        return;
    };
    let thumbs = &mut *thumbs;
    if !settings.enabled {
        return;
    }
    if settings.pregenerate
        && thumbs.seen_revision != Some(catalogue.revision)
        && !catalogue.entries.is_empty()
    {
        if settings.force_regenerate && thumbs.seen_revision.is_none() && thumbs.rendered == 0 {
            thumbs.regenerate(&settings);
        }
        thumbs.seen_revision = Some(catalogue.revision);
        prune_cache(&settings, &catalogue);
        let ids: Vec<String> = catalogue
            .entries
            .iter()
            .filter_map(|e| Some((e, e.model.as_deref()?)))
            .filter(|(e, model)| fresh_cache(&settings, &e.id, model).is_none())
            .map(|(e, _)| e.id.clone())
            .filter(|id| !matches!(thumbs.states.get(id), Some(State::Ready(_))))
            .collect();
        for id in &ids {
            if !matches!(
                thumbs.states.get(id),
                Some(State::Queued | State::Rendering)
            ) {
                thumbs.states.insert(id.clone(), State::Queued);
                thumbs.queue.push_back(id.clone());
            }
        }
        if !ids.is_empty() {
            info!("thumbnails: {} to render", ids.len());
            thumbs.batch = Some(Batch {
                total: ids.len(),
                done: 0,
                hidden: false,
                current: String::new(),
                ids,
            });
        }
    }
    // Cached files first; anything without a model fails immediately.
    let mut loads = 0;
    let mut deferred = VecDeque::new();
    while let Some(id) = thumbs.queue.pop_front() {
        let Some(model) = catalogue.get(&id).and_then(|e| e.model.clone()) else {
            thumbs.states.insert(id, State::Failed);
            continue;
        };
        let free = thumbs.slots.iter().position(|s| s.job.is_none());
        match (fresh_cache(&settings, &id, &model), free) {
            (Some(path), _) if loads < CACHE_LOADS_PER_FRAME => {
                loads += 1;
                let state = match load_cached(&path) {
                    Some(image) => State::Ready(images.add(image)),
                    None => State::Failed,
                };
                thumbs.states.insert(id, state);
            }
            (None, Some(slot)) => {
                let scene = thumbs.prefetch.remove(&id).unwrap_or_else(|| {
                    assets.load(GltfAssetLabel::Scene(0).from_asset(model.clone()))
                });
                if let Some(batch) = thumbs.batch.as_mut() {
                    batch.current = catalogue.get(&id).map_or(id.clone(), |e| e.label.clone());
                }
                let root = commands
                    .spawn((
                        Name::new(format!("Thumbnail {id}")),
                        WorldAssetRoot(scene.clone()),
                        Transform::from_translation(thumbs.slots[slot].origin),
                        RenderLayers::layer(settings.layer),
                    ))
                    .id();
                thumbs.states.insert(id.clone(), State::Rendering);
                thumbs.slots[slot].job = Some(Job {
                    id,
                    model,
                    root,
                    scene,
                    stage: Stage::Loading,
                    frames: 0,
                    retries: 0,
                    captured: Arc::new(Mutex::new(None)),
                });
            }
            _ => deferred.push_back(id),
        }
    }
    thumbs.queue = deferred;
    // Start loading the next models while the slots render.
    let busy = thumbs.slots.iter().any(|s| s.job.is_some());
    if busy {
        let want = PREFETCH_PER_SLOT * thumbs.slots.len();
        let upcoming: Vec<(String, String)> = thumbs
            .queue
            .iter()
            .filter(|id| !thumbs.prefetch.contains_key(*id))
            .filter_map(|id| Some((id.clone(), catalogue.get(id)?.model.clone()?)))
            .filter(|(id, model)| fresh_cache(&settings, id, model).is_none())
            .take(want.saturating_sub(thumbs.prefetch.len()))
            .collect();
        for (id, model) in upcoming {
            thumbs
                .prefetch
                .insert(id, assets.load(GltfAssetLabel::Scene(0).from_asset(model)));
        }
    }

    for slot in thumbs.slots.iter_mut() {
        let Some(job) = slot.job.as_mut() else {
            continue;
        };
        let finished = step_job(
            job,
            slot.camera,
            &slot.target,
            &mut commands,
            &settings,
            &assets,
            &children,
            &meshes,
            &unlayered,
            &mut cameras,
        );
        let Some(finished) = finished else {
            continue;
        };
        let job = slot.job.take().unwrap();
        // A host may already have despawned it (session teardown).
        commands.entity(job.root).try_despawn();
        if let Ok((mut camera, _)) = cameras.get_mut(slot.camera) {
            camera.is_active = false;
        }
        let state = match finished {
            Finished::Image(image) => {
                thumbs.rendered += 1;
                State::Ready(images.add(image))
            }
            Finished::Failed => State::Failed,
        };
        if let Some(batch) = thumbs.batch.as_mut() {
            if batch.ids.contains(&job.id) {
                batch.done += 1;
            }
        }
        thumbs.states.insert(job.id, state);
    }
    if thumbs.slots.iter().all(|s| s.job.is_none())
        && thumbs.batch.as_ref().is_some_and(|b| {
            !b.ids.iter().any(|id| {
                matches!(
                    thumbs.states.get(id),
                    Some(State::Queued | State::Rendering)
                )
            })
        })
    {
        let batch = thumbs.batch.take().unwrap();
        info!("thumbnails: batch of {} finished", batch.total);
    }
}

enum Finished {
    Image(Image),
    Failed,
}

/// Advance one slot's job by a frame; Some when it is done.
#[allow(clippy::too_many_arguments)]
fn step_job(
    job: &mut Job,
    camera: Entity,
    target: &Handle<Image>,
    commands: &mut Commands,
    settings: &ThumbnailSettings,
    assets: &AssetServer,
    children: &Query<&Children>,
    meshes: &Query<(&Aabb, &GlobalTransform)>,
    unlayered: &Query<(), Without<RenderLayers>>,
    cameras: &mut Query<(&mut Camera, &mut Transform)>,
) -> Option<Finished> {
    job.frames += 1;
    // RenderLayers do not inherit: tag the spawned hierarchy.
    let descendants: Vec<Entity> = children.iter_descendants(job.root).collect();
    for &e in &descendants {
        if unlayered.contains(e) {
            commands
                .entity(e)
                .try_insert(RenderLayers::layer(settings.layer));
        }
    }
    let mut finished = None;
    match job.stage {
        Stage::Loading => {
            if matches!(assets.load_state(&job.scene), LoadState::Failed(_)) {
                finished = Some(Finished::Failed);
            } else if assets.is_loaded_with_dependencies(&job.scene) && !descendants.is_empty() {
                job.stage = Stage::Settling(2);
            }
        }
        Stage::Settling(0) => {
            let mut min = Vec3::splat(f32::MAX);
            let mut max = Vec3::splat(f32::MIN);
            for &e in &descendants {
                if let Ok((aabb, g)) = meshes.get(e) {
                    let c = Vec3::from(aabb.center);
                    let h = Vec3::from(aabb.half_extents);
                    for i in 0..8 {
                        let corner = c + h * Vec3::new(
                            if i & 1 == 0 { -1.0 } else { 1.0 },
                            if i & 2 == 0 { -1.0 } else { 1.0 },
                            if i & 4 == 0 { -1.0 } else { 1.0 },
                        );
                        let p = g.transform_point(corner);
                        min = min.min(p);
                        max = max.max(p);
                    }
                }
            }
            if min.x > max.x {
                // Bounds appear a frame after meshes; keep waiting.
                job.stage = Stage::Settling(2);
            } else if let Ok((mut cam, mut transform)) = cameras.get_mut(camera) {
                let center = (min + max) * 0.5;
                *transform = frame(
                    center,
                    (max - min).length() * 0.5,
                    FOV,
                    settings.view_direction,
                );
                cam.is_active = true;
                job.stage = Stage::Rendering(2);
            }
        }
        Stage::Settling(n) => job.stage = Stage::Settling(n - 1),
        Stage::Rendering(0) => {
            let slot = job.captured.clone();
            commands.spawn(Screenshot::image(target.clone())).observe(
                move |captured: On<ScreenshotCaptured>| {
                    *slot.lock().unwrap() = Some(captured.image.clone());
                },
            );
            job.stage = Stage::Capturing;
        }
        Stage::Rendering(n) => job.stage = Stage::Rendering(n - 1),
        Stage::Capturing => {
            if let Some(image) = job.captured.lock().unwrap().take() {
                finished = match image.try_into_dynamic() {
                    Ok(dynamic) => {
                        let rgba = dynamic.to_rgba8();
                        if rgba.pixels().any(|p| p[3] > 0) {
                            if let Some(dir) = &settings.cache_dir {
                                let _ = std::fs::create_dir_all(dir);
                                let path = dir.join(cache_name(
                                    &job.id,
                                    &job.model,
                                    settings.size,
                                    settings.view_direction,
                                ));
                                if let Err(e) = rgba.save(&path) {
                                    warn!("thumbnail cache {}: {e}", path.display());
                                }
                            }
                            Some(Finished::Image(Image::from_dynamic(
                                rgba.into(),
                                true,
                                RenderAssetUsages::default(),
                            )))
                        } else if job.retries < BLANK_RETRIES {
                            // Nothing drawn yet: render pipelines compile
                            // asynchronously the first time a material is seen.
                            job.retries += 1;
                            job.stage = Stage::Rendering(20);
                            None
                        } else {
                            warn!("thumbnail {}: model renders nothing", job.id);
                            Some(Finished::Failed)
                        }
                    }
                    Err(e) => {
                        warn!("thumbnail {}: {e:?}", job.id);
                        Some(Finished::Failed)
                    }
                };
            }
        }
    }
    if finished.is_none() && job.frames > settings.timeout_frames {
        warn!("thumbnail {}: model did not finish loading", job.id);
        finished = Some(Finished::Failed);
    }
    finished
}

pub(crate) fn register_textures(mut contexts: EguiContexts, thumbs: Option<ResMut<Thumbnails>>) {
    let Some(mut thumbs) = thumbs else {
        return;
    };
    let thumbs = &mut *thumbs;
    for (id, state) in &thumbs.states {
        if let State::Ready(handle) = state {
            if !thumbs.textures.contains_key(id) {
                let texture = contexts.add_image(EguiTextureHandle::Strong(handle.clone()));
                thumbs.textures.insert(id.clone(), texture);
            }
        }
    }
}

/// Progress screen for a pre-generation batch.
pub(crate) fn progress_screen(
    mut contexts: EguiContexts,
    thumbs: Option<ResMut<Thumbnails>>,
    settings: Res<ThumbnailSettings>,
) -> Result {
    let Some(mut thumbs) = thumbs else {
        return Ok(());
    };
    let Some(batch) = thumbs.batch.clone().filter(|b| !b.hidden) else {
        return Ok(());
    };
    let Ok(ctx) = contexts.ctx_mut() else {
        return Ok(());
    };
    let mut hide = false;
    let mut skip = false;
    egui::Area::new(egui::Id::new("toolmode_thumbnail_dim"))
        .order(egui::Order::Background)
        .fixed_pos(egui::Pos2::ZERO)
        .interactable(false)
        .show(ctx, |ui| {
            ui.painter().rect_filled(
                ctx.content_rect(),
                0.0,
                egui::Color32::from_black_alpha(140),
            );
        });
    egui::Window::new("Preparing spawn menu thumbnails")
        .id(egui::Id::new("toolmode_thumbnail_progress"))
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .collapsible(false)
        .resizable(false)
        .default_width(440.0)
        .show(ctx, |ui| {
            // A fixed width: the per-frame text below must not resize (and so
            // re-centre) the window every frame.
            ui.set_width(420.0);
            ui.label(
                "Each spawnable's model is rendered once into a small picture for the \
                 spawn menu. This happens on the first run and when content is new or \
                 re-exported; later runs load the saved pictures.",
            );
            if let Some(dir) = &settings.cache_dir {
                ui.label(
                    egui::RichText::new(format!("Saved in {}", dir.display()))
                        .small()
                        .weak(),
                );
            }
            ui.add_space(6.0);
            let fraction = batch.done as f32 / batch.total.max(1) as f32;
            ui.add(
                egui::ProgressBar::new(fraction)
                    .text(format!("{} / {}", batch.done, batch.total))
                    .animate(true),
            );
            ui.add(egui::Label::new(format!("Rendering: {}", batch.current)).truncate());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                hide = ui
                    .button("Continue in background")
                    .on_hover_text("Keep rendering; progress stays in the spawn menu")
                    .clicked();
                skip = ui
                    .button("Skip for now")
                    .on_hover_text("Stop; thumbnails render when the spawn menu shows them")
                    .clicked();
            });
        });
    if hide {
        if let Some(b) = thumbs.batch.as_mut() {
            b.hidden = true;
        }
    }
    if skip {
        thumbs.skip_batch();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_names_are_stable_and_distinguish_models_and_sizes() {
        let v = Vec3::Z;
        let a = cache_name("Goblin pirate", "gltf/a.glb", 96, v);
        assert_eq!(a, cache_name("Goblin pirate", "gltf/a.glb", 96, v));
        assert_ne!(a, cache_name("Goblin pirate", "gltf/a.glb", 96, -v));
        assert!(a.starts_with("Goblin_pirate-"));
        assert_ne!(a, cache_name("Goblin pirate", "gltf/b.glb", 96, v));
        assert_ne!(a, cache_name("Goblin pirate", "gltf/a.glb", 128, v));
    }

    #[test]
    fn framing_keeps_the_sphere_inside_the_fov() {
        let center = Vec3::new(1.0, 2.0, 3.0);
        let t = frame(center, 2.0, FOV, Vec3::new(0.55, 0.45, -1.0));
        let to_center = center - t.translation;
        assert!(to_center.normalize().distance(*t.forward()) < 1e-4);
        // Sphere's angular radius fits within half the fov.
        assert!((2.0 / to_center.length()).asin() <= FOV * 0.5 + 1e-4);
    }
}
