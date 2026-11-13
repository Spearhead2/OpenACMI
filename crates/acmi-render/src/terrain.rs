//! Streamed terrain: a quadtree of elevation tiles draped over the globe.
//!
//! Every frame the tree is walked from a fixed set of root tiles. A tile is
//! split into its four children when its texels would cover more than about
//! one screen pixel, provided all four children are loaded; otherwise the
//! tile itself is drawn and the missing children are requested. The view is
//! therefore always complete and simply sharpens as data arrives.
//!
//! Each tile is a small grid mesh displaced by its surface heights (water
//! is flat; land in basins below sea level is negative) with a skirt around the edge
//! that hides the cracks between neighbours of different detail.

use std::sync::Arc;
use std::time::{Duration, Instant};

use acmi_core::geo::{self, EARTH_RADIUS_M};
use acmi_terrain::{ElevationTile, FetchError, TileData, TileKey, TileLoader};
use glam::DVec3;
use rustc_hash::FxHashMap;
use wgpu::util::DeviceExt;

use crate::camera::Camera;
use crate::geometry::SphereVertex;

/// Quads along one side of a tile mesh.
const GRID: u32 = 64;
const GRID_VERTICES: u32 = GRID + 1;
/// Tiles uploaded to the GPU per frame at most, to keep frames short.
const UPLOADS_PER_FRAME: usize = 8;
/// Tiles requested from the loader at a time.
const MAX_REQUESTS: usize = 48;
/// How long before a failed download is tried again.
const RETRY_AFTER: Duration = Duration::from_secs(20);
/// Frames a tile may go unused before it can be evicted.
const EVICT_AFTER_FRAMES: u64 = 600;
/// Colour of tiles whose data has not arrived yet (sRGB).
const PLACEHOLDER_COLOR: [u8; 4] = [20, 48, 92, 255];
/// Highest terrain on Earth, for conservative horizon culling.
const MAX_ELEVATION_M: f64 = 9000.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainConfig {
    /// Zoom level of the always-present base tiles.
    pub root_zoom: u8,
    /// Deepest zoom level requested.
    pub max_zoom: u8,
    /// Screen pixels one terrain texel may cover before the tile is split.
    /// Lower is sharper and loads more tiles.
    pub pixels_per_texel: f32,
    /// Multiplier applied to heights.
    pub exaggeration: f32,
    /// Tiles kept on the GPU before unused ones are dropped.
    pub max_tiles: usize,
}

impl Default for TerrainConfig {
    fn default() -> Self {
        Self { root_zoom: 3, max_zoom: 12, pixels_per_texel: 1.5, exaggeration: 1.0, max_tiles: 900 }
    }
}

/// Counters for the render-information overlay.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TerrainStats {
    pub drawn: usize,
    pub loaded: usize,
    pub pending: usize,
}

struct Tile {
    vertices: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    elevation: Arc<ElevationTile>,
    /// Bounding sphere in world space.
    center: DVec3,
    radius: f64,
    /// Latitude of the tile centre, for its ground resolution.
    latitude: f64,
    last_used: u64,
    /// True until real data replaces the flat stand-in.
    placeholder: bool,
}

pub(crate) struct TerrainLayer {
    loader: TileLoader,
    config: TerrainConfig,
    tiles: FxHashMap<TileKey, Tile>,
    /// Tiles that could not be loaded, with the time to retry (`None`: never).
    failed: FxHashMap<TileKey, Option<Instant>>,
    indices: wgpu::Buffer,
    index_count: u32,
    sampler: wgpu::Sampler,
    placeholder_view: wgpu::TextureView,
    frame: u64,
    draw_list: Vec<TileKey>,
    wanted: Vec<(f64, TileKey)>,
}

