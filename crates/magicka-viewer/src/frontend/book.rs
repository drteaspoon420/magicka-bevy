//! Book page widgets. Submenus describe each frame's page content as the
//! original's GUI draws (atlas quads and bitmap-font text in 1024-pixel page
//! coordinates, Y down); this module keeps one retained entity per widget on
//! the Tome's page sheets and measures text for hit testing.
use crate::{
    frontend_tome::{MenuId, SheetRoot, PAGE_LAYER},
    gui_basic::GuiBasicMaterial,
    native_text::{geometry_mesh, upload_atlas},
    text_box_visual::read_asset,
};
use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::RenderLayers,
    image::{ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use magicka_content::{assets::Asset, texture::decode_rgba};
use magicka_runtime::{
    play_content::{bitmap_font, resolve},
    text_geometry::{GeometryFont, TextAlign, TextGeometry, TextStyle},
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

/// MagickaFont, by its content file name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Font {
    MenuOption,
    MenuTitle,
    MenuDefault,
    Maiandra14,
    Maiandra18,
}
impl Font {
    fn file(self) -> &'static str {
        match self {
            Font::MenuOption => "MenuOption",
            Font::MenuTitle => "MenuTitle",
            Font::MenuDefault => "MenuDefault",
            Font::Maiandra14 => "Maiandra14",
            Font::Maiandra18 => "Maiandra18",
        }
    }
}

/// GUI textures the submenus draw from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Atlas {
    /// `UI/Menu/Pages` (SubMenu.sPagesTexture).
    Pages,
    /// Any content texture by its path without extension.
    Content(String),
}
impl Atlas {
    fn path(&self) -> String {
        match self {
            Atlas::Pages => "UI/Menu/pages.xnb".into(),
            Atlas::Content(path) => format!("{path}.xnb"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    /// A texture region `source` (pixels) drawn at `size`.
    Image {
        atlas: Atlas,
        source: [f32; 4],
        size: [f32; 2],
    },
    /// Text drawn with Text.Draw at the widget position.
    Text {
        font: Font,
        text: String,
        align: TextAlign,
    },
    /// A flat colour rectangle of `size`.
    Fill { size: [f32; 2] },
    /// `image` through the alpha of `mask` (SubMenuCutscene.DrawMap,
    /// SubMenuIntro's illustrations): the mask spans the quad, the image is
    /// placed by the widget's `uv_transform` and clipped to its own extent.
    Masked {
        image: Atlas,
        mask: Atlas,
        size: [f32; 2],
    },
    /// A runtime image (the end-of-game capture) with a UV rectangle.
    Texture {
        image: Handle<Image>,
        uv: [f32; 4],
        size: [f32; 2],
    },
    /// A centred atlas quad of half-extent `half`, mapped through the native
    /// row-vector 2x2 transform `m` = [M11, M12, M21, M22] (Y down).
    Sprite {
        atlas: Atlas,
        source: [f32; 4],
        half: [f32; 2],
        m: [f32; 4],
    },
    /// SubMenuCharacterSelect.DrawAvatar: the thumbnail's upper half centred
    /// on the position, its lower-half mask soft-lit with the robe colour.
    Avatar {
        thumb: String,
        tint: [u8; 3],
        size: [f32; 2],
    },
}

/// One GUI draw on a page.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Widget {
    pub kind: Kind,
    /// Top-left (images) or the Text.Draw anchor (text), page pixels.
    pub position: [f32; 2],
    pub color: [f32; 4],
    pub saturation: f32,
    pub scale: f32,
    /// GUIBasicEffect TextureScale/TextureOffset (scale xy, offset zw),
    /// updated every frame without rebuilding the widget.
    pub uv_transform: [f32; 4],
}
impl Widget {
    pub(crate) fn image(
        atlas: Atlas,
        source: [f32; 4],
        position: [f32; 2],
        size: [f32; 2],
    ) -> Self {
        Self {
            kind: Kind::Image {
                atlas,
                source,
                size,
            },
            position,
            color: [1.0; 4],
            saturation: 1.0,
            scale: 1.0,
            uv_transform: [1.0, 1.0, 0.0, 0.0],
        }
    }
    pub(crate) fn text(
        font: Font,
        text: impl Into<String>,
        position: [f32; 2],
        align: TextAlign,
    ) -> Self {
        Self {
            kind: Kind::Text {
                font,
                text: text.into(),
                align,
            },
            position,
            color: magicka_runtime::menu_text_item::MENU_COLOR,
            saturation: 1.0,
            scale: 1.0,
            uv_transform: [1.0, 1.0, 0.0, 0.0],
        }
    }
    pub(crate) fn avatar(
        thumb: impl Into<String>,
        tint: [f32; 3],
        center: [f32; 2],
        size: [f32; 2],
    ) -> Self {
        Self {
            kind: Kind::Avatar {
                thumb: thumb.into(),
                // Quantised so a retained mesh is rebuilt only on real changes.
                tint: tint.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8),
                size,
            },
            position: center,
            color: [1.0; 4],
            saturation: 1.0,
            scale: 1.0,
            uv_transform: [1.0, 1.0, 0.0, 0.0],
        }
    }
    pub(crate) fn sprite(
        atlas: Atlas,
        source: [f32; 4],
        half: [f32; 2],
        m: [f32; 4],
        position: [f32; 2],
    ) -> Self {
        Self {
            kind: Kind::Sprite {
                atlas,
                source,
                half,
                m,
            },
            position,
            color: [1.0; 4],
            saturation: 1.0,
            scale: 1.0,
            uv_transform: [1.0, 1.0, 0.0, 0.0],
        }
    }
    pub(crate) fn masked(image: Atlas, mask: Atlas, position: [f32; 2], size: [f32; 2]) -> Self {
        Self {
            kind: Kind::Masked { image, mask, size },
            position,
            color: [1.0; 4],
            saturation: 1.0,
            scale: 1.0,
            uv_transform: [1.0, 1.0, 0.0, 0.0],
        }
    }
    pub(crate) fn uv_transform(mut self, uv_transform: [f32; 4]) -> Self {
        self.uv_transform = uv_transform;
        self
    }
    pub(crate) fn texture(
        image: Handle<Image>,
        uv: [f32; 4],
        position: [f32; 2],
        size: [f32; 2],
    ) -> Self {
        Self {
            kind: Kind::Texture { image, uv, size },
            position,
            color: [1.0; 4],
            saturation: 1.0,
            scale: 1.0,
            uv_transform: [1.0, 1.0, 0.0, 0.0],
        }
    }
    pub(crate) fn fill(position: [f32; 2], size: [f32; 2], color: [f32; 4]) -> Self {
        Self {
            kind: Kind::Fill { size },
            position,
            color,
            saturation: 1.0,
            scale: 1.0,
            uv_transform: [1.0, 1.0, 0.0, 0.0],
        }
    }
    pub(crate) fn color(mut self, color: [f32; 4]) -> Self {
        self.color = color;
        self
    }
    pub(crate) fn saturation(mut self, saturation: f32) -> Self {
        self.saturation = saturation;
        self
    }
    pub(crate) fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }
}

