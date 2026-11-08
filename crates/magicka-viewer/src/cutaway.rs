//! Optional camera-dependent wall cutaway (`docs/CUTAWAY_IMPLEMENTATION.md`).
//! Not an original Magicka feature: when enabled (play mode, F1 debug menu),
//! level architecture that hides the navigable floor from the current camera
//! is trimmed above a knee height, with a faint darkened cut edge.
//!
//! The navmesh is drawn alone by a depth-only helper camera; its depth is
//! copied (`effect_depth::copy_depth`) into [`CutawayBindings::depth`], which
//! `terrain.wgsl` compares against eligible fragments. A navmesh height grid
//! ([`HeightGrid`]) supplies the floor elevation under each wall column, so
//! the whole column above the knee is cut, not only the pixels that happen to
//! have floor directly behind them. Presentation only: collision, navigation
//! and gameplay never see this.
use bevy::{
    asset::RenderAssetUsages,
    core_pipeline::{
        core_3d::{main_opaque_pass_3d, main_transparent_pass_3d},
        schedule::{Core3d, Core3dSystems},
    },
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::{
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_resource::{Extent3d, PipelineCache, ShaderType, TextureDimension, TextureFormat},
        renderer::{RenderContext, ViewQuery},
        storage::ShaderBuffer,
        texture::GpuImage,
        view::ViewDepthTexture,
        RenderApp,
    },
};
use magicka_content::level::NavMesh;

/// Render layer holding only the navmesh floor proxy.
pub const LAYER: usize = 13;
/// Height-grid value for cells with no navigable floor nearby.
pub const NO_FLOOR: f32 = 1.0e30;
/// `MovementProperties.Fly` (research/navmesh.md): fly-only triangles are not
/// floor the player walks on.
const FLY: u8 = 4;
/// Largest height-grid side; the cell grows for very large levels.
const MAX_GRID: u32 = 1024;

/// Shared shader parameters (`terrain.wgsl` `Cutaway`, binding 91).
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct GpuCutaway {
    /// x enabled, y knee height and z depth epsilon (world units), w cut-edge
    /// width (physical pixels).
    pub params: Vec4,
    /// x cut-edge darkening, yz floor-depth target size (pixels), w grid cell.
    pub target: Vec4,
    /// xy grid origin (world x, z), zw grid size (cells).
    pub grid: Vec4,
}

/// GPU resources every [`crate::terrain::TerrainMaterial`] binds. The handles
/// never change; only their contents do. Bind groups keep the texture views
/// they were built with, so materials must be re-prepared after an image is
/// replaced (resize, new level; see `play/cutaway.rs`).
#[derive(Resource, Clone, ExtractResource)]
pub struct CutawayBindings {
    pub buffer: Handle<ShaderBuffer>,
    /// Navmesh floor depth (`R32Float`, Bevy reverse-Z, 0 = no floor).
    pub depth: Handle<Image>,
    /// [`HeightGrid`] heights (`R32Float`, [`NO_FLOOR`] where empty).
    pub heights: Handle<Image>,
}

impl FromWorld for CutawayBindings {
    fn from_world(world: &mut World) -> Self {
        let buffer = world
            .resource_mut::<Assets<ShaderBuffer>>()
            .add(ShaderBuffer::from(GpuCutaway::default()));
        let mut images = world.resource_mut::<Assets<Image>>();
        Self {
            buffer,
            depth: images.add(crate::effect_depth::depth_image(UVec2::ONE)),
            heights: images.add(height_image(&HeightGrid::empty())),
        }
    }
}

/// The helper camera whose depth becomes [`CutawayBindings::depth`].
#[derive(Component, Clone, Copy, ExtractComponent)]
pub struct CutawayDepthCamera;

/// Render-side copy of the helper camera's depth. Requires
/// `EffectDepthPlugin` (its copy pipelines), which play mode always has.
pub struct CutawayRenderPlugin;

impl Plugin for CutawayRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            ExtractResourcePlugin::<CutawayBindings>::default(),
            ExtractComponentPlugin::<CutawayDepthCamera>::default(),
        ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        // The helper camera has a lower order, so its copy finishes before
        // the gameplay camera's passes sample it in the same frame.
        render_app.add_systems(
            Core3d,
            snapshot
                .in_set(Core3dSystems::MainPass)
                .after(main_opaque_pass_3d)
                .before(main_transparent_pass_3d),
        );
    }
}

fn snapshot(
    view: ViewQuery<(&ViewDepthTexture, &Msaa), With<CutawayDepthCamera>>,
    bindings: Res<CutawayBindings>,
    images: Res<RenderAssets<GpuImage>>,
    pipelines: Option<Res<crate::effect_depth::Pipelines>>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (source, msaa) = view.into_inner();
    let (Some(target), Some(pipelines)) = (images.get(&bindings.depth), pipelines) else {
        return;
    };
    // A stale (pre-resize) image would be indexed with the wrong pixels.
    if target.texture_descriptor.size.width != source.texture.width()
        || target.texture_descriptor.size.height != source.texture.height()
    {
        return;
    }
    crate::effect_depth::copy_depth(source, msaa, target, &pipelines, &cache, &mut ctx);
}

