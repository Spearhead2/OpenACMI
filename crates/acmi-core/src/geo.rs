//! Geodesy helpers and the world-space convention.
//!
//! World space is Earth-centred, in metres, Y-up, on a spherical Earth:
//!
//! ```text
//! X = r · cos(lat) · sin(lon)     (towards 90°E on the equator)
//! Y = r · sin(lat)                (towards the north pole)
//! Z = r · cos(lat) · cos(lon)     (towards 0°N 0°E)
//! ```
//!
//! Object models point their nose along local −Z with +Y up and +X to the
//! right, so a heading of 0° faces north.

use glam::{DMat3, DQuat, DVec3};

pub const EARTH_RADIUS_M: f64 = 6_371_000.0;
pub const METERS_PER_FOOT: f64 = 0.3048;
pub const METERS_PER_NM: f64 = 1852.0;
/// Metres per second in one knot.
pub const MPS_PER_KNOT: f64 = METERS_PER_NM / 3600.0;

/// Geodetic position to world space. `alt_m` is height above the sphere.
#[inline]
pub fn geo_to_world(lat_deg: f64, lon_deg: f64, alt_m: f64) -> DVec3 {
    let (sin_lat, cos_lat) = lat_deg.to_radians().sin_cos();
    let (sin_lon, cos_lon) = lon_deg.to_radians().sin_cos();
    let r = EARTH_RADIUS_M + alt_m;
    DVec3::new(r * cos_lat * sin_lon, r * sin_lat, r * cos_lat * cos_lon)
}

/// Inverse of [`geo_to_world`]: returns `(lat_deg, lon_deg, alt_m)`.
#[inline]
pub fn world_to_geo(p: DVec3) -> (f64, f64, f64) {
    let r = p.length();
    if r == 0.0 {
        return (0.0, 0.0, -EARTH_RADIUS_M);
    }
    let lat = (p.y / r).clamp(-1.0, 1.0).asin().to_degrees();
    let lon = p.x.atan2(p.z).to_degrees();
    (lat, lon, r - EARTH_RADIUS_M)
}

/// East/north/up unit vectors at a world position.
#[derive(Debug, Clone, Copy)]
pub struct LocalFrame {
    pub east: DVec3,
    pub north: DVec3,
    pub up: DVec3,
}

impl LocalFrame {
    pub fn at(world_pos: DVec3) -> Self {
        let up = world_pos.try_normalize().unwrap_or(DVec3::Y);
        // Near the poles the Y axis is degenerate as a reference.
        let reference = if up.dot(DVec3::Y).abs() < 0.999 { DVec3::Y } else { DVec3::NEG_Z };
        let east = reference.cross(up).normalize();
        let north = up.cross(east);
        Self { east, north, up }
    }

    /// Rotation taking model space (−Z forward, +Y up) to world space, facing north.
    pub fn rotation(&self) -> DQuat {
        DQuat::from_mat3(&DMat3::from_cols(self.east, self.up, -self.north))
    }
}

/// World-space orientation of an object from aeronautical angles (degrees).
///
/// Heading is clockwise from true north, pitch is positive nose-up and roll
/// is positive right-wing-down, matching the ACMI `T` field.
pub fn orientation(world_pos: DVec3, heading_deg: f64, pitch_deg: f64, roll_deg: f64) -> DQuat {
    LocalFrame::at(world_pos).rotation()
        * DQuat::from_rotation_y(-heading_deg.to_radians())
        * DQuat::from_rotation_x(pitch_deg.to_radians())
        * DQuat::from_rotation_z(-roll_deg.to_radians())
}

/// Great-circle distance in metres along the surface of the sphere.
pub fn great_circle_distance_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let a = (dp * 0.5).sin().powi(2) + p1.cos() * p2.cos() * (dl * 0.5).sin().powi(2);
    2.0 * EARTH_RADIUS_M * a.sqrt().min(1.0).asin()
}

