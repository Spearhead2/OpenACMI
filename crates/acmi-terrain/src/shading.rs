//! Colouring terrain from its elevation.
//!
//! Without imagery the ground texture is synthesised from the DEM, in one
//! of several colour schemes ([`ElevationPalette`]). In all of them:
//!
//! * Water is blue, darkening with depth. What counts as water is decided
//!   in [`crate::surface`]: the sea, and lakes in basins below sea level.
//! * Land follows a ramp of colours by height (in the natural scheme green
//!   lowlands → tan → rock → snow, with sand below sea level).
//! * The snow line drops towards the poles, so ice caps come out white.
//! * Relief shading from the local slope makes the terrain readable from
//!   straight above. It is baked in, independent of the scene's sun.

use crate::surface::{Surface, SurfaceTile};
use crate::tile::TileKey;

type Rgb = [f32; 3];

/// A colour scheme for elevation: what [`shade_tile`] paints with.
///
/// To add one: a variant here, a row in [`ElevationPalette::ALL`], its names in
/// [`ElevationPalette::name`] and [`ElevationPalette::label`], and its colours in `colors`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ElevationPalette {
    /// Green lowlands, tan hills, grey rock, snow.
    #[default]
    Natural,
    /// The tints of a printed atlas: green, yellow, orange, brown, white.
    Atlas,
    /// Sand and rock, for theatres where green lowlands look wrong.
    Arid,
    /// Height as brightness only.
    Greyscale,
    /// Dim and low in contrast, so objects and trails stand out.
    Dark,
}

/// The colours of one palette.
struct Colors {
    /// `(elevation in metres, colour)` stops of the land ramp, ascending.
    land: &'static [(f32, Rgb)],
    snow: Rgb,
    shallow_water: Rgb,
    deep_water: Rgb,
}

impl ElevationPalette {
    pub const ALL: [ElevationPalette; 5] = [
        ElevationPalette::Natural,
        ElevationPalette::Atlas,
        ElevationPalette::Arid,
        ElevationPalette::Greyscale,
        ElevationPalette::Dark,
    ];

    /// Stable identifier, used in settings files.
    pub fn name(self) -> &'static str {
        match self {
            ElevationPalette::Natural => "natural",
            ElevationPalette::Atlas => "atlas",
            ElevationPalette::Arid => "arid",
            ElevationPalette::Greyscale => "greyscale",
            ElevationPalette::Dark => "dark",
        }
    }

    /// Name to show to people.
    pub fn label(self) -> &'static str {
        match self {
            ElevationPalette::Natural => "Natural",
            ElevationPalette::Atlas => "Atlas",
            ElevationPalette::Arid => "Arid",
            ElevationPalette::Greyscale => "Greyscale",
            ElevationPalette::Dark => "Dark",
        }
    }

    /// The palette with this [`Self::name`], ignoring case.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|palette| palette.name().eq_ignore_ascii_case(name.trim()))
    }

    fn colors(self) -> &'static Colors {
        match self {
            ElevationPalette::Natural => &Colors {
                land: &[
                    // Ground below sea level is desert nearly everywhere.
                    (-150.0, [0.62, 0.56, 0.39]),
                    (0.0, [0.27, 0.42, 0.23]),
                    (250.0, [0.38, 0.51, 0.27]),
                    (700.0, [0.56, 0.58, 0.35]),
                    (1400.0, [0.66, 0.58, 0.40]),
                    (2400.0, [0.58, 0.48, 0.37]),
                    (3400.0, [0.56, 0.53, 0.50]),
                    (4500.0, [0.78, 0.78, 0.78]),
                ],
                snow: [0.95, 0.96, 0.98],
                shallow_water: [0.22, 0.47, 0.67],
                deep_water: [0.03, 0.10, 0.27],
            },
            ElevationPalette::Atlas => &Colors {
                land: &[
                    (-400.0, [0.24, 0.42, 0.33]),
                    (0.0, [0.44, 0.66, 0.42]),
                    (200.0, [0.66, 0.78, 0.50]),
                    (500.0, [0.93, 0.89, 0.60]),
                    (1000.0, [0.90, 0.74, 0.47]),
                    (2000.0, [0.76, 0.54, 0.36]),
                    (3000.0, [0.61, 0.42, 0.32]),
                    (4000.0, [0.66, 0.58, 0.56]),
                    (5500.0, [0.93, 0.92, 0.92]),
                ],
                snow: [1.0, 1.0, 1.0],
                shallow_water: [0.71, 0.86, 0.95],
                deep_water: [0.20, 0.41, 0.68],
            },
            ElevationPalette::Arid => &Colors {
                land: &[
                    (-400.0, [0.66, 0.60, 0.46]),
                    (0.0, [0.80, 0.73, 0.54]),
                    (500.0, [0.76, 0.64, 0.45]),
                    (1500.0, [0.65, 0.50, 0.36]),
                    (3000.0, [0.52, 0.42, 0.35]),
                    (4500.0, [0.74, 0.72, 0.69]),
                ],
                snow: [0.95, 0.96, 0.98],
                shallow_water: [0.25, 0.52, 0.66],
                deep_water: [0.04, 0.13, 0.30],
            },
            ElevationPalette::Greyscale => &Colors {
                land: &[
                    (-400.0, [0.22, 0.22, 0.22]),
                    (0.0, [0.34, 0.34, 0.34]),
                    (2000.0, [0.62, 0.62, 0.62]),
                    (5000.0, [0.90, 0.90, 0.90]),
                ],
                snow: [0.97, 0.97, 0.97],
                // Still bluish: land and sea must not be confused.
                shallow_water: [0.20, 0.26, 0.34],
                deep_water: [0.05, 0.08, 0.14],
            },
            ElevationPalette::Dark => &Colors {
                land: &[
                    (-400.0, [0.13, 0.14, 0.12]),
                    (0.0, [0.16, 0.19, 0.16]),
                    (1000.0, [0.24, 0.25, 0.20]),
                    (3000.0, [0.33, 0.31, 0.27]),
                    (5000.0, [0.44, 0.44, 0.43]),
                ],
                snow: [0.58, 0.60, 0.62],
                shallow_water: [0.07, 0.11, 0.17],
                deep_water: [0.02, 0.04, 0.08],
            },
        }
    }
}

