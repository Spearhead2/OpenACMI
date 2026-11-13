//! Elevation data for the globe.
//!
//! Terrain is streamed as Web-Mercator tiles of a digital elevation model
//! (DEM), cached on disk, and turned into two things per tile: a grid of
//! heights for geometry and a colour image derived from those heights.
//!
//! * [`TileKey`]: tile addressing and Mercator maths.
//! * [`TileSource`]: where tiles come from (disk cache first, then HTTP).
//! * [`ElevationTile`]: decoded heights of one tile.
//! * [`surface`]: ground or water? Knows the basins of land below sea level.
//! * [`shading`]: surface → colour (blue water, hypsometric land, relief).
//! * [`Imagery`]: optional satellite imagery to colour the tiles instead.
//! * [`TileLoader`]: worker threads that fetch, decode and shade on demand.
//!
//! Nothing here touches the GPU; `acmi-render` consumes [`TileData`].
//!
//! The default source is the public "Terrain Tiles" dataset on AWS Open
//! Data (Mapzen Terrarium encoding), which needs no account or key.

pub mod elevation;
pub mod imagery;
pub mod loader;
pub mod shading;
pub mod source;
pub mod surface;
pub mod tile;

pub use elevation::ElevationTile;
pub use imagery::{Imagery, DEFAULT_IMAGERY_ATTRIBUTION, DEFAULT_IMAGERY_MAX_ZOOM, DEFAULT_IMAGERY_URL_TEMPLATE};
pub use loader::{TileData, TileLoader, TileResult};
pub use shading::ElevationPalette;
pub use source::{FetchError, TileSource, DEFAULT_URL_TEMPLATE};
pub use surface::{basin_at, Basin, Surface, SurfaceTile, BASINS};
pub use tile::{TileKey, TILE_SIZE};
