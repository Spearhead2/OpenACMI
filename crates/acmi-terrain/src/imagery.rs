//! Satellite (or map) imagery draped over the terrain.
//!
//! Imagery comes from any server of Web-Mercator "XYZ" tiles, the same
//! addressing the elevation tiles use, so one image covers exactly one
//! terrain tile. Tiles are cached on disk like elevation tiles.
//!
//! The default is "Sentinel-2 cloudless" by EOX: a global, cloud-free mosaic
//! at about 10 m per pixel, free to use with attribution and no key.

use image::imageops::{self, FilterType};
use image::RgbaImage;

use crate::source::{FetchError, TileSource};
use crate::tile::{TileKey, TILE_SIZE};

/// Sentinel-2 cloudless 2016 by EOX, licensed CC BY 4.0. Note the `{y}/{x}` order.
pub const DEFAULT_IMAGERY_URL_TEMPLATE: &str = "https://tiles.maps.eox.at/wmts/1.0.0/s2cloudless_3857/default/g/{z}/{y}/{x}.jpg";
/// Credit the licence of the default imagery requires to be shown.
pub const DEFAULT_IMAGERY_ATTRIBUTION: &str =
    "Sentinel-2 cloudless by EOX IT Services GmbH (contains modified Copernicus Sentinel data 2016 & 2017)";
/// Deepest level at which the default imagery still gains detail.
pub const DEFAULT_IMAGERY_MAX_ZOOM: u8 = 13;

/// An imagery layer.
pub struct Imagery {
    source: TileSource,
    max_zoom: u8,
    relief: f32,
    brightness: f32,
    saturation: f32,
    gamma: f32,
}

impl Imagery {
    /// `max_zoom` is the deepest level the server has; deeper tiles are cut
    /// out of it. `relief` (0 to 1) is how strongly the terrain's relief
    /// shading is laid over the image.
    pub fn new(source: TileSource, max_zoom: u8, relief: f32) -> Self {
        Self { source, max_zoom, relief: relief.clamp(0.0, 1.0), brightness: 1.0, saturation: 1.0, gamma: 1.0 }
    }

    /// Adjusts the pictures: both factors are 1 for "as served". Satellite
    /// mosaics tend to be glaring over deserts, so a little less of each
    /// usually reads better behind aircraft and labels.
    ///
    /// `gamma` below 1 lifts the shadows (dark forest) without pushing the
    /// highlights (desert, snow) any further; 1 leaves the contrast alone.
    pub fn with_tone(mut self, brightness: f32, saturation: f32, gamma: f32) -> Self {
        self.brightness = brightness.clamp(0.0, 4.0);
        self.saturation = saturation.clamp(0.0, 4.0);
        self.gamma = gamma.clamp(0.2, 5.0);
        self
    }

    pub fn source(&self) -> &TileSource {
        &self.source
    }

    /// Strength of the relief shading over the image, 0 to 1.
    pub fn relief(&self) -> f32 {
        self.relief
    }

    /// The image of a tile as tightly packed sRGB RGBA, [`TILE_SIZE`] pixels square.
    pub fn tile(&self, key: TileKey) -> Result<Vec<u8>, FetchError> {
        let (available, offset_x, offset_y, span) = ancestor(key, self.max_zoom);
        let image = image::load_from_memory(&self.source.fetch(available)?)?.into_rgba8();
        let mut pixels = cut(&image, offset_x, offset_y, span).into_raw();
        if self.brightness != 1.0 || self.saturation != 1.0 || self.gamma != 1.0 {
            tone(&mut pixels, self.brightness, self.saturation, self.gamma);
        }
        Ok(pixels)
    }
}

/// Adjusts RGBA pixels in place: saturation first, then `brightness × value^gamma`.
fn tone(pixels: &mut [u8], brightness: f32, saturation: f32, gamma: f32) {
    let curve: Vec<u8> =
        (0..=255).map(|v| ((v as f32 / 255.0).powf(gamma) * brightness * 255.0).clamp(0.0, 255.0).round() as u8).collect();
    for pixel in pixels.chunks_exact_mut(4) {
        let [r, g, b] = [pixel[0] as f32, pixel[1] as f32, pixel[2] as f32];
        let grey = 0.299 * r + 0.587 * g + 0.114 * b;
        for (channel, value) in pixel.iter_mut().zip([r, g, b]) {
            *channel = curve[(grey + (value - grey) * saturation).clamp(0.0, 255.0).round() as usize];
        }
    }
}

/// The tile at `max_zoom` or above that covers `key`, and where `key` lies in
/// it: its column and row among the `span × span` tiles of its own level.
fn ancestor(key: TileKey, max_zoom: u8) -> (TileKey, u32, u32, u32) {
    let levels = key.z.saturating_sub(max_zoom).min(16);
    let available = TileKey::new(key.z - levels, key.x >> levels, key.y >> levels);
    let mask = (1u32 << levels) - 1;
    (available, key.x & mask, key.y & mask, 1 << levels)
}

