//! Tile addressing in the standard Web-Mercator "slippy map" scheme.
//!
//! At zoom `z` the world is a `2^z × 2^z` grid; `x` grows eastwards from
//! 180°W and `y` southwards from about 85.05°N. Within a tile, `u` and `v`
//! run 0→1 in the same directions.

use std::f64::consts::PI;

use acmi_core::geo::EARTH_RADIUS_M;

/// Pixels along one side of a tile image.
pub const TILE_SIZE: usize = 256;

/// Latitude beyond which Web Mercator has no data.
pub const MAX_LATITUDE: f64 = 85.051_128_779_806_59;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

impl TileKey {
    pub const fn new(z: u8, x: u32, y: u32) -> Self {
        Self { z, x, y }
    }

    /// Tiles per side at this zoom.
    #[inline]
    pub fn grid_size(self) -> f64 {
        (1u64 << self.z) as f64
    }

    pub fn children(self) -> [TileKey; 4] {
        let (z, x, y) = (self.z + 1, self.x * 2, self.y * 2);
        [TileKey::new(z, x, y), TileKey::new(z, x + 1, y), TileKey::new(z, x, y + 1), TileKey::new(z, x + 1, y + 1)]
    }

    pub fn parent(self) -> Option<TileKey> {
        (self.z > 0).then(|| TileKey::new(self.z - 1, self.x / 2, self.y / 2))
    }

    /// Longitude in degrees at horizontal position `u` within the tile.
    #[inline]
    pub fn longitude(self, u: f64) -> f64 {
        (self.x as f64 + u) / self.grid_size() * 360.0 - 180.0
    }

    /// Latitude in degrees at vertical position `v` within the tile.
    #[inline]
    pub fn latitude(self, v: f64) -> f64 {
        (PI * (1.0 - 2.0 * (self.y as f64 + v) / self.grid_size())).sinh().atan().to_degrees()
    }

    /// The tile at zoom `z` containing a location, and the `(u, v)` position
    /// of the location inside it. `None` outside the Mercator latitude range.
    pub fn locate(z: u8, latitude: f64, longitude: f64) -> Option<(TileKey, f64, f64)> {
        if latitude.abs() >= MAX_LATITUDE {
            return None;
        }
        let n = (1u64 << z) as f64;
        let x = ((longitude + 180.0).rem_euclid(360.0) / 360.0 * n).min(n - 1e-9);
        let lat = latitude.to_radians();
        let y = ((1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / PI) * 0.5 * n).clamp(0.0, n - 1e-9);
        Some((TileKey::new(z, x as u32, y as u32), x.fract(), y.fract()))
    }

    /// Ground distance covered by one pixel at `latitude`, in metres.
    pub fn meters_per_pixel(self, latitude: f64) -> f64 {
        2.0 * PI * EARTH_RADIUS_M * latitude.to_radians().cos() / (TILE_SIZE as f64 * self.grid_size())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_of_known_tiles() {
        let world = TileKey::new(0, 0, 0);
        assert_eq!(world.longitude(0.0), -180.0);
        assert_eq!(world.longitude(1.0), 180.0);
        assert!((world.latitude(0.0) - MAX_LATITUDE).abs() < 1e-9);
        assert!(world.latitude(0.5).abs() < 1e-12);

        let tile = TileKey::new(2, 2, 1);
        assert_eq!((tile.longitude(0.0), tile.longitude(1.0)), (0.0, 90.0));
        assert!(tile.latitude(1.0).abs() < 1e-12);
        assert!((tile.latitude(0.0) - 66.513_260).abs() < 1e-5);
    }

    #[test]
    fn locate_inverts_bounds() {
        // Las Vegas at zoom 10.
        let (key, u, v) = TileKey::locate(10, 36.17, -115.14).unwrap();
        assert_eq!(key, TileKey::new(10, 184, 401));
        assert!((key.longitude(u) + 115.14).abs() < 1e-9);
        assert!((key.latitude(v) - 36.17).abs() < 1e-9);
        assert!(TileKey::locate(5, 89.0, 0.0).is_none());
        // The antimeridian wraps instead of overflowing the grid.
        assert_eq!(TileKey::locate(3, 0.0, 180.0).unwrap().0.x, 0);
    }

    #[test]
    fn family_relations() {
        let key = TileKey::new(4, 5, 9);
        assert!(key.children().iter().all(|child| child.parent() == Some(key)));
        assert_eq!(TileKey::new(0, 0, 0).parent(), None);
    }

    #[test]
    fn ground_resolution() {
        // The well-known ~156.5 km per pixel at zoom 0 scales with the sphere radius used here.
        let expected = 2.0 * PI * EARTH_RADIUS_M / 256.0;
        assert!((TileKey::new(0, 0, 0).meters_per_pixel(0.0) - expected).abs() < 1e-6);
        assert!((TileKey::new(1, 0, 0).meters_per_pixel(60.0) - expected / 4.0).abs() < 1e-6);
    }
}
