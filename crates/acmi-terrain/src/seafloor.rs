//! Fills in sea floor that an elevation tile is missing.
//!
//! The elevation set is assembled from several surveys, and they do not
//! agree about the sea: most tiles hold the sea floor, but some, at some
//! zoom levels, hold the flat water surface instead (exactly 0 m). Coloured
//! by depth, those come out as pale rectangles in the middle of deep water,
//! appearing and disappearing as the view changes level of detail.
//!
//! Where a tile has samples of exactly 0 m, the same spot is looked up in
//! coarser tiles, a few levels up at a time, until one of them knows the
//! depth. Land is not affected: a coarser tile only overrides a sample when
//! it says that sample is below sea level.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::elevation::ElevationTile;
use crate::tile::TileKey;

/// Zoom levels climbed per attempt: a tile four times as wide each way is
/// usually from a different source than the one with the gap.
const LEVELS_PER_STEP: u8 = 2;
/// Attempts before giving up and leaving the water shallow.
const MAX_STEPS: u8 = 3;
/// Coarse tiles kept decoded; each serves many of the tiles below it.
const CACHE_TILES: usize = 96;

/// Looks up missing sea floor in coarser tiles, remembering those it has
/// already fetched. Shared between the threads that load tiles.
#[derive(Default)]
pub struct SeaFloor {
    coarse: Mutex<HashMap<TileKey, Option<Arc<ElevationTile>>>>,
}

impl SeaFloor {
    /// Returns `heights` with its samples of exactly 0 m replaced by the
    /// depth a coarser tile gives for the same place, where one does.
    /// `fetch` gets the elevation of another tile, or `None` if it cannot.
    pub fn fill(&self, key: TileKey, heights: ElevationTile, fetch: &dyn Fn(TileKey) -> Option<ElevationTile>) -> ElevationTile {
        let size = heights.size();
        let mut open: Vec<usize> = heights.heights().iter().enumerate().filter(|(_, height)| **height == 0.0).map(|(i, _)| i).collect();
        if open.is_empty() {
            return heights;
        }
        let mut patched = heights.heights().to_vec();
        for step in 1..=MAX_STEPS {
            let up = step * LEVELS_PER_STEP;
            if key.z < up || open.is_empty() {
                break;
            }
            let ancestor = TileKey::new(key.z - up, key.x >> up, key.y >> up);
            let Some(tile) = self.coarse_tile(ancestor, fetch) else { continue };
            // Where this tile lies within its ancestor, in tile widths.
            let (span, mask) = ((1u32 << up) as f64, (1u32 << up) - 1);
            let (offset_x, offset_y) = ((key.x & mask) as f64, (key.y & mask) as f64);
            open.retain(|&index| {
                let (column, row) = ((index % size) as f64 + 0.5, (index / size) as f64 + 0.5);
                let coarse = tile.sample((offset_x + column / size as f64) / span, (offset_y + row / size as f64) / span);
                if coarse < 0.0 {
                    patched[index] = coarse;
                }
                // Still exactly at the surface there too: ask further up.
                coarse == 0.0
            });
        }
        ElevationTile::from_heights(size, patched)
    }

    fn coarse_tile(&self, key: TileKey, fetch: &dyn Fn(TileKey) -> Option<ElevationTile>) -> Option<Arc<ElevationTile>> {
        if let Some(known) = self.coarse.lock().unwrap().get(&key) {
            return known.clone();
        }
        // Fetched outside the lock: other threads keep working meanwhile.
        let tile = fetch(key).map(Arc::new);
        let mut coarse = self.coarse.lock().unwrap();
        if coarse.len() >= CACHE_TILES {
            coarse.clear();
        }
        coarse.insert(key, tile.clone());
        tile
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn tile(heights: [f32; 4]) -> ElevationTile {
        ElevationTile::from_heights(2, heights.to_vec())
    }

    #[test]
    fn flat_sea_takes_its_depth_from_a_coarser_tile() {
        // A tile of flat sea with one hill, under an ancestor two levels up
        // that is 1200 m deep everywhere.
        let key = TileKey::new(10, 600, 400);
        let fetched = Cell::new(0);
        let fetch = |wanted: TileKey| {
            fetched.set(fetched.get() + 1);
            assert_eq!(wanted, TileKey::new(8, 150, 100));
            Some(tile([-1200.0; 4]))
        };
        let sea_floor = SeaFloor::default();
        let filled = sea_floor.fill(key, tile([0.0, 0.0, 35.0, 0.0]), &fetch);
        assert_eq!(filled.heights(), [-1200.0, -1200.0, 35.0, -1200.0]);

        // The neighbouring tile shares the ancestor, which is not fetched again.
        sea_floor.fill(TileKey::new(10, 601, 400), tile([0.0; 4]), &fetch);
        assert_eq!(fetched.get(), 1);
    }

    #[test]
    fn tiles_without_gaps_cost_nothing() {
        let fetch = |_: TileKey| -> Option<ElevationTile> { panic!("nothing to look up") };
        let heights = tile([-300.0, -0.5, 12.0, 800.0]);
        assert_eq!(SeaFloor::default().fill(TileKey::new(9, 1, 1), heights.clone(), &fetch), heights);
    }

    #[test]
    fn the_search_climbs_until_a_tile_knows() {
        // Two levels up is flat as well; four levels up has the sea floor.
        let fetch = |wanted: TileKey| match wanted.z {
            6 => Some(tile([0.0; 4])),
            4 => Some(tile([-80.0; 4])),
            other => panic!("unexpected level {other}"),
        };
        let filled = SeaFloor::default().fill(TileKey::new(8, 130, 90), tile([0.0; 4]), &fetch);
        assert_eq!(filled.heights(), [-80.0; 4]);
    }

    #[test]
    fn land_at_sea_level_and_missing_tiles_are_left_alone() {
        // The coarser tile says land: the sample stays at 0 m.
        let land = |_: TileKey| Some(tile([40.0; 4]));
        assert_eq!(SeaFloor::default().fill(TileKey::new(8, 1, 1), tile([0.0; 4]), &land).heights(), [0.0; 4]);
        // Nothing can be fetched (offline): unchanged, and no panic.
        let offline = |_: TileKey| None;
        assert_eq!(SeaFloor::default().fill(TileKey::new(8, 1, 1), tile([0.0; 4]), &offline).heights(), [0.0; 4]);
        // Too close to the top of the pyramid to have such an ancestor.
        assert_eq!(SeaFloor::default().fill(TileKey::new(1, 0, 0), tile([0.0; 4]), &offline).heights(), [0.0; 4]);
    }
}
