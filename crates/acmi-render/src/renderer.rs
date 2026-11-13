//! The wgpu renderer.
//!
//! Renders into its own off-screen target (multisampled colour + depth,
//! resolved to a texture), so it is independent of whatever presents the
//! result: the application shows [`Renderer::color_view`] inside its UI.
//!
//! Draw order per frame:
//!
//! 1. globe, or streamed terrain plus polar caps (opaque, writes depth)
//! 2. sky (full screen, only where no ground was drawn)
//! 4. object models (opaque, instanced)
//! 5. trails: ribbons, then lines (alpha blended, depth tested)
//! 6. overlay lines (always on top)

use std::ops::Range;
use std::path::{Path, PathBuf};

use acmi_core::geo::EARTH_RADIUS_M;
use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Mat4, Quat, Vec3};
use rustc_hash::FxHashMap;
use wgpu::util::DeviceExt;

use crate::camera::Camera;
use crate::earth_texture::EarthImage;
use crate::geometry::{self, GridStyle, LineVertex, RibbonVertex, RtePosition, SphereVertex};
use crate::mesh::{self, CpuMesh, MeshVertex};
use crate::terrain::{TerrainConfig, TerrainLayer, TerrainStats};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Format of the texture handed to the UI. Shaders output linear colour.
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Format of [`Renderer::color_view`]: the same bytes, not decoded when sampled.
const DISPLAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Identifies a model uploaded with [`Renderer::load_mesh`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MeshId(u32);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RendererConfig {
    /// 1 disables multisampling; 4 is supported everywhere.
    pub msaa_samples: u32,
    pub grid: GridStyle,
    pub border_color: [u8; 4],
}

/// One object model to draw this frame.
#[derive(Debug, Clone, Copy)]
pub struct ModelInstance {
    pub mesh: MeshId,
    pub position: DVec3,
    pub rotation: Quat,
    pub scale: f32,
    /// sRGB colour.
    pub color: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Atmosphere {
    /// Amount of air and haze relative to the real atmosphere (1 = Earth).
    pub density: f32,
    /// Brightness of the scattered light (1 = default).
    pub exposure: f32,
}

/// Everything that varies from frame to frame.
pub struct Frame<'a> {
    pub camera: &'a Camera,
    /// sRGB background colour.
    pub clear_color: [f32; 3],
    /// Unit vector from the Earth towards the sun.
    pub sun_direction: Vec3,
    pub ambient: f32,
    pub lighting: bool,
    pub show_grid: bool,
    /// Also draw the fine grid (meant for when the camera is near the ground).
    pub show_fine_grid: bool,
    pub show_borders: bool,
    pub show_trails: bool,
    /// Draw streamed terrain instead of the plain globe (needs
    /// [`Renderer::enable_terrain`] to have been called).
    pub show_terrain: bool,
    pub atmosphere: Option<Atmosphere>,
    pub instances: &'a [ModelInstance],
    pub overlay_lines: &'a [LineVertex],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    eye_high: [f32; 4],
    eye_low: [f32; 4],
    sun: [f32; 4],
    eye_unit: [f32; 4],
    atmosphere: [f32; 4],
    params: [f32; 4],
    sky_right: [f32; 4],
    sky_up: [f32; 4],
    sky_forward: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuInstance {
    model: [[f32; 4]; 4],
    color: [f32; 4],
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    vertex_count: u32,
    radius: f32,
}

struct IndexedMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

/// A vertex buffer that is rewritten wholesale and grows as needed.
struct DynamicBuffer {
    label: &'static str,
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
    len: u32,
}

impl DynamicBuffer {
    fn new(label: &'static str) -> Self {
        Self { label, buffer: None, capacity: 0, len: 0 }
    }

    fn upload<T: Pod>(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[T]) {
        let bytes: &[u8] = bytemuck::cast_slice(data);
        self.len = data.len() as u32;
        if bytes.is_empty() {
            return;
        }
        if self.buffer.is_none() || bytes.len() as u64 > self.capacity {
            self.capacity = (bytes.len() as u64).next_power_of_two().max(64 * 1024);
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: self.capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        queue.write_buffer(self.buffer.as_ref().unwrap(), 0, bytes);
    }

    /// The buffer, if it currently holds anything to draw.
    fn filled(&self) -> Option<&wgpu::Buffer> {
        self.buffer.as_ref().filter(|_| self.len > 0)
    }
}

struct Target {
    size: (u32, u32),
    /// Multisampled colour attachment; `None` when MSAA is off.
    msaa_view: Option<wgpu::TextureView>,
    resolved_view: wgpu::TextureView,
    /// The resolved image again, as the plain sRGB-encoded bytes a UI expects.
    display_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
}

struct Pipelines {
    globe: wgpu::RenderPipeline,
    terrain: wgpu::RenderPipeline,
    polar_caps: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    surface_lines: wgpu::RenderPipeline,
    trails: wgpu::RenderPipeline,
    ribbons: wgpu::RenderPipeline,
    overlay: wgpu::RenderPipeline,
    models: wgpu::RenderPipeline,
}

pub struct Renderer {
    samples: u32,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    earth_layout: wgpu::BindGroupLayout,
    earth_sampler: wgpu::Sampler,
    earth_bind_group: wgpu::BindGroup,
    has_earth_texture: bool,
    pipelines: Pipelines,
    globe: IndexedMesh,
    surface_lines: wgpu::Buffer,
    grid_range: Range<u32>,
    fine_grid_range: Range<u32>,
    border_range: Range<u32>,
    trails: DynamicBuffer,
    ribbons: DynamicBuffer,
    overlay: DynamicBuffer,
    instances: DynamicBuffer,
    meshes: Vec<GpuMesh>,
    mesh_by_path: FxHashMap<PathBuf, Option<MeshId>>,
    target: Option<Target>,
    // Reused every frame to avoid reallocating.
    instance_scratch: Vec<GpuInstance>,
    draw_order: Vec<u32>,
    terrain: Option<TerrainLayer>,
    batches: Vec<(MeshId, Range<u32>)>,
}

impl Renderer {
    /// `borders` is the content of the bundled border file (see
    /// [`geometry::border_lines`]); pass an empty slice to omit borders.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, config: &RendererConfig, borders: &[u8]) -> Self {
        let samples = if config.msaa_samples > 1 { 4 } else { 1 };

        // ── Bind groups ───────────────────────────────────────────────────────
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals_buffer.as_entire_binding() }],
        });

        let earth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("earth texture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let earth_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("earth sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        // A 1×1 placeholder keeps the bind group valid until the map has loaded.
        let placeholder = EarthImage { width: 1, height: 1, mips: vec![vec![255; 4]] };
        let earth_bind_group = create_earth_bind_group(device, queue, &earth_layout, &earth_sampler, &placeholder);

        // ── Pipelines ─────────────────────────────────────────────────────────
        let shader = |label: &str, source: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(format!("{}\n{source}", include_str!("shaders/globals.wgsl")).into()),
            })
        };
        let globe_shader = shader("globe", include_str!("shaders/globe.wgsl"));
        let line_shader = shader("lines", include_str!("shaders/lines.wgsl"));
        let model_shader = shader("models", include_str!("shaders/model.wgsl"));
        let ribbon_shader = shader("ribbons", include_str!("shaders/ribbon.wgsl"));

        let plain_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("globals only"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });
        let textured_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("globals + earth texture"),
            bind_group_layouts: &[Some(&globals_layout), Some(&earth_layout)],
            immediate_size: 0,
        });

        let sphere_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
        let sphere_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<SphereVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &sphere_attributes,
        })];
        let line_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Unorm8x4];
        let line_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &line_attributes,
        })];
        let ribbon_attributes = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32, 4 => Float32, 5 => Unorm8x4
        ];
        let ribbon_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<RibbonVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ribbon_attributes,
        })];
        let mesh_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let instance_attributes =
            wgpu::vertex_attr_array![2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4];
        let model_buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<MeshVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &mesh_attributes,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<GpuInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &instance_attributes,
            }),
        ];

        let triangles = wgpu::PrimitiveTopology::TriangleList;
        let lines = wgpu::PrimitiveTopology::LineList;
        let build = |spec: PipelineSpec<'_>| create_pipeline(device, samples, spec);

        let pipelines = Pipelines {
            globe: build(PipelineSpec {
                label: "globe",
                layout: &textured_layout,
                shader: &globe_shader,
                entry_points: ("vs_sphere", "fs_globe"),
                buffers: &sphere_buffers,
                topology: triangles,
                cull: Some(wgpu::Face::Back),
                depth: Depth::TestAndWrite,
                blend: None,
            }),
            terrain: build(PipelineSpec {
                label: "terrain",
                layout: &textured_layout,
                shader: &globe_shader,
                entry_points: ("vs_sphere", "fs_terrain"),
                buffers: &sphere_buffers,
                topology: triangles,
                // Skirts face every way; depth sorts it out.
                cull: None,
                depth: Depth::TestAndWrite,
                blend: None,
            }),
            polar_caps: build(PipelineSpec {
                label: "polar caps",
                layout: &textured_layout,
                shader: &globe_shader,
                entry_points: ("vs_polar", "fs_polar"),
                buffers: &sphere_buffers,
                topology: triangles,
                cull: Some(wgpu::Face::Back),
                depth: Depth::TestAndWrite,
                blend: None,
            }),
            sky: build(PipelineSpec {
                label: "sky",
                layout: &plain_layout,
                shader: &globe_shader,
                entry_points: ("vs_sky", "fs_sky"),
                buffers: &[],
                topology: triangles,
                cull: None,
                depth: Depth::Background,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            }),
            surface_lines: build(PipelineSpec {
                label: "surface lines",
                layout: &plain_layout,
                shader: &line_shader,
                entry_points: ("vs_surface_line", "fs_line"),
                buffers: &line_buffers,
                topology: lines,
                cull: None,
                depth: Depth::Ignore,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            }),
            trails: build(PipelineSpec {
                label: "trails",
                layout: &plain_layout,
                shader: &line_shader,
                entry_points: ("vs_line", "fs_line"),
                buffers: &line_buffers,
                topology: lines,
                cull: None,
                depth: Depth::TestOnly,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            }),
            ribbons: build(PipelineSpec {
                label: "ribbon trails",
                layout: &plain_layout,
                shader: &ribbon_shader,
                entry_points: ("vs_ribbon", "fs_ribbon"),
                buffers: &ribbon_buffers,
                topology: triangles,
                // Both faces are drawn; the shader colours them differently.
                cull: None,
                depth: Depth::TestOnly,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            }),
            overlay: build(PipelineSpec {
                label: "overlay lines",
                layout: &plain_layout,
                shader: &line_shader,
                entry_points: ("vs_line", "fs_line"),
                buffers: &line_buffers,
                topology: lines,
                cull: None,
                depth: Depth::Ignore,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            }),
            models: build(PipelineSpec {
                label: "models",
                layout: &plain_layout,
                shader: &model_shader,
                entry_points: ("vs_model", "fs_model"),
                buffers: &model_buffers,
                topology: triangles,
                cull: None,
                depth: Depth::TestAndWrite,
                blend: None,
            }),
        };

        // ── Static geometry ───────────────────────────────────────────────────
        // 0.5-degree facets keep the mesh within ~60 m of the true sphere.
        let globe = create_sphere(device, "globe", EARTH_RADIUS_M, 720, 360);

        let mut surface = geometry::grid_lines(&config.grid);
        let grid_range = 0..surface.len() as u32;
        surface.extend(geometry::fine_grid_lines(&config.grid));
        let fine_grid_range = grid_range.end..surface.len() as u32;
        surface.extend(geometry::border_lines(borders, config.border_color));
        let border_range = fine_grid_range.end..surface.len() as u32;
        let surface_lines = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("surface lines"),
            contents: bytemuck::cast_slice(&surface),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            samples,
            globals_buffer,
            globals_bind_group,
            earth_layout,
            earth_sampler,
            earth_bind_group,
            has_earth_texture: false,
            pipelines,
            globe,
            surface_lines,
            grid_range,
            fine_grid_range,
            border_range,
            trails: DynamicBuffer::new("trails"),
            ribbons: DynamicBuffer::new("ribbon trails"),
            overlay: DynamicBuffer::new("overlay lines"),
            instances: DynamicBuffer::new("model instances"),
            meshes: Vec::new(),
            mesh_by_path: FxHashMap::default(),
            target: None,
            instance_scratch: Vec::new(),
            draw_order: Vec::new(),
            terrain: None,
            batches: Vec::new(),
        }
    }

    /// Resolved colour texture of the last rendered frame.
    pub fn color_view(&self) -> Option<&wgpu::TextureView> {
        self.target.as_ref().map(|target| &target.display_view)
    }

    /// Size in pixels of the current render target.
    pub fn target_size(&self) -> Option<(u32, u32)> {
        self.target.as_ref().map(|target| target.size)
    }

    /// (Re)creates the render target. Returns `true` if it changed, in which
    /// case [`Self::color_view`] is a new texture.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) -> bool {
        let limit = device.limits().max_texture_dimension_2d;
        let size = (width.clamp(1, limit), height.clamp(1, limit));
        if self.target.as_ref().is_some_and(|target| target.size == size) {
            return false;
        }
        let texture = |label: &str, format, sample_count, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
        // The scene is rendered through an sRGB view, which encodes on write.
        // The UI samples the same texture through a non-sRGB view, so it
        // receives those encoded bytes untouched: egui expects textures that
        // are not decoded on sampling, and would otherwise show the linear
        // values as if they were encoded, i.e. far too dark.
        let resolved = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scene colour"),
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DISPLAY_FORMAT,
            usage: attachment | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[COLOR_FORMAT],
        });
        let view_as = |format| resolved.create_view(&wgpu::TextureViewDescriptor { format: Some(format), ..Default::default() });
        self.target = Some(Target {
            size,
            msaa_view: (self.samples > 1).then(|| texture("scene msaa", COLOR_FORMAT, self.samples, attachment)),
            resolved_view: view_as(COLOR_FORMAT),
            display_view: view_as(DISPLAY_FORMAT),
            depth_view: texture("scene depth", DEPTH_FORMAT, self.samples, attachment),
        });
        true
    }

    pub fn set_earth_texture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, image: &EarthImage) {
        self.earth_bind_group = create_earth_bind_group(device, queue, &self.earth_layout, &self.earth_sampler, image);
        self.has_earth_texture = true;
    }

    /// Largest Earth map width this device can hold.
    pub fn max_texture_width(device: &wgpu::Device) -> u32 {
        device.limits().max_texture_dimension_2d
    }

    /// Starts streaming terrain through `loader`. Replaces any previous layer.
    pub fn enable_terrain(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        loader: acmi_terrain::TileLoader,
        config: TerrainConfig,
    ) {
        self.terrain = Some(TerrainLayer::new(device, queue, &self.earth_layout, loader, config));
    }

    /// Stops streaming and frees the terrain; the plain globe is drawn again.
    pub fn disable_terrain(&mut self) {
        self.terrain = None;
    }

    pub fn has_terrain(&self) -> bool {
        self.terrain.is_some()
    }

    /// Whether terrain tiles for the current view are still loading.
    pub fn terrain_busy(&self) -> bool {
        self.terrain.as_ref().is_some_and(TerrainLayer::is_busy)
    }

    pub fn terrain_stats(&self) -> Option<TerrainStats> {
        self.terrain.as_ref().map(TerrainLayer::stats)
    }

    /// Ground height in metres at a location (water is 0), from the most
    /// detailed terrain loaded there. `None` where nothing is loaded.
    pub fn terrain_height(&self, latitude: f64, longitude: f64) -> Option<f32> {
        self.terrain.as_ref()?.height_at(latitude, longitude)
    }

    /// Replaces the trail geometry: a line list, and a triangle list of
    /// ribbons (see [`RibbonVertex::push_band`]). Call only when it changes.
    pub fn set_trails(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, lines: &[LineVertex], ribbons: &[RibbonVertex]) {
        self.trails.upload(device, queue, lines);
        self.ribbons.upload(device, queue, ribbons);
    }

    /// Loads a model file, uploading it on first use. Failures are cached
    /// too, so a broken file is reported once rather than every frame.
    pub fn load_mesh(&mut self, device: &wgpu::Device, path: &Path) -> Option<MeshId> {
        if let Some(&cached) = self.mesh_by_path.get(path) {
            return cached;
        }
        let id = match mesh::load_model(path) {
            Ok(cpu) => Some(self.add_mesh(device, &cpu)),
            Err(err) => {
                log::warn!("cannot load model {}: {err}", path.display());
                None
            }
        };
        self.mesh_by_path.insert(path.to_owned(), id);
        id
    }

    pub fn add_mesh(&mut self, device: &wgpu::Device, cpu: &CpuMesh) -> MeshId {
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("model vertices"),
            contents: bytemuck::cast_slice(&cpu.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        self.meshes.push(GpuMesh { vertices, vertex_count: cpu.vertices.len() as u32, radius: cpu.radius });
        MeshId(self.meshes.len() as u32 - 1)
    }

    /// Bounding-sphere radius of a model, in metres.
    pub fn mesh_radius(&self, mesh: MeshId) -> f32 {
        self.meshes[mesh.0 as usize].radius
    }

    /// Renders a frame into the off-screen target. [`Self::resize`] must
    /// have been called first.
    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &Frame<'_>) {
        let Some(size) = self.target_size() else { return };
        let camera = frame.camera;

        // ── Per-frame uploads ─────────────────────────────────────────────────
        let eye = RtePosition::from(camera.eye);
        let (density, exposure) = frame.atmosphere.map_or((0.0, 1.0), |a| (a.density.max(0.0), a.exposure.max(0.0)));
        let aspect = size.0 as f32 / size.1 as f32;
        // View rays for the sky: forward plus the half-extents of the view.
        let half_height = (camera.fov_y * 0.5).tan();
        let rotation = camera.rotation.as_quat();
        let globals = Globals {
            view_proj: camera.view_proj(aspect).to_cols_array_2d(),
            eye_high: Vec3::from_array(eye.high).extend(0.0).to_array(),
            eye_low: Vec3::from_array(eye.low).extend(0.0).to_array(),
            sun: frame.sun_direction.normalize_or(Vec3::Y).extend(frame.ambient).to_array(),
            eye_unit: (camera.eye / EARTH_RADIUS_M).as_vec3().extend(0.0).to_array(),
            atmosphere: [density, exposure, (EARTH_RADIUS_M / 1000.0) as f32, 0.0],
            params: [self.has_earth_texture as u8 as f32, frame.lighting as u8 as f32, 2.0 * half_height / size.1 as f32, 0.0],
            sky_right: (rotation * Vec3::X * (half_height * aspect)).extend(0.0).to_array(),
            sky_up: (rotation * Vec3::Y * half_height).extend(0.0).to_array(),
            sky_forward: (rotation * Vec3::NEG_Z).extend(0.0).to_array(),
        };
        queue.write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));

        self.prepare_instances(camera, frame.instances);
        self.instances.upload(device, queue, &self.instance_scratch);
        self.overlay.upload(device, queue, frame.overlay_lines);

        // ── Render pass ───────────────────────────────────────────────────────
        let terrain = match &mut self.terrain {
            Some(layer) if frame.show_terrain => {
                layer.update(device, queue, &self.earth_layout, camera, size);
                Some(&*layer)
            }
            _ => None,
        };

        let target = self.target.as_ref().unwrap();
        let clear = srgb_to_linear(frame.clear_color);
        let (view, resolve_target) = match &target.msaa_view {
            Some(msaa) => (msaa, Some(&target.resolved_view)),
            None => (&target.resolved_view, None),
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("scene") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear[0] as f64,
                            g: clear[1] as f64,
                            b: clear[2] as f64,
                            a: 1.0,
                        }),
                        // Only the resolved image is needed when multisampling.
                        store: if resolve_target.is_some() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    // Reversed depth: 0 is infinitely far.
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.globals_bind_group, &[]);

            match terrain {
                Some(terrain) => {
                    pass.set_pipeline(&self.pipelines.terrain);
                    terrain.draw(&mut pass);
                    // Web-Mercator tiles stop at 85°; plain caps close the poles.
                    pass.set_pipeline(&self.pipelines.polar_caps);
                    pass.set_bind_group(1, &self.earth_bind_group, &[]);
                    draw_indexed(&mut pass, &self.globe);
                }
                None => {
                    pass.set_pipeline(&self.pipelines.globe);
                    pass.set_bind_group(1, &self.earth_bind_group, &[]);
                    draw_indexed(&mut pass, &self.globe);
                }
            }

            // Before anything else is drawn, so that only the ground hides the sky.
            if frame.atmosphere.is_some() {
                pass.set_pipeline(&self.pipelines.sky);
                pass.draw(0..3, 0..1);
            }

            pass.set_pipeline(&self.pipelines.surface_lines);
            pass.set_vertex_buffer(0, self.surface_lines.slice(..));
            if frame.show_grid && frame.show_fine_grid && !self.fine_grid_range.is_empty() {
                pass.draw(self.fine_grid_range.clone(), 0..1);
            }
            if frame.show_grid && !self.grid_range.is_empty() {
                pass.draw(self.grid_range.clone(), 0..1);
            }
            if frame.show_borders && !self.border_range.is_empty() {
                pass.draw(self.border_range.clone(), 0..1);
            }

            if let Some(instances) = self.instances.filled() {
                pass.set_pipeline(&self.pipelines.models);
                pass.set_vertex_buffer(1, instances.slice(..));
                for (mesh, range) in &self.batches {
                    let mesh = &self.meshes[mesh.0 as usize];
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.draw(0..mesh.vertex_count, range.clone());
                }
            }

            if let Some(ribbons) = self.ribbons.filled().filter(|_| frame.show_trails) {
                pass.set_pipeline(&self.pipelines.ribbons);
                pass.set_vertex_buffer(0, ribbons.slice(..));
                pass.draw(0..self.ribbons.len, 0..1);
            }
            if let Some(trails) = self.trails.filled().filter(|_| frame.show_trails) {
                pass.set_pipeline(&self.pipelines.trails);
                pass.set_vertex_buffer(0, trails.slice(..));
                pass.draw(0..self.trails.len, 0..1);
            }

            if let Some(overlay) = self.overlay.filled() {
                pass.set_pipeline(&self.pipelines.overlay);
                pass.set_vertex_buffer(0, overlay.slice(..));
                pass.draw(0..self.overlay.len, 0..1);
            }
        }
        queue.submit([encoder.finish()]);
    }

    /// Sorts instances by mesh and converts them to eye-relative matrices.
    fn prepare_instances(&mut self, camera: &Camera, instances: &[ModelInstance]) {
        self.draw_order.clear();
        self.draw_order.extend(0..instances.len() as u32);
        self.draw_order.sort_unstable_by_key(|&i| instances[i as usize].mesh);

        self.instance_scratch.clear();
        self.batches.clear();
        for &index in &self.draw_order {
            let instance = &instances[index as usize];
            let slot = self.instance_scratch.len() as u32;
            match self.batches.last_mut() {
                Some((mesh, range)) if *mesh == instance.mesh => range.end = slot + 1,
                _ => self.batches.push((instance.mesh, slot..slot + 1)),
            }
            let translation = (instance.position - camera.eye).as_vec3();
            let model = Mat4::from_scale_rotation_translation(Vec3::splat(instance.scale), instance.rotation, translation);
            self.instance_scratch.push(GpuInstance { model: model.to_cols_array_2d(), color: instance.color });
        }
    }
}

