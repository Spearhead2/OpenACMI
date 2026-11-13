//! User settings, persisted as TOML in the platform's configuration directory.
//!
//! Every section and field has a default, so old files keep loading as
//! settings are added: a new setting is just a new field here.

use crate::quantity::{PairQuantity, Quantity};
use acmi_render::acmi_terrain::ElevationPalette;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// sRGB colour with straight alpha, written as `#rrggbb` or `#rrggbbaa`.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b, 255])
    }

    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.trim().trim_start_matches('#');
        let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        match hex.len() {
            6 => Some(Self([byte(0)?, byte(2)?, byte(4)?, 255])),
            8 => Some(Self([byte(0)?, byte(2)?, byte(4)?, byte(6)?])),
            _ => None,
        }
    }

    pub fn with_alpha(self, alpha: u8) -> Self {
        Self([self.0[0], self.0[1], self.0[2], alpha])
    }

    /// Normalised RGB components (still sRGB encoded).
    pub fn rgb_f32(self) -> [f32; 3] {
        [self.0[0], self.0[1], self.0[2]].map(|c| c as f32 / 255.0)
    }

    pub fn rgba_f32(self) -> [f32; 4] {
        self.0.map(|c| c as f32 / 255.0)
    }

    pub fn to_egui(self) -> egui::Color32 {
        let [r, g, b, a] = self.0;
        egui::Color32::from_rgba_unmultiplied(r, g, b, a)
    }
}