/// Initial true bearing from point 1 to point 2, in degrees `[0, 360)`.
pub fn initial_bearing_deg(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dl = (lon2 - lon1).to_radians();
    let x = p2.cos() * dl.sin();
    let y = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
    x.atan2(y).to_degrees().rem_euclid(360.0)
}

/// Shortest signed difference `b − a` between two angles, in `[-180, 180)`.
#[inline]
pub fn angle_delta_deg(a: f64, b: f64) -> f64 {
    (b - a + 180.0).rem_euclid(360.0) - 180.0
}

/// Interpolates between two angles along the shortest arc; result in `[0, 360)`.
#[inline]
pub fn lerp_angle_deg(a: f64, b: f64, t: f64) -> f64 {
    (a + angle_delta_deg(a, b) * t).rem_euclid(360.0)
}

/// Distance along a ray (unit `dir`) to a sphere centred on the origin.
///
/// Returns the near hit when the origin is outside the sphere, the far hit
/// when inside, and `None` when the ray misses.
pub fn ray_sphere(origin: DVec3, dir: DVec3, radius: f64) -> Option<f64> {
    let b = origin.dot(dir);
    let c = origin.length_squared() - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    let (near, far) = (-b - sq, -b + sq);
    if near > 0.0 {
        Some(near)
    } else if far > 0.0 {
        Some(far)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geo_round_trip() {
        for &(lat, lon, alt) in &[(0.0, 0.0, 0.0), (45.5, -122.25, 9000.0), (-33.0, 151.0, 12.5), (89.0, 10.0, 1.0)] {
            let (la, lo, al) = world_to_geo(geo_to_world(lat, lon, alt));
            assert!((la - lat).abs() < 1e-9 && (lo - lon).abs() < 1e-9 && (al - alt).abs() < 1e-6);
        }
    }

    #[test]
    fn local_frame_axes() {
        let f = LocalFrame::at(geo_to_world(0.0, 0.0, 0.0));
        assert!(f.east.abs_diff_eq(DVec3::X, 1e-12));
        assert!(f.north.abs_diff_eq(DVec3::Y, 1e-12));
        assert!(f.up.abs_diff_eq(DVec3::Z, 1e-12));
    }

    #[test]
    fn orientation_conventions() {
        let pos = geo_to_world(0.0, 0.0, 0.0);
        let f = LocalFrame::at(pos);
        // Heading 90° points the nose east.
        let nose = orientation(pos, 90.0, 0.0, 0.0) * DVec3::NEG_Z;
        assert!(nose.abs_diff_eq(f.east, 1e-9));
        // Positive pitch raises the nose.
        let nose = orientation(pos, 0.0, 90.0, 0.0) * DVec3::NEG_Z;
        assert!(nose.abs_diff_eq(f.up, 1e-9));
        // Positive roll drops the right wing.
        let right_wing = orientation(pos, 0.0, 0.0, 90.0) * DVec3::X;
        assert!(right_wing.abs_diff_eq(-f.up, 1e-9));
    }

    #[test]
    fn bearing_and_distance() {
        assert!((initial_bearing_deg(0.0, 0.0, 1.0, 0.0) - 0.0).abs() < 1e-9);
        assert!((initial_bearing_deg(0.0, 0.0, 0.0, 1.0) - 90.0).abs() < 1e-9);
        let one_degree = great_circle_distance_m(0.0, 0.0, 0.0, 1.0);
        assert!((one_degree - EARTH_RADIUS_M.to_radians()).abs() < 1e-3);
    }

    #[test]
    fn angles_wrap() {
        assert!((lerp_angle_deg(350.0, 10.0, 0.5) - 0.0).abs() < 1e-9);
        assert!((angle_delta_deg(10.0, 350.0) + 20.0).abs() < 1e-9);
    }

    #[test]
    fn ray_hits_sphere() {
        let t = ray_sphere(DVec3::new(0.0, 0.0, 10.0), DVec3::NEG_Z, 1.0).unwrap();
        assert!((t - 9.0).abs() < 1e-12);
        assert!(ray_sphere(DVec3::new(0.0, 5.0, 10.0), DVec3::NEG_Z, 1.0).is_none());
    }
}
