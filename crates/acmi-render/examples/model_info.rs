//! Prints the triangle count and bounding box of model files.
//!
//! `cargo run -p acmi-render --example model_info -- path/to/Model.glb ...`

use acmi_render::mesh::load_model;

fn main() {
    for path in std::env::args().skip(1) {
        match load_model(std::path::Path::new(&path)) {
            Ok(mesh) => {
                let mut min = [f32::MAX; 3];
                let mut max = [f32::MIN; 3];
                for vertex in &mesh.vertices {
                    for axis in 0..3 {
                        min[axis] = min[axis].min(vertex.position[axis]);
                        max[axis] = max[axis].max(vertex.position[axis]);
                    }
                }
                println!(
                    "{path}: {} triangles, radius {:.2}, min {min:.2?}, max {max:.2?}",
                    mesh.vertices.len() / 3,
                    mesh.radius
                );
            }
            Err(err) => println!("{path}: {err}"),
        }
    }
}
