//! Where the sun and the stars are.

use acmi_core::geo;
use glam::{Mat3, Vec3};

/// Unix time of 2000-01-01T00:00:00Z.
pub const Y2000: f64 = 946_684_800.0;

/// Greenwich mean sidereal time in degrees for a UTC instant given as Unix
/// seconds: how far the Earth has turned against the stars.
pub fn sidereal_angle(unix_seconds: f64) -> f64 {
    // Days since the epoch J2000.0, which is noon on 1 January 2000.
    let days = (unix_seconds - Y2000) / 86_400.0 - 0.5;
    (280.460_618_37 + 360.985_647_366_29 * days).rem_euclid(360.0)
}

/// Rotation from the equatorial frame of a star catalogue (x to the March
/// equinox, z to the celestial north pole) to world space at a UTC instant.
pub fn sky_rotation(unix_seconds: f64) -> Mat3 {
    // A star at right ascension `a` stands over longitude `a` minus the
    // sidereal angle, at the latitude of its declination.
    let turned = sidereal_angle(unix_seconds);
    let over = |latitude: f64, ascension: f64| geo::geo_to_world(latitude, ascension - turned, 0.0).normalize().as_vec3();
    Mat3::from_cols(over(0.0, 0.0), over(0.0, 90.0), over(90.0, 0.0))
}

/// Approximate sub-solar point `(latitude, longitude)` in degrees for a UTC
/// instant given as Unix seconds. Good to about a degree, which is plenty
/// for lighting a globe.
pub fn subsolar_point(unix_seconds: f64) -> (f64, f64) {
    const SECONDS_PER_DAY: f64 = 86_400.0;
    const TROPICAL_YEAR_DAYS: f64 = 365.2422;
    let days = (unix_seconds - Y2000) / SECONDS_PER_DAY;
    let day_of_year = days.rem_euclid(TROPICAL_YEAR_DAYS);
    let declination = -23.44 * (std::f64::consts::TAU * (day_of_year + 10.0) / TROPICAL_YEAR_DAYS).cos();

    let utc_hours = (unix_seconds / 3600.0).rem_euclid(24.0);
    let longitude = (12.0 - utc_hours) * 15.0;
    (declination, (longitude + 180.0).rem_euclid(360.0) - 180.0)
}

/// Unit vector from the Earth's centre towards the sun, in world space.
pub fn direction(latitude: f64, longitude: f64) -> Vec3 {
    geo::geo_to_world(latitude, longitude, 0.0).normalize().as_vec3()
}

#[cfg(test)]
mod tests {
    use super::*;
    use acmi_core::time::parse_iso8601;

    #[test]
    fn the_sky_turns_with_the_earth() {
        // The textbook value at the epoch J2000.0.
        let epoch = parse_iso8601("2000-01-01T12:00:00Z").unwrap();
        assert!((sidereal_angle(epoch) - 280.4606).abs() < 1e-3);
        // One sidereal day later the sky is back where it was.
        assert!((sidereal_angle(epoch + 86_164.090_5) - 280.4606).abs() < 1e-3);

        // The pole star stays over the pole; the frame stays right-handed.
        let rotation = sky_rotation(epoch);
        assert!((rotation * Vec3::Z - direction(90.0, 0.0)).length() < 1e-5);
        assert!((rotation.determinant().abs() - 1.0).abs() < 1e-5);

        // At the March equinox the sun is at right ascension 0: the stars
        // and the sun, worked out separately, must agree on where that is.
        let equinox = parse_iso8601("2024-03-20T03:06:00Z").unwrap();
        let (latitude, longitude) = subsolar_point(equinox);
        let towards_equinox = sky_rotation(equinox) * Vec3::X;
        assert!(towards_equinox.dot(direction(latitude, longitude)) > 3f32.to_radians().cos());
    }

    #[test]
    fn solstices_and_noon() {
        let (lat, lon) = subsolar_point(parse_iso8601("2024-06-21T12:00:00Z").unwrap());
        assert!((lat - 23.44).abs() < 0.5, "{lat}");
        assert!(lon.abs() < 0.01, "{lon}");

        let (lat, lon) = subsolar_point(parse_iso8601("2023-12-22T18:00:00Z").unwrap());
        assert!((lat + 23.44).abs() < 0.5, "{lat}");
        assert!((lon + 90.0).abs() < 0.01, "{lon}");
    }
}
