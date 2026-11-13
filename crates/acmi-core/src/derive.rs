//! Estimates of flight data a recording does not carry, worked out from
//! what it does: positions over time and, where present, attitude.
//!
//! Everything here is an approximation and should be shown as such:
//!
//! * the air is the International Standard Atmosphere and has no wind, so
//!   true airspeed is taken to be the speed along the flight path;
//! * velocity and acceleration come from a curve fitted to a few seconds of
//!   positions, which smooths over short, sharp manoeuvres;
//! * indicated airspeed is taken to equal calibrated airspeed.

use glam::DVec3;

use crate::geo::{self, LocalFrame};
use crate::object::Track;

/// Standard gravity, m/s².
pub const G0: f64 = 9.806_65;
/// Sea-level speed of sound in the standard atmosphere, m/s.
pub const SEA_LEVEL_SPEED_OF_SOUND: f64 = 340.294;

const SEA_LEVEL_TEMPERATURE_K: f64 = 288.15;
const SEA_LEVEL_PRESSURE_PA: f64 = 101_325.0;
const LAPSE_RATE_K_PER_M: f64 = 0.0065;
const TROPOPAUSE_M: f64 = 11_000.0;
const GAS_CONSTANT_AIR: f64 = 287.052_87;
const HEAT_RATIO: f64 = 1.4;

/// Positions within this many seconds either side of a moment are fitted to
/// find the velocity and acceleration at it.
pub const FIT_HALF_WINDOW_S: f64 = 2.0;
/// Fewest positions a fit is made from.
const FIT_MIN_SAMPLES: usize = 4;
/// Shortest stretch of time a fit is made from.
const FIT_MIN_SPAN_S: f64 = 1.0;

/// The standard atmosphere at one altitude.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Air {
    pub temperature_k: f64,
    pub pressure_pa: f64,
    /// m/s.
    pub speed_of_sound: f64,
}

/// The International Standard Atmosphere at `altitude_m` above mean sea
/// level: a steady lapse rate up to 11 km and constant temperature above.
pub fn standard_atmosphere(altitude_m: f64) -> Air {
    let low = altitude_m.min(TROPOPAUSE_M);
    let temperature_k = SEA_LEVEL_TEMPERATURE_K - LAPSE_RATE_K_PER_M * low;
    let exponent = G0 / (GAS_CONSTANT_AIR * LAPSE_RATE_K_PER_M);
    let mut pressure_pa = SEA_LEVEL_PRESSURE_PA * (temperature_k / SEA_LEVEL_TEMPERATURE_K).powf(exponent);
    if altitude_m > TROPOPAUSE_M {
        pressure_pa *= (-G0 * (altitude_m - TROPOPAUSE_M) / (GAS_CONSTANT_AIR * temperature_k)).exp();
    }
    Air { temperature_k, pressure_pa, speed_of_sound: (HEAT_RATIO * GAS_CONSTANT_AIR * temperature_k).sqrt() }
}

/// Mach number of a true airspeed (m/s) at an altitude.
pub fn mach_from_tas(tas: f64, altitude_m: f64) -> f64 {
    tas / standard_atmosphere(altitude_m).speed_of_sound
}

/// True airspeed (m/s) of a Mach number at an altitude.
pub fn tas_from_mach(mach: f64, altitude_m: f64) -> f64 {
    mach * standard_atmosphere(altitude_m).speed_of_sound
}

/// Pressure rise in a pitot tube over the static pressure, as a fraction of
/// that static pressure, at a Mach number.
fn impact_pressure_ratio(mach: f64) -> f64 {
    let m2 = mach * mach;
    if mach <= 1.0 {
        (1.0 + 0.2 * m2).powf(3.5) - 1.0
    } else {
        // Rayleigh's formula: there is a shock ahead of the tube.
        (1.2 * m2).powf(3.5) * (6.0 / (7.0 * m2 - 1.0)).powf(2.5) - 1.0
    }
}

/// Calibrated airspeed (m/s) of a Mach number at an altitude: the speed
/// that would cause the same pitot pressure at sea level.
pub fn cas_from_mach(mach: f64, altitude_m: f64) -> f64 {
    let impact = impact_pressure_ratio(mach.max(0.0)) * standard_atmosphere(altitude_m).pressure_pa / SEA_LEVEL_PRESSURE_PA;
    let subsonic = (5.0 * ((impact + 1.0).powf(2.0 / 7.0) - 1.0)).sqrt();
    if subsonic <= 1.0 {
        return subsonic * SEA_LEVEL_SPEED_OF_SOUND;
    }
    // Faster than sound at sea level: invert Rayleigh's formula by bisection.
    let (mut low, mut high) = (1.0, 10.0);
    for _ in 0..50 {
        let middle = 0.5 * (low + high);
        if impact_pressure_ratio(middle) < impact {
            low = middle;
        } else {
            high = middle;
        }
    }
    0.5 * (low + high) * SEA_LEVEL_SPEED_OF_SOUND
}