/// Cuts cell (`column`, `row`) out of an image divided `span × span`, scaled to the tile size.
fn cut(image: &RgbaImage, column: u32, row: u32, span: u32) -> RgbaImage {
    let size = TILE_SIZE as u32;
    let (width, height) = image.dimensions();
    if span == 1 && (width, height) == (size, size) {
        return image.clone();
    }
    let (x, y) = (column * width / span, row * height / span);
    let part = imageops::crop_imm(image, x, y, (width / span).max(1), (height / span).max(1)).to_image();
    imageops::resize(&part, size, size, FilterType::Triangle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn tiles_within_range_are_used_directly() {
        assert_eq!(ancestor(TileKey::new(9, 300, 200), 13), (TileKey::new(9, 300, 200), 0, 0, 1));
        assert_eq!(ancestor(TileKey::new(13, 5, 6), 13), (TileKey::new(13, 5, 6), 0, 0, 1));
    }

    #[test]
    fn deeper_tiles_are_cut_from_the_deepest_available() {
        // Two levels down: the tile is one of 4 × 4 cells of its ancestor.
        assert_eq!(ancestor(TileKey::new(15, 4 * 100 + 3, 4 * 70 + 1), 13), (TileKey::new(13, 100, 70), 3, 1, 4));

        let mut image = RgbaImage::from_pixel(256, 256, Rgba([10, 10, 10, 255]));
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            if x >= 192 && (64..128).contains(&y) {
                *pixel = Rgba([200, 100, 50, 255]);
            }
        }
        let part = cut(&image, 3, 1, 4);
        assert_eq!(part.dimensions(), (256, 256));
        assert!(part.pixels().all(|pixel| *pixel == Rgba([200, 100, 50, 255])));
        assert_eq!(cut(&image, 0, 0, 4).get_pixel(128, 128), &Rgba([10, 10, 10, 255]));
    }

    #[test]
    fn tone_scales_brightness_and_saturation() {
        let mut pixels = [200, 100, 50, 255, 80, 80, 80, 255];
        tone(&mut pixels, 0.5, 1.0, 1.0);
        assert_eq!(pixels, [100, 50, 25, 255, 40, 40, 40, 255]);
        let mut pixels = [200, 100, 50, 255];
        tone(&mut pixels, 1.0, 0.0, 1.0);
        assert!(pixels[0] == pixels[1] && pixels[1] == pixels[2] && pixels[3] == 255, "grey: {pixels:?}");
    }

    #[test]
    fn gamma_lifts_shadows_more_than_highlights() {
        let mut pixels = [40, 40, 40, 255, 240, 240, 240, 255, 0, 0, 0, 255];
        tone(&mut pixels, 1.0, 1.0, 0.7);
        let (shadow, highlight) = (pixels[0] as i32 - 40, pixels[4] as i32 - 240);
        assert!(shadow > 20 && (0..10).contains(&highlight), "shadow +{shadow}, highlight +{highlight}");
        assert_eq!(pixels[8], 0, "black stays black");
    }

    #[test]
    fn odd_sized_tiles_are_scaled() {
        let image = RgbaImage::from_pixel(512, 512, Rgba([1, 2, 3, 255]));
        assert_eq!(cut(&image, 0, 0, 1).dimensions(), (256, 256));
    }

    #[test]
    fn loads_cached_imagery() {
        let dir = std::env::temp_dir().join(format!("openacmi-imagery-test-{}", std::process::id()));
        let key = TileKey::new(4, 3, 5);
        // Row before column, as in the default template.
        let path = dir.join("4/3/5.jpg");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        RgbaImage::from_pixel(256, 256, Rgba([90, 120, 60, 255])).save_with_format(&path, image::ImageFormat::Png).unwrap();

        let imagery = Imagery::new(TileSource::new("http://invalid.invalid/{z}/{y}/{x}.jpg", Some(dir.clone()), false), 13, 0.5);
        let pixels = imagery.tile(key).expect("cached tile");
        assert_eq!(pixels.len(), 256 * 256 * 4);
        assert_eq!(&pixels[..4], &[90, 120, 60, 255]);
        // A child beyond the deepest level comes from the same file.
        let deep = Imagery::new(TileSource::new("http://invalid.invalid/{z}/{y}/{x}.jpg", Some(dir.clone()), false), 4, 0.5);
        assert_eq!(&deep.tile(TileKey::new(6, 13, 21)).expect("cut from parent")[..4], &[90, 120, 60, 255]);
        assert!(matches!(imagery.tile(TileKey::new(4, 0, 0)), Err(FetchError::Offline)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