impl fmt::Debug for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b, a] = self.0;
        write!(f, "#{r:02x}{g:02x}{b:02x}")?;
        if a != 255 {
            write!(f, "{a:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for Rgba {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Rgba::parse(&text).ok_or_else(|| serde::de::Error::custom(format!("invalid colour {text:?}")))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    /// Feet, knots, nautical miles.
    #[default]
    Aviation,
    /// Metres, km/h, kilometres.
    Metric,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub general: General,
    pub connection: Connection,
    pub loading: Loading,
    pub earth: Earth,
    pub terrain: Terrain,
    pub grid: Grid,
    pub borders: Borders,
    pub objects: Objects,
    pub trails: Trails,
    pub lighting: Lighting,
    pub camera: CameraSettings,
    pub panels: Panels,
    pub graphs: Graphs,
    pub recent_files: RecentFiles,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub units: Units,
    /// Folder with user content: `meshesobj/*.obj` models and
    /// `acmi_colors.json`. Empty means `data` next to the executable.
    pub user_data_dir: String,
    pub background: Rgba,
    /// 4× multisampling of the 3D view. Takes effect on restart.
    pub antialiasing: bool,
}

impl Default for General {
    fn default() -> Self {
        Self { units: Units::Aviation, user_data_dir: String::new(), background: Rgba::rgb(4, 5, 10), antialiasing: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Connection {
    pub host: String,
    pub port: u16,
    pub client_name: String,
    /// Stored in plain text, as the telemetry protocol offers no real security.
    pub password: String,
    pub clear_on_connect: bool,
}

impl Default for Connection {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: acmi_io::rtt::DEFAULT_PORT,
            client_name: "OpenACMI".into(),
            password: String::new(),
            clear_on_connect: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Loading {
    /// Keep one in every N frames when loading files. 1 is lossless; raise
    /// it only for recordings too large for memory.
    pub keep_every_nth_frame: usize,
}

impl Default for Loading {
    fn default() -> Self {
        Self { keep_every_nth_frame: 1 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Earth {
    /// Equirectangular world map. Empty uses the bundled one.
    pub texture: String,
    /// The map is downscaled to this width (and to what the GPU supports).
    pub texture_max_width: u32,
    pub show_atmosphere: bool,
    /// Amount of air and haze: 1 is the real atmosphere, less is clearer.
    pub atmosphere_strength: f32,
    /// Brightness of the sky and haze.
    pub atmosphere_exposure: f32,
}

impl Default for Earth {
    fn default() -> Self {
        Self {
            texture: String::new(),
            texture_max_width: 8192,
            show_atmosphere: true,
            atmosphere_strength: 1.0,
            atmosphere_exposure: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Grid {
    pub show: bool,
    pub lat_step_deg: f64,
    pub lon_step_deg: f64,
    pub color: Rgba,
    pub equator_color: Rgba,
    /// A finer grid appears when the camera is below `fine_below_altitude_m`.
    pub fine_step_deg: f64,
    pub fine_color: Rgba,
    pub fine_below_altitude_m: f64,
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            show: true,
            lat_step_deg: 10.0,
            lon_step_deg: 10.0,
            color: Rgba([84, 204, 63, 60]),
            equator_color: Rgba([102, 204, 255, 150]),
            fine_step_deg: 1.0,
            fine_color: Rgba([84, 204, 63, 28]),
            fine_below_altitude_m: 2_500_000.0,
        }
    }
}

/// Streamed elevation data draped over the globe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Terrain {
    pub enabled: bool,
    /// Allow downloading tiles. When false only the disk cache is used.
    pub online: bool,
    /// Tile server address with `{z}`, `{x}` and `{y}` placeholders, serving
    /// Terrarium-encoded Web-Mercator PNG tiles.
    pub url_template: String,
    /// Where downloaded tiles are kept. Empty uses the platform cache
    /// directory. The cache is never pruned automatically.
    pub cache_dir: String,
    /// Deepest zoom level streamed (12 is roughly 30 m per sample).
    pub max_zoom: u8,
    /// Screen pixels a terrain sample may cover before more detail is
    /// loaded. Lower is sharper and downloads more.
    pub pixels_per_sample: f32,
    /// Multiplier applied to heights.
    pub vertical_exaggeration: f32,
    /// Simultaneous downloads.
    pub download_threads: usize,
    /// Colour the ground from satellite imagery instead of from elevation.
    pub imagery: bool,
    /// Colour scheme used when the ground is coloured from elevation:
    /// `natural`, `atlas`, `arid`, `greyscale` or `dark`.
    pub elevation_colors: String,
    /// Imagery tile server with `{z}`, `{x}` and `{y}` placeholders, serving
    /// Web-Mercator ("XYZ") JPEG or PNG tiles. Mind the server's terms of use.
    pub imagery_url_template: String,
    /// Deepest zoom level the imagery server has; closer views enlarge it.
    pub imagery_max_zoom: u8,
    /// How strongly relief shading is laid over the imagery, 0 to 1.
    pub imagery_relief: f32,
    /// Brightness and colour saturation of the imagery; 1 is as served.
    pub imagery_brightness: f32,
    pub imagery_saturation: f32,
    /// Contrast curve of the imagery: below 1 lifts dark areas such as
    /// forest, 1 is as served.
    pub imagery_gamma: f32,
    /// Credit shown in the corner of the view while imagery is on.
    pub imagery_attribution: String,
}

impl Default for Terrain {
    fn default() -> Self {
        Self {
            enabled: true,
            online: true,
            url_template: acmi_render::acmi_terrain::DEFAULT_URL_TEMPLATE.to_owned(),
            cache_dir: String::new(),
            max_zoom: 12,
            pixels_per_sample: 1.5,
            vertical_exaggeration: 1.0,
            download_threads: 6,
            imagery: true,
            elevation_colors: ElevationPalette::default().name().to_owned(),
            imagery_url_template: acmi_render::acmi_terrain::DEFAULT_IMAGERY_URL_TEMPLATE.to_owned(),
            imagery_max_zoom: acmi_render::acmi_terrain::DEFAULT_IMAGERY_MAX_ZOOM,
            imagery_relief: 0.5,
            imagery_brightness: 0.9,
            imagery_saturation: 0.85,
            imagery_gamma: 1.0,
            imagery_attribution: acmi_render::acmi_terrain::DEFAULT_IMAGERY_ATTRIBUTION.to_owned(),
        }
    }
}

/// What the ground is drawn with; a view of three settings as one choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ground {
    /// 3D terrain textured with satellite imagery.
    Satellite,
    /// 3D terrain coloured by height.
    Elevation(ElevationPalette),
    /// The bundled flat world map, without terrain.
    WorldMap,
}

impl Ground {
    pub fn label(self) -> String {
        match self {
            Ground::Satellite => "Satellite imagery".to_owned(),
            Ground::Elevation(palette) => format!("Elevation: {}", palette.label()),
            Ground::WorldMap => "World map (flat)".to_owned(),
        }
    }

    /// Every choice, in menu order.
    pub fn all() -> impl Iterator<Item = Ground> {
        std::iter::once(Ground::Satellite).chain(ElevationPalette::ALL.map(Ground::Elevation)).chain([Ground::WorldMap])
    }
}

impl Terrain {
    /// The elevation colour scheme; unknown names fall back to the default.
    pub fn palette(&self) -> ElevationPalette {
        ElevationPalette::from_name(&self.elevation_colors).unwrap_or_default()
    }

    pub fn ground(&self) -> Ground {
        match (self.enabled, self.imagery) {
            (false, _) => Ground::WorldMap,
            (true, true) => Ground::Satellite,
            (true, false) => Ground::Elevation(self.palette()),
        }
    }

    pub fn set_ground(&mut self, ground: Ground) {
        self.enabled = ground != Ground::WorldMap;
        match ground {
            Ground::Satellite => self.imagery = true,
            Ground::Elevation(palette) => {
                self.imagery = false;
                self.elevation_colors = palette.name().to_owned();
            }
            // Keep the other two, so switching back restores the last look.
            Ground::WorldMap => {}
        }
    }

    /// Directory of the tile cache, if one can be determined.
    pub fn resolved_cache_dir(&self) -> Option<PathBuf> {
        if !self.cache_dir.is_empty() {
            return Some(PathBuf::from(&self.cache_dir));
        }
        directories::ProjectDirs::from("", "", "OpenACMI").map(|dirs| dirs.cache_dir().join("terrain").join("terrarium"))
    }

    /// Directory of the imagery cache. Each server gets its own folder, named
    /// after its address, so changing `imagery_url_template` never mixes tiles.
    pub fn resolved_imagery_cache_dir(&self) -> Option<PathBuf> {
        let address = self.imagery_url_template.split("://").last().unwrap_or_default();
        let address = address.split(['{', '?']).next().unwrap_or_default();
        let mut folder: String =
            address.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
        folder = folder.trim_matches(['_', '.']).to_owned();
        if folder.is_empty() {
            folder.push_str("imagery");
        }
        let base = if self.cache_dir.is_empty() {
            directories::ProjectDirs::from("", "", "OpenACMI")?.cache_dir().join("imagery")
        } else {
            PathBuf::from(&self.cache_dir).join("imagery")
        };
        Some(base.join(folder))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Borders {
    pub show: bool,
    pub color: Rgba,
}

impl Default for Borders {
    fn default() -> Self {
        Self { show: true, color: Rgba([150, 150, 150, 255]) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Objects {
    /// Models never shrink below this on-screen radius, so objects stay
    /// visible from any distance.
    pub min_pixel_radius: f32,
    pub show_labels: bool,
    /// Labels are drawn for at most this many objects, nearest first.
    pub max_labels: usize,
    pub relation_line_color: Rgba,
    /// Visibility per category, in `acmi_core::Category::ALL` order.
    pub visible_categories: [bool; acmi_core::Category::COUNT],
}

impl Default for Objects {
    fn default() -> Self {
        Self {
            min_pixel_radius: 14.0,
            show_labels: true,
            max_labels: 250,
            relation_line_color: Rgba([255, 204, 51, 217]),
            visible_categories: [true; acmi_core::Category::COUNT],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Trails {
    pub show: bool,
    /// Seconds of history drawn behind each object.
    pub past_seconds: f32,
    /// Seconds of future track drawn ahead of each object.
    pub future_seconds: f32,
    /// Trails longer than this many points are thinned.
    pub max_points: usize,
    /// Vertical lines dropped from the trail to the ground.
    pub show_altitude_lines: bool,
    /// Seconds of flight between altitude lines.
    pub altitude_line_spacing_s: f32,
    /// Aircraft leave a ribbon lying in the plane of their wings, which
    /// shows rolls, instead of a line.
    pub ribbons: bool,
    /// Ribbon width as a fraction of the size of the aircraft's model.
    pub ribbon_width: f32,
    /// How solid ribbons are at their most visible: 0 invisible, 1 opaque.
    pub ribbon_opacity: f32,
}

impl Default for Trails {
    fn default() -> Self {
        Self {
            show: true,
            past_seconds: 60.0,
            future_seconds: 0.0,
            max_points: 512,
            show_altitude_lines: true,
            altitude_line_spacing_s: 0.75,
            ribbons: true,
            ribbon_width: 1.0,
            ribbon_opacity: 0.55,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Lighting {
    pub enabled: bool,
    pub ambient: f32,
    /// Place the sun where it really was at the recording's date and time.
    pub follow_recording_time: bool,
    /// Sub-solar point used otherwise (or when the recording has no date).
    pub sun_longitude_deg: f32,
    pub sun_latitude_deg: f32,
}

impl Default for Lighting {
    fn default() -> Self {
        Self { enabled: true, ambient: 0.35, follow_recording_time: true, sun_longitude_deg: 30.0, sun_latitude_deg: 20.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraSettings {
    pub fov_deg: f32,
    /// Degrees of orbit per pixel dragged, in tracking and free modes.
    pub orbit_sensitivity: f32,
    /// Degrees per second when orbiting with the keyboard.
    pub key_orbit_speed: f32,
    /// Starting distance to a tracked object, metres.
    pub tracking_distance_m: f64,
    /// Starting speed of the free camera, metres per second.
    pub free_speed_mps: f64,
}

impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            fov_deg: 45.0,
            orbit_sensitivity: 0.25,
            key_orbit_speed: 60.0,
            tracking_distance_m: 120.0,
            free_speed_mps: 2_000.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Panels {
    pub track_selection: bool,
    pub render_info: bool,
    /// Tab in front in each dock.
    pub left_tab: PanelId,
    pub right_tab: PanelId,
    pub top_tab: PanelId,
    pub bottom_tab: PanelId,
    pub telemetry_1: PanelSlot,
    pub telemetry_2: PanelSlot,
    pub session: PanelSlot,
    pub events: PanelSlot,
    pub view: PanelSlot,
    pub graph_1: PanelSlot,
    pub graph_2: PanelSlot,
    pub compare: PanelSlot,
}

impl Default for Panels {
    fn default() -> Self {
        let left = PanelSlot { visible: true, dock: Dock::Left };
        Self {
            track_selection: true,
            render_info: false,
            left_tab: PanelId::Telemetry1,
            right_tab: PanelId::View,
            top_tab: PanelId::Graph1,
            bottom_tab: PanelId::Compare,
            telemetry_1: left,
            telemetry_2: left,
            session: left,
            events: left,
            view: PanelSlot { visible: true, dock: Dock::Floating },
            graph_1: left,
            graph_2: left,
            compare: left,
        }
    }
}

/// What the graph panels plot: [`Graphs::primary`] follows the primary
/// object (Graph 1), [`Graphs::secondary`] the secondary one (Graph 2), and
/// [`Graphs::comparison`] overlays both (Compare).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Graphs {
    pub primary: GraphSettings,
    pub secondary: GraphSettings,
    pub comparison: GraphSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphSettings {
    /// Width of the time axis in seconds, centred on the current time;
    /// 0 shows the object's whole flight.
    pub window_s: f64,
    /// Plotted quantities, one strip each.
    pub quantities: Vec<Quantity>,
    /// Quantities between the two objects; only the comparison graph has them.
    pub pair_quantities: Vec<PairQuantity>,
}

impl Default for GraphSettings {
    fn default() -> Self {
        Self { window_s: 0.0, quantities: vec![Quantity::AltitudeMsl, Quantity::GroundSpeed], pair_quantities: Vec::new() }
    }
}

impl Default for Graphs {
    fn default() -> Self {
        Self {
            primary: GraphSettings::default(),
            secondary: GraphSettings::default(),
            comparison: GraphSettings {
                pair_quantities: vec![PairQuantity::Distance, PairQuantity::AltitudeDelta],
                ..GraphSettings::default()
            },
        }
    }
}

/// A panel that can be docked beside the 3D view or float over it. To add
/// one: a variant here, a slot in [`Panels`], and its contents in
/// `ui::panels::contents`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelId {
    #[serde(rename = "telemetry_1")]
    Telemetry1,
    #[serde(rename = "telemetry_2")]
    Telemetry2,
    Session,
    Events,
    View,
    #[serde(rename = "graph_1")]
    Graph1,
    #[serde(rename = "graph_2")]
    Graph2,
    Compare,
}

impl PanelId {
    pub const ALL: [PanelId; 8] = [
        Self::Telemetry1,
        Self::Telemetry2,
        Self::Graph1,
        Self::Graph2,
        Self::Compare,
        Self::Session,
        Self::Events,
        Self::View,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Telemetry1 => "Telemetry 1",
            Self::Telemetry2 => "Telemetry 2",
            Self::Session => "Session",
            Self::Events => "Event Log",
            Self::View => "View",
            Self::Graph1 => "Graph 1",
            Self::Graph2 => "Graph 2",
            Self::Compare => "Compare",
        }
    }
}

/// Where a panel sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dock {
    Left,
    Right,
    Top,
    Bottom,
    Floating,
}

impl Dock {
    /// The four edges of the 3D view a panel can be docked to.
    pub const EDGES: [Dock; 4] = [Self::Left, Self::Right, Self::Top, Self::Bottom];

    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Top => "Top",
            Self::Bottom => "Bottom",
            Self::Floating => "Float",
        }
    }
}

/// Whether one panel is shown, and where.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PanelSlot {
    pub visible: bool,
    pub dock: Dock,
}

impl PanelSlot {
    /// Shown at `dock`.
    pub fn is(&self, dock: Dock) -> bool {
        self.visible && self.dock == dock
    }
}

impl Panels {
    pub fn slot(&self, id: PanelId) -> &PanelSlot {
        match id {
            PanelId::Telemetry1 => &self.telemetry_1,
            PanelId::Telemetry2 => &self.telemetry_2,
            PanelId::Session => &self.session,
            PanelId::Events => &self.events,
            PanelId::View => &self.view,
            PanelId::Graph1 => &self.graph_1,
            PanelId::Graph2 => &self.graph_2,
            PanelId::Compare => &self.compare,
        }
    }

    pub fn slot_mut(&mut self, id: PanelId) -> &mut PanelSlot {
        match id {
            PanelId::Telemetry1 => &mut self.telemetry_1,
            PanelId::Telemetry2 => &mut self.telemetry_2,
            PanelId::Session => &mut self.session,
            PanelId::Events => &mut self.events,
            PanelId::View => &mut self.view,
            PanelId::Graph1 => &mut self.graph_1,
            PanelId::Graph2 => &mut self.graph_2,
            PanelId::Compare => &mut self.compare,
        }
    }

    /// Visible panels docked at `side`, in tab order.
    pub fn docked(&self, side: Dock) -> Vec<PanelId> {
        PanelId::ALL.into_iter().filter(|&id| self.slot(id).is(side)).collect()
    }

    /// The tab in front at `side`; `None` for [`Dock::Floating`].
    pub fn active_tab(&self, side: Dock) -> Option<PanelId> {
        match side {
            Dock::Left => Some(self.left_tab),
            Dock::Right => Some(self.right_tab),
            Dock::Top => Some(self.top_tab),
            Dock::Bottom => Some(self.bottom_tab),
            Dock::Floating => None,
        }
    }

    pub fn set_active_tab(&mut self, side: Dock, id: PanelId) {
        match side {
            Dock::Left => self.left_tab = id,
            Dock::Right => self.right_tab = id,
            Dock::Top => self.top_tab = id,
            Dock::Bottom => self.bottom_tab = id,
            Dock::Floating => {}
        }
    }

    /// Puts every panel back where it starts out, keeping the other options.
    pub fn reset_layout(&mut self) {
        *self = Self { track_selection: self.track_selection, render_info: self.render_info, ..Self::default() };
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RecentFiles {
    pub paths: Vec<PathBuf>,
    pub max_files: usize,
}

impl Default for RecentFiles {
    fn default() -> Self {
        Self { paths: Vec::new(), max_files: 8 }
    }
}

impl RecentFiles {
    pub fn add(&mut self, path: &Path) {
        self.paths.retain(|p| p != path);
        self.paths.insert(0, path.to_owned());
        self.paths.truncate(self.max_files);
    }
}

impl Settings {
    /// Location of the settings file, if the platform has a config directory.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "OpenACMI").map(|dirs| dirs.config_dir().join("settings.toml"))
    }

    /// Loads settings, falling back to defaults when the file is missing or invalid.
    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
                log::error!("ignoring invalid settings file {}: {err}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) {
        let result = toml::to_string_pretty(self).map_err(|e| e.to_string()).and_then(|text| {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            fs::write(path, text).map_err(|e| e.to_string())
        });
        if let Err(err) = result {
            log::error!("cannot save settings to {}: {err}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_layout_round_trips_and_old_settings_still_load() {
        let mut settings = Settings::default();
        assert_eq!(settings.panels.docked(Dock::Left).len(), 7);
        assert!(settings.panels.view.is(Dock::Floating));

        settings.panels.view.dock = Dock::Right;
        settings.panels.events = PanelSlot { visible: true, dock: Dock::Floating };
        settings.panels.session.visible = false;
        settings.panels.compare.dock = Dock::Bottom;
        settings.panels.set_active_tab(Dock::Left, PanelId::Telemetry2);
        let text = toml::to_string_pretty(&settings).unwrap();
        let loaded: Settings = toml::from_str(&text).unwrap();
        assert_eq!(loaded.panels.docked(Dock::Left), [PanelId::Telemetry1, PanelId::Telemetry2, PanelId::Graph1, PanelId::Graph2]);
        assert_eq!(loaded.panels.docked(Dock::Right), [PanelId::View]);
        assert_eq!(loaded.panels.docked(Dock::Bottom), [PanelId::Compare]);
        assert_eq!(loaded.panels.active_tab(Dock::Left), Some(PanelId::Telemetry2));
        assert!(loaded.panels.events.is(Dock::Floating));
        assert_eq!(loaded.graphs.primary.quantities, [Quantity::AltitudeMsl, Quantity::GroundSpeed]);
        assert!(loaded.graphs.primary.pair_quantities.is_empty());
        assert_eq!(loaded.graphs.comparison.pair_quantities, [PairQuantity::Distance, PairQuantity::AltitudeDelta]);

        // Settings written before panels could be moved.
        let old: Settings = toml::from_str("[panels]\ntrack_selection = false\nside_panel = true\nview_options = true\n").unwrap();
        assert!(!old.panels.track_selection);
        assert_eq!(old.panels.docked(Dock::Left).len(), 7);

        settings.panels.reset_layout();
        assert_eq!(settings.panels.docked(Dock::Right), []);
    }

    #[test]
    fn colours_parse_and_print() {
        assert_eq!(Rgba::parse("#3b90ff"), Some(Rgba([0x3b, 0x90, 0xff, 255])));
        assert_eq!(Rgba::parse("3b90ff00"), Some(Rgba([0x3b, 0x90, 0xff, 0])));
        assert_eq!(Rgba::parse("#12"), None);
        assert_eq!(Rgba([1, 2, 3, 4]).to_string(), "#01020304");
        assert_eq!(Rgba::rgb(255, 0, 16).to_string(), "#ff0010");
    }

    #[test]
    fn settings_round_trip_and_tolerate_partial_files() {
        let mut settings = Settings::default();
        settings.connection.port = 1234;
        settings.grid.color = Rgba([1, 2, 3, 4]);
        settings.recent_files.add(Path::new("a.acmi"));
        let text = toml::to_string_pretty(&settings).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back.connection.port, 1234);
        assert_eq!(back.grid.color, Rgba([1, 2, 3, 4]));
        assert_eq!(back.recent_files.paths, vec![PathBuf::from("a.acmi")]);

        let partial: Settings = toml::from_str("[trails]\npast_seconds = 5.0\n").unwrap();
        assert_eq!(partial.trails.past_seconds, 5.0);
        assert_eq!(partial.trails.max_points, 512);
        assert!(partial.grid.show);
    }

    #[test]
    fn ground_choice_round_trips_through_the_settings() {
        let mut terrain = Terrain::default();
        assert_eq!(terrain.ground(), Ground::Satellite);
        for ground in Ground::all() {
            terrain.set_ground(ground);
            assert_eq!(terrain.ground(), ground, "{}", ground.label());
        }
        // The flat map remembers what was shown before it.
        terrain.set_ground(Ground::Elevation(ElevationPalette::Atlas));
        terrain.set_ground(Ground::WorldMap);
        assert_eq!((terrain.enabled, terrain.palette()), (false, ElevationPalette::Atlas));
        // A misspelt scheme in the file is not fatal.
        terrain.elevation_colors = "sepia".to_owned();
        assert_eq!(terrain.palette(), ElevationPalette::Natural);
    }
}