/// Navigable floor triangles in world space. Fly-only triangles and
/// malformed indices are skipped. Winding is left as authored (retail is
/// clockwise from above); the proxy is drawn double-sided.
pub fn floor_triangles(nav: &NavMesh) -> Vec<[Vec3; 3]> {
    nav.triangles
        .iter()
        .filter(|t| t.properties & FLY == 0)
        .filter_map(|t| {
            let v = t
                .vertices
                .map(|i| nav.vertices.get(usize::from(i)).copied().map(Vec3::from));
            let [Some(a), Some(b), Some(c)] = v else {
                return None;
            };
            let finite = [a, b, c].iter().all(|p| p.is_finite());
            (finite && (b - a).cross(c - a).length_squared() > 1e-12).then_some([a, b, c])
        })
        .collect()
}

/// Render-only floor mesh for the depth camera.
pub fn proxy_mesh(triangles: &[[Vec3; 3]]) -> Mesh {
    let positions: Vec<[f32; 3]> = triangles
        .iter()
        .flat_map(|t| t.map(|p| p.to_array()))
        .collect();
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    let indices = (0..positions.len() as u32).collect();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}

/// Lowest navigable elevation per XZ cell, dilated by a padding radius so wall
/// columns standing just outside the navmesh find the floor next to them.
/// Stacked floors use the lowest one (a known limitation).
#[derive(Clone, Debug, PartialEq)]
pub struct HeightGrid {
    pub origin: Vec2,
    pub cell: f32,
    pub size: UVec2,
    pub heights: Vec<f32>,
}

impl HeightGrid {
    pub fn empty() -> Self {
        Self {
            origin: Vec2::ZERO,
            cell: 1.0,
            size: UVec2::ONE,
            heights: vec![NO_FLOOR],
        }
    }

    pub fn build(triangles: &[[Vec3; 3]], cell: f32, padding: f32) -> Option<Self> {
        if triangles.is_empty() || cell <= 0.0 {
            return None;
        }
        let padding = padding.max(0.0);
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for p in triangles.iter().flatten() {
            lo = lo.min(p.xz());
            hi = hi.max(p.xz());
        }
        lo -= Vec2::splat(padding + cell);
        hi += Vec2::splat(padding + cell);
        let extent = hi - lo;
        let cell = cell.max(extent.max_element() / MAX_GRID as f32);
        let size = (extent / cell).ceil().as_uvec2().max(UVec2::ONE);
        let mut grid = Self {
            origin: lo,
            cell,
            size,
            heights: vec![NO_FLOOR; (size.x * size.y) as usize],
        };
        for t in triangles {
            grid.rasterize(t);
        }
        grid.dilate((padding / cell).ceil() as u32);
        Some(grid)
    }

    fn index(&self, c: IVec2) -> Option<usize> {
        (c.x >= 0 && c.y >= 0 && (c.x as u32) < self.size.x && (c.y as u32) < self.size.y)
            .then(|| (c.y as u32 * self.size.x + c.x as u32) as usize)
    }

    fn cell_of(&self, xz: Vec2) -> IVec2 {
        ((xz - self.origin) / self.cell).floor().as_ivec2()
    }

    fn lower(&mut self, c: IVec2, y: f32) {
        if let Some(i) = self.index(c) {
            self.heights[i] = self.heights[i].min(y);
        }
    }

