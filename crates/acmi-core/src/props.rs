//! Object property definitions.
//!
//! Every property name is interned to a small [`PropId`]. The properties of
//! the ACMI 2.2 specification get fixed, compile-time ids (see [`known`]) with
//! a declared type and unit; anything else found in a recording is registered
//! on first sight, so exporter-specific properties are kept rather than
//! dropped.
//!
//! Adding a specified property is a one-line change to the table below.

use rustc_hash::FxHashMap;

use self::PropKind::{Flag, Index, Number, Text};

/// Interned property name. Only meaningful together with its [`PropRegistry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PropId(pub(crate) u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    None,
    /// Normalised `0..=1`.
    Ratio,
    Meters,
    MetersPerSecond,
    G,
    Degrees,
}

/// How a property's values are stored and sampled over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
    Text,
    /// Continuous quantity: linearly interpolated between samples.
    Number(Unit),
    /// Boolean stored as 0/1: holds its last value.
    Flag,
    /// Discrete integer (modes, slots): holds its last value.
    Index,
}

macro_rules! known_props {
    ($($ident:ident = $name:literal => $kind:expr;)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u16)]
        enum Slot { $($ident),* }

        /// Ids of the properties defined by the ACMI specification.
        pub mod known {
            use super::{PropId, Slot};
            $(pub const $ident: PropId = PropId(Slot::$ident as u16);)*
        }

        const KNOWN: &[(&str, PropKind)] = &[$(($name, $kind)),*];
    };
}

const RATIO: PropKind = Number(Unit::Ratio);
const METERS: PropKind = Number(Unit::Meters);
const SPEED: PropKind = Number(Unit::MetersPerSecond);
const G: PropKind = Number(Unit::G);
const DEGREES: PropKind = Number(Unit::Degrees);
const PLAIN: PropKind = Number(Unit::None);