impl TerrainLayer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_layout: &wgpu::BindGroupLayout,
        loader: TileLoader,
        mut config: TerrainConfig,
    ) -> Self {
        config.root_zoom = config.root_zoom.clamp(1, 6);
        config.max_zoom = config.max_zoom.clamp(config.root_zoom, 15);

        let indices = grid_indices();
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("terrain sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let mut layer = Self {
            loader,
            config,
            tiles: FxHashMap::default(),
            failed: FxHashMap::default(),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("terrain indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            index_count: indices.len() as u32,
            sampler,
            placeholder_view: create_texture(device, queue, 1, &PLACEHOLDER_COLOR),
            frame: 0,
            draw_list: Vec::new(),
            wanted: Vec::new(),
        };

        // The base level always exists, flat and plain until data arrives,
        // so the globe is never incomplete.
        let flat = Arc::new(ElevationTile::flat());
        let side = 1u32 << config.root_zoom;
        for y in 0..side {
            for x in 0..side {
                let key = TileKey::new(config.root_zoom, x, y);
                let tile = layer.build_tile(device, texture_layout, key, Arc::clone(&flat), None);
                layer.tiles.insert(key, tile);
            }
        }
        layer
    }

    pub fn stats(&self) -> TerrainStats {
        TerrainStats {
            drawn: self.draw_list.len(),
            loaded: self.tiles.values().filter(|tile| !tile.placeholder).count(),
            pending: self.loader.pending(),
        }
    }

    /// Whether tiles are still on their way for the current view.
    pub fn is_busy(&self) -> bool {
        !self.wanted.is_empty() || self.loader.pending() > 0
    }

    /// Height of the ground (water counts as 0) at a location, from the most
    /// detailed tile currently loaded there. Includes vertical exaggeration.
    pub fn height_at(&self, latitude: f64, longitude: f64) -> Option<f32> {
        (self.config.root_zoom..=self.config.max_zoom).rev().find_map(|zoom| {
            let (key, u, v) = TileKey::locate(zoom, latitude, longitude)?;
            let tile = self.tiles.get(&key).filter(|tile| !tile.placeholder)?;
            Some(tile.elevation.sample(u, v) * self.config.exaggeration)
        })
    }

    /// Takes in finished tiles, chooses what to draw and what to request.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_layout: &wgpu::BindGroupLayout,
        camera: &Camera,
        viewport: (u32, u32),
    ) {
        self.frame += 1;
        self.receive(device, queue, texture_layout);

        self.draw_list.clear();
        self.wanted.clear();
        let view = View::new(camera, viewport);
        let side = 1u32 << self.config.root_zoom;
        for y in 0..side {
            for x in 0..side {
                self.visit(TileKey::new(self.config.root_zoom, x, y), &view);
            }
        }

        // Nearest first: what the camera looks at sharpens before the horizon.
        self.wanted.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        self.wanted.truncate(MAX_REQUESTS);
        let keys: Vec<TileKey> = self.wanted.iter().map(|(_, key)| *key).collect();
        self.loader.set_wanted(&keys);

        self.evict();
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        for key in &self.draw_list {
            let tile = &self.tiles[key];
            pass.set_bind_group(1, &tile.bind_group, &[]);
            pass.set_vertex_buffer(0, tile.vertices.slice(..));
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }
    }

    fn receive(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, texture_layout: &wgpu::BindGroupLayout) {
        for _ in 0..UPLOADS_PER_FRAME {
            let Some((key, result)) = self.loader.poll() else { break };
            match result {
                // Nothing to do if the tile is already there.
                Ok(_) if self.tiles.get(&key).is_some_and(|tile| !tile.placeholder) => {}
                Ok(TileData { elevation, color }) => {
                    // The image need not have the resolution of the heights.
                    let view = create_texture(device, queue, (color.len() / 4).isqrt() as u32, &color);
                    let tile = self.build_tile(device, texture_layout, key, elevation, Some(&view));
                    self.tiles.insert(key, tile);
                    self.failed.remove(&key);
                }
                Err(error) => {
                    let retry = error.is_transient().then(|| Instant::now() + RETRY_AFTER);
                    if !matches!(error, FetchError::Offline) {
                        log::warn!("terrain tile {}/{}/{}: {error}", key.z, key.x, key.y);
                    }
                    self.failed.insert(key, retry);
                }
            }
        }
    }

    /// Whether a tile that is not loaded should be asked for.
    fn may_request(&self, key: TileKey) -> bool {
        match self.failed.get(&key) {
            None => true,
            Some(Some(retry_at)) => Instant::now() >= *retry_at,
            Some(None) => false,
        }
    }

    fn visit(&mut self, key: TileKey, view: &View) {
        let Some(tile) = self.tiles.get_mut(&key) else { return };
        tile.last_used = self.frame;
        let (center, radius, latitude, placeholder) = (tile.center, tile.radius, tile.latitude, tile.placeholder);
        if !view.can_see(center, radius) {
            return;
        }
        let distance = (center.distance(view.eye) - radius).max(1.0);
        if placeholder && self.may_request(key) {
            self.wanted.push((distance, key));
        }

        let texel_pixels = key.meters_per_pixel(latitude) / view.camera.meters_per_pixel(distance, view.height);
        if key.z < self.config.max_zoom && texel_pixels > self.config.pixels_per_texel as f64 {
            let children = key.children();
            let mut all_loaded = true;
            for child in children {
                match self.tiles.get_mut(&child) {
                    // Waiting for its siblings: still needed, so not evictable.
                    Some(tile) => tile.last_used = self.frame,
                    None => {
                        all_loaded = false;
                        if self.may_request(child) {
                            self.wanted.push((distance, child));
                        }
                    }
                }
            }
            if all_loaded {
                for child in children {
                    self.visit(child, view);
                }
                return;
            }
        }
        self.draw_list.push(key);
    }

    /// Drops the least recently used tiles once over budget. Base tiles stay.
    fn evict(&mut self) {
        if self.tiles.len() <= self.config.max_tiles {
            return;
        }
        let root_zoom = self.config.root_zoom;
        let stale_before = self.frame.saturating_sub(EVICT_AFTER_FRAMES);
        let mut candidates: Vec<(u64, TileKey)> = self
            .tiles
            .iter()
            .filter(|(key, tile)| key.z > root_zoom && tile.last_used < stale_before)
            .map(|(key, tile)| (tile.last_used, *key))
            .collect();
        candidates.sort_unstable();
        let excess = self.tiles.len() - self.config.max_tiles;
        for (_, key) in candidates.into_iter().take(excess) {
            self.tiles.remove(&key);
        }
    }

    /// Builds the mesh and GPU resources of a tile. Without a `texture` the
    /// tile is a placeholder using the plain stand-in colour.
    fn build_tile(
        &self,
        device: &wgpu::Device,
        texture_layout: &wgpu::BindGroupLayout,
        key: TileKey,
        elevation: Arc<ElevationTile>,
        texture: Option<&wgpu::TextureView>,
    ) -> Tile {
        let exaggeration = self.config.exaggeration as f64;
        let height = |u: f64, v: f64| elevation.sample(u, v) as f64 * exaggeration;
        let latitude = key.latitude(0.5);
        // Deep enough to cover the height difference to a coarser neighbour.
        let skirt = (key.meters_per_pixel(latitude) * acmi_terrain::TILE_SIZE as f64 * 0.04).max(200.0);

        let (low, high) = elevation.range();
        let mid_height = (low + high) as f64 * 0.5 * exaggeration;
        let center = geo::geo_to_world(latitude, key.longitude(0.5), mid_height);

        let mut vertices: Vec<SphereVertex> = Vec::with_capacity((GRID_VERTICES * GRID_VERTICES + 4 * GRID_VERTICES) as usize);
        let mut radius_squared: f64 = 0.0;
        let step = 1.0 / GRID as f64;
        for row in 0..GRID_VERTICES {
            let v = row as f64 * step;
            let lat = key.latitude(v);
            for column in 0..GRID_VERTICES {
                let u = column as f64 * step;
                let world = geo::geo_to_world(lat, key.longitude(u), height(u, v));
                radius_squared = radius_squared.max(world.distance_squared(center));
                vertices.push(SphereVertex { position: world.into(), uv: [u as f32, v as f32] });
            }
        }
        // Skirts: the four edges again, pushed down. Order matches `grid_indices`.
        let mut skirt_vertex = |u: f64, v: f64| {
            let world = geo::geo_to_world(key.latitude(v), key.longitude(u), height(u, v) - skirt);
            vertices.push(SphereVertex { position: world.into(), uv: [u as f32, v as f32] });
        };
        for edge in 0..4 {
            for i in 0..GRID_VERTICES {
                let t = i as f64 * step;
                match edge {
                    0 => skirt_vertex(t, 0.0),
                    1 => skirt_vertex(t, 1.0),
                    2 => skirt_vertex(0.0, t),
                    _ => skirt_vertex(1.0, t),
                }
            }
        }

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain tile"),
            layout: texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(texture.unwrap_or(&self.placeholder_view)),
                },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        Tile {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("terrain tile"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            bind_group,
            elevation,
            center,
            radius: radius_squared.sqrt(),
            latitude,
            last_used: self.frame,
            placeholder: texture.is_none(),
        }
    }
}

