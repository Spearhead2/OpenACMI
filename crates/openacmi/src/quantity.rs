//! Flight quantities that can be plotted over time.
//!
//! To add one: a variant of [`Quantity`], an entry in [`Quantity::ALL`], and
//! an arm in each of `label`, `dimension` and `sample`. [`PairQuantity`] is
//! the same for numbers that relate two objects, such as their distance.

use acmi_core::geo::{METERS_PER_FOOT, METERS_PER_NM, MPS_PER_KNOT};
use acmi_core::{derive, geo, known, Object, PropId, PropKind, Unit};
use serde::{Deserialize, Serialize};

use crate::config::Units;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quantity {
    AltitudeMsl,
    AltitudeAgl,
    GroundSpeed,
    Ias,
    Cas,
    Tas,
    Mach,
    VerticalSpeed,
    Heading,
    Pitch,
    Roll,
    Aoa,
    G,
    TurnRate,
}

/// What kind of number a quantity is, which decides its display unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Altitude,
    /// A range between objects.
    Distance,
    Speed,
    VerticalSpeed,
    Angle,
    /// A compass direction: plotted on a fixed 0–360 scale and wraps around.
    Bearing,
    AngleRate,
    G,
    Number,
}

impl Dimension {
    /// Factor from SI to the display unit, and that unit's name.
    pub fn display(self, units: Units) -> (f64, &'static str) {
        match (self, units) {
            (Self::Altitude, Units::Aviation) => (1.0 / METERS_PER_FOOT, "ft"),
            (Self::Altitude, Units::Metric) => (1.0, "m"),
            (Self::Distance, Units::Aviation) => (1.0 / METERS_PER_NM, "nm"),
            (Self::Distance, Units::Metric) => (0.001, "km"),
            (Self::Speed, Units::Aviation) => (1.0 / MPS_PER_KNOT, "kt"),
            (Self::Speed, Units::Metric) => (3.6, "km/h"),
            (Self::VerticalSpeed, Units::Aviation) => (60.0 / METERS_PER_FOOT, "ft/min"),
            (Self::VerticalSpeed, Units::Metric) => (1.0, "m/s"),
            (Self::Angle | Self::Bearing, _) => (1.0, "°"),
            (Self::AngleRate, _) => (1.0, "°/s"),
            (Self::G, _) => (1.0, "G"),
            (Self::Number, _) => (1.0, ""),
        }
    }
}

