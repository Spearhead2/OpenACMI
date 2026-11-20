//! Looking after the tile cache on disk: how much it holds, and emptying it.
//!
//! Only files laid out the way [`crate::TileSource`] writes them count, and
//! only those are deleted: `<cache>/<z>/<x>/<y>.<ext>`, all three names
//! numbers. A cache folder is chosen by the user and may hold other things;
//! anything that does not look like a tile is left alone.

use std::fs;
use std::path::Path;

/// What a cache folder holds, or what was removed from it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheUsage {
    pub tiles: u64,
    pub bytes: u64,
}

impl std::ops::AddAssign for CacheUsage {
    fn add_assign(&mut self, other: Self) {
        self.tiles += other.tiles;
        self.bytes += other.bytes;
    }
}

/// Counts the tiles cached under `root`. A missing folder is an empty cache.
pub fn usage(root: &Path) -> CacheUsage {
    let mut total = CacheUsage::default();
    for_each_tile(root, false, &mut |_, bytes| total += CacheUsage { tiles: 1, bytes });
    total
}

/// Deletes the tiles cached under `root`, and the folders this leaves
/// empty. Returns what was removed; files that cannot be deleted (in use,
/// read-only) are skipped.
pub fn clear(root: &Path) -> CacheUsage {
    let mut removed = CacheUsage::default();
    for_each_tile(root, true, &mut |path, bytes| {
        if fs::remove_file(path).is_ok() {
            removed += CacheUsage { tiles: 1, bytes };
        }
    });
    removed
}

/// Whether a file or folder name is a number, as zoom levels, columns and
/// (before the extension) rows are.
fn is_number(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Folders directly under `folder` whose names are numbers.
fn numbered_folders(folder: &Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = fs::read_dir(folder) else { return Vec::new() };
    entries
        .flatten()
        .filter(|entry| is_number(&entry.file_name()) && entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect()
}

/// Calls `visit` with the path and size of each tile under `root`. With
/// `tidy`, folders found empty afterwards are removed.
fn for_each_tile(root: &Path, tidy: bool, visit: &mut dyn FnMut(&Path, u64)) {
    for zoom in numbered_folders(root) {
        for column in numbered_folders(&zoom) {
            let Ok(entries) = fs::read_dir(&column) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let is_tile = path.file_stem().is_some_and(is_number) && entry.file_type().is_ok_and(|kind| kind.is_file());
                if is_tile {
                    visit(&path, entry.metadata().map_or(0, |metadata| metadata.len()));
                }
            }
            if tidy {
                // Fails, as it should, when something is left inside.
                let _ = fs::remove_dir(&column);
            }
        }
        if tidy {
            let _ = fs::remove_dir(&zoom);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A fresh folder under the temp directory, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("openacmi-cache-test-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, relative: &str, bytes: usize) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, vec![0u8; bytes]).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn tiles_are_counted_and_cleared() {
        let cache = Scratch::new("clear");
        cache.write("5/17/11.png", 100);
        cache.write("5/17/12.png", 250);
        cache.write("12/2400/1650.jpg", 4000);
        // A download that was cut short leaves a temporary file behind.
        cache.write("12/2400/1651.tmp4242", 7);
        assert_eq!(usage(&cache.0), CacheUsage { tiles: 4, bytes: 4357 });

        assert_eq!(clear(&cache.0), CacheUsage { tiles: 4, bytes: 4357 });
        assert_eq!(usage(&cache.0), CacheUsage::default());
        // The emptied folders are gone; the cache folder itself stays.
        assert!(cache.0.is_dir() && fs::read_dir(&cache.0).unwrap().next().is_none());
    }

    #[test]
    fn anything_that_is_not_a_tile_is_left_alone() {
        let cache = Scratch::new("keep");
        cache.write("7/3/9.png", 10);
        cache.write("notes.txt", 5);
        cache.write("imagery/server/7/3/9.jpg", 20);
        cache.write("7/3/readme.md", 3);
        cache.write("7/photos/1.png", 8);
        cache.write("2024/holiday.png", 9);

        assert_eq!(usage(&cache.0), CacheUsage { tiles: 1, bytes: 10 });
        assert_eq!(clear(&cache.0), CacheUsage { tiles: 1, bytes: 10 });
        for kept in ["notes.txt", "imagery/server/7/3/9.jpg", "7/3/readme.md", "7/photos/1.png", "2024/holiday.png"] {
            assert!(cache.0.join(kept).is_file(), "{kept} was deleted");
        }
        assert!(!cache.0.join("7/3/9.png").exists());
    }

    #[test]
    fn a_missing_folder_is_an_empty_cache() {
        let nowhere = std::env::temp_dir().join("openacmi-cache-test-does-not-exist");
        assert_eq!(usage(&nowhere), CacheUsage::default());
        assert_eq!(clear(&nowhere), CacheUsage::default());
    }
}
