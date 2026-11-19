//! Exports a flat map picture of a latitude / longitude box, coloured the
//! way the globe is: by elevation, or with satellite imagery.
//!
//! ```text
//! cargo run --release -p acmi-terrain --example export_map -- \
//!     <south> <west> <north> <east> <width px> <look> <out.png> [cache dir]
//! ```
//!
//! `look` is `satellite` or an elevation colour scheme (`natural`, `atlas`,
//! `arid`, `greyscale`, `dark`). The picture is an equirectangular (plate
//! carrée) map: longitude runs evenly across and latitude evenly down, so a
//! pixel's position converts to coordinates with plain proportions. Its
//! height follows from the width and the box.
//!
//! Tiles are streamed from the same servers the application uses and kept
//! in `cache dir` (default: `map-export-cache` in the temp directory).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use acmi_terrain::shading::{shade_image, shade_tile};
use acmi_terrain::{
    ElevationPalette, ElevationTile, Imagery, SeaFloor, SurfaceTile, TileKey, TileSource, DEFAULT_IMAGERY_MAX_ZOOM,
    DEFAULT_IMAGERY_URL_TEMPLATE, DEFAULT_URL_TEMPLATE, TILE_SIZE,
};

/// How the ground is coloured.
enum Look {
    Elevation(ElevationPalette),
    /// Imagery toned and relief-shaded as the application does by default.
    Satellite,
}

const WORKERS: usize = 12;
const ATTEMPTS: usize = 4;
const IMAGERY_RELIEF: f32 = 0.5;
const IMAGERY_TONE: (f32, f32, f32) = (0.9, 0.85, 1.0);