impl Quantity {
    pub const ALL: [Quantity; 14] = [
        Self::AltitudeMsl,
        Self::AltitudeAgl,
        Self::GroundSpeed,
        Self::Ias,
        Self::Cas,
        Self::Tas,
        Self::Mach,
        Self::VerticalSpeed,
        Self::Heading,
        Self::Pitch,
        Self::Roll,
        Self::Aoa,
        Self::G,
        Self::TurnRate,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::AltitudeMsl => "Altitude MSL",
            Self::AltitudeAgl => "Altitude AGL",
            Self::GroundSpeed => "Ground speed",
            Self::Ias => "IAS",
            Self::Cas => "CAS",
            Self::Tas => "TAS",
            Self::Mach => "Mach",
            Self::VerticalSpeed => "Vertical speed",
            Self::Heading => "Heading",
            Self::Pitch => "Pitch",
            Self::Roll => "Roll",
            Self::Aoa => "AOA",
            Self::G => "G",
            Self::TurnRate => "Turn rate",
        }
    }

    pub fn dimension(self) -> Dimension {
        match self {
            Self::AltitudeMsl | Self::AltitudeAgl => Dimension::Altitude,
            Self::GroundSpeed | Self::Ias | Self::Cas | Self::Tas => Dimension::Speed,
            Self::VerticalSpeed => Dimension::VerticalSpeed,
            Self::Heading => Dimension::Bearing,
            Self::Pitch | Self::Roll | Self::Aoa => Dimension::Angle,
            Self::TurnRate => Dimension::AngleRate,
            Self::G => Dimension::G,
            Self::Mach => Dimension::Number,
        }
    }

    /// The recorded property this quantity is read from, if it is not
    /// worked out from the flight path.
    fn recorded(self) -> Option<(PropId, Unit)> {
        Some(match self {
            Self::AltitudeAgl => (known::AGL, Unit::Meters),
            Self::Ias => (known::IAS, Unit::MetersPerSecond),
            Self::Cas => (known::CAS, Unit::MetersPerSecond),
            Self::Tas => (known::TAS, Unit::MetersPerSecond),
            Self::Mach => (known::MACH, Unit::None),
            Self::Aoa => (known::AOA, Unit::Degrees),
            Self::G => (known::VERTICAL_G_FORCE, Unit::G),
            _ => return None,
        })
    }

    /// Whether the recording itself carries this quantity for `object`.
    fn is_recorded(self, object: &Object) -> bool {
        self.recorded().is_some_and(|(prop, _)| object.number_series(prop).is_some_and(|series| !series.is_empty()))
    }

    /// How this quantity is estimated when the recording lacks it; `None`
    /// for quantities that are never estimated.
    pub fn derivation(self) -> Option<&'static str> {
        Some(match self {
            Self::Tas => "Estimated: speed along the flight path, assuming no wind (or from recorded Mach).",
            Self::Mach => "Estimated: true airspeed over the speed of sound in the standard atmosphere at this altitude.",
            Self::Cas => "Estimated: from Mach and standard-atmosphere pressure at this altitude (or recorded IAS).",
            Self::Ias => "Estimated: taken equal to calibrated airspeed.",
            Self::G => "Estimated: acceleration of the flight path plus gravity, along the airframe's vertical axis.",
            Self::Aoa => "Estimated: angle between the recorded nose direction and the flight path, assuming no wind.",
            _ => return None,
        })
    }

    /// Where `object`'s values for this quantity come from, if it has any.
    pub fn source(self, object: &Object) -> Option<Source> {
        if object.track.is_empty() {
            return None;
        }
        if self.recorded().is_none() || self.is_recorded(object) {
            return Some(Source::Recorded);
        }
        let derivable = match self {
            // The nose direction has to be in the recording.
            Self::Aoa => object.track.has_orientation(),
            _ => self.derivation().is_some(),
        };
        derivable.then_some(Source::Derived)
    }

    /// Value at `time` in SI units (metres, metres per second, degrees),
    /// with where it came from.
    pub fn read(self, object: &Object, time: f64) -> Option<Reading> {
        if !object.is_active_at(time) {
            return None;
        }
        let recorded = |quantity: Quantity| {
            let (prop, unit) = quantity.recorded()?;
            object.number_at(prop, PropKind::Number(unit), time).map(f64::from)
        };
        if self.is_recorded(object) {
            return recorded(self).map(Reading::recorded);
        }
        let track = &object.track;
        let altitude = || object.pose_at(time).map(|pose| pose.alt);
        // The best available true airspeed and Mach number: recorded where
        // the recording has either, else from the flight path.
        let tas = || {
            recorded(Self::Tas)
                .or_else(|| Some(derive::tas_from_mach(recorded(Self::Mach)?, altitude()?)))
                .or_else(|| track.kinematics(time).map(|k| k.speed))
        };
        let mach = || recorded(Self::Mach).or_else(|| Some(derive::mach_from_tas(tas()?, altitude()?)));
        let cas = || recorded(Self::Cas).or_else(|| recorded(Self::Ias)).or_else(|| Some(derive::cas_from_mach(mach()?, altitude()?)));
        let derived = match self {
            Self::Tas => tas(),
            Self::Mach => mach(),
            Self::Cas => cas(),
            Self::Ias => cas(),
            Self::G => derive::load_factor(track, time),
            Self::Aoa => derive::angle_of_attack(track, time),
            // Never estimated: read straight off the flight path.
            Self::AltitudeMsl => return altitude().map(Reading::recorded),
            Self::Heading => return object.pose_at(time).map(|pose| Reading::recorded((pose.heading as f64).rem_euclid(360.0))),
            Self::Pitch => return object.pose_at(time).map(|pose| Reading::recorded(pose.pitch as f64)),
            Self::Roll => return object.pose_at(time).map(|pose| Reading::recorded(pose.roll as f64)),
            Self::GroundSpeed => return track.kinematics(time).map(|k| Reading::recorded(k.speed)),
            Self::VerticalSpeed => return track.kinematics(time).map(|k| Reading::recorded(k.vertical_speed)),
            Self::TurnRate => return track.kinematics(time).map(|k| Reading::recorded(k.turn_rate)),
            Self::AltitudeAgl => None,
        };
        derived.map(|value| Reading { value, source: Source::Derived })
    }

    /// Value at `time` in SI units, whatever its source.
    pub fn sample(self, object: &Object, time: f64) -> Option<f64> {
        self.read(object, time).map(|reading| reading.value)
    }
}