/// Depth at which water reaches its darkest colour, metres.
const DEEP_WATER_DEPTH: f32 = 6000.0;

/// Elevation above which the ground is snow-covered at a latitude.
fn snow_line(latitude: f64) -> f32 {
    let t = (latitude.abs() / 72.0).clamp(0.0, 1.0) as f32;
    // About 5 km in the tropics, sea level in the polar regions.
    5000.0 * (1.0 - t * t)
}

fn lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Colour of a point from its surface and latitude, before relief shading.
pub fn base_color(surface: Surface, latitude: f64, palette: ElevationPalette) -> Rgb {
    let colors = palette.colors();
    let height = match surface {
        Surface::Water { depth, .. } => {
            // Square root: most of the visible change happens on the shelves.
            let depth = (depth / DEEP_WATER_DEPTH).clamp(0.0, 1.0).sqrt();
            return lerp(colors.shallow_water, colors.deep_water, depth);
        }
        Surface::Land(height) => height,
    };
    let ramp = colors.land;
    let upper = ramp.partition_point(|(elevation, _)| *elevation <= height);
    let land = match (ramp.get(upper.wrapping_sub(1)), ramp.get(upper)) {
        (Some((e0, c0)), Some((e1, c1))) => lerp(*c0, *c1, (height - e0) / (e1 - e0)),
        (Some((_, last)), None) => *last,
        _ => ramp[0].1,
    };
    // Fade into snow over a few hundred metres around the snow line.
    let snow = ((height - snow_line(latitude)) / 400.0 + 0.5).clamp(0.0, 1.0);
    lerp(land, colors.snow, snow)
}

