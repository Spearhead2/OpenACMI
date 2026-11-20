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
//! * A lake above sea level is in the DEM as a flat stretch at the height of
//!   its surface, which nothing tells apart from a plain. The large ones are
//!   listed in [`BASINS`] too, each as a box with the narrow band of heights
//!   the DEM has for its surface: a sample in the box and in the band is the
//!   lake. The band is what keeps the shore dry, so the box can be generous.
//!
//! Basins are plain latitude/longitude boxes. Those of land below sea level
//! are chosen so that they contain no ocean; low-lying land directly on a
//! coast (the Dutch polders, New Orleans) cannot be boxed that way and is
//! still drawn as sea.
//!
//! To add a basin or a lake, measure it with
//! `cargo run -p acmi-terrain --example probe` (a lake shows up as a spike in
//! the height histogram) and add a row. Measure a lake at several zoom
//! levels: the tiles come from different surveys and can disagree about its
//! height by metres, in which case it gets a row for each height.

use crate::elevation::ElevationTile;
use crate::tile::TileKey;

/// An enclosed area whose ground lies below sea level, or a box around a
/// lake above it.
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
    /// Lowest height that is still the lake. Below sea level a lake's bed
    /// may be in the DEM, so anything under the surface is water; above it
    /// only the flat surface itself is.
    pub water_floor: f32,
}

/// Land below sea level, with the level of its lake if it has one.
const fn basin(name: &'static str, west: f64, south: f64, east: f64, north: f64, water_level: Option<f32>) -> Basin {
    Basin { name, west, south, east, north, water_level, water_floor: f32::NEG_INFINITY }
}

/// A lake above sea level: samples from `lowest` to `highest` are its surface.
const fn lake(name: &'static str, west: f64, south: f64, east: f64, north: f64, lowest: f32, highest: f32) -> Basin {
    Basin { name, west, south, east, north, water_level: Some(highest), water_floor: lowest }
}

