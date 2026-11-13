//! Rebuilds the shading normals of a model from its geometry.
//!
//! Model files come with normals of every quality: missing (flat shaded),
//! exported with the wrong smoothing, mangled by a format conversion, or not
//! unit length. Rather than trust them, every loaded model gets normals
//! computed the same way, so all models are lit consistently:
//!
//! * corners that share a position are treated as one point of the surface,
//!   whether or not the file shared the vertex;
//! * a corner's normal is the average of the faces around that point,
//!   weighted by each face's area and its angle at the point, which gives
//!   even shading on irregular triangulations;
//! * faces that meet at more than the crease angle are not averaged
//!   together, so wing edges, intakes and panel steps stay sharp while
//!   fuselages and canopies come out smooth.

use glam::Vec3;
use rustc_hash::FxHashMap;

use crate::mesh::{CpuMesh, MeshVertex};

/// Faces meeting at more than this angle keep a hard edge between them.
pub const DEFAULT_CREASE_ANGLE_DEG: f32 = 50.0;

/// Positions closer than this fraction of the model's size are one point.
const WELD_TOLERANCE: f32 = 1.0e-5;

/// One triangle's contribution to the points it touches.
struct Face {
    /// Unit normal by the triangle's winding; zero for a degenerate triangle.
    normal: Vec3,
    area: f32,
}

