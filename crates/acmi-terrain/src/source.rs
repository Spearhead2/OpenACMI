//! Where tile images come from: a disk cache backed by an HTTP server.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::tile::TileKey;

/// Mapzen "Terrain Tiles" on AWS Open Data: global, free, no key required.
/// Elevation data © Mapzen and its sources (SRTM, GMTED, NED, ETOPO1 and others).
pub const DEFAULT_URL_TEMPLATE: &str = "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png";

/// Tiles are a few tens of kilobytes; anything much larger is not a tile.
const MAX_TILE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("not cached and downloads are disabled")]
    Offline,
    /// The server has no such tile; retrying will not help.
    #[error("the server has no tile here")]
    NotFound,
    #[error("download failed: {0}")]
    Http(String),
    #[error("cache error: {0}")]
    Io(#[from] io::Error),
    #[error("cannot decode tile: {0}")]
    Decode(#[from] image::ImageError),
}

impl FetchError {
    /// Whether asking again later could succeed.
    pub fn is_transient(&self) -> bool {
        matches!(self, FetchError::Http(_) | FetchError::Io(_))
    }
}

/// Fetches tile images, keeping every downloaded tile on disk.
///
/// The cache is laid out `<cache>/<z>/<x>/<y>.<ext>`, with the extension of
/// the URL template, and holds the files exactly as served, so it can be pre-seeded, shared or inspected
/// with ordinary tools. Nothing is ever evicted.
pub struct TileSource {
    url_template: String,
    cache_dir: Option<PathBuf>,
    /// File extension of cached tiles, taken from the URL template.
    extension: String,
    /// `None` when downloads are disabled.
    agent: Option<ureq::Agent>,
}

impl TileSource {
    /// `url_template` contains `{z}`, `{x}` and `{y}` placeholders.
    /// With `online` false only the cache is used.
    pub fn new(url_template: impl Into<String>, cache_dir: Option<PathBuf>, online: bool) -> Self {
        let agent = online.then(|| {
            let config = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(20)))
                // Generic client name: requests carry nothing about the user.
                .user_agent("OpenACMI")
                .build();
            ureq::Agent::new_with_config(config)
        });
        let url_template = url_template.into();
        let extension = match url_template.split(['?', '#']).next().and_then(|path| path.rsplit_once('.')) {
            Some((_, ext)) if (1..=4).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric()) => {
                ext.to_ascii_lowercase()
            }
            _ => "png".to_owned(),
        };
        Self { url_template, cache_dir, extension, agent }
    }

    pub fn cache_dir(&self) -> Option<&Path> {
        self.cache_dir.as_deref()
    }

    pub fn url(&self, key: TileKey) -> String {
        self.url_template.replace("{z}", &key.z.to_string()).replace("{x}", &key.x.to_string()).replace("{y}", &key.y.to_string())
    }

    fn cache_path(&self, key: TileKey) -> Option<PathBuf> {
        Some(
            self.cache_dir
                .as_ref()?
                .join(key.z.to_string())
                .join(key.x.to_string())
                .join(format!("{}.{}", key.y, self.extension)),
        )
    }

    /// Returns the encoded image of a tile, from the cache if possible.
    pub fn fetch(&self, key: TileKey) -> Result<Vec<u8>, FetchError> {
        let cache_path = self.cache_path(key);
        if let Some(path) = &cache_path {
            match fs::read(path) {
                Ok(bytes) if !bytes.is_empty() => return Ok(bytes),
                Ok(_) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => log::warn!("cannot read cached tile {}: {err}", path.display()),
            }
        }

        let agent = self.agent.as_ref().ok_or(FetchError::Offline)?;
        let mut response = match agent.get(self.url(key)).call() {
            Ok(response) => response,
            Err(ureq::Error::StatusCode(403 | 404)) => return Err(FetchError::NotFound),
            Err(err) => return Err(FetchError::Http(err.to_string())),
        };
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_TILE_BYTES)
            .read_to_vec()
            .map_err(|err| FetchError::Http(err.to_string()))?;

        if let Some(path) = &cache_path {
            // A failed cache write costs a re-download later, nothing more.
            if let Err(err) = write_atomically(path, &bytes) {
                log::warn!("cannot cache tile {}: {err}", path.display());
            }
        }
        Ok(bytes)
    }
}

/// Writes via a temporary file so a crash never leaves a truncated tile behind.
fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_template_expands() {
        let source = TileSource::new(DEFAULT_URL_TEMPLATE, None, false);
        assert_eq!(
            source.url(TileKey::new(10, 184, 402)),
            "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/10/184/402.png"
        );
    }

    #[test]
    fn offline_source_serves_only_the_cache() {
        let dir = std::env::temp_dir().join(format!("openacmi-tile-cache-test-{}", std::process::id()));
        let source = TileSource::new("http://invalid.invalid/{z}/{x}/{y}.png", Some(dir.clone()), false);
        let key = TileKey::new(3, 1, 2);

        assert!(matches!(source.fetch(key), Err(FetchError::Offline)));
        write_atomically(&source.cache_path(key).unwrap(), b"tile bytes").unwrap();
        assert_eq!(source.fetch(key).unwrap(), b"tile bytes");
        assert!(dir.join("3/1/2.png").is_file());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cache_files_keep_the_extension_of_the_template() {
        let path = |template: &str| TileSource::new(template, Some(PathBuf::from("c")), false).cache_path(TileKey::new(3, 1, 2));
        assert_eq!(path("https://host/{z}/{y}/{x}.jpg"), Some(PathBuf::from("c").join("3").join("1").join("2.jpg")));
        assert_eq!(path("https://host/{z}/{x}/{y}.JPEG?key=a.b"), Some(PathBuf::from("c").join("3").join("1").join("2.jpeg")));
        assert_eq!(path("https://host.example/tiles/{z}/{y}/{x}"), Some(PathBuf::from("c").join("3").join("1").join("2.png")));
    }
}