/// Known lakes and basins. Lakes above sea level come first, as a basin
/// of land below sea level decides everything in its box: among those the
/// first match wins, so a lake comes before the dry basin around it. Water
/// levels are those found in the DEM, which can differ from today's by a
/// few metres.
pub const BASINS: &[Basin] = &[
    // Lakes above sea level, by the heights of their surface in the DEM.
    // The Great Lakes: each varies by a few metres between zoom levels.
    lake("Lake Superior", -92.20, 46.40, -84.35, 49.05, 178.5, 184.9),
    // Below the locks at Sault Ste. Marie the river is at Lake Huron's level.
    lake("St. Marys River", -84.35, 46.35, -83.40, 46.60, 174.5, 176.9),
    lake("Lake Michigan", -88.10, 41.60, -84.75, 46.10, 174.5, 176.9),
    lake("Lake Huron", -84.75, 43.00, -79.65, 46.35, 174.5, 176.9),
    lake("Lake St. Clair", -83.20, 42.08, -82.35, 43.00, 172.5, 175.9),
    lake("Lake Erie", -83.55, 41.30, -78.80, 42.95, 171.5, 174.9),
    lake("Lake Ontario", -79.95, 43.15, -76.00, 44.30, 72.5, 75.9),
    // North and Central America.
    lake("Great Bear Lake", -125.20, 64.80, -117.30, 67.30, 139.5, 142.9),
    lake("Great Bear Lake", -125.20, 64.80, -117.30, 67.30, 154.5, 157.9),
    lake("Great Slave Lake", -117.50, 60.70, -108.80, 63.00, 125.5, 127.9),
    lake("Great Slave Lake", -117.50, 60.70, -108.80, 63.00, 154.5, 160.9),
    lake("Lake Athabasca", -111.80, 58.50, -106.20, 59.70, 206.5, 209.9),
    lake("Lake Athabasca", -111.80, 58.50, -106.20, 59.70, 211.5, 214.9),
    lake("Reindeer Lake", -103.30, 56.30, -101.30, 58.20, 334.5, 336.9),
    lake("Lake Winnipeg", -99.60, 50.20, -95.90, 54.00, 214.5, 218.9),
    lake("Lake Winnipegosis", -101.30, 51.40, -99.00, 53.50, 250.5, 253.9),
    lake("Lake Manitoba", -99.90, 49.90, -97.60, 52.00, 244.5, 248.9),
    lake("Lake Nipigon", -89.30, 49.20, -87.80, 50.60, 257.5, 259.9),
    lake("Lake of the Woods", -95.40, 48.70, -93.80, 49.80, 319.5, 324.9),
    lake("Lake of the Woods", -95.40, 48.70, -93.80, 49.80, 326.5, 328.9),
    lake("Lake Mistassini", -74.40, 50.30, -72.80, 51.60, 370.5, 375.9),
    lake("Great Salt Lake", -113.10, 40.65, -111.90, 41.75, 1277.5, 1282.9),
    lake("Lake Tahoe", -120.20, 38.90, -119.90, 39.30, 1896.5, 1898.9),
    lake("Lake Champlain", -73.50, 43.55, -73.05, 45.10, 27.5, 28.9),
    lake("Lake Nicaragua", -86.00, 10.95, -84.75, 12.15, 30.5, 31.9),
    lake("Lake Managua", -86.65, 12.10, -86.00, 12.55, 40.5, 41.9),
    // South America.
    lake("Lake Titicaca", -70.20, -16.70, -68.50, -15.10, 3814.5, 3815.9),
    lake("Lake General Carrera", -73.00, -47.00, -71.00, -46.10, 201.5, 202.9),
    lake("Lake Argentino", -73.40, -50.60, -71.80, -49.90, 178.5, 179.9),
    lake("Lake Viedma", -73.10, -49.90, -71.90, -49.30, 251.5, 252.9),
    // Europe.
    lake("Lake Ladoga", 30.95, 59.90, 33.10, 60.40, 0.5, 1.9),
    lake("Lake Ladoga", 29.90, 60.40, 33.10, 61.85, 0.5, 1.9),
    lake("Lake Onega", 34.00, 60.85, 36.60, 62.95, 22.5, 23.9),
    lake("Lake Onega", 34.00, 60.85, 36.60, 62.95, 46.5, 47.9),
    lake("Vänern", 12.20, 58.30, 14.20, 59.45, 43.5, 44.9),
    lake("Vättern", 14.05, 57.70, 15.00, 58.30, 85.5, 86.9),
    lake("Vättern", 14.20, 58.30, 15.00, 58.95, 85.5, 86.9),
    lake("Vättern", 14.05, 57.70, 15.00, 58.30, 87.5, 88.9),
    lake("Vättern", 14.20, 58.30, 15.00, 58.95, 87.5, 88.9),
    lake("Lake Peipus", 26.90, 57.80, 28.20, 59.05, 26.5, 28.9),
    lake("Lake Balaton", 17.20, 46.60, 18.20, 47.10, 99.5, 100.9),
    lake("Lake Geneva", 6.13, 46.20, 6.95, 46.55, 369.5, 371.9),
    lake("Lake Constance", 8.85, 47.45, 9.80, 47.85, 388.5, 390.9),
    lake("Lake Constance", 8.85, 47.45, 9.80, 47.85, 391.5, 393.9),
    // Asia.
    lake("Lake Baikal", 103.60, 51.42, 106.00, 51.85, 448.5, 449.9),
    lake("Lake Baikal", 104.90, 51.85, 107.60, 52.60, 448.5, 449.9),
    lake("Lake Baikal", 105.90, 52.60, 109.00, 53.60, 448.5, 449.9),
    lake("Lake Baikal", 107.20, 53.60, 110.10, 55.90, 448.5, 449.9),
    lake("Lake Balkhash", 73.30, 44.75, 79.40, 46.90, 337.5, 342.9),
    lake("Issyk-Kul", 76.00, 42.05, 78.50, 42.80, 1600.5, 1601.9),
    lake("Lake Urmia", 44.90, 37.00, 46.10, 38.35, 1266.5, 1267.9),
    lake("Lake Van", 42.20, 38.20, 43.75, 39.05, 1644.5, 1647.9),
    lake("Lake Sevan", 44.90, 40.10, 45.75, 40.70, 1897.5, 1898.9),
    lake("Lake Khövsgöl", 100.10, 50.40, 100.95, 51.75, 1641.5, 1642.9),
    lake("Uvs Lake", 92.00, 49.95, 93.60, 50.75, 758.5, 759.9),
    lake("Qinghai Lake", 99.50, 36.50, 100.85, 37.30, 3193.5, 3194.9),
    lake("Lake Khanka", 131.90, 44.40, 133.00, 45.40, 63.5, 64.9),
    lake("Lake Toba", 98.45, 2.25, 99.30, 3.00, 899.5, 900.9),
    lake("Namtso", 90.20, 30.45, 91.10, 30.95, 4723.5, 4724.9),
    lake("Siling Lake", 88.50, 31.50, 89.40, 32.15, 4538.5, 4539.9),
    lake("Lake Alakol", 81.10, 45.75, 82.30, 46.50, 346.5, 347.9),
    lake("Lake Zaysan", 83.00, 47.60, 85.00, 48.40, 387.5, 388.9),
    lake("Hulun Lake", 116.90, 48.50, 117.90, 49.40, 539.5, 540.9),
    lake("Lake Biwa", 135.85, 34.95, 136.30, 35.55, 80.5, 81.9),
    // Africa.
    lake("Lake Victoria", 31.50, -3.10, 34.95, 0.45, 1132.5, 1135.9),
    lake("Lake Tanganyika", 30.10, -8.85, 31.25, -7.00, 766.5, 767.9),
    lake("Lake Tanganyika", 29.30, -7.00, 30.80, -5.00, 766.5, 767.9),
    lake("Lake Tanganyika", 29.00, -5.00, 29.95, -3.30, 766.5, 767.9),
    lake("Lake Malawi", 33.85, -14.45, 35.30, -9.45, 475.5, 476.9),
    lake("Lake Turkana", 35.75, 2.35, 36.75, 4.75, 360.5, 361.9),
    lake("Lake Albert", 30.30, 1.00, 31.50, 2.35, 614.5, 615.9),
    lake("Lake Edward", 29.25, -0.75, 29.95, 0.00, 911.5, 912.9),
    lake("Lake Tana", 36.95, 11.55, 37.70, 12.35, 1785.5, 1786.9),
    lake("Lake Mweru", 28.20, -9.55, 29.20, -8.45, 920.5, 921.9),
    lake("Lake Rukwa", 31.20, -8.70, 32.95, -7.30, 800.5, 801.9),
    lake("Lake Kariba", 26.90, -18.10, 29.10, -16.40, 486.5, 487.9),
    lake("Lake Nasser", 30.90, 21.50, 33.30, 23.97, 178.5, 179.9),
    // Oceania.
    lake("Lake Taupo", 175.60, -39.05, 176.15, -38.60, 355.5, 357.9),
    // Land below sea level.
    // Jordan Rift Valley.
    // The Dead Sea is two bodies of water at different levels: the sea
    // proper, and the evaporation ponds that fill its former southern basin.
    basin("Dead Sea evaporation ponds", 35.33, 30.90, 35.55, 31.27, Some(-389.0)),
    basin("Dead Sea", 35.33, 31.27, 35.62, 31.80, Some(-410.0)),
    basin("Sea of Galilee", 35.50, 32.68, 35.68, 32.92, Some(-212.0)),
    // East to take in the Yarmouk gorge, south and west for the Arava and
    // the plain of Sodom; the Mediterranean is nowhere near either box.
    basin("Jordan Rift Valley", 35.30, 30.20, 35.95, 33.20, None),
    basin("Arava and Sodom plain", 35.00, 30.20, 35.30, 31.45, None),
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
    // The sea and the whole Caspian Depression around it: the lowland runs
    // far north and west of the shore, all of it below sea level.
    basin("Caspian Sea and Depression", 43.00, 36.30, 56.00, 51.00, Some(-27.0)),
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

/// Depth every basin lake is drawn with, metres: a mid blue.
pub const LAKE_DEPTH_M: f32 = 150.0;

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
        Self::classify_among(height, basin)
    }

    /// Classifies a DEM height at a point inside each of `basins`, given in
    /// the order of [`BASINS`].
    pub fn classify_among<'a>(height: f32, basins: impl IntoIterator<Item = &'a Basin>) -> Self {
        for basin in basins {
            match basin.water_level {
                // The DEM has the sea floor under the oceans, but under lakes
                // it has the bed in some tiles and the flat water surface in
                // others, which would colour a lake in squares. Lakes get one
                // depth.
                Some(level) if (basin.water_floor..=level).contains(&height) => return Surface::Water { level, depth: LAKE_DEPTH_M },
                // Beside a lake above sea level the usual rule holds, and
                // another lake's box may cover the same place.
                Some(level) if level > 0.0 => {}
                // Land below sea level keeps its height.
                _ => return Surface::Land(height),
            }
        }
        if height <= 0.0 {
            Surface::Water { level: 0.0, depth: -height }
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
                    let here = basins.iter().copied().filter(|basin| basin.contains(latitude, longitude));
                    push(Surface::classify_among(raw.heights()[row * size + column], here));
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
    fn the_caspian_depression_is_land_not_sea() {
        // Astrakhan, Atyrau and the steppe towards the Kuma: 20 to 27 m below
        // sea level, well away from the shore, and dry.
        for (latitude, longitude) in [(46.35, 48.04), (47.10, 51.90), (49.50, 48.00), (45.00, 44.50)] {
            let basin = basin_at(latitude, longitude);
            assert_eq!(basin.map(|basin| basin.name), Some("Caspian Sea and Depression"), "{latitude} {longitude}");
            assert_eq!(Surface::classify(-22.0, basin), Surface::Land(-22.0));
        }
        // The sea itself, however deep the DEM says it is there.
        let sea = basin_at(41.0, 51.0);
        assert!(Surface::classify(-28.0, sea).is_water() && Surface::classify(-900.0, sea).is_water());
        assert_eq!(Surface::classify(-28.0, sea), Surface::classify(-900.0, sea));
        // The Black Sea and the Sea of Azov are ocean, outside the box.
        assert_eq!(basin_at(46.0, 37.0), None);
        assert_eq!(basin_at(43.0, 39.0), None);
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
        // Whatever the DEM says lies under it, a lake has one depth: the data
        // holds the bed in some tiles and the water surface in others.
        assert_eq!(Surface::classify(-416.0, dead_sea), Surface::Water { level: -410.0, depth: LAKE_DEPTH_M });
        assert_eq!(Surface::classify(-700.0, dead_sea), Surface::Water { level: -410.0, depth: LAKE_DEPTH_M });
        assert_eq!(Surface::classify(-380.0, dead_sea), Surface::Land(-380.0));

        assert!(Surface::classify(-80.0, basin_at(36.23, -116.83)) == Surface::Land(-80.0), "Badwater, Death Valley");
        assert!(Surface::classify(-300.0, basin_at(42.0, 51.0)).is_water(), "Caspian Sea");
        assert!(!Surface::classify(-20.0, basin_at(46.3, 48.0)).is_water(), "Volga delta");
    }

    #[test]
    fn basins_are_well_formed() {
        for basin in BASINS {
            assert!(basin.west < basin.east && basin.south < basin.north, "{}", basin.name);
            // A lake's band is narrow: it is what keeps its shores dry.
            if let Some(level) = basin.water_level.filter(|level| *level > 0.0) {
                assert!(basin.water_floor > 0.0 && basin.water_floor < level && level - basin.water_floor < 8.0, "{}", basin.name);
            }
        }
    }

    #[test]
    fn the_great_lakes_are_water() {
        // A point out on each lake, with the surface heights the DEM has for
        // it at different zoom levels.
        let lakes: [(&str, f64, f64, [f32; 2]); 8] = [
            ("Lake Superior", 47.7, -87.5, [179.4, 184.6]),
            ("Lake Michigan", 43.5, -87.0, [175.2, 176.4]),
            ("Lake Michigan", 44.8, -88.0, [175.5, 176.5]),
            ("Lake Huron", 44.7, -82.5, [175.3, 176.6]),
            ("Lake Huron", 45.3, -80.9, [175.0, 176.9]),
            ("Lake St. Clair", 42.43, -82.7, [173.5, 175.8]),
            ("Lake Erie", 42.1, -81.2, [172.3, 174.8]),
            ("Lake Ontario", 43.6, -77.9, [73.2, 75.7]),
        ];
        for (name, latitude, longitude, heights) in lakes {
            let lake = basin_at(latitude, longitude);
            assert_eq!(lake.map(|lake| lake.name), Some(name));
            for height in heights {
                let surface = Surface::classify(height, lake);
                assert!(surface.is_water(), "{name} at {height} m");
                // One level and one depth per lake, whatever the tile says.
                assert_eq!(surface, Surface::classify(heights[0], lake), "{name}");
            }
        }

        // The cities on their shores stand above them and stay dry.
        for (city, latitude, longitude, height) in [
            ("Chicago", 41.88, -87.63, 181.0),
            ("Detroit", 42.33, -83.05, 183.0),
            ("Cleveland", 41.50, -81.69, 199.0),
            ("Toronto", 43.65, -79.38, 80.0),
            ("Duluth", 46.79, -92.10, 214.0),
            ("Milwaukee", 43.04, -87.91, 188.0),
        ] {
            assert!(!Surface::classify(height, basin_at(latitude, longitude)).is_water(), "{city}");
        }
        // Away from the lakes the rule is unchanged: low ground is not
        // flooded for being as low as a lake somewhere else.
        assert_eq!(basin_at(40.0, -89.0), None);
        assert_eq!(Surface::classify(150.0, None), Surface::Land(150.0));
        // The St. Lawrence below Kingston is outside the Lake Ontario box.
        assert_eq!(basin_at(45.0, -74.8), None);
    }

    #[test]
    fn a_lake_is_only_its_own_surface() {
        // Lake Titicaca's box reaches down the far side of the Andes.
        let here = || BASINS.iter().filter(|basin| basin.contains(-15.8, -69.4));
        assert!(Surface::classify_among(3815.0, here()).is_water());
        // Ground lower than the lake is not flooded, nor is the shore.
        assert_eq!(Surface::classify_among(1200.0, here()), Surface::Land(1200.0));
        assert_eq!(Surface::classify_among(3818.0, here()), Surface::Land(3818.0));

        // Sea inside a lake's box is still sea: the Pacific by Lake Nicaragua.
        let nicaragua = || BASINS.iter().filter(|basin| basin.contains(11.2, -85.9));
        assert_eq!(Surface::classify_among(-40.0, nicaragua()), Surface::Water { level: 0.0, depth: 40.0 });
        assert!(matches!(Surface::classify_among(31.0, nicaragua()), Surface::Water { level, .. } if level > 30.0));

        // A lake the DEM has at two heights is water at both, and dry between.
        let onega = || BASINS.iter().filter(|basin| basin.contains(61.7, 35.5));
        assert!(Surface::classify_among(23.0, onega()).is_water() && Surface::classify_among(47.2, onega()).is_water());
        assert_eq!(Surface::classify_among(35.0, onega()), Surface::Land(35.0));

        // Where the boxes of two lakes overlap, each height finds its own.
        let both = || BASINS.iter().filter(|basin| basin.contains(42.5, -82.7));
        assert!(both().count() >= 1);
        assert!(Surface::classify_among(175.2, both()).is_water());
        assert_eq!(Surface::classify_among(181.0, both()), Surface::Land(181.0));
    }
}
