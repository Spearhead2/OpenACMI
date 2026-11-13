//! Renderer for the OpenACMI 3D view.
//!
//! The crate knows nothing about recordings or the UI: the application
//! describes each frame with plain data ([`Frame`], [`ModelInstance`],
//! [`LineVertex`], [`RibbonVertex`]) and receives a texture to display. Adding a new kind of
//! visual means adding a pipeline here and data to [`Frame`].

pub mod camera;
pub mod earth_texture;
pub mod geometry;
pub mod mesh;
pub mod normals;
mod renderer;
mod terrain;

pub use camera::Camera;
pub use earth_texture::{load_earth_image, EarthImage};
pub use geometry::{GridStyle, LineVertex, RibbonVertex, RtePosition};
pub use mesh::{CpuMesh, MeshLibrary};
pub use renderer::{Atmosphere, Frame, MeshId, ModelInstance, Renderer, RendererConfig, COLOR_FORMAT};
pub use terrain::{TerrainConfig, TerrainStats};

pub use acmi_terrain;
pub use wgpu;
