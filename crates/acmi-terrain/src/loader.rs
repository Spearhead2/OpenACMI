//! Background tile loading.
//!
//! The renderer decides each frame which tiles it would like and hands the
//! list to [`TileLoader::set_wanted`]. Worker threads fetch, decode and
//! shade them and the results come back through [`TileLoader::poll`].
//! Because the wanted list is *replaced* every frame, requests for tiles
//! that scrolled out of view before a worker got to them simply vanish.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

use crate::elevation::ElevationTile;
use crate::imagery::Imagery;
use crate::shading::{shade_image, shade_tile, ElevationPalette};
use crate::source::{FetchError, TileSource};
use crate::surface::SurfaceTile;
use crate::tile::TileKey;

/// A tile ready for the renderer.
#[derive(Debug)]
pub struct TileData {
    /// Height of the visible surface: the ground, or the top of the water
    /// (sea level, or a lake's own level). Never the sea floor.
    pub elevation: Arc<ElevationTile>,
    /// sRGB RGBA colour image, square: imagery, or colours derived from the
    /// elevation. Not necessarily the resolution of `elevation`.
    pub color: Vec<u8>,
}

pub type TileResult = (TileKey, Result<TileData, FetchError>);

#[derive(Default)]
struct Queue {
    /// Most important first.
    wanted: Vec<TileKey>,
    /// Tiles a worker has taken whose result the owner has not collected yet.
    /// A tile stays here while its result waits in the channel: otherwise the
    /// owner, not having seen it, would ask for it again.
    in_flight: HashSet<TileKey>,
}

struct Shared {
    source: TileSource,
    imagery: Option<Imagery>,
    palette: ElevationPalette,
    queue: Mutex<Queue>,
    wake: Condvar,
    stop: AtomicBool,
}

/// Owns the worker threads; dropping it stops them.
pub struct TileLoader {
    shared: Arc<Shared>,
    results: Receiver<TileResult>,
}

