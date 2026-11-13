//! Decoding of the Earth surface map.
//!
//! Decoding and downscaling a world map takes a noticeable moment, so this
//! is plain CPU work that the application runs on a worker thread; the
//! result is handed to [`crate::Renderer::set_earth_texture`].

use std::path::Path;

use image::imageops;
use image::{ImageReader, RgbaImage};

/// An equirectangular map (180°W at the left edge, north at the top) with
/// its full mip chain, as tightly packed sRGB RGBA8.
pub struct EarthImage {
    pub width: u32,
    pub height: u32,
    /// Level 0 first; each level halves the previous one down to 1×1.
    pub mips: Vec<Vec<u8>>,
}

pub fn load_earth_image(path: &Path, max_width: u32) -> image::ImageResult<EarthImage> {
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    // World maps are legitimately huge; the default allocation cap rejects them.
    reader.no_limits();
    let decoded = reader.decode()?;

    let max_width = max_width.max(2);
    let base: RgbaImage = if decoded.width() > max_width {
        let height = (decoded.height() as u64 * max_width as u64 / decoded.width() as u64).max(1) as u32;
        decoded.thumbnail_exact(max_width, height).into_rgba8()
    } else {
        decoded.into_rgba8()
    };

    let (width, height) = base.dimensions();
    let mut mips = Vec::new();
    let mut level = base;
    loop {
        let (w, h) = level.dimensions();
        let next = (w > 1 || h > 1).then(|| imageops::thumbnail(&level, (w / 2).max(1), (h / 2).max(1)));
        mips.push(level.into_raw());
        match next {
            Some(next) => level = next,
            None => break,
        }
    }
    Ok(EarthImage { width, height, mips })
}