/// Velocity and acceleration at one moment, in world axes (m/s, m/s²).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    pub position: DVec3,
    pub velocity: DVec3,
    pub acceleration: DVec3,
}

/// Motion at `t`, from a parabola fitted by least squares to the positions
/// recorded within [`FIT_HALF_WINDOW_S`] of it. A fit over several samples
/// is far less sensitive to rounded timestamps and coordinates than
/// differences between neighbouring samples, which matters most for the
/// acceleration. `None` where the track is too sparse or `t` is outside it.
pub fn motion(track: &Track, t: f64) -> Option<Motion> {
    let times = track.times();
    let (first, last) = (*times.first()?, *times.last()?);
    if t < first || t > last {
        return None;
    }
    let start = times.partition_point(|&time| time < t - FIT_HALF_WINDOW_S);
    let end = times.partition_point(|&time| time <= t + FIT_HALF_WINDOW_S);
    if end - start < FIT_MIN_SAMPLES || times[end - 1] - times[start] < FIT_MIN_SPAN_S {
        return None;
    }
    // Relative to a nearby point: world coordinates are millions of metres.
    let origin = track.world_position(track.index_at(t));
    // Normal equations of p(τ) = c0 + c1·τ + c2·τ², one right-hand side per axis.
    let mut sums = [0.0f64; 5];
    let mut rhs = [DVec3::ZERO; 3];
    for (i, time) in times.iter().enumerate().take(end).skip(start) {
        let tau = time - t;
        let p = track.world_position(i) - origin;
        let mut power = 1.0;
        for (k, sum) in sums.iter_mut().enumerate() {
            *sum += power;
            if k < 3 {
                rhs[k] += p * power;
            }
            power *= tau;
        }
    }
    let matrix = glam::DMat3::from_cols(
        DVec3::new(sums[0], sums[1], sums[2]),
        DVec3::new(sums[1], sums[2], sums[3]),
        DVec3::new(sums[2], sums[3], sums[4]),
    );
    if matrix.determinant().abs() < 1e-9 {
        return None;
    }
    let inverse = matrix.inverse();
    // Row k of the inverse (it is symmetric) combines the right-hand sides.
    let coefficient = |k: usize| {
        let row = inverse.col(k);
        rhs[0] * row.x + rhs[1] * row.y + rhs[2] * row.z
    };
    Some(Motion { position: origin + coefficient(0), velocity: coefficient(1), acceleration: coefficient(2) * 2.0 })
}

/// World-space `(forward, up)` axes of the airframe at `t`, where the track
/// records its pitch and heading. A missing roll is taken as wings level.
fn body_axes(track: &Track, t: f64) -> Option<(DVec3, DVec3)> {
    let [_, pitch, yaw] = track.recorded_orientation(track.index_at(t));
    if pitch.is_nan() || yaw.is_nan() {
        return None;
    }
    let pose = track.sample(t)?;
    let rotation = geo::orientation(pose.world_position(), pose.heading as f64, pose.pitch as f64, pose.roll as f64);
    // Model space is −Z forward, +Y up.
    Some((rotation * DVec3::NEG_Z, rotation * DVec3::Y))
}

/// Load factor in G at `t`: what the pilot's G meter would read, 1 in level
/// flight. With recorded attitude this is the force along the airframe's
/// vertical axis (negative when pushed out of the seat); without it, the
/// size of the total force, which is the same thing in a coordinated turn.
pub fn load_factor(track: &Track, t: f64) -> Option<f64> {
    let motion = motion(track, t)?;
    // What the airframe must supply: the acceleration, less gravity's share.
    let force = motion.acceleration + LocalFrame::at(motion.position).up * G0;
    Some(match body_axes(track, t) {
        Some((_, up)) => force.dot(up) / G0,
        None => force.length() / G0,
    })
}