impl TileLoader {
    /// `source` serves the elevation. With `imagery`, tiles are coloured from
    /// it; without, or where it has nothing, from the elevation itself, in
    /// the colours of `palette`.
    pub fn new(source: TileSource, imagery: Option<Imagery>, palette: ElevationPalette, workers: usize) -> Self {
        let shared = Arc::new(Shared {
            source,
            imagery,
            palette,
            queue: Mutex::default(),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let (tx, results) = mpsc::channel();
        for index in 0..workers.max(1) {
            let (shared, tx) = (Arc::clone(&shared), tx.clone());
            std::thread::Builder::new()
                .name(format!("terrain-{index}"))
                .spawn(move || worker(&shared, &tx))
                .expect("failed to spawn terrain thread");
        }
        Self { shared, results }
    }

    pub fn source(&self) -> &TileSource {
        &self.shared.source
    }

    /// Replaces the list of tiles to load, most important first. Tiles
    /// already being worked on, or finished but not yet collected with
    /// [`Self::poll`], are not started again.
    pub fn set_wanted(&self, keys: &[TileKey]) {
        let mut queue = self.shared.queue.lock().unwrap();
        queue.wanted.clear();
        let Queue { wanted, in_flight } = &mut *queue;
        wanted.extend(keys.iter().filter(|key| !in_flight.contains(key)));
        if !queue.wanted.is_empty() {
            self.shared.wake.notify_all();
        }
    }

    /// Number of tiles queued, being loaded, or waiting to be collected.
    pub fn pending(&self) -> usize {
        let queue = self.shared.queue.lock().unwrap();
        queue.wanted.len() + queue.in_flight.len()
    }

    /// Next finished tile, if any. Never blocks.
    pub fn poll(&self) -> Option<TileResult> {
        let result = self.results.try_recv().ok()?;
        self.shared.queue.lock().unwrap().in_flight.remove(&result.0);
        Some(result)
    }
}

impl Drop for TileLoader {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.wake.notify_all();
    }
}

fn worker(shared: &Shared, results: &Sender<TileResult>) {
    loop {
        let key = {
            let mut queue = shared.queue.lock().unwrap();
            loop {
                if shared.stop.load(Ordering::Relaxed) {
                    return;
                }
                if !queue.wanted.is_empty() {
                    break;
                }
                queue = shared.wake.wait(queue).unwrap();
            }
            let key = queue.wanted.remove(0);
            queue.in_flight.insert(key);
            key
        };

        // The in-flight mark is cleared by `poll`, when the owner takes the result.
        if results.send((key, load(&shared.source, shared.imagery.as_ref(), shared.palette, key))).is_err() {
            return;
        }
    }
}

fn load(source: &TileSource, imagery: Option<&Imagery>, palette: ElevationPalette, key: TileKey) -> Result<TileData, FetchError> {
    let bytes = source.fetch(key)?;
    let surface = SurfaceTile::resolve(key, &ElevationTile::decode_terrarium(&bytes)?);
    let color = match imagery.map(|imagery| (imagery, imagery.tile(key))) {
        Some((imagery, Ok(mut image))) => {
            shade_image(key, &surface, &mut image, imagery.relief());
            image
        }
        // A failed download is worth another try; the whole tile is retried.
        Some((_, Err(err))) if err.is_transient() => return Err(err),
        // No imagery here (or offline and not cached): elevation colours.
        Some((_, Err(err))) => {
            log::debug!("no imagery for tile {key:?}: {err}");
            shade_tile(key, &surface, palette)
        }
        None => shade_tile(key, &surface, palette),
    };
    Ok(TileData { elevation: Arc::new(surface.heights), color })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elevation::encode_terrarium;
    use std::time::{Duration, Instant};

    #[test]
    fn loads_wanted_tiles_from_cache() {
        let dir = std::env::temp_dir().join(format!("openacmi-loader-test-{}", std::process::id()));
        let cached = TileKey::new(2, 1, 1);
        let missing = TileKey::new(2, 3, 3);
        let path = dir.join("2/1/1.png");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, encode_terrarium(2, &[-50.0, 10.0, 20.0, 30.0])).unwrap();

        let loader = TileLoader::new(
            TileSource::new("http://invalid.invalid/{z}/{x}/{y}.png", Some(dir.clone()), false),
            None,
            ElevationPalette::Natural,
            2,
        );
        loader.set_wanted(&[cached, missing]);

        let mut results = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while results.len() < 2 && Instant::now() < deadline {
            match loader.poll() {
                Some(result) => results.push(result),
                None => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        results.sort_by_key(|(key, _)| *key);
        assert_eq!(results.len(), 2);

        let (key, data) = &results[0];
        let data = data.as_ref().expect("cached tile loads");
        assert_eq!(*key, cached);
        assert_eq!(data.elevation.range(), (0.0, 30.0), "sea floor is replaced by the sea surface");
        assert_eq!(data.color.len(), 2 * 2 * 4);
        assert!(data.color[2] > data.color[0], "the below-sea-level sample is blue");

        assert!(matches!(&results[1], (key, Err(FetchError::Offline)) if *key == missing));
        assert_eq!(loader.pending(), 0);

        // A finished tile that has not been collected must not be loaded again,
        // however often it is asked for in the meantime.
        loader.set_wanted(&[cached]);
        let deadline = Instant::now() + Duration::from_secs(10);
        while loader.shared.queue.lock().unwrap().wanted.len() == 1 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        std::thread::sleep(Duration::from_millis(50));
        for _ in 0..20 {
            loader.set_wanted(&[cached]);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(loader.poll().is_some());
        std::thread::sleep(Duration::from_millis(50));
        assert!(loader.poll().is_none(), "the tile was loaded more than once");
        drop(loader);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn imagery_colours_tiles_and_elevation_fills_the_gaps() {
        let dir = std::env::temp_dir().join(format!("openacmi-loader-imagery-test-{}", std::process::id()));
        let (with_image, without_image) = (TileKey::new(2, 1, 1), TileKey::new(2, 2, 1));
        for key in [with_image, without_image] {
            let path = dir.join(format!("dem/{}/{}/{}.png", key.z, key.x, key.y));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, encode_terrarium(2, &[100.0; 4])).unwrap();
        }
        let path = dir.join("image/2/1/1.jpg");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let picture = image::RgbaImage::from_pixel(256, 256, image::Rgba([200, 30, 30, 255]));
        picture.save_with_format(&path, image::ImageFormat::Png).unwrap();

        let offline = |folder: &str| TileSource::new("http://invalid.invalid/{z}/{x}/{y}.jpg", Some(dir.join(folder)), false);
        let dem = TileSource::new("http://invalid.invalid/{z}/{x}/{y}.png", Some(dir.join("dem")), false);
        let loader = TileLoader::new(dem, Some(Imagery::new(offline("image"), 13, 0.0)), ElevationPalette::Natural, 1);
        loader.set_wanted(&[with_image, without_image]);

        let mut results = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while results.len() < 2 && Instant::now() < deadline {
            match loader.poll() {
                Some((key, data)) => results.push((key, data.expect("tile loads"))),
                None => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        results.sort_by_key(|(key, _)| *key);
        assert_eq!(results.len(), 2);
        // The picture, at its own resolution.
        assert_eq!(results[0].1.color.len(), 256 * 256 * 4);
        assert_eq!(&results[0].1.color[..4], &[200, 30, 30, 255]);
        // No picture cached for the other tile: green lowland from the elevation.
        let fallback = &results[1].1.color;
        assert_eq!(fallback.len(), 2 * 2 * 4);
        assert!(fallback[1] > fallback[0] && fallback[1] > fallback[2]);
        drop(loader);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
