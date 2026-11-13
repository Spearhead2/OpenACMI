//! Decoded elevation of one tile.

use crate::tile::TILE_SIZE;

/// Heights in metres relative to mean sea level, row-major from the
/// north-west corner. Negative values are sea floor (bathymetry).
#[derive(Debug, Clone, PartialEq)]
pub struct ElevationTile {
    size: usize,
    heights: Box<[f32]>,
    min: f32,
    max: f32,
}

impl ElevationTile {
    /// Builds a tile from raw heights. `heights.len()` must be `size * size`.
    pub fn from_heights(size: usize, heights: Vec<f32>) -> Self {
        assert_eq!(heights.len(), size * size, "height grid must be square");
        let (min, max) = heights.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &h| (lo.min(h), hi.max(h)));
        Self { size, heights: heights.into_boxed_slice(), min, max }
    }

    /// A tile at sea level everywhere, used before real data arrives.
    pub fn flat() -> Self {
        Self::from_heights(1, vec![0.0])
    }

    /// Decodes a Terrarium-encoded image:
    /// `height = red × 256 + green + blue / 256 − 32768`.
    pub fn decode_terrarium(image_bytes: &[u8]) -> image::ImageResult<Self> {
        let image = image::load_from_memory(image_bytes)?.into_rgb8();
        let (width, height) = image.dimensions();
        if width != height || width == 0 {
            return Err(image::ImageError::Parameter(image::error::ParameterError::from_kind(
                image::error::ParameterErrorKind::DimensionMismatch,
            )));
        }
        let heights =
            image.pixels().map(|p| p[0] as f32 * 256.0 + p[1] as f32 + p[2] as f32 / 256.0 - 32_768.0).collect::<Vec<f32>>();
        Ok(Self::from_heights(width as usize, heights))
    }

    /// Pixels per side (normally [`TILE_SIZE`]).
    #[inline]
    pub fn size(&self) -> usize {
        self.size
    }

    #[inline]
    pub fn heights(&self) -> &[f32] {
        &self.heights
    }

    /// Lowest and highest value in the tile.
    #[inline]
    pub fn range(&self) -> (f32, f32) {
        (self.min, self.max)
    }

    /// Height of the pixel at column `x`, row `y`, clamped to the tile.
    #[inline]
    pub fn pixel(&self, x: isize, y: isize) -> f32 {
        let clamp = |v: isize| v.clamp(0, self.size as isize - 1) as usize;
        self.heights[clamp(y) * self.size + clamp(x)]
    }

    /// Bilinearly interpolated height at `(u, v)` in `0..=1`.
    pub fn sample(&self, u: f64, v: f64) -> f32 {
        // Pixel centres sit at half-integer positions.
        let x = u * self.size as f64 - 0.5;
        let y = v * self.size as f64 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let top = self.pixel(x0, y0) * (1.0 - fx) + self.pixel(x0 + 1, y0) * fx;
        let bottom = self.pixel(x0, y0 + 1) * (1.0 - fx) + self.pixel(x0 + 1, y0 + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

impl Default for ElevationTile {
    fn default() -> Self {
        Self::flat()
    }
}

/// Encodes heights as a Terrarium PNG. The inverse of
/// [`ElevationTile::decode_terrarium`]; used by tests and tools.
pub fn encode_terrarium(size: u32, heights: &[f32]) -> Vec<u8> {
    let mut image = image::RgbImage::new(size, size);
    for (pixel, &height) in image.pixels_mut().zip(heights) {
        let value = ((height + 32_768.0) * 256.0).round().clamp(0.0, 16_777_215.0) as u32;
        *pixel = image::Rgb([(value >> 16) as u8, (value >> 8) as u8, value as u8]);
    }
    let mut bytes = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).expect("encoding to memory cannot fail");
    bytes
}

const _: () = assert!(TILE_SIZE.is_power_of_two());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrarium_round_trip() {
        let heights = vec![-10_994.0, -0.5, 0.0, 8_848.25];
        let tile = ElevationTile::decode_terrarium(&encode_terrarium(2, &heights)).unwrap();
        assert_eq!(tile.size(), 2);
        for (decoded, expected) in tile.heights().iter().zip(&heights) {
            assert!((decoded - expected).abs() < 1.0 / 256.0 + 1e-3, "{decoded} vs {expected}");
        }
        assert_eq!(tile.range().1, 8_848.25);
    }

    #[test]
    fn rejects_garbage() {
        assert!(ElevationTile::decode_terrarium(b"not a png").is_err());
    }

    #[test]
    fn bilinear_sampling() {
        let tile = ElevationTile::from_heights(2, vec![0.0, 100.0, 200.0, 300.0]);
        assert_eq!(tile.sample(0.25, 0.25), 0.0, "pixel centre");
        assert_eq!(tile.sample(0.5, 0.5), 150.0);
        assert_eq!(tile.sample(0.0, 0.0), 0.0, "edges clamp");
        assert_eq!(tile.sample(1.0, 1.0), 300.0);
        assert_eq!(ElevationTile::flat().sample(0.3, 0.9), 0.0);
    }
}
