//! Object type tags (`Type=Air+FixedWing`) and the coarse display category.

use std::fmt;

/// Bit set of the type tags defined by the ACMI specification.
///
/// Tags this crate does not know are ignored here; the raw `Type` string is
/// still available on the object.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TagSet(u64);

macro_rules! tags {
    ($($ident:ident = $name:literal),* $(,)?) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u8)]
        enum Bit { $($ident),* }

        impl TagSet {
            $(pub const $ident: TagSet = TagSet(1 << Bit::$ident as u64);)*

            const NAMES: &'static [(&'static str, TagSet)] = &[$(($name, TagSet::$ident)),*];
        }
    };
}

tags! {
    // Class
    AIR = "Air", GROUND = "Ground", SEA = "Sea", WEAPON = "Weapon", SENSOR = "Sensor",
    NAVAID = "Navaid", MISC = "Misc",
    // Attributes
    STATIC = "Static", HEAVY = "Heavy", MEDIUM = "Medium", LIGHT = "Light", MINOR = "Minor",
    // Basic types
    FIXED_WING = "FixedWing", ROTORCRAFT = "Rotorcraft", ARMOR = "Armor",
    ANTI_AIRCRAFT = "AntiAircraft", VEHICLE = "Vehicle", WATERCRAFT = "Watercraft",
    HUMAN = "Human", BIOLOGIC = "Biologic", MISSILE = "Missile", ROCKET = "Rocket",
    BOMB = "Bomb", TORPEDO = "Torpedo", PROJECTILE = "Projectile", BEAM = "Beam",
    DECOY = "Decoy", BUILDING = "Building", BULLSEYE = "Bullseye", WAYPOINT = "Waypoint",
    // Specific types
    TANK = "Tank", WARSHIP = "Warship", AIRCRAFT_CARRIER = "AircraftCarrier",
    SUBMARINE = "Submarine", INFANTRY = "Infantry", PARACHUTIST = "Parachutist",
    SHELL = "Shell", BULLET = "Bullet", GRENADE = "Grenade", FLARE = "Flare", CHAFF = "Chaff",
    SMOKE_GRENADE = "SmokeGrenade", AERODROME = "Aerodrome", CONTAINER = "Container",
    SHRAPNEL = "Shrapnel", EXPLOSION = "Explosion",
}

impl TagSet {
    pub const EMPTY: TagSet = TagSet(0);

    /// Parses a `+`-separated tag list such as `Air+FixedWing`.
    pub fn parse(type_str: &str) -> Self {
        let mut set = TagSet::EMPTY;
        for tag in type_str.split('+') {
            let tag = tag.trim();
            if let Some(&(_, bit)) = Self::NAMES.iter().find(|(name, _)| *name == tag) {
                set.0 |= bit.0;
            }
        }
        set
    }

    #[inline]
    pub const fn contains(self, other: TagSet) -> bool {
        self.0 & other.0 == other.0
    }

    #[inline]
    pub const fn intersects(self, other: TagSet) -> bool {
        self.0 & other.0 != 0
    }

    #[inline]
    pub const fn union(self, other: TagSet) -> TagSet {
        TagSet(self.0 | other.0)
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Names of the tags in the set, in specification order.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Self::NAMES.iter().filter(move |(_, bit)| self.contains(*bit)).map(|(name, _)| *name)
    }
}

impl fmt::Debug for TagSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.names()).finish()
    }
}

/// Coarse grouping used for visibility toggles, lists and default models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum Category {
    Aircraft,
    Weapon,
    Ground,
    Ship,
    Navaid,
    Static,
    #[default]
    Misc,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::Aircraft,
        Category::Weapon,
        Category::Ground,
        Category::Ship,
        Category::Navaid,
        Category::Static,
        Category::Misc,
    ];
    pub const COUNT: usize = Self::ALL.len();

    pub fn from_tags(tags: TagSet) -> Self {
        const FIXED: TagSet = TagSet::STATIC.union(TagSet::BUILDING).union(TagSet::AERODROME);
        if tags.contains(TagSet::AIR) {
            Category::Aircraft
        } else if tags.intersects(TagSet::WEAPON.union(TagSet::PROJECTILE)) {
            Category::Weapon
        } else if tags.contains(TagSet::SEA) {
            Category::Ship
        } else if tags.contains(TagSet::NAVAID) {
            Category::Navaid
        } else if tags.intersects(FIXED) {
            Category::Static
        } else if tags.contains(TagSet::GROUND) {
            Category::Ground
        } else {
            Category::Misc
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Category::Aircraft => "Aircraft",
            Category::Weapon => "Weapons",
            Category::Ground => "Ground Vehicles",
            Category::Ship => "Ships",
            Category::Navaid => "Navaids",
            Category::Static => "Static Objects",
            Category::Misc => "Other",
        }
    }

    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_categorises() {
        let tags = TagSet::parse("Air+FixedWing");
        assert!(tags.contains(TagSet::AIR) && tags.contains(TagSet::FIXED_WING));
        assert_eq!(Category::from_tags(tags), Category::Aircraft);

        assert_eq!(Category::from_tags(TagSet::parse("Weapon+Missile")), Category::Weapon);
        assert_eq!(Category::from_tags(TagSet::parse("Projectile+Shell")), Category::Weapon);
        assert_eq!(Category::from_tags(TagSet::parse("Sea+Watercraft+Warship")), Category::Ship);
        assert_eq!(Category::from_tags(TagSet::parse("Ground+Static+Building")), Category::Static);
        assert_eq!(Category::from_tags(TagSet::parse("Ground+Heavy+Armor+Vehicle+Tank")), Category::Ground);
        assert_eq!(Category::from_tags(TagSet::parse("Navaid+Static+Bullseye")), Category::Navaid);
        assert_eq!(Category::from_tags(TagSet::parse("Misc+Decoy+Flare")), Category::Misc);
        assert_eq!(Category::from_tags(TagSet::parse("SomethingNew")), Category::Misc);
    }

    #[test]
    fn debug_lists_names() {
        assert_eq!(format!("{:?}", TagSet::parse("Weapon+Missile")), r#"{"Weapon", "Missile"}"#);
    }
}
