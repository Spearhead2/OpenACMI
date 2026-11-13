//! Prints the mean colour of one tile at each stage of the imagery path.
//!
//! `cargo run -p acmi-terrain --example imagery_probe -- <dem cache> <imagery cache> <z> <x> <y>`

use acmi_terrain::shading::shade_image;
use acmi_terrain::{
    ElevationTile, Imagery, SurfaceTile, TileKey, TileSource, DEFAULT_IMAGERY_URL_TEMPLATE, DEFAULT_URL_TEMPLATE,
};

fn mean(pixels: &[u8]) -> [u32; 3] {
    let count = (pixels.len() / 4) as u32;
    let mut sum = [0u32; 3];
    for pixel in pixels.chunks_exact(4) {
        for (total, value) in sum.iter_mut().zip(pixel) {
            *total += *value as u32;
        }
    }
    sum.map(|total| total / count.max(1))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let number = |i: usize| args[i].parse::<u32>().expect("number");
    let key = TileKey::new(number(2) as u8, number(3), number(4));
    let imagery = |brightness, saturation, gamma| {
        Imagery::new(TileSource::new(DEFAULT_IMAGERY_URL_TEMPLATE, Some(args[1].clone().into()), true), 13, 0.5)
            .with_tone(brightness, saturation, gamma)
    };

    let raw = imagery(1.0, 1.0, 1.0).tile(key).expect("imagery");
    println!("as served      {:?}", mean(&raw));
    let mut toned = imagery(0.8, 0.8, 0.7).tile(key).expect("imagery");
    println!("toned          {:?}", mean(&toned));

    let dem = TileSource::new(DEFAULT_URL_TEMPLATE, Some(args[0].clone().into()), true);
    let surface = SurfaceTile::resolve(key, &ElevationTile::decode_terrarium(&dem.fetch(key).expect("dem")).expect("decode"));
    println!("heights        {:?}", surface.heights.range());
    shade_image(key, &surface, &mut toned, 0.5);
    println!("with relief    {:?}", mean(&toned));
}