    /// Cells whose centre lies in the triangle (in XZ), plus each vertex's
    /// cell so slivers narrower than a cell still register.
    fn rasterize(&mut self, t: &[Vec3; 3]) {
        for p in t {
            self.lower(self.cell_of(p.xz()), p.y);
        }
        let [a, b, c] = t.map(|p| p.xz());
        let det = (b - a).perp_dot(c - a);
        if det.abs() < 1e-12 {
            return;
        }
        let lo = self.cell_of(a.min(b).min(c));
        let hi = self.cell_of(a.max(b).max(c));
        for y in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                let p = self.origin + (Vec2::new(x as f32, y as f32) + 0.5) * self.cell;
                let u = (p - a).perp_dot(c - a) / det;
                let v = (b - a).perp_dot(p - a) / det;
                let w = 1.0 - u - v;
                const E: f32 = -1e-4;
                if u >= E && v >= E && w >= E {
                    self.lower(IVec2::new(x, y), t[0].y * w + t[1].y * u + t[2].y * v);
                }
            }
        }
    }

    /// Fill empty cells from their lowest filled 8-neighbour, `steps` times.
    fn dilate(&mut self, steps: u32) {
        for _ in 0..steps {
            let source = self.heights.clone();
            let mut changed = false;
            for y in 0..self.size.y as i32 {
                for x in 0..self.size.x as i32 {
                    let i = self.index(IVec2::new(x, y)).unwrap();
                    if source[i] < NO_FLOOR {
                        continue;
                    }
                    let mut best = NO_FLOOR;
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            if let Some(j) = self.index(IVec2::new(x + dx, y + dy)) {
                                best = best.min(source[j]);
                            }
                        }
                    }
                    if best < NO_FLOOR {
                        self.heights[i] = best;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// Floor height at a world XZ position (nearest cell; the shader
    /// interpolates).
    #[cfg(test)]
    pub fn at(&self, xz: Vec2) -> f32 {
        self.index(self.cell_of(xz))
            .map_or(NO_FLOOR, |i| self.heights[i])
    }
}

pub fn height_image(grid: &HeightGrid) -> Image {
    let data = grid.heights.iter().flat_map(|h| h.to_le_bytes()).collect();
    Image::new(
        Extent3d {
            width: grid.size.x,
            height: grid.size.y,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::R32Float,
        // Kept in the main world so the next level can replace it in place.
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use magicka_content::level::NavTriangle;

    fn tri(vertices: [u16; 3], properties: u8) -> NavTriangle {
        NavTriangle {
            vertices,
            neighbours: [0xFFFF; 3],
            costs: [0.0; 3],
            properties,
        }
    }

    /// A 4×4 floor at y = 1, clockwise from above like retail data.
    fn square() -> NavMesh {
        NavMesh {
            vertices: vec![
                [0.0, 1.0, 0.0],
                [4.0, 1.0, 0.0],
                [4.0, 1.0, 4.0],
                [0.0, 1.0, 4.0],
            ],
            triangles: vec![tri([0, 2, 1], 0), tri([0, 3, 2], 0)],
        }
    }

    #[test]
    fn floor_triangles_skip_fly_degenerate_and_bad_indices() {
        let mut nav = square();
        nav.triangles.push(tri([0, 1, 2], FLY));
        nav.triangles.push(tri([0, 1, 9], 0));
        nav.triangles.push(tri([0, 0, 1], 0));
        nav.triangles.push(tri([0, 1, 2], 1)); // water is still floor
        assert_eq!(floor_triangles(&nav).len(), 3);
        assert!(floor_triangles(&NavMesh::default()).is_empty());
    }

    #[test]
    fn empty_navmesh_has_no_grid() {
        assert_eq!(HeightGrid::build(&[], 0.5, 2.0), None);
    }

    #[test]
    fn grid_covers_floor_and_padding_only() {
        let grid = HeightGrid::build(&floor_triangles(&square()), 0.5, 1.0).unwrap();
        assert_eq!(grid.at(Vec2::new(2.0, 2.0)), 1.0);
        // Inside the padding ring next to the floor.
        assert_eq!(grid.at(Vec2::new(-0.7, 2.0)), 1.0);
        assert_eq!(grid.at(Vec2::new(4.7, 4.7)), 1.0);
        // Beyond it.
        assert_eq!(grid.at(Vec2::new(-1.3, 2.0)), NO_FLOOR);
        assert_eq!(grid.at(Vec2::new(100.0, 2.0)), NO_FLOOR);
    }

    #[test]
    fn grid_follows_ramps_and_keeps_the_lowest_stacked_floor() {
        let ramp = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(10.0, 5.0, 0.0),
            Vec3::new(0.0, 0.0, 10.0),
        ];
        let grid = HeightGrid::build(&[ramp], 0.5, 0.0).unwrap();
        assert!((grid.at(Vec2::new(4.25, 2.25)) - 2.125).abs() < 0.3);
        let upper = ramp.map(|p| p + Vec3::Y * 8.0);
        let stacked = HeightGrid::build(&[upper, ramp], 0.5, 0.0).unwrap();
        assert_eq!(
            stacked.at(Vec2::new(1.25, 1.25)),
            grid.at(Vec2::new(1.25, 1.25))
        );
    }

    #[test]
    fn grid_size_is_capped_for_huge_levels() {
        let huge = [
            Vec3::ZERO,
            Vec3::new(5000.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 3000.0),
        ];
        let grid = HeightGrid::build(&[huge], 0.5, 2.0).unwrap();
        assert!(grid.size.max_element() <= MAX_GRID);
        assert_eq!(grid.heights.len(), (grid.size.x * grid.size.y) as usize);
    }

    #[test]
    fn height_image_is_r32_float_of_grid_size() {
        let grid = HeightGrid::build(&floor_triangles(&square()), 0.5, 1.0).unwrap();
        let image = height_image(&grid);
        assert_eq!(image.width(), grid.size.x);
        assert_eq!(image.height(), grid.size.y);
        assert_eq!(image.texture_descriptor.format, TextureFormat::R32Float);
    }
}
