//! ACMI colour schemes: how an object's `Color` property maps to the colours
//! of its model, trail and label.
//!
//! Schemes come from `acmi_colors.json` (bundled, then the user's copy,
//! which may override or add entries).

use std::path::Path;

use acmi_core::{known, Object, TagSet};
use rustc_hash::FxHashMap;
use serde::Deserialize;

use crate::config::Rgba;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct ObjectColors {
    #[serde(rename = "display_color")]
    pub display: Rgba,
    #[serde(rename = "trail_color_main")]
    pub trail: Rgba,
    #[serde(rename = "trail_color_tails")]
    pub trail_tails: Rgba,
    pub label_text: Rgba,
    pub label_border: Rgba,
    pub label_line: Rgba,
    pub label_background: Rgba,
}

/// Neutral scheme for objects with no or an unknown `Color`.
const SLATE: ObjectColors = ObjectColors {
    display: Rgba::rgb(110, 114, 122),
    trail: Rgba::rgb(110, 114, 122),
    trail_tails: Rgba::rgb(110, 114, 122),
    label_text: Rgba::rgb(0, 0, 0),
    label_border: Rgba::rgb(0, 0, 0),
    label_line: Rgba::rgb(255, 255, 255),
    label_background: Rgba::rgb(255, 255, 255),
};

/// Scheme for shells and countermeasures, whatever their coalition.
const GOLD: ObjectColors = ObjectColors {
    display: Rgba::rgb(219, 179, 0),
    trail: Rgba::rgb(219, 179, 0),
    trail_tails: Rgba::rgb(219, 179, 0),
    label_text: Rgba::rgb(219, 179, 0),
    label_border: Rgba::rgb(255, 208, 0),
    label_line: Rgba::rgb(219, 179, 0),
    label_background: Rgba::rgb(255, 255, 255),
};

#[derive(Debug, Default)]
pub struct Palette {
    schemes: FxHashMap<String, ObjectColors>,
}

impl Palette {
    /// Merges the schemes of a JSON file; later files win.
    pub fn load_file(&mut self, path: &Path) {
        let parsed = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|text| serde_json::from_str::<FxHashMap<String, ObjectColors>>(&text).map_err(|e| e.to_string()));
        match parsed {
            Ok(schemes) => {
                log::info!("loaded {} colour schemes from {}", schemes.len(), path.display());
                self.schemes.extend(schemes);
            }
            Err(err) => log::warn!("cannot load colours from {}: {err}", path.display()),
        }
    }

    /// Names of the colour schemes, sorted: the values `Color` can take.
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.schemes.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    /// The colour objects of the scheme `name` are drawn in.
    pub fn display_color(&self, name: &str) -> Rgba {
        self.schemes.get(name).unwrap_or(&SLATE).display
    }

    pub fn for_object(&self, object: &Object) -> &ObjectColors {
        const ALWAYS_GOLD: TagSet = TagSet::SHELL.union(TagSet::CHAFF).union(TagSet::FLARE);
        if object.tags().intersects(ALWAYS_GOLD) {
            return &GOLD;
        }
        self.schemes.get(object.text(known::COLOR)).unwrap_or(&SLATE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_colours_parse() {
        let mut palette = Palette::default();
        palette.load_file(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/acmi_colors.json"));
        assert_eq!(palette.schemes["Blue"].display, Rgba::rgb(0x3b, 0x90, 0xff));
        assert_eq!(palette.schemes["Red"].trail_tails.0[3], 0);
        assert!(palette.schemes.len() >= 8);
    }
}