impl CpuMesh {
    /// Replaces every normal with one computed from the triangles; see the
    /// module documentation. `crease_angle_deg` of 0 gives flat shading and
    /// 180 smooths everything.
    pub fn regenerate_normals(&mut self, crease_angle_deg: f32) {
        let vertices = &mut self.vertices;
        let position = |vertex: &MeshVertex| Vec3::from_array(vertex.position);
        let faces: Vec<Face> = vertices
            .chunks_exact(3)
            .map(|triangle| {
                let cross = (position(&triangle[1]) - position(&triangle[0])).cross(position(&triangle[2]) - position(&triangle[0]));
                Face { normal: cross.normalize_or_zero(), area: cross.length() * 0.5 }
            })
            .collect();

        // Corners grouped by welded position.
        let extent = vertices.iter().map(|vertex| position(vertex).abs().max_element()).fold(0.0, f32::max);
        let cell = (extent * WELD_TOLERANCE).max(f32::MIN_POSITIVE);
        let key = |p: Vec3| ((p.x / cell).round() as i64, (p.y / cell).round() as i64, (p.z / cell).round() as i64);
        let mut points: FxHashMap<(i64, i64, i64), Vec<u32>> = FxHashMap::default();
        for (index, vertex) in vertices.iter().enumerate().take(faces.len() * 3) {
            points.entry(key(position(vertex))).or_default().push(index as u32);
        }

        // Angle of each triangle at each of its corners.
        let corner_angle = |index: usize| {
            let base = index - index % 3;
            let at = position(&vertices[index]);
            let (next, previous) = (position(&vertices[base + (index + 1) % 3]), position(&vertices[base + (index + 2) % 3]));
            (next - at).angle_between(previous - at)
        };
        let angles: Vec<f32> = (0..faces.len() * 3).map(corner_angle).map(|angle| if angle.is_finite() { angle } else { 0.0 }).collect();

        // A little slack so faces exactly at the crease angle are smoothed.
        let smooth_above = (crease_angle_deg.clamp(0.0, 180.0).to_radians()).cos() - 1.0e-4;
        let mut normals = vec![Vec3::ZERO; vertices.len()];
        for corners in points.values() {
            for &corner in corners {
                let own = &faces[corner as usize / 3];
                let sum: Vec3 = corners
                    .iter()
                    .map(|&other| (&faces[other as usize / 3], angles[other as usize]))
                    .filter(|(face, _)| face.normal.dot(own.normal) >= smooth_above)
                    .map(|(face, angle)| face.normal * (face.area * angle))
                    .sum();
                normals[corner as usize] = sum.try_normalize().unwrap_or(own.normal);
            }
        }

        for (vertex, normal) in vertices.iter_mut().zip(normals) {
            // A degenerate triangle has no direction of its own: keep
            // something unit length so shading stays finite.
            vertex.normal = if normal == Vec3::ZERO { Vec3::Y } else { normal }.to_array();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh(triangles: &[[Vec3; 3]]) -> CpuMesh {
        // Deliberately useless normals: regeneration must not depend on them.
        let vertices = triangles.iter().flatten().map(|p| MeshVertex { position: p.to_array(), normal: [9.0, 0.0, 0.0] }).collect();
        CpuMesh { vertices, radius: 1.0 }
    }

    /// A sphere of radius 1 as unshared triangles.
    fn sphere(segments: usize, rings: usize) -> CpuMesh {
        let point = |segment: usize, ring: usize| {
            let (theta, phi) = (std::f32::consts::TAU * segment as f32 / segments as f32, std::f32::consts::PI * ring as f32 / rings as f32);
            Vec3::new(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin())
        };
        let mut triangles = Vec::new();
        for ring in 0..rings {
            for segment in 0..segments {
                let (a, b, c, d) = (point(segment, ring), point(segment + 1, ring), point(segment, ring + 1), point(segment + 1, ring + 1));
                if ring > 0 {
                    triangles.push([a, b, c]);
                }
                if ring + 1 < rings {
                    triangles.push([b, d, c]);
                }
            }
        }
        mesh(&triangles)
    }

    fn cube() -> CpuMesh {
        let mut triangles = Vec::new();
        for axis in 0..3 {
            for sign in [-1.0f32, 1.0] {
                let corner = |u: f32, v: f32| {
                    let mut p = [0.0; 3];
                    p[axis] = sign;
                    p[(axis + 1) % 3] = u;
                    p[(axis + 2) % 3] = v;
                    Vec3::from_array(p)
                };
                let quad = [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)];
                // Wound to face outwards on both sides of the axis.
                let quad = if sign > 0.0 { quad } else { [quad[3], quad[2], quad[1], quad[0]] };
                triangles.push([quad[0], quad[1], quad[2]]);
                triangles.push([quad[0], quad[2], quad[3]]);
            }
        }
        mesh(&triangles)
    }

    #[test]
    fn curved_surfaces_come_out_smooth() {
        let mut sphere = sphere(16, 8);
        sphere.regenerate_normals(DEFAULT_CREASE_ANGLE_DEG);
        for vertex in &sphere.vertices {
            let (position, normal) = (Vec3::from_array(vertex.position), Vec3::from_array(vertex.normal));
            assert!((normal.length() - 1.0).abs() < 1e-4);
            // On a sphere the true normal is the position itself.
            assert!(normal.dot(position).abs() > 0.995, "{position:?} got {normal:?}");
        }
        // Corners at the same point agree, though the file did not share them.
        let at = |target: Vec3| sphere.vertices.iter().filter(move |v| Vec3::from_array(v.position).distance(target) < 1e-4);
        let first = at(Vec3::Y).next().unwrap().normal;
        assert!(at(Vec3::Y).count() >= 16 && at(Vec3::Y).all(|vertex| Vec3::from_array(vertex.normal).distance(Vec3::from_array(first)) < 1e-4));
    }

    #[test]
    fn sharp_edges_stay_sharp() {
        let mut cube = cube();
        cube.regenerate_normals(DEFAULT_CREASE_ANGLE_DEG);
        for triangle in cube.vertices.chunks_exact(3) {
            let [a, b, c] = [0, 1, 2].map(|i| Vec3::from_array(triangle[i].position));
            let flat = (b - a).cross(c - a).normalize();
            for vertex in triangle {
                assert!(Vec3::from_array(vertex.normal).distance(flat) < 1e-4, "a cube face is flat right up to its edges");
            }
        }
        // With no crease limit the corners round off instead.
        cube.regenerate_normals(180.0);
        let corner = cube.vertices.iter().find(|v| v.position == [1.0, 1.0, 1.0]).unwrap();
        assert!(Vec3::from_array(corner.normal).distance(Vec3::ONE.normalize()) < 0.02, "{:?}", corner.normal);
    }

    #[test]
    fn a_crease_of_zero_is_flat_shading() {
        let mut sphere = sphere(12, 6);
        sphere.regenerate_normals(0.0);
        for triangle in sphere.vertices.chunks_exact(3) {
            let [a, b, c] = [0, 1, 2].map(|i| Vec3::from_array(triangle[i].normal));
            assert!(a.distance(b) < 1e-5 && b.distance(c) < 1e-5, "{a:?} {b:?} {c:?}");
        }
    }

    #[test]
    fn degenerate_triangles_do_no_harm() {
        let sliver = [Vec3::ZERO, Vec3::X, Vec3::X * 2.0];
        let good = [Vec3::ZERO, Vec3::X, Vec3::Z];
        let mut mesh = mesh(&[sliver, good, [Vec3::ONE; 3]]);
        mesh.regenerate_normals(DEFAULT_CREASE_ANGLE_DEG);
        for vertex in &mesh.vertices {
            let normal = Vec3::from_array(vertex.normal);
            assert!(normal.is_finite() && (normal.length() - 1.0).abs() < 1e-4, "{normal:?}");
        }
        // The real triangle is untouched by its degenerate neighbour.
        assert_eq!(mesh.vertices[3].normal, [0.0, -1.0, 0.0]);
        CpuMesh::default().regenerate_normals(DEFAULT_CREASE_ANGLE_DEG);
    }
}