/// Render layer of screen-space GUI (message boxes), drawn by the menu
/// overlay camera after the Tome.
pub(crate) const SCREEN_LAYER: usize = 21;

/// Where widgets draw: one side of a menu's page, or the screen (pixels from
/// the window's top-left).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Surface {
    /// A menu's left/right page; the last flag is its previous content.
    Page(MenuId, bool, bool),
    Screen,
}

/// This frame's widgets for each surface.
#[derive(Resource, Default)]
pub(crate) struct PageDraw {
    pages: HashMap<Surface, Vec<Widget>>,
}
impl PageDraw {
    pub(crate) fn right(&mut self, menu: MenuId) -> PageWriter<'_> {
        PageWriter {
            list: self
                .pages
                .entry(Surface::Page(menu, true, false))
                .or_default(),
        }
    }
    #[allow(dead_code)]
    pub(crate) fn left(&mut self, menu: MenuId) -> PageWriter<'_> {
        PageWriter {
            list: self
                .pages
                .entry(Surface::Page(menu, false, false))
                .or_default(),
        }
    }
    /// The right page as it was before a riffle within `menu`.
    pub(crate) fn right_old(&mut self, menu: MenuId) -> PageWriter<'_> {
        PageWriter {
            list: self
                .pages
                .entry(Surface::Page(menu, true, true))
                .or_default(),
        }
    }
    pub(crate) fn screen(&mut self) -> PageWriter<'_> {
        PageWriter {
            list: self.pages.entry(Surface::Screen).or_default(),
        }
    }
}
pub(crate) struct PageWriter<'a> {
    list: &'a mut Vec<Widget>,
}
impl PageWriter<'_> {
    pub(crate) fn push(&mut self, widget: Widget) {
        self.list.push(widget);
    }
}

