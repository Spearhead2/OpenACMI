//! Prints elevation statistics of a region, for tuning `acmi-terrain/src/depressions.rs`.
//!
//! `cargo run -p acmi-terrain --example probe -- <cache dir> <zoom> <west> <south> <east> <north>`

use acmi_terrain::{ElevationTile, TileKey, TileSource, DEFAULT_URL_TEMPLATE};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let number = |i: usize| args[i].parse::<f64>().expect("number");
    let (zoom, west, south, east, north) = (number(1) as u8, number(2), number(3), number(4), number(5));
    let source = TileSource::new(DEFAULT_URL_TEMPLATE, Some(args[0].clone().into()), true);

    let (first, ..) = TileKey::locate(zoom, north, west).expect("in range");
    let (last, ..) = TileKey::locate(zoom, south, east).expect("in range");
    let mut below = Vec::new();
    let mut total = 0usize;
    for y in first.y..=last.y {
        for x in first.x..=last.x {
            let key = TileKey::new(zoom, x, y);
            let tile = ElevationTile::decode_terrarium(&source.fetch(key).expect("fetch")).expect("decode");
            let size = tile.size();
            for row in 0..size {
                let latitude = key.latitude((row as f64 + 0.5) / size as f64);
                for column in 0..size {
                    let longitude = key.longitude((column as f64 + 0.5) / size as f64);
                    if latitude < south || latitude > north || longitude < west || longitude > east {
                        continue;
                    }
                    total += 1;
                    let height = tile.pixel(column as isize, row as isize);
                    if height <= 0.0 {
                        below.push(height);
                    }
                }
            }
        }
    }
    below.sort_by(f32::total_cmp);
    println!("{total} samples, {} at or below sea level", below.len());
    if below.is_empty() {
        return;
    }
    println!("lowest {:.1}, median {:.1}, highest {:.1}", below[0], below[below.len() / 2], below[below.len() - 1]);
    // Most common heights, in 2 m bins: a lake surface shows up as a spike.
    let mut bins = std::collections::BTreeMap::<i32, usize>::new();
    for height in &below {
        *bins.entry((height / 2.0).floor() as i32 * 2).or_default() += 1;
    }
    let mut common: Vec<_> = bins.into_iter().collect();
    common.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
    for (height, count) in common.iter().take(6) {
        println!("  {height:>6} m: {:.1}%", *count as f64 * 100.0 / below.len() as f64);
    }
}