known_props! {
    // ── Text ────────────────────────────────────────────────────────────────
    NAME = "Name" => Text;
    TYPE = "Type" => Text;
    ADDITIONAL_TYPE = "AdditionalType" => Text;
    PARENT = "Parent" => Text;
    NEXT = "Next" => Text;
    SHORT_NAME = "ShortName" => Text;
    LONG_NAME = "LongName" => Text;
    FULL_NAME = "FullName" => Text;
    CALL_SIGN = "CallSign" => Text;
    REGISTRATION = "Registration" => Text;
    SQUAWK = "Squawk" => Text;
    ICAO24 = "ICAO24" => Text;
    PILOT = "Pilot" => Text;
    GROUP = "Group" => Text;
    COUNTRY = "Country" => Text;
    COALITION = "Coalition" => Text;
    COLOR = "Color" => Text;
    SHAPE = "Shape" => Text;
    DEBUG = "Debug" => Text;
    LABEL = "Label" => Text;
    FOCUSED_TARGET = "FocusedTarget" => Text;
    LOCKED_TARGET = "LockedTarget" => Text;
    LOCKED_TARGET2 = "LockedTarget2" => Text;
    LOCKED_TARGET3 = "LockedTarget3" => Text;
    LOCKED_TARGET4 = "LockedTarget4" => Text;
    LOCKED_TARGET5 = "LockedTarget5" => Text;
    LOCKED_TARGET6 = "LockedTarget6" => Text;
    LOCKED_TARGET7 = "LockedTarget7" => Text;
    LOCKED_TARGET8 = "LockedTarget8" => Text;
    LOCKED_TARGET9 = "LockedTarget9" => Text;
    // ── Ratios ──────────────────────────────────────────────────────────────
    IMPORTANCE = "Importance" => RATIO;
    VISIBLE = "Visible" => RATIO;
    HEALTH = "Health" => RATIO;
    NR = "NR" => RATIO;
    NR2 = "NR2" => RATIO;
    AIR_BRAKES = "AirBrakes" => RATIO;
    FLAPS = "Flaps" => RATIO;
    LANDING_GEAR = "LandingGear" => RATIO;
    LANDING_GEAR_HANDLE = "LandingGearHandle" => RATIO;
    TAILHOOK = "Tailhook" => RATIO;
    PARACHUTE = "Parachute" => RATIO;
    DRAG_CHUTE = "DragChute" => RATIO;
    ROLL_CONTROL_INPUT = "RollControlInput" => RATIO;
    PITCH_CONTROL_INPUT = "PitchControlInput" => RATIO;
    YAW_CONTROL_INPUT = "YawControlInput" => RATIO;
    ROLL_CONTROL_POSITION = "RollControlPosition" => RATIO;
    PITCH_CONTROL_POSITION = "PitchControlPosition" => RATIO;
    YAW_CONTROL_POSITION = "YawControlPosition" => RATIO;
    ROLL_TRIM_TAB = "RollTrimTab" => RATIO;
    PITCH_TRIM_TAB = "PitchTrimTab" => RATIO;
    YAW_TRIM_TAB = "YawTrimTab" => RATIO;
    AILERON_LEFT = "AileronLeft" => RATIO;
    AILERON_RIGHT = "AileronRight" => RATIO;
    ELEVATOR = "Elevator" => RATIO;
    RUDDER = "Rudder" => RATIO;
    ENL = "ENL" => RATIO;
    SPO2 = "SpO2" => RATIO;
    THROTTLE = "Throttle" => RATIO;
    THROTTLE2 = "Throttle2" => RATIO;
    AFTERBURNER = "Afterburner" => RATIO;
    // ── Metres ──────────────────────────────────────────────────────────────
    LENGTH = "Length" => METERS;
    WIDTH = "Width" => METERS;
    HEIGHT = "Height" => METERS;
    RADAR_RANGE = "RadarRange" => METERS;
    RADIUS = "Radius" => METERS;
    AGL = "AGL" => METERS;
    LOCKED_TARGET_RANGE = "LockedTargetRange" => METERS;
    ENGAGEMENT_RANGE = "EngagementRange" => METERS;
    ENGAGEMENT_RANGE2 = "EngagementRange2" => METERS;
    VERTICAL_ENGAGEMENT_RANGE = "VerticalEngagementRange" => METERS;
    VERTICAL_ENGAGEMENT_RANGE2 = "VerticalEngagementRange2" => METERS;
    LOCALIZER_LATERAL_DEVIATION = "LocalizerLateralDeviation" => METERS;
    GLIDESLOPE_VERTICAL_DEVIATION = "GlideslopeVerticalDeviation" => METERS;
    // ── Metres per second ───────────────────────────────────────────────────
    IAS = "IAS" => SPEED;
    CAS = "CAS" => SPEED;
    TAS = "TAS" => SPEED;
    WIND_SPEED = "WindSpeed" => SPEED;
    // ── G ───────────────────────────────────────────────────────────────────
    VERTICAL_G_FORCE = "VerticalGForce" => G;
    LONGITUDINAL_G_FORCE = "LongitudinalGForce" => G;
    LATERAL_G_FORCE = "LateralGForce" => G;
    // ── Degrees ─────────────────────────────────────────────────────────────
    AOA = "AOA" => DEGREES;
    AOS = "AOS" => DEGREES;
    HDG = "HDG" => DEGREES;
    HDM = "HDM" => DEGREES;
    RADAR_AZIMUTH = "RadarAzimuth" => DEGREES;
    RADAR_ELEVATION = "RadarElevation" => DEGREES;
    RADAR_ROLL = "RadarRoll" => DEGREES;
    RADAR_HORIZONTAL_BEAMWIDTH = "RadarHorizontalBeamwidth" => DEGREES;
    RADAR_VERTICAL_BEAMWIDTH = "RadarVerticalBeamwidth" => DEGREES;
    LOCKED_TARGET_AZIMUTH = "LockedTargetAzimuth" => DEGREES;
    LOCKED_TARGET_ELEVATION = "LockedTargetElevation" => DEGREES;
    LOCALIZER_ANGULAR_DEVIATION = "LocalizerAngularDeviation" => DEGREES;
    GLIDESLOPE_ANGULAR_DEVIATION = "GlideslopeAngularDeviation" => DEGREES;
    WIND_DIRECTION = "WindDirection" => DEGREES;
    WIND_PITCH = "WindPitch" => DEGREES;
    PILOT_HEAD_ROLL = "PilotHeadRoll" => DEGREES;
    PILOT_HEAD_PITCH = "PilotHeadPitch" => DEGREES;
    PILOT_HEAD_YAW = "PilotHeadYaw" => DEGREES;
    PILOT_EYE_GAZE_PITCH = "PilotEyeGazePitch" => DEGREES;
    PILOT_EYE_GAZE_YAW = "PilotEyeGazeYaw" => DEGREES;
    // ── Unitless / mixed ────────────────────────────────────────────────────
    MACH = "Mach" => PLAIN;
    ALTIMETER_SETTING = "AltimeterSetting" => PLAIN;
    QNH = "QNH" => PLAIN;
    ENGINE_RPM = "EngineRPM" => PLAIN;
    ENGINE_RPM2 = "EngineRPM2" => PLAIN;
    ROTOR_RPM = "RotorRPM" => PLAIN;
    ROTOR_RPM2 = "RotorRPM2" => PLAIN;
    FUEL_WEIGHT = "FuelWeight" => PLAIN;
    FUEL_VOLUME = "FuelVolume" => PLAIN;
    HEART_RATE = "HeartRate" => PLAIN;
    // ── Flags ───────────────────────────────────────────────────────────────
    DISABLED = "Disabled" => Flag;
    ON_GROUND = "OnGround" => Flag;
    TRIGGER_PRESSED = "TriggerPressed" => Flag;
    // ── Indices ─────────────────────────────────────────────────────────────
    SLOT = "Slot" => Index;
    RADAR_MODE = "RadarMode" => Index;
    LOCKED_TARGET_MODE = "LockedTargetMode" => Index;
    ENGAGEMENT_MODE = "EngagementMode" => Index;
    ENGAGEMENT_MODE2 = "EngagementMode2" => Index;
}