/// Calls `visit(row, column, latitude, surface, relief)` for every sample of
/// a tile. `relief` is the brightness factor of the relief shading: 1 on
/// flat ground and on water, above on slopes facing the light, below on
/// slopes facing away.
fn for_each_sample(key: TileKey, tile: &SurfaceTile, mut visit: impl FnMut(usize, usize, f64, Surface, f32)) {
    /// Direction towards the light: north-west, 45° above the horizon,
    /// in (east, south, up) image axes.
    const LIGHT: [f32; 3] = [-0.5, -0.5, 0.707_106_77];

    let size = tile.size();
    for row in 0..size {
        let latitude = key.latitude((row as f64 + 0.5) / size as f64);
        // Spacing between samples; tiles smaller than 256 px cover the same ground.
        let spacing = (key.meters_per_pixel(latitude) * crate::tile::TILE_SIZE as f64 / size as f64) as f32;
        // Coarse levels have slopes far too gentle to see; exaggerate them.
        let exaggeration = (spacing / 30.0).max(1.0).powf(0.6);

        for column in 0..size {
            let (x, y) = (column as isize, row as isize);
            let surface = tile.surface(x, y);
            let relief = if surface.is_water() {
                1.0
            } else {
                // Water neighbours count at their surface, or coasts would get cliffs.
                let land = |x, y| tile.heights.pixel(x, y);
                let dz_dx = (land(x + 1, y) - land(x - 1, y)) / (2.0 * spacing) * exaggeration;
                let dz_dy = (land(x, y + 1) - land(x, y - 1)) / (2.0 * spacing) * exaggeration;
                let length = (dz_dx * dz_dx + dz_dy * dz_dy + 1.0).sqrt();
                let normal = [-dz_dx / length, -dz_dy / length, 1.0 / length];
                let lambert = (normal[0] * LIGHT[0] + normal[1] * LIGHT[1] + normal[2] * LIGHT[2]).max(0.0);
                // Flat ground (lambert ≈ 0.71) keeps its colour.
                0.45 + 0.78 * lambert
            };
            visit(row, column, latitude, surface, relief);
        }
    }
}

/// Renders the colour image of a tile from its elevation: tightly packed
/// sRGB RGBA, one pixel per height sample.
pub fn shade_tile(key: TileKey, tile: &SurfaceTile, palette: ElevationPalette) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(tile.size() * tile.size() * 4);
    for_each_sample(key, tile, |_, _, latitude, surface, relief| {
        let color = base_color(surface, latitude, palette).map(|c| c * relief);
        pixels.extend(color.map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8));
        pixels.push(255);
    });
    pixels
}