fn draw_indexed(pass: &mut wgpu::RenderPass<'_>, mesh: &IndexedMesh) {
    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
    pass.draw_indexed(0..mesh.index_count, 0, 0..1);
}

fn srgb_to_linear(color: [f32; 3]) -> [f32; 3] {
    color.map(|c| c.max(0.0).powf(2.2))
}

fn create_sphere(device: &wgpu::Device, label: &str, radius: f64, segments: u32, rings: u32) -> IndexedMesh {
    let (vertices, indices) = geometry::uv_sphere(radius, segments, rings);
    IndexedMesh {
        vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        }),
        index_count: indices.len() as u32,
    }
}

fn create_earth_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    image: &EarthImage,
) -> wgpu::BindGroup {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("earth"),
        size: wgpu::Extent3d { width: image.width, height: image.height, depth_or_array_layers: 1 },
        mip_level_count: image.mips.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, pixels) in image.mips.iter().enumerate() {
        let width = (image.width >> level).max(1);
        let height = (image.height >> level).max(1);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * width), rows_per_image: Some(height) },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("earth"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Depth {
    TestAndWrite,
    TestOnly,
    /// Drawn at the far plane: visible only where nothing has been drawn.
    Background,
    Ignore,
}

struct PipelineSpec<'a> {
    label: &'a str,
    layout: &'a wgpu::PipelineLayout,
    shader: &'a wgpu::ShaderModule,
    /// `(vertex, fragment)`.
    entry_points: (&'a str, &'a str),
    buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    topology: wgpu::PrimitiveTopology,
    cull: Option<wgpu::Face>,
    depth: Depth,
    blend: Option<wgpu::BlendState>,
}

fn create_pipeline(device: &wgpu::Device, samples: u32, spec: PipelineSpec<'_>) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(spec.label),
        layout: Some(spec.layout),
        vertex: wgpu::VertexState {
            module: spec.shader,
            entry_point: Some(spec.entry_points.0),
            compilation_options: Default::default(),
            buffers: spec.buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: spec.topology,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: spec.cull,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(spec.depth == Depth::TestAndWrite),
            // Reversed depth: nearer fragments have larger values.
            depth_compare: Some(match spec.depth {
                Depth::Ignore => wgpu::CompareFunction::Always,
                Depth::Background => wgpu::CompareFunction::GreaterEqual,
                Depth::TestAndWrite | Depth::TestOnly => wgpu::CompareFunction::Greater,
            }),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState { count: samples, mask: !0, alpha_to_coverage_enabled: false },
        fragment: Some(wgpu::FragmentState {
            module: spec.shader,
            entry_point: Some(spec.entry_points.1),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: spec.blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