#[derive(Debug, Clone)]
pub struct PropDef {
    pub name: Box<str>,
    pub kind: PropKind,
}

/// Name ↔ id table for the properties of one recording.
#[derive(Debug, Clone)]
pub struct PropRegistry {
    defs: Vec<PropDef>,
    by_name: FxHashMap<Box<str>, PropId>,
}

impl Default for PropRegistry {
    fn default() -> Self {
        let mut registry = Self { defs: Vec::with_capacity(KNOWN.len() + 32), by_name: FxHashMap::default() };
        for &(name, kind) in KNOWN {
            registry.insert(name, kind);
        }
        registry
    }
}

impl PropRegistry {
    #[inline]
    pub fn get(&self, name: &str) -> Option<PropId> {
        self.by_name.get(name).copied()
    }

    #[inline]
    pub fn def(&self, id: PropId) -> &PropDef {
        &self.defs[id.0 as usize]
    }

    #[inline]
    pub fn name(&self, id: PropId) -> &str {
        &self.def(id).name
    }

    #[inline]
    pub fn kind(&self, id: PropId) -> PropKind {
        self.def(id).kind
    }

    pub fn iter(&self) -> impl Iterator<Item = (PropId, &PropDef)> {
        self.defs.iter().enumerate().map(|(i, def)| (PropId(i as u16), def))
    }

    /// Registers a property that is not part of the specification.
    ///
    /// The kind is inferred from the first value seen: anything that parses
    /// as a number becomes a unitless [`PropKind::Number`], the rest is text.
    pub(crate) fn intern(&mut self, name: &str, first_value: &str) -> PropId {
        if let Some(id) = self.get(name) {
            return id;
        }
        let kind = if first_value.trim().parse::<f64>().is_ok() { PLAIN } else { Text };
        self.insert(name, kind)
    }

    fn insert(&mut self, name: &str, kind: PropKind) -> PropId {
        let id = PropId(u16::try_from(self.defs.len()).expect("more than 65535 distinct property names"));
        self.defs.push(PropDef { name: name.into(), kind });
        self.by_name.insert(name.into(), id);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_ids_match_table() {
        let registry = PropRegistry::default();
        assert_eq!(registry.get("Name"), Some(known::NAME));
        assert_eq!(registry.get("IAS"), Some(known::IAS));
        assert_eq!(registry.name(known::ENGAGEMENT_MODE2), "EngagementMode2");
        assert_eq!(registry.kind(known::ON_GROUND), PropKind::Flag);
    }

    #[test]
    fn unknown_props_are_inferred() {
        let mut registry = PropRegistry::default();
        let fuel = registry.intern("FuelWeight3", "1250.5");
        let note = registry.intern("Remark", "bingo");
        assert_eq!(registry.kind(fuel), PropKind::Number(Unit::None));
        assert_eq!(registry.kind(note), PropKind::Text);
        assert_eq!(registry.intern("Remark", "42"), note);
    }
}