/// Camera-derived values used to cull and rank tiles.
struct View<'a> {
    camera: &'a Camera,
    eye: DVec3,
    /// Viewport height in pixels.
    height: f32,
    /// Unit vector from the Earth's centre to the eye.
    eye_direction: DVec3,
    /// Angle from the eye's nadir to the farthest ground it can see.
    horizon_angle: f64,
    /// `(sin, cos)` of the half field of view, horizontally and vertically.
    side_planes: [(f64, f64); 2],
}

impl<'a> View<'a> {
    fn new(camera: &'a Camera, viewport: (u32, u32)) -> Self {
        let eye_distance = camera.eye.length().max(EARTH_RADIUS_M + 1.0);
        // The horizon of the eye plus that of the highest possible peak.
        let horizon_angle = (EARTH_RADIUS_M / eye_distance).acos() + (EARTH_RADIUS_M / (EARTH_RADIUS_M + MAX_ELEVATION_M)).acos();
        let tan_y = (camera.fov_y as f64 * 0.5).tan();
        let tan_x = tan_y * viewport.0 as f64 / viewport.1.max(1) as f64;
        Self {
            camera,
            eye: camera.eye,
            height: viewport.1 as f32,
            eye_direction: camera.eye.normalize_or(DVec3::Z),
            horizon_angle,
            side_planes: [tan_x.atan().sin_cos(), tan_y.atan().sin_cos()],
        }
    }

