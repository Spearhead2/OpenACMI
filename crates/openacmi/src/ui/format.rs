//! Formatting of quantities in the user's unit system.

use acmi_core::geo::{METERS_PER_FOOT, METERS_PER_NM, MPS_PER_KNOT};
use acmi_core::Unit;

use crate::config::Units;

pub const NO_DATA: &str = "---";

pub fn altitude(meters: f64, units: Units) -> String {
    match units {
        Units::Aviation => format!("{:.0} ft", meters / METERS_PER_FOOT),
        Units::Metric => format!("{meters:.0} m"),
    }
}

pub fn speed(mps: f64, units: Units) -> String {
    match units {
        Units::Aviation => format!("{:.0} kt", mps / MPS_PER_KNOT),
        Units::Metric => format!("{:.0} km/h", mps * 3.6),
    }
}

pub fn vertical_speed(mps: f64, units: Units) -> String {
    match units {
        Units::Aviation => format!("{:+.0} ft/min", mps / METERS_PER_FOOT * 60.0),
        Units::Metric => format!("{mps:+.1} m/s"),
    }
}

pub fn distance(meters: f64, units: Units) -> String {
    match units {
        Units::Aviation => format!("{:.1} nm", meters / METERS_PER_NM),
        Units::Metric => format!("{:.1} km", meters / 1000.0),
    }
}

pub fn heading(degrees: f64) -> String {
    format!("{:05.1}°", degrees.rem_euclid(360.0))
}

pub fn latitude(degrees: f64) -> String {
    format!("{:.5}° {}", degrees.abs(), if degrees >= 0.0 { 'N' } else { 'S' })
}

pub fn longitude(degrees: f64) -> String {
    format!("{:.5}° {}", degrees.abs(), if degrees >= 0.0 { 'E' } else { 'W' })
}

/// Formats a recorded property value according to its declared unit.
pub fn property(value: f32, unit: Unit, units: Units) -> String {
    let value = value as f64;
    match unit {
        Unit::Ratio => format!("{:.0} %", value * 100.0),
        Unit::Meters => altitude(value, units),
        Unit::MetersPerSecond => speed(value, units),
        Unit::G => format!("{value:.2} G"),
        Unit::Degrees => format!("{value:.1}°"),
        Unit::None => {
            if value.fract() == 0.0 && value.abs() < 1e9 {
                format!("{value:.0}")
            } else {
                format!("{value:.3}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_systems() {
        assert_eq!(altitude(3048.0, Units::Aviation), "10000 ft");
        assert_eq!(altitude(3048.0, Units::Metric), "3048 m");
        assert_eq!(speed(MPS_PER_KNOT * 250.0, Units::Aviation), "250 kt");
        assert_eq!(distance(18_520.0, Units::Aviation), "10.0 nm");
        assert_eq!(heading(-90.0), "270.0°");
        assert_eq!(latitude(-12.5), "12.50000° S");
        assert_eq!(property(0.5, Unit::Ratio, Units::Metric), "50 %");
    }
}