/// Text measurement for page layout (Font.MeasureText, kerned).
pub(crate) struct Measure<'a> {
    pub assets: &'a mut BookAssets,
    pub images: &'a mut Assets<Image>,
}
impl Measure<'_> {
    pub(crate) fn text(&mut self, font: Font, text: &str) -> [f32; 2] {
        self.assets.measure(font, text, self.images)
    }
    pub(crate) fn line_height(&mut self, font: Font) -> f32 {
        self.assets.line_height(font, self.images)
    }
}

struct LoadedFont {
    font: Arc<GeometryFont>,
    atlas: Handle<Image>,
}
/// Fonts and atlases, decoded on first use.
#[derive(Resource)]
pub(crate) struct BookAssets {
    content: PathBuf,
    fonts: HashMap<Font, LoadedFont>,
    atlases: HashMap<Atlas, (Handle<Image>, [f32; 2])>,
    white: Option<Handle<Image>>,
}
impl BookAssets {
    pub(crate) fn new(content: &str) -> Self {
        Self {
            content: PathBuf::from(content),
            fonts: HashMap::new(),
            atlases: HashMap::new(),
            white: None,
        }
    }
    fn font(&mut self, font: Font, images: &mut Assets<Image>) -> Result<&LoadedFont, String> {
        if !self.fonts.contains_key(&font) {
            let bitmap = bitmap_font(
                &self.content,
                &format!("Languages/eng/Font/{}", font.file()),
            )
            .map_err(|e| format!("font {}: {e:?}", font.file()))?;
            let (geometry, atlas) = upload_atlas(&bitmap, images)?;
            self.fonts.insert(
                font,
                LoadedFont {
                    font: geometry,
                    atlas,
                },
            );
        }
        Ok(&self.fonts[&font])
    }
    /// Text.Font.MeasureText (kerned): width and height in page pixels.
    pub(crate) fn measure(
        &mut self,
        font: Font,
        text: &str,
        images: &mut Assets<Image>,
    ) -> [f32; 2] {
        self.font(font, images)
            .ok()
            .and_then(|f| f.font.metrics.measure(text, true).ok())
            .unwrap_or([0.0; 2])
    }
    /// BitmapFont.Wrap (kerned) to a target width.
    pub(crate) fn wrap(
        &mut self,
        font: Font,
        text: &str,
        width: i32,
        images: &mut Assets<Image>,
    ) -> String {
        self.font(font, images)
            .ok()
            .and_then(|f| f.font.metrics.wrap(text, width, true).ok())
            .map(|w| w.text)
            .unwrap_or_else(|| text.to_string())
    }
    pub(crate) fn line_height(&mut self, font: Font, images: &mut Assets<Image>) -> f32 {
        self.font(font, images)
            .map(|f| f.font.metrics.line_height as f32)
            .unwrap_or(0.0)
    }
    fn atlas(
        &mut self,
        atlas: &Atlas,
        images: &mut Assets<Image>,
    ) -> Result<(Handle<Image>, [f32; 2]), String> {
        if let Some(found) = self.atlases.get(atlas) {
            return Ok(found.clone());
        }
        let path = resolve(&self.content, std::path::Path::new(&atlas.path()))
            .ok_or_else(|| format!("missing {}", atlas.path()))?;
        let Asset::Texture2D(texture) = read_asset(&path)? else {
            return Err(format!("expected texture {}", atlas.path()));
        };
        let size = [texture.width as f32, texture.height as f32];
        let handle = images.add(upload(&texture)?);
        self.atlases.insert(atlas.clone(), (handle.clone(), size));
        Ok((handle, size))
    }
    fn white(&mut self, images: &mut Assets<Image>) -> Handle<Image> {
        self.white
            .get_or_insert_with(|| {
                images.add(Image::new(
                    Extent3d::default(),
                    TextureDimension::D2,
                    vec![255; 4],
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::RENDER_WORLD,
                ))
            })
            .clone()
    }
}