/// Lays the relief shading of a tile over an image of it (sRGB RGBA, as
/// from [`crate::Imagery::tile`]). `strength` runs from 0 (image untouched)
/// to 1 (as strong as on elevation colours). The image may have a different
/// resolution than the heights.
pub fn shade_image(key: TileKey, tile: &SurfaceTile, image: &mut [u8], strength: f32) {
    let size = tile.size();
    let image_size = (image.len() / 4).isqrt();
    if strength <= 0.0 || size == 0 || image_size == 0 || !image_size.is_multiple_of(size) {
        return;
    }
    let scale = image_size / size;
    for_each_sample(key, tile, |row, column, _, _, relief| {
        let factor = 1.0 + (relief - 1.0) * strength;
        for y in row * scale..(row + 1) * scale {
            let line = &mut image[(y * image_size + column * scale) * 4..][..scale * 4];
            for pixel in line.chunks_exact_mut(4) {
                for channel in &mut pixel[..3] {
                    *channel = (*channel as f32 * factor).min(255.0) as u8;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elevation::ElevationTile;
    use crate::surface::basin_at;

    fn sea_or_land(height: f32) -> Surface {
        Surface::classify(height, None)
    }

    /// The natural palette, which the colour expectations below describe.
    fn base_color(surface: Surface, latitude: f64) -> Rgb {
        super::base_color(surface, latitude, ElevationPalette::Natural)
    }

    #[test]
    fn every_palette_is_usable() {
        for palette in ElevationPalette::ALL {
            assert_eq!(ElevationPalette::from_name(palette.name()), Some(palette));
            assert!(!palette.label().is_empty());
            let colors = palette.colors();
            assert!(colors.land.windows(2).all(|pair| pair[0].0 < pair[1].0), "{palette:?}: stops ascend");
            // Sea and low land must be told apart, and water is never redder than blue.
            let sea = super::base_color(sea_or_land(-50.0), 20.0, palette);
            let land = super::base_color(sea_or_land(50.0), 20.0, palette);
            assert!(sea[2] >= sea[0], "{palette:?}: water {sea:?}");
            assert!(sea.iter().zip(land).any(|(a, b)| (a - b).abs() > 0.05), "{palette:?}: {sea:?} vs {land:?}");
        }
        assert_eq!(ElevationPalette::from_name(" Atlas "), Some(ElevationPalette::Atlas));
        assert_eq!(ElevationPalette::from_name("nope"), None);
        // Palettes really differ.
        let lowland = |palette| super::base_color(sea_or_land(100.0), 20.0, palette);
        assert_ne!(lowland(ElevationPalette::Natural), lowland(ElevationPalette::Arid));
        let grey = lowland(ElevationPalette::Greyscale);
        assert!(grey[0] == grey[1] && grey[1] == grey[2]);
    }

    fn shade(key: TileKey, size: usize, heights: Vec<f32>) -> Vec<u8> {
        shade_tile(key, &SurfaceTile::resolve(key, &ElevationTile::from_heights(size, heights)), ElevationPalette::Natural)
    }

    fn is_blue(color: Rgb) -> bool {
        color[2] > color[0] && color[2] > color[1]
    }

    #[test]
    fn water_is_blue_and_darkens_with_depth() {
        let shallow = base_color(sea_or_land(-5.0), 30.0);
        let deep = base_color(sea_or_land(-7000.0), 30.0);
        assert!(is_blue(shallow) && is_blue(deep) && is_blue(base_color(sea_or_land(0.0), 0.0)));
        assert!(deep.iter().sum::<f32>() < shallow.iter().sum::<f32>());
    }

    #[test]
    fn land_follows_elevation_and_latitude() {
        let base_color = |height, latitude| base_color(sea_or_land(height), latitude);
        let lowland = base_color(50.0, 40.0);
        assert!(lowland[1] > lowland[0] && lowland[1] > lowland[2], "lowlands are green");
        assert!(!is_blue(base_color(1.0, 40.0)));
        // High mountains and polar ground are snow.
        assert!(base_color(6000.0, 28.0).iter().all(|&c| c > 0.9));
        assert!(base_color(300.0, 80.0).iter().all(|&c| c > 0.9));
        assert!(base_color(300.0, 10.0)[2] < 0.5);
    }

    #[test]
    fn relief_shading_distinguishes_slopes() {
        // A ridge running north-south: west faces are lit, east faces shaded.
        let size = 8;
        let heights: Vec<f32> = (0..size * size).map(|i| 3000.0 - ((i % size) as f32 - 3.5).abs() * 400.0).collect();
        let pixels = shade(TileKey::new(12, 2048, 2048), size, heights);
        assert_eq!(pixels.len(), size * size * 4);
        let brightness = |column: usize| pixels[(4 * size + column) * 4..][..3].iter().map(|&c| c as u32).sum::<u32>();
        assert!(brightness(1) > brightness(6), "west slope {} vs east slope {}", brightness(1), brightness(6));
    }

    #[test]
    fn flat_sea_tile_is_uniformly_blue() {
        let pixels = shade(TileKey::new(3, 0, 3), 4, vec![-200.0; 16]);
        assert!(pixels.chunks_exact(4).all(|p| p == &pixels[..4] && p[2] > p[0] && p[3] == 255));
    }

    #[test]
    fn relief_is_laid_over_imagery() {
        let size = 8;
        let ridge: Vec<f32> = (0..size * size).map(|i| 3000.0 - ((i % size) as f32 - 3.5).abs() * 400.0).collect();
        let key = TileKey::new(12, 2048, 2048);
        let tile = SurfaceTile::resolve(key, &ElevationTile::from_heights(size, ridge));
        let grey = vec![128u8; size * size * 4];
        let red = |image: &[u8], column: usize| image[(4 * size + column) * 4];

        let mut image = grey.clone();
        shade_image(key, &tile, &mut image, 0.6);
        assert!(red(&image, 1) > red(&image, 6) && red(&image, 6) < 128, "lit west slope, shaded east slope");
        assert_eq!(image[3], 128, "alpha is untouched");

        let mut untouched = grey.clone();
        shade_image(key, &tile, &mut untouched, 0.0);
        assert_eq!(untouched, grey);

        // An image at twice the resolution of the heights.
        let mut fine = vec![128u8; size * size * 16];
        shade_image(key, &tile, &mut fine, 0.6);
        assert_eq!(fine[(8 * 2 * size + 2) * 4], red(&image, 1));
    }

    #[test]
    fn land_below_sea_level_is_not_blue() {
        // The Dead Sea shore: dry ground 380 m down, next to the lake.
        let basin = basin_at(31.5, 35.48);
        let shore = base_color(Surface::classify(-380.0, basin), 31.5);
        assert!(!is_blue(shore) && shore[0] > shore[2], "sand, got {shore:?}");
        assert!(is_blue(base_color(Surface::classify(-416.0, basin), 31.5)), "the lake is water");
        // The same height outside any basin is sea floor.
        assert!(is_blue(base_color(sea_or_land(-380.0), 31.5)));
    }
}