/// Angle of attack in degrees at `t`: how far the nose points above the
/// direction of flight, measured in the airframe's plane of symmetry.
/// Needs recorded attitude, and a speed fast enough to have a direction.
pub fn angle_of_attack(track: &Track, t: f64) -> Option<f64> {
    let velocity = motion(track, t)?.velocity;
    let (forward, up) = body_axes(track, t)?;
    if velocity.length() < 5.0 {
        return None;
    }
    Some((-velocity.dot(up)).atan2(velocity.dot(forward)).to_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::TransformUpdate;

    /// A track sampled every 0.4 s from `(latitude, longitude, altitude,
    /// [roll, pitch, yaw])` as functions of time.
    fn track(seconds: f64, at: impl Fn(f64) -> (f64, f64, f64, Option<[f32; 3]>)) -> Track {
        let mut track = Track::default();
        let mut time = 0.0;
        while time <= seconds {
            let (lat, lon, alt, attitude) = at(time);
            let [roll, pitch, yaw] = attitude.map_or([None; 3], |a| a.map(Some));
            track.push(time, &TransformUpdate { lon: Some(lon), lat: Some(lat), alt: Some(alt as f32), roll, pitch, yaw });
            time += 0.4;
        }
        track
    }

    const DEG_PER_M: f64 = 1.0 / 111_194.93;

    #[test]
    fn atmosphere_matches_the_standard_tables() {
        let sea = standard_atmosphere(0.0);
        assert!((sea.speed_of_sound - 340.29).abs() < 0.05, "{}", sea.speed_of_sound);
        let high = standard_atmosphere(11_000.0);
        assert!((high.temperature_k - 216.65).abs() < 0.01);
        assert!((high.pressure_pa - 22_632.0).abs() < 30.0, "{}", high.pressure_pa);
        assert!((high.speed_of_sound - 295.07).abs() < 0.05);
        let higher = standard_atmosphere(20_000.0);
        assert!((higher.pressure_pa - 5_474.9).abs() < 20.0, "{}", higher.pressure_pa);
        assert_eq!(higher.temperature_k, high.temperature_k);
    }

    #[test]
    fn airspeeds_convert() {
        // At sea level calibrated and true airspeed are the same thing.
        assert!((cas_from_mach(0.5, 0.0) - 0.5 * SEA_LEVEL_SPEED_OF_SOUND).abs() < 0.01);
        // Mach 0.8 at 10 km is about 286 knots calibrated.
        let cas_knots = cas_from_mach(0.8, 10_000.0) / geo::MPS_PER_KNOT;
        assert!((cas_knots - 286.0).abs() < 1.5, "{cas_knots}");
        // Supersonic at sea level goes through Rayleigh's formula both ways.
        assert!((cas_from_mach(1.5, 0.0) - 1.5 * SEA_LEVEL_SPEED_OF_SOUND).abs() < 0.01);
        assert!((mach_from_tas(tas_from_mach(1.2, 8000.0), 8000.0) - 1.2).abs() < 1e-12);
    }

    #[test]
    fn straight_and_level_is_one_g_with_no_angle_of_attack() {
        let flight = track(20.0, |t| (200.0 * t * DEG_PER_M, 10.0, 5000.0, Some([0.0, 0.0, 0.0])));
        let motion = motion(&flight, 10.0).unwrap();
        assert!((motion.velocity.length() - 200.0).abs() < 0.5, "{}", motion.velocity.length());
        assert!((load_factor(&flight, 10.0).unwrap() - 1.0).abs() < 0.02);
        assert!(angle_of_attack(&flight, 10.0).unwrap().abs() < 0.1);
        // Outside the recording there is nothing to estimate.
        assert_eq!(load_factor(&flight, 30.0), None);
    }

    #[test]
    fn nose_above_the_flight_path_is_angle_of_attack() {
        // Level flight north with the nose 6 degrees up.
        let flight = track(20.0, |t| (150.0 * t * DEG_PER_M, 10.0, 3000.0, Some([0.0, 6.0, 0.0])));
        assert!((angle_of_attack(&flight, 10.0).unwrap() - 6.0).abs() < 0.1);
        // Climbing at 6 degrees with the nose on the flight path: none.
        let climb = track(20.0, |t| (150.0 * t * DEG_PER_M, 10.0, 3000.0 + 150.0 * t * 6f64.to_radians().tan(), Some([0.0, 6.0, 0.0])));
        assert!(angle_of_attack(&climb, 10.0).unwrap().abs() < 0.2);
        // Without attitude there is no nose to measure from.
        let bare = track(20.0, |t| (150.0 * t * DEG_PER_M, 10.0, 3000.0, None));
        assert_eq!(angle_of_attack(&bare, 10.0), None);
    }

    #[test]
    fn a_level_turn_pulls_g() {
        // 150 m/s around a circle at 6 degrees per second: 1.6 G sideways,
        // 1.88 G in total. Near the equator, so degrees are the same both ways.
        let rate = 6f64.to_radians();
        let radius_deg = 150.0 / rate * DEG_PER_M;
        let bare = track(60.0, |t| (radius_deg * (rate * t).sin(), radius_deg * (rate * t).cos(), 4000.0, None));
        let total = load_factor(&bare, 30.0).unwrap();
        assert!((total - 1.88).abs() < 0.04, "{total}");

        // The same turn flown with the matching bank reads the same on the G meter.
        let bank = (150.0 * rate / G0).atan().to_degrees() as f32;
        let banked = track(60.0, |t| {
            // Starts heading north at the eastern point and turns left.
            let heading = (-(rate * t).to_degrees()).rem_euclid(360.0) as f32;
            (radius_deg * (rate * t).sin(), radius_deg * (rate * t).cos(), 4000.0, Some([-bank, 0.0, heading]))
        });
        let felt = load_factor(&banked, 30.0).unwrap();
        assert!((felt - 1.88).abs() < 0.04, "{felt}");
    }
}
