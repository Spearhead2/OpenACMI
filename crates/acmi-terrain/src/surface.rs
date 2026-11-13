//! What is on top at each sample of a tile: ground or water.
//!
//! The DEM stores the solid surface, sea floor included, so by itself it
//! cannot tell the Dead Sea shore (dry land 400 m below sea level) from the
//! floor of the Mediterranean. The rule is:
//!
//! * A sample at or below sea level is ocean, drawn flat at sea level...
//! * ...unless it lies in one of the [`BASINS`]: enclosed areas of land below
//!   sea level. There the ground keeps its true (negative) height, and a
//!   basin may hold a lake with its own water level.
//!
//! Basins are plain latitude/longitude boxes, chosen so that they contain no
//! ocean. Low-lying land directly on a coast (the Dutch polders, New Orleans)
//! cannot be boxed that way and is still drawn as sea.
//!
//! To add a basin, measure it with `cargo run -p acmi-terrain --example probe`
//! (a lake shows up as a spike in the height histogram) and add a row.

use crate::elevation::ElevationTile;
use crate::tile::TileKey;

/// An enclosed area whose ground lies below sea level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Basin {
    pub name: &'static str,
    /// Bounds in degrees.
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
    /// Surface of the lake in the basin, metres: samples at or below it are
    /// water. `None` for a dry basin.
    pub water_level: Option<f32>,
}

const fn basin(name: &'static str, west: f64, south: f64, east: f64, north: f64, water_level: Option<f32>) -> Basin {
    Basin { name, west, south, east, north, water_level }
}

/// Known basins. Where boxes overlap the first match wins, so a lake comes
/// before the dry basin around it. Water levels are those found in the DEM,
/// which can differ from today's by a few metres.
pub const BASINS: &[Basin] = &[
    // Jordan Rift Valley.
    basin("Dead Sea", 35.33, 30.90, 35.62, 31.80, Some(-412.0)),
    basin("Sea of Galilee", 35.50, 32.68, 35.68, 32.92, Some(-212.0)),
    basin("Jordan Rift Valley", 35.30, 30.50, 35.75, 33.20, None),
    // North America.
    basin("Salton Sea", -116.15, 33.00, -115.50, 33.55, Some(-70.0)),
    basin("Salton Trough", -116.30, 32.00, -115.00, 33.80, None),
    basin("Death Valley", -117.30, 35.80, -116.50, 36.80, None),
    basin("Lake Enriquillo", -71.90, 18.35, -71.40, 18.60, None),
    // Africa.
    basin("Lake Assal", 42.30, 11.55, 42.45, 11.75, Some(-150.0)),
    basin("Danakil Depression (north)", 40.00, 13.90, 40.70, 14.60, None),
    basin("Danakil Depression (south)", 40.20, 13.00, 41.20, 13.90, None),
    basin("Qattara Depression and Siwa", 25.00, 28.50, 30.50, 30.50, None),
    basin("Chott Melrhir and Chott el Gharsa", 5.50, 33.60, 8.40, 34.60, None),
    // Asia.
    basin("Karagiye Depression", 51.55, 43.30, 52.00, 43.80, None),
    basin("Caspian Sea", 46.00, 36.40, 55.00, 47.30, Some(-27.0)),
    basin("Sarygamysh and Akchakaya", 56.50, 40.30, 58.50, 42.30, None),
    basin("Turpan Depression", 88.50, 42.30, 90.50, 43.20, None),
    // Elsewhere.
    basin("Lake Eyre", 136.50, -29.60, 138.50, -27.80, None),
    basin("Laguna del Carbon", -68.60, -49.80, -68.00, -49.40, None),
];

impl Basin {
    #[inline]
    pub fn contains(&self, latitude: f64, longitude: f64) -> bool {
        (self.south..=self.north).contains(&latitude) && (self.west..=self.east).contains(&longitude)
    }

    fn overlaps(&self, key: TileKey) -> bool {
        let (west, east) = (key.longitude(0.0), key.longitude(1.0));
        let (north, south) = (key.latitude(0.0), key.latitude(1.0));
        self.west <= east && self.east >= west && self.south <= north && self.north >= south
    }
}

/// The basin containing a point, if any.
pub fn basin_at(latitude: f64, longitude: f64) -> Option<&'static Basin> {
    BASINS.iter().find(|basin| basin.contains(latitude, longitude))
}

/// Top surface at one sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Surface {
    /// Ground at this height above (or, in a basin, below) sea level.
    Land(f32),
    /// Water whose surface is at `level`, `depth` metres deep.
    Water { level: f32, depth: f32 },
}

impl Surface {
    /// Classifies a DEM height. `basin` is the basin the sample lies in.
    pub fn classify(height: f32, basin: Option<&Basin>) -> Self {
        let level = match basin {
            None => 0.0,
            Some(Basin { water_level: Some(level), .. }) => *level,
            Some(_) => return Surface::Land(height),
        };
        if height <= level {
            Surface::Water { level, depth: level - height }
        } else {
            Surface::Land(height)
        }
    }

    /// Height of the visible surface, metres.
    #[inline]
    pub fn height(self) -> f32 {
        match self {
            Surface::Land(height) => height,
            Surface::Water { level, .. } => level,
        }
    }

    #[inline]
    pub fn is_water(self) -> bool {
        matches!(self, Surface::Water { .. })
    }
}

/// A tile resolved into its visible surface.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceTile {
    /// Height of the visible surface: ground, or the top of the water.
    pub heights: ElevationTile,
    /// Water depth per sample; negative where the sample is land.
    water_depth: Box<[f32]>,
}