/// Where a value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// In the recording, or read directly off the recorded flight path.
    Recorded,
    /// Estimated because the recording lacks it; see [`Quantity::derivation`].
    Derived,
}

/// A value and where it came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    pub value: f64,
    pub source: Source,
}

impl Reading {
    fn recorded(value: f64) -> Self {
        Self { value, source: Source::Recorded }
    }

    pub fn is_derived(&self) -> bool {
        self.source == Source::Derived
    }
}

/// A number relating two objects, plotted in the comparison graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairQuantity {
    Distance,
    ClosureRate,
    Bearing,
    AltitudeDelta,
}

/// Half the time over which the closure rate is measured.
const CLOSURE_HALF_WINDOW_S: f64 = 1.0;

impl PairQuantity {
    pub const ALL: [PairQuantity; 4] = [Self::Distance, Self::ClosureRate, Self::Bearing, Self::AltitudeDelta];

    pub fn label(self) -> &'static str {
        match self {
            Self::Distance => "Distance",
            Self::ClosureRate => "Closure rate",
            Self::Bearing => "Bearing to target",
            Self::AltitudeDelta => "Altitude delta",
        }
    }

    pub fn dimension(self) -> Dimension {
        match self {
            Self::Distance => Dimension::Distance,
            Self::ClosureRate => Dimension::Speed,
            Self::Bearing => Dimension::Bearing,
            Self::AltitudeDelta => Dimension::Altitude,
        }
    }

    /// Value at `time` in SI units, while both objects are in the world.
    /// Differences are `second` relative to `first`, as in the track bar: a
    /// positive altitude delta means the second object is higher.
    pub fn sample(self, first: &Object, second: &Object, time: f64) -> Option<f64> {
        if !first.is_active_at(time) || !second.is_active_at(time) {
            return None;
        }
        let (a, b) = (first.pose_at(time)?, second.pose_at(time)?);
        // Straight-line (slant) range.
        let range = |time: f64| Some(first.pose_at(time)?.world_position().distance(second.pose_at(time)?.world_position()));
        Some(match self {
            Self::Distance => range(time)?,
            // How fast the range is shrinking: positive while closing.
            Self::ClosureRate => {
                let both = |time: f64| first.is_active_at(time) && second.is_active_at(time);
                let (before, after) = (time - CLOSURE_HALF_WINDOW_S, time + CLOSURE_HALF_WINDOW_S);
                let (before, after) = (if both(before) { before } else { time }, if both(after) { after } else { time });
                if after <= before {
                    return None;
                }
                (range(before)? - range(after)?) / (after - before)
            }
            // True bearing from the first object to the second.
            Self::Bearing => geo::initial_bearing_deg(a.lat, a.lon, b.lat, b.lon),
            Self::AltitudeDelta => b.alt - a.alt,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_quantity_is_listed_once_with_a_label() {
        for (index, quantity) in Quantity::ALL.iter().enumerate() {
            assert!(!quantity.label().is_empty());
            assert!(!Quantity::ALL[..index].contains(quantity));
        }
    }

    #[test]
    fn display_units_follow_the_unit_system() {
        let (factor, unit) = Dimension::Altitude.display(Units::Aviation);
        assert_eq!(unit, "ft");
        assert!((3048.0 * factor - 10_000.0).abs() < 1e-6);
        assert_eq!(Dimension::Speed.display(Units::Metric), (3.6, "km/h"));
        let (factor, unit) = Dimension::Distance.display(Units::Aviation);
        assert_eq!(unit, "nm");
        assert!((18_520.0 * factor - 10.0).abs() < 1e-9);
        let (factor, unit) = Dimension::VerticalSpeed.display(Units::Aviation);
        assert_eq!(unit, "ft/min");
        assert!((5.08 * factor - 1000.0).abs() < 1e-6);
    }

    /// A recording of aircraft flying north at 5 km with the nose 3 degrees
    /// up, one per `(longitude, speed in m/s)`, with ids from 1.
    fn recording(aircraft: &[(f64, f64)]) -> acmi_core::Recording {
        let mut text = String::from("FileType=text/acmi/tacview
FileVersion=2.2
");
        for step in 0..=50 {
            let time = step as f64 * 0.4;
            text.push_str(&format!("#{time:.2}
"));
            for (index, (lon, speed)) in aircraft.iter().enumerate() {
                let lat = speed * time / 111_194.93;
                text.push_str(&format!("{:x},T={lon:.7}|{lat:.7}|5000|0|3|0,Type=Air+FixedWing
", index + 1));
            }
        }
        let mut recording = acmi_core::Recording::new();
        acmi_core::Parser::new().feed_text(&mut recording, &text, |_| {}).unwrap();
        recording
    }

    #[test]
    fn missing_flight_data_is_derived_and_says_so() {
        let recording = recording(&[(10.0, 200.0)]);
        let object = recording.object(recording.find(1).unwrap());
        for quantity in [Quantity::Tas, Quantity::Mach, Quantity::Cas, Quantity::Ias, Quantity::G, Quantity::Aoa] {
            assert_eq!(quantity.source(object), Some(Source::Derived), "{quantity:?}");
            assert!(quantity.read(object, 10.0).unwrap().is_derived(), "{quantity:?}");
            assert!(quantity.derivation().is_some());
        }
        let value = |quantity: Quantity| quantity.sample(object, 10.0).unwrap();
        assert!((value(Quantity::Tas) - 200.0).abs() < 1.0);
        assert!((value(Quantity::Mach) - 0.624).abs() < 0.005, "{}", value(Quantity::Mach));
        // Thinner air: calibrated is well below true airspeed.
        assert!((value(Quantity::Cas) - 156.5).abs() < 2.0, "{}", value(Quantity::Cas));
        assert_eq!(value(Quantity::Ias), value(Quantity::Cas));
        assert!((value(Quantity::G) - 1.0).abs() < 0.02);
        assert!((value(Quantity::Aoa) - 3.0).abs() < 0.1);

        // What comes straight off the flight path is not an estimate.
        for quantity in [Quantity::AltitudeMsl, Quantity::GroundSpeed, Quantity::Heading] {
            assert_eq!(quantity.source(object), Some(Source::Recorded));
            assert!(!quantity.read(object, 10.0).unwrap().is_derived());
        }
        // Nothing to go on for height above ground.
        assert_eq!(Quantity::AltitudeAgl.source(object), None);
    }

    #[test]
    fn pair_quantities_relate_two_objects() {
        // Side by side 0.1 degrees of longitude apart on the equator, the
        // second one faster: it pulls ahead to the north-east.
        let recording = recording(&[(10.0, 200.0), (10.1, 250.0)]);
        let (first, second) = (recording.object(recording.find(1).unwrap()), recording.object(recording.find(2).unwrap()));
        let value = |quantity: PairQuantity, time| quantity.sample(first, second, time).unwrap();
        assert!((value(PairQuantity::Distance, 0.0) - 11_128.0).abs() < 30.0, "{}", value(PairQuantity::Distance, 0.0));
        assert!((value(PairQuantity::Bearing, 0.0) - 90.0).abs() < 0.1);
        assert!(value(PairQuantity::Bearing, 10.0) < 90.0 && value(PairQuantity::Bearing, 10.0) > 80.0);
        assert_eq!(value(PairQuantity::AltitudeDelta, 5.0), 0.0);
        // Opening, so the closure rate is negative; at the very start it has
        // only the time after to measure over.
        assert!(value(PairQuantity::ClosureRate, 10.0) < -0.5);
        assert!(value(PairQuantity::ClosureRate, 0.0) <= 0.0);
        // Before either exists there is nothing to relate.
        assert_eq!(PairQuantity::Distance.sample(first, second, -5.0), None);
    }

    #[test]
    fn names_are_stable_in_the_settings_file() {
        #[derive(Serialize, Deserialize)]
        struct Holder {
            plot: Vec<Quantity>,
        }
        let text = toml::to_string(&Holder { plot: vec![Quantity::AltitudeMsl, Quantity::Ias] }).unwrap();
        assert_eq!(text.trim(), r#"plot = ["altitude_msl", "ias"]"#);
        assert_eq!(toml::from_str::<Holder>(&text).unwrap().plot, [Quantity::AltitudeMsl, Quantity::Ias]);
    }
}
