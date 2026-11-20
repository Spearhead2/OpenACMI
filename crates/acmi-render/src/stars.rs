//! The night sky: real stars from a catalogue, drawn as points of light.
//!
//! Each star is a small soft disc at its true place on the sky, sized and
//! lit by its magnitude and tinted by its colour index. Being points rather
//! than a picture, they stay sharp at any zoom.
//!
//! Directions are equatorial (J2000): x towards the March equinox, z towards
//! the celestial north pole. The application supplies the rotation from
//! that frame to the world for the time shown (see `Stars`).

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// Start of a star file (see `tools/build_stars.py`).
const MAGIC: &[u8; 8] = b"OASTARS1";
const RECORD_BYTES: usize = 8;

/// One star of the catalogue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Star {
    /// Unit vector towards the star, equatorial.
    pub direction: Vec3,
    /// Visual magnitude: smaller is brighter.
    pub magnitude: f32,
    /// B−V colour index: about −0.3 for blue stars to 2 for red ones.
    pub color_index: f32,
}

/// Reads a star file; `None` when it is not one.
pub fn parse_catalogue(bytes: &[u8]) -> Option<Vec<Star>> {
    let rest = bytes.strip_prefix(MAGIC)?;
    let (count, records) = rest.split_first_chunk::<4>()?;
    let count = u32::from_le_bytes(*count) as usize;
    if records.len() != count * RECORD_BYTES {
        return None;
    }
    let stars = records.chunks_exact(RECORD_BYTES).map(|record| {
        let field = |i: usize| [record[i * 2], record[i * 2 + 1]];
        let ascension = u16::from_le_bytes(field(0)) as f32 / 65536.0 * std::f32::consts::TAU;
        let declination = i16::from_le_bytes(field(1)) as f32 / 32767.0 * std::f32::consts::FRAC_PI_2;
        Star {
            direction: Vec3::new(declination.cos() * ascension.cos(), declination.cos() * ascension.sin(), declination.sin()),
            magnitude: i16::from_le_bytes(field(2)) as f32 / 1000.0,
            color_index: i16::from_le_bytes(field(3)) as f32 / 1000.0,
        }
    });
    Some(stars.collect())
}

/// A star as the GPU draws it.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub(crate) struct StarInstance {
    direction: [f32; 3],
    /// Radius of the disc in pixels.
    radius: f32,
    /// Linear colour at the centre of the disc.
    color: [f32; 3],
    _pad: f32,
}

impl From<&Star> for StarInstance {
    fn from(star: &Star) -> Self {
        // Light received, relative to a star of magnitude 0, compressed: the
        // eye (and a screen) cannot span the factor of 1500 between Sirius
        // and the faintest star seen without a telescope.
        let light = 10f32.powf(-0.4 * star.magnitude).powf(0.6);
        // Bright stars grow, faint ones dim: a disc cannot shrink below a pixel.
        let radius = (1.3 + 2.2 * light.sqrt()).min(5.5);
        let peak = (light * 7.0).clamp(0.18, 1.0);
        let color = star_color(star.color_index) * peak;
        Self { direction: star.direction.to_array(), radius, color: color.to_array(), _pad: 0.0 }
    }
}

/// Linear colour of a star with B−V colour index `color_index`, brightest
/// channel at 1: its black-body colour, washed out as the eye sees it.
pub fn star_color(color_index: f32) -> Vec3 {
    let index = color_index.clamp(-0.4, 2.0);
    // Ballesteros' formula for the temperature, in kelvin.
    let kelvin = 4600.0 * (1.0 / (0.92 * index + 1.7) + 1.0 / (0.92 * index + 0.62));
    // Black-body colour in sRGB (Tanner Helland's fit), temperature in hundreds.
    let t = (kelvin / 100.0).clamp(10.0, 400.0);
    let (red, green, blue);
    if t <= 66.0 {
        red = 1.0;
        green = (0.390_081_58 * t.ln() - 0.631_841_4).clamp(0.0, 1.0);
        blue = if t <= 19.0 { 0.0 } else { (0.543_206_8 * (t - 10.0).ln() - 1.196_254_1).clamp(0.0, 1.0) };
    } else {
        red = (1.292_936_2 * (t - 60.0).powf(-0.133_204_76)).clamp(0.0, 1.0);
        green = (1.129_890_9 * (t - 60.0).powf(-0.075_514_85)).clamp(0.0, 1.0);
        blue = 1.0;
    }
    // Starlight is too faint for the eye to see much colour in it.
    const SATURATION: f32 = 0.55;
    let srgb = Vec3::ONE.lerp(Vec3::new(red, green, blue), SATURATION);
    let linear = srgb.powf(2.2);
    linear / linear.max_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(records: &[(u16, i16, i16, i16)]) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend((records.len() as u32).to_le_bytes());
        for (ascension, declination, magnitude, color) in records {
            bytes.extend(ascension.to_le_bytes());
            bytes.extend(declination.to_le_bytes());
            bytes.extend(magnitude.to_le_bytes());
            bytes.extend(color.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn a_star_file_is_read_back() {
        // On the equinox, a quarter turn east of it, and at the north pole.
        let stars = parse_catalogue(&file(&[(0, 0, -1460, 0), (16384, 0, 2500, 1500), (0, 32767, 6500, -200)])).unwrap();
        assert_eq!(stars.len(), 3);
        assert!((stars[0].direction - Vec3::X).length() < 1e-4);
        assert!((stars[1].direction - Vec3::Y).length() < 1e-4);
        assert!((stars[2].direction - Vec3::Z).length() < 1e-4);
        assert_eq!((stars[0].magnitude, stars[1].color_index), (-1.46, 1.5));
    }

    #[test]
    fn other_files_are_refused() {
        assert_eq!(parse_catalogue(b"not a star file"), None);
        // A count that does not match the data.
        let mut short = file(&[(0, 0, 0, 0), (1, 1, 1, 1)]);
        short.truncate(short.len() - 3);
        assert_eq!(parse_catalogue(&short), None);
        assert_eq!(parse_catalogue(&file(&[])), Some(Vec::new()));
    }

    #[test]
    fn brighter_stars_are_drawn_larger_and_no_dimmer() {
        let drawn = |magnitude: f32| StarInstance::from(&Star { direction: Vec3::X, magnitude, color_index: 0.6 });
        let (sirius, middling, faint) = (drawn(-1.46), drawn(3.0), drawn(6.5));
        assert!(sirius.radius > middling.radius && middling.radius > faint.radius);
        let peak = |star: StarInstance| star.color.into_iter().fold(0.0, f32::max);
        assert!(peak(sirius) >= peak(middling) && peak(middling) > peak(faint) && peak(faint) > 0.0);
    }

    #[test]
    fn colour_follows_the_colour_index() {
        let (blue, sun_like, red) = (star_color(-0.25), star_color(0.65), star_color(1.85));
        // Hot stars lean blue, cool ones red; none is strongly coloured.
        assert!(blue.z > blue.x && red.x > red.z);
        assert!((sun_like.x - sun_like.z).abs() < 0.35);
        for color in [blue, sun_like, red] {
            assert!((color.max_element() - 1.0).abs() < 1e-5 && color.min_element() > 0.1);
        }
    }
}