pub(crate) fn upload(texture: &magicka_content::assets::Texture2D) -> Result<Image, String> {
    let rgba = decode_rgba(
        texture.format,
        texture.width as usize,
        texture.height as usize,
        &texture.mips[0],
    )
    .ok_or("texture decoding failed")?;
    let mut image = Image::new(
        Extent3d {
            width: texture.width as u32,
            height: texture.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    Ok(image)
}

/// A page-space quad from (0, 0) to (w, -h) with the given UV rectangle.
pub(crate) fn quad(size: [f32; 2], uv: [f32; 4]) -> Mesh {
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

/// The original GUI writes `1 - 2^-x` of its gamma-space colour into the
/// page target (GUIBasicEffect ScaleToHDR); the page models decode it.
pub(crate) fn page_material(
    texture: Handle<Image>,
    color: [f32; 4],
    vertex_color: bool,
) -> GuiBasicMaterial {
    let mut material = GuiBasicMaterial::textured(texture, Vec4::from_array(color));
    material.flags.y = u32::from(vertex_color);
    material.flags.w = 2;
    material
}

struct Retained {
    entity: Entity,
    kind: Kind,
    material: Handle<GuiBasicMaterial>,
}
#[derive(Resource, Default)]
struct Retain {
    roots: HashMap<Surface, Entity>,
    widgets: HashMap<Surface, Vec<Retained>>,
}

pub(crate) struct BookPlugin {
    pub content: String,
}
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct BookDrawSet;
impl Plugin for BookPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(BookAssets::new(&self.content))
            .init_resource::<PageDraw>()
            .init_resource::<Retain>()
            .add_systems(
                Update,
                reconcile
                    .after(BookDrawSet)
                    .before(crate::frontend_tome::TomeSet),
            )
            .add_systems(First, |mut draw: ResMut<PageDraw>| draw.pages.clear());
    }
}

/// Rebuild meshes whose content changed, restyle the rest, and drop widgets
/// a page no longer draws.
fn reconcile(
    mut commands: Commands,
    draw: Res<PageDraw>,
    mut retain: ResMut<Retain>,
    mut assets: ResMut<BookAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GuiBasicMaterial>>,
    mut transforms: Query<&mut Transform>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
) {
    let retain = &mut *retain;
    let window = windows
        .single()
        .map(|w| Vec2::new(w.width(), w.height()))
        .unwrap_or(Vec2::new(1024.0, 768.0));
    for (key, list) in retain.widgets.iter_mut() {
        if !draw.pages.contains_key(key) {
            for old in list.drain(..) {
                commands.entity(old.entity).despawn();
            }
        }
    }
    for (&key, widgets) in &draw.pages {
        let root = *retain.roots.entry(key).or_insert_with(|| match key {
            Surface::Page(menu, right, old) => commands
                .spawn((
                    SheetRoot { menu, right, old },
                    Transform::default(),
                    Visibility::Hidden,
                ))
                .id(),
            Surface::Screen => commands
                .spawn((Transform::default(), Visibility::Inherited))
                .id(),
        });
        let layer = match key {
            Surface::Page(..) => PAGE_LAYER,
            Surface::Screen => {
                // Widget (x - 512, 512 - y) lands on window pixel (x, y).
                if let Ok(mut transform) = transforms.get_mut(root) {
                    transform.translation =
                        Vec3::new(512.0 - window.x * 0.5, window.y * 0.5 - 512.0, 0.0);
                }
                SCREEN_LAYER
            }
        };
        let list = retain.widgets.entry(key).or_default();
        while list.len() > widgets.len() {
            let old = list.pop().unwrap();
            commands.entity(old.entity).despawn();
        }
        for (i, widget) in widgets.iter().enumerate() {
            // Later widgets draw above earlier ones, as in the native order.
            let translation = Vec3::new(
                widget.position[0] - 512.0,
                512.0 - widget.position[1],
                i as f32 * 0.01,
            );
            if let Some(retained) = list.get_mut(i) {
                if retained.kind == widget.kind {
                    if let Ok(mut transform) = transforms.get_mut(retained.entity) {
                        transform.translation = translation;
                        transform.scale = Vec3::new(widget.scale, widget.scale, 1.0);
                    }
                    if let Some(mut material) = materials.get_mut(&retained.material) {
                        material.color = Vec4::from_array(widget.color);
                        material.settings.x = widget.saturation;
                        if matches!(widget.kind, Kind::Masked { .. }) {
                            material.uv_transform = Vec4::from_array(widget.uv_transform);
                        }
                    }
                    continue;
                }
                commands.entity(retained.entity).despawn();
            }
            let Some((mesh, mut material)) = build(widget, &mut assets, &mut images) else {
                continue;
            };
            material.settings.x = widget.saturation;
            let material = materials.add(material);
            let entity = commands
                .spawn((
                    Mesh2d(meshes.add(mesh)),
                    MeshMaterial2d(material.clone()),
                    Transform::from_translation(translation).with_scale(Vec3::new(
                        widget.scale,
                        widget.scale,
                        1.0,
                    )),
                    RenderLayers::layer(layer),
                    ChildOf(root),
                ))
                .id();
            let retained = Retained {
                entity,
                kind: widget.kind.clone(),
                material,
            };
            if i < list.len() {
                list[i] = retained;
            } else {
                list.push(retained);
            }
        }
    }
}

fn build(
    widget: &Widget,
    assets: &mut BookAssets,
    images: &mut Assets<Image>,
) -> Option<(Mesh, GuiBasicMaterial)> {
    match &widget.kind {
        Kind::Image {
            atlas,
            source,
            size,
        } => {
            let (texture, dims) = assets
                .atlas(atlas, images)
                .map_err(|e| error!("menu page: {e}"))
                .ok()?;
            let [x, y, w, h] = *source;
            Some((
                quad(*size, [x / dims[0], y / dims[1], w / dims[0], h / dims[1]]),
                page_material(texture, widget.color, false),
            ))
        }
        Kind::Sprite {
            atlas,
            source,
            half,
            m,
        } => {
            let (texture, dims) = assets
                .atlas(atlas, images)
                .map_err(|e| error!("menu page: {e}"))
                .ok()?;
            let [x, y, w, h] = *source;
            let (u0, v0, u1, v1) = (
                x / dims[0],
                y / dims[1],
                (x + w) / dims[0],
                (y + h) / dims[1],
            );
            let corners = [
                ([-half[0], -half[1]], [u0, v0]),
                ([half[0], -half[1]], [u1, v0]),
                ([half[0], half[1]], [u1, v1]),
                ([-half[0], half[1]], [u0, v1]),
            ];
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
            );
            // Native Y is down; the page camera's is up.
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_POSITION,
                corners
                    .iter()
                    .map(|([px, py], _)| [px * m[0] + py * m[2], -(px * m[1] + py * m[3]), 0.0])
                    .collect::<Vec<_>>(),
            );
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_UV_0,
                corners.iter().map(|(_, uv)| *uv).collect::<Vec<_>>(),
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32; 4]; 4]);
            mesh.insert_indices(Indices::U16(vec![0, 2, 1, 0, 3, 2, 0, 1, 2, 0, 2, 3]));
            Some((mesh, page_material(texture, widget.color, false)))
        }
        Kind::Avatar { thumb, tint, size } => {
            let (texture, _) = assets
                .atlas(&Atlas::Content(thumb.clone()), images)
                .map_err(|e| error!("menu page: {e}"))
                .ok()?;
            let mut mesh = quad(*size, [0.0, 0.0, 1.0, 0.5]);
            // Centred on the position, as the avatar vertex buffer is.
            if let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
                mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
            {
                for p in positions.iter_mut() {
                    p[0] -= size[0] * 0.5;
                    p[1] += size[1] * 0.5;
                }
            }
            let mut material = page_material(texture, widget.color, false);
            material.flags.z = 1;
            material.settings.y = 0.5;
            material.overlay_tint = Vec4::new(
                tint[0] as f32 / 255.0,
                tint[1] as f32 / 255.0,
                tint[2] as f32 / 255.0,
                1.0,
            );
            Some((mesh, material))
        }
        Kind::Masked { image, mask, size } => {
            let (image, _) = assets
                .atlas(image, images)
                .map_err(|e| error!("menu page: {e}"))
                .ok()?;
            let (mask, _) = assets
                .atlas(mask, images)
                .map_err(|e| error!("menu page: {e}"))
                .ok()?;
            let mut material = page_material(image, widget.color, false);
            material.overlay = mask;
            material.flags.z = 2;
            material.uv_transform = Vec4::from_array(widget.uv_transform);
            Some((quad(*size, [0.0, 0.0, 1.0, 1.0]), material))
        }
        Kind::Texture { image, uv, size } => Some((
            quad(*size, *uv),
            page_material(image.clone(), widget.color, false),
        )),
        Kind::Fill { size } => {
            let white = assets.white(images);
            Some((
                quad(*size, [0.0, 0.0, 1.0, 1.0]),
                page_material(white, widget.color, false),
            ))
        }
        Kind::Text { font, text, align } => {
            let loaded = assets
                .font(*font, images)
                .map_err(|e| error!("menu page: {e}"))
                .ok()?;
            let geometry = TextGeometry::build(
                &loaded.font,
                &text.encode_utf16().collect::<Vec<_>>(),
                TextStyle {
                    align: *align,
                    // Text.Draw colours come from the effect, not the string.
                    default_color: [1.0; 4],
                    ..default()
                },
            )
            .ok()?;
            let mesh = geometry_mesh(&geometry, geometry.quads.len() as i32).ok()?;
            Some((
                mesh,
                page_material(loaded.atlas.clone(), widget.color, true),
            ))
        }
    }
}
