//! Where the sun is.

use acmi_core::geo;
use glam::Vec3;

/// Approximate sub-solar point `(latitude, longitude)` in degrees for a UTC
/// instant given as Unix seconds. Good to about a degree, which is plenty
/// for lighting a globe.
pub fn subsolar_point(unix_seconds: f64) -> (f64, f64) {
    const SECONDS_PER_DAY: f64 = 86_400.0;
    const TROPICAL_YEAR_DAYS: f64 = 365.2422;
    /// Unix time of 2000-01-01T00:00:00Z.
    const Y2000: f64 = 946_684_800.0;

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
    fn solstices_and_noon() {
        let (lat, lon) = subsolar_point(parse_iso8601("2024-06-21T12:00:00Z").unwrap());
        assert!((lat - 23.44).abs() < 0.5, "{lat}");
        assert!(lon.abs() < 0.01, "{lon}");

        let (lat, lon) = subsolar_point(parse_iso8601("2023-12-22T18:00:00Z").unwrap());
        assert!((lat + 23.44).abs() < 0.5, "{lat}");
        assert!((lon + 90.0).abs() < 0.01, "{lon}");
    }
}