    /// Conservative visibility of a bounding sphere: not beyond the horizon
    /// and not outside the view frustum.
    fn can_see(&self, center: DVec3, radius: f64) -> bool {
        let angle = self.eye_direction.dot(center.normalize_or(DVec3::Z)).clamp(-1.0, 1.0).acos();
        let angular_radius = (radius / EARTH_RADIUS_M).min(1.0).asin();
        if angle > self.horizon_angle + angular_radius {
            return false;
        }

        let local = self.camera.rotation.conjugate() * (center - self.eye);
        if local.z > radius {
            return false; // entirely behind the camera
        }
        let [(sin_x, cos_x), (sin_y, cos_y)] = self.side_planes;
        // Signed distance to each side plane; negative is outside.
        let inside = |lateral: f64, sin: f64, cos: f64| -lateral.abs() * cos - local.z * sin >= -radius;
        inside(local.x, sin_x, cos_x) && inside(local.y, sin_y, cos_y)
    }
}

/// Index buffer shared by all tiles: the grid, then a strip per edge
/// joining it to the skirt vertices.
fn grid_indices() -> Vec<u32> {
    let mut indices = Vec::with_capacity((GRID * GRID * 6 + 4 * GRID * 6) as usize);
    for row in 0..GRID {
        for column in 0..GRID {
            let a = row * GRID_VERTICES + column;
            let (b, c, d) = (a + 1, a + GRID_VERTICES, a + GRID_VERTICES + 1);
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let skirt_base = GRID_VERTICES * GRID_VERTICES;
    // Grid index of the i-th vertex along each edge, in skirt order.
    let edge_vertex = |edge: u32, i: u32| match edge {
        0 => i,
        1 => GRID * GRID_VERTICES + i,
        2 => i * GRID_VERTICES,
        _ => i * GRID_VERTICES + GRID,
    };
    for edge in 0..4 {
        for i in 0..GRID {
            let (top_a, top_b) = (edge_vertex(edge, i), edge_vertex(edge, i + 1));
            let (low_a, low_b) = (skirt_base + edge * GRID_VERTICES + i, skirt_base + edge * GRID_VERTICES + i + 1);
            indices.extend_from_slice(&[top_a, low_a, top_b, top_b, low_a, low_b]);
        }
    }
    indices
}

fn create_texture(device: &wgpu::Device, queue: &wgpu::Queue, size: u32, rgba: &[u8]) -> wgpu::TextureView {
    let extent = wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 };
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("terrain tile"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        rgba,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_buffer_is_well_formed() {
        let indices = grid_indices();
        let vertex_count = GRID_VERTICES * GRID_VERTICES + 4 * GRID_VERTICES;
        assert_eq!(indices.len() as u32, GRID * GRID * 6 + 4 * GRID * 6);
        assert!(indices.iter().all(|&i| i < vertex_count));
        // Every skirt vertex is referenced.
        let used: std::collections::HashSet<u32> = indices.iter().copied().collect();
        assert!((GRID_VERTICES * GRID_VERTICES..vertex_count).all(|i| used.contains(&i)));
    }

    #[test]
    fn view_culls_far_side_and_off_screen() {
        let mut camera = Camera { eye: geo::geo_to_world(0.0, 0.0, 400_000.0), ..Default::default() };
        camera.look_at(DVec3::ZERO, DVec3::Y);
        let view = View::new(&camera, (1280, 720));

        let below = geo::geo_to_world(0.0, 0.0, 0.0);
        let far_side = geo::geo_to_world(0.0, 180.0, 0.0);
        let near_but_off_screen = geo::geo_to_world(0.0, 15.0, 0.0);
        assert!(view.can_see(below, 10_000.0));
        assert!(!view.can_see(far_side, 10_000.0));
        assert!(!view.can_see(near_but_off_screen, 10_000.0), "within the horizon but outside the frustum");
        // A huge tile centred off screen still reaches into view.
        assert!(view.can_see(near_but_off_screen, 2_000_000.0));
    }
}