/// Web-Mercator position of a point, in pixels of the whole world at `zoom`.
fn world_pixel(latitude: f64, longitude: f64, zoom: u8) -> (f64, f64) {
    let size = (1u64 << zoom) as f64 * TILE_SIZE as f64;
    let phi = latitude.to_radians();
    let x = (longitude + 180.0) / 360.0 * size;
    let y = (1.0 - (phi.tan() + 1.0 / phi.cos()).ln() / std::f64::consts::PI) * 0.5 * size;
    (x, y)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 7 {
        eprintln!("usage: export_map <south> <west> <north> <east> <width px> <satellite|natural|atlas|arid|greyscale|dark> <out.png> [cache dir]");
        std::process::exit(2);
    }
    let number = |i: usize| args[i].parse::<f64>().unwrap_or_else(|_| panic!("argument {} is not a number: {}", i + 1, args[i]));
    let (south, west, north, east) = (number(0), number(1), number(2), number(3));
    assert!(south < north && west < east && north < 85.0 && south > -85.0, "the box must be south < north, west < east, within 85 degrees");
    let width = number(4) as usize;
    let height = (width as f64 * (north - south) / (east - west)).round() as usize;
    let look = match args[5].as_str() {
        "satellite" => Look::Satellite,
        name => Look::Elevation(ElevationPalette::from_name(name).unwrap_or_else(|| panic!("unknown look: {name}"))),
    };
    let out = PathBuf::from(&args[6]);
    let cache = args.get(7).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("map-export-cache"));

    // The first zoom level at least as detailed as the picture, both ways.
    let per_degree = (width as f64 / (east - west)).max(height as f64 / (north - south));
    let zoom = ((per_degree * 360.0 / TILE_SIZE as f64).log2().ceil() as u8).clamp(1, 13);
    let (left, top) = world_pixel(north, west, zoom);
    let (right, bottom) = world_pixel(south, east, zoom);
    let tile = TILE_SIZE as f64;
    // One pixel of margin for the interpolation at the edges.
    let (x0, y0) = (((left - 1.0) / tile).floor() as u32, ((top - 1.0) / tile).floor() as u32);
    let (x1, y1) = (((right + 1.0) / tile).floor() as u32, ((bottom + 1.0) / tile).floor() as u32);
    let (columns, rows) = ((x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
    println!("{width} x {height} px, zoom {zoom}, {} tiles ({columns} x {rows}), cache {}", columns * rows, cache.display());

    let elevation = TileSource::new(DEFAULT_URL_TEMPLATE, Some(cache.join("terrain")), true);
    let imagery = matches!(look, Look::Satellite).then(|| {
        let source = TileSource::new(DEFAULT_IMAGERY_URL_TEMPLATE, Some(cache.join("imagery")), true);
        Imagery::new(source, DEFAULT_IMAGERY_MAX_ZOOM, IMAGERY_RELIEF).with_tone(IMAGERY_TONE.0, IMAGERY_TONE.1, IMAGERY_TONE.2)
    });

    // Sea floor for tiles that hold the flat sea surface instead.
    let sea_floor = SeaFloor::default();

    // Colour of one tile, RGBA at tile size, as the application bakes it.
    let render = |key: TileKey| -> Result<Vec<u8>, String> {
        let raw = elevation.fetch(key).map_err(|err| err.to_string())?;
        let heights = ElevationTile::decode_terrarium(&raw).map_err(|err| err.to_string())?;
        let coarser = |other: TileKey| ElevationTile::decode_terrarium(&elevation.fetch(other).ok()?).ok();
        let heights = sea_floor.fill(key, heights, &coarser);
        let surface = SurfaceTile::resolve(key, &heights);
        Ok(match (&look, &imagery) {
            (Look::Elevation(palette), _) => shade_tile(key, &surface, *palette),
            (Look::Satellite, Some(imagery)) => {
                let mut pixels = imagery.tile(key).map_err(|err| err.to_string())?;
                shade_image(key, &surface, &mut pixels, IMAGERY_RELIEF);
                pixels
            }
            (Look::Satellite, None) => unreachable!(),
        })
    };

    let jobs: Vec<TileKey> = (y0..=y1).flat_map(|y| (x0..=x1).map(move |x| TileKey::new(zoom, x, y))).collect();
    let (next, done, failed) = (AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0));
    let tiles: Mutex<HashMap<(u32, u32), Vec<u8>>> = Mutex::new(HashMap::with_capacity(jobs.len()));
    std::thread::scope(|scope| {
        for _ in 0..WORKERS {
            scope.spawn(|| while let Some(&key) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) {
                let mut result = render(key);
                for attempt in 1..ATTEMPTS {
                    if result.is_ok() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(400 * attempt as u64));
                    result = render(key);
                }
                match result {
                    Ok(pixels) => {
                        tiles.lock().unwrap().insert((key.x, key.y), pixels);
                    }
                    Err(err) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                        eprintln!("tile {}/{}/{} failed: {err}", key.z, key.x, key.y);
                    }
                }
                let count = done.fetch_add(1, Ordering::Relaxed) + 1;
                if count.is_multiple_of(200) || count == jobs.len() {
                    println!("  {count} / {} tiles", jobs.len());
                }
            });
        }
    });
    let tiles = tiles.into_inner().unwrap();

    // Colour at a world pixel; missing tiles come out mid grey.
    let texel = |x: i64, y: i64| -> [f32; 3] {
        let (tile_x, tile_y) = (x.div_euclid(TILE_SIZE as i64) as u32, y.div_euclid(TILE_SIZE as i64) as u32);
        let (tile_x, tile_y) = (tile_x.clamp(x0, x1), tile_y.clamp(y0, y1));
        let Some(pixels) = tiles.get(&(tile_x, tile_y)) else { return [128.0; 3] };
        let side = (pixels.len() / 4).isqrt() as i64;
        let scale = side / TILE_SIZE as i64;
        let (u, v) = (x.rem_euclid(TILE_SIZE as i64) * scale, y.rem_euclid(TILE_SIZE as i64) * scale);
        let i = ((v * side + u) * 4) as usize;
        [pixels[i] as f32, pixels[i + 1] as f32, pixels[i + 2] as f32]
    };

    let mut image = vec![0u8; width * height * 3];
    for (row, line) in image.chunks_exact_mut(width * 3).enumerate() {
        let latitude = north - (row as f64 + 0.5) / height as f64 * (north - south);
        for (column, pixel) in line.chunks_exact_mut(3).enumerate() {
            let longitude = west + (column as f64 + 0.5) / width as f64 * (east - west);
            // Texel centres sit at half-integer world pixels.
            let (x, y) = world_pixel(latitude, longitude, zoom);
            let (x, y) = (x - 0.5, y - 0.5);
            let (ix, iy) = (x.floor() as i64, y.floor() as i64);
            let (fx, fy) = ((x - x.floor()) as f32, (y - y.floor()) as f32);
            let (a, b, c, d) = (texel(ix, iy), texel(ix + 1, iy), texel(ix, iy + 1), texel(ix + 1, iy + 1));
            for channel in 0..3 {
                let top = a[channel] + (b[channel] - a[channel]) * fx;
                let bottom = c[channel] + (d[channel] - c[channel]) * fx;
                pixel[channel] = (top + (bottom - top) * fy).round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    if let Some(folder) = out.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        std::fs::create_dir_all(folder).expect("cannot create the output folder");
    }
    image::save_buffer(&out, &image, width as u32, height as u32, image::ColorType::Rgb8).expect("cannot write the picture");
    let failed = failed.into_inner();
    println!("wrote {} ({width} x {height}){}", out.display(), if failed > 0 { format!(", {failed} tiles missing (grey)") } else { String::new() });
    if failed > 0 {
        std::process::exit(1);
    }
}