impl SurfaceTile {
    /// Resolves the raw DEM heights of tile `key`.
    pub fn resolve(key: TileKey, raw: &ElevationTile) -> Self {
        let size = raw.size();
        let basins: Vec<&Basin> = BASINS.iter().filter(|basin| basin.overlaps(key)).collect();
        let mut heights = Vec::with_capacity(size * size);
        let mut water_depth = Vec::with_capacity(size * size);
        let mut push = |surface: Surface| {
            heights.push(surface.height());
            water_depth.push(match surface {
                Surface::Land(_) => -1.0,
                Surface::Water { depth, .. } => depth,
            });
        };

        if basins.is_empty() {
            // Nearly every tile: no coordinates needed.
            raw.heights().iter().for_each(|&height| push(Surface::classify(height, None)));
        } else {
            let longitudes: Vec<f64> = (0..size).map(|column| key.longitude((column as f64 + 0.5) / size as f64)).collect();
            for row in 0..size {
                let latitude = key.latitude((row as f64 + 0.5) / size as f64);
                for (column, &longitude) in longitudes.iter().enumerate() {
                    let basin = basins.iter().find(|basin| basin.contains(latitude, longitude)).copied();
                    push(Surface::classify(raw.heights()[row * size + column], basin));
                }
            }
        }
        Self { heights: ElevationTile::from_heights(size, heights), water_depth: water_depth.into_boxed_slice() }
    }

    #[inline]
    pub fn size(&self) -> usize {
        self.heights.size()
    }

    /// Surface of the sample at column `x`, row `y`, clamped to the tile.
    #[inline]
    pub fn surface(&self, x: isize, y: isize) -> Surface {
        let size = self.size();
        let clamp = |v: isize| v.clamp(0, size as isize - 1) as usize;
        let index = clamp(y) * size + clamp(x);
        let (level, depth) = (self.heights.heights()[index], self.water_depth[index]);
        if depth < 0.0 {
            Surface::Land(level)
        } else {
            Surface::Water { level, depth }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_sea_is_water_at_sea_level() {
        assert_eq!(Surface::classify(-300.0, None), Surface::Water { level: 0.0, depth: 300.0 });
        assert_eq!(Surface::classify(0.0, None).height(), 0.0);
        assert_eq!(Surface::classify(12.0, None), Surface::Land(12.0));
        // The Mediterranean off Tel Aviv is not part of the rift valley.
        assert_eq!(basin_at(32.08, 34.70), None);
    }

    #[test]
    fn basins_keep_their_ground_below_sea_level() {
        // Jericho, about 250 m below sea level.
        let jericho = basin_at(31.86, 35.46);
        assert_eq!(jericho.map(|basin| basin.name), Some("Jordan Rift Valley"));
        assert_eq!(Surface::classify(-250.0, jericho), Surface::Land(-250.0));

        // The Dead Sea itself is water at its own level; its shore is land.
        let dead_sea = basin_at(31.5, 35.48);
        assert_eq!(dead_sea.map(|basin| basin.name), Some("Dead Sea"));
        assert_eq!(Surface::classify(-416.0, dead_sea), Surface::Water { level: -412.0, depth: 4.0 });
        assert_eq!(Surface::classify(-380.0, dead_sea), Surface::Land(-380.0));

        assert!(Surface::classify(-80.0, basin_at(36.23, -116.83)) == Surface::Land(-80.0), "Badwater, Death Valley");
        assert!(Surface::classify(-300.0, basin_at(42.0, 51.0)).is_water(), "Caspian Sea");
        assert!(!Surface::classify(-20.0, basin_at(46.3, 48.0)).is_water(), "Volga delta");
    }

    #[test]
    fn basins_are_well_formed() {
        for basin in BASINS {
            assert!(basin.west < basin.east && basin.south < basin.north, "{}", basin.name);
            assert!(basin.water_level.is_none_or(|level| level < 0.0), "{}", basin.name);
        }
    }

    #[test]
    fn tiles_are_resolved_per_sample() {
        // Zoom 9 tile over the northern Dead Sea and the hills west of it.
        let (key, ..) = TileKey::locate(9, 31.6, 35.5).unwrap();
        let size = 64;
        let raw = ElevationTile::from_heights(size, vec![-416.0; size * size]);
        let tile = SurfaceTile::resolve(key, &raw);
        let column = |longitude: f64| {
            ((longitude - key.longitude(0.0)) / (key.longitude(1.0) - key.longitude(0.0)) * size as f64) as isize
        };
        let row = (0..size as isize).find(|&row| key.latitude((row as f64 + 0.5) / size as f64) < 31.6).unwrap();

        // In the lake: water at the lake's level.
        assert_eq!(tile.surface(column(35.5), row), Surface::Water { level: -412.0, depth: 4.0 });
        // West of the basin the same height would be sea floor.
        assert_eq!(tile.surface(column(35.2), row), Surface::Water { level: 0.0, depth: 416.0 });
        // North of the lake the tile reaches the dry valley floor, which keeps its height.
        assert_eq!(tile.surface(column(35.5), 0), Surface::Land(-416.0));
        assert_eq!(tile.heights.range(), (-416.0, 0.0));

        // A tile far from any basin clamps everything below sea level.
        let ocean = SurfaceTile::resolve(TileKey::new(3, 0, 3), &ElevationTile::from_heights(2, vec![-50.0, 10.0, -1.0, 0.0]));
        assert_eq!(ocean.heights.heights(), &[0.0, 10.0, 0.0, 0.0]);
        assert!(ocean.surface(0, 0).is_water() && !ocean.surface(1, 0).is_water());
    }
}
