//! The Preferences window: every setting that is not changed often enough
//! to live in the View panel, grouped by what it affects.
//!
//! Changes apply as they are made unless a row says "restart". To add a
//! setting: a field in `config.rs` and a row in the matching section here.

use crate::app::App;
use crate::config::{Rgba, Settings, Units};

pub fn show(app: &mut App, ctx: &egui::Context) {
    if !app.ui.preferences_open {
        return;
    }
    let mut open = true;
    let mut pick_model_folder = false;
    egui::Window::new("Preferences")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([540.0, 620.0])
        .default_pos(ctx.content_rect().center() - egui::vec2(270.0, 320.0))
        .min_size([420.0, 260.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.weak("Changes apply at once and are saved on exit. Hover a setting for what it does.");
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let user_data = app.user_data.display().to_string();
                let user_models = app.user_models;
                let settings = &mut app.settings;
                general(ui, settings, &user_data, user_models, &mut pick_model_folder);
                camera(ui, settings);
                objects(ui, settings);
                trails(ui, settings);
                lighting(ui, settings);
                atmosphere(ui, settings);
                terrain(ui, settings);
                imagery(ui, settings);
                grid(ui, settings);
                ui.add_space(8.0);
            });
        });
    app.ui.preferences_open = open;
    if pick_model_folder {
        app.model_folder_dialog();
    }
}

/// Set by `--preferences`, so a screenshot shows every setting.
pub static EXPAND_ALL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A collapsible group of settings laid out as `name | control` rows, with
/// a button that puts the group back to its defaults.
fn section(ui: &mut egui::Ui, title: &str, open: bool, reset: impl FnOnce(), rows: impl FnOnce(&mut egui::Ui)) {
    let mut reset = Some(reset);
    // The development flag that opens this window also unfolds every section.
    let open = open || EXPAND_ALL.load(std::sync::atomic::Ordering::Relaxed);
    egui::CollapsingHeader::new(egui::RichText::new(title).strong()).default_open(open).show(ui, |ui| {
        egui::Grid::new(title).num_columns(2).spacing([16.0, 6.0]).min_col_width(170.0).show(ui, rows);
        if ui.small_button("Reset to defaults").clicked() {
            if let Some(reset) = reset.take() {
                reset();
            }
        }
        ui.add_space(4.0);
    });
}

/// One row: the setting's name, explained on hover, and its control.
fn row(ui: &mut egui::Ui, name: &str, about: &str, control: impl FnOnce(&mut egui::Ui)) {
    ui.label(name).on_hover_text(about);
    ui.horizontal(control);
    ui.end_row();
}

/// A note beside settings that are only read when the program starts.
fn restart(ui: &mut egui::Ui) {
    ui.weak("restart").on_hover_text("Takes effect the next time the program starts");
}

fn color(ui: &mut egui::Ui, color: &mut Rgba) {
    ui.color_edit_button_srgba_unmultiplied(&mut color.0);
}

fn text(ui: &mut egui::Ui, value: &mut String, hint: &str) {
    ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(300.0));
}

fn general(ui: &mut egui::Ui, settings: &mut Settings, user_data: &str, user_models: usize, pick_model_folder: &mut bool) {
    // The model folder is not something to reset by accident.
    let reset = |general: &mut crate::config::General, loading: &mut crate::config::Loading| {
        let folder = std::mem::take(&mut general.user_data_dir);
        *general = crate::config::General { user_data_dir: folder, ..Default::default() };
        *loading = Default::default();
    };
    let (general, loading) = (&mut settings.general, &mut settings.loading);
    let mut do_reset = false;
    section(ui, "General", true, || do_reset = true, |ui| {
        row(ui, "Units", "Units used throughout: feet, knots and nautical miles, or metric", |ui| {
            ui.selectable_value(&mut general.units, Units::Aviation, "ft / kt / nm");
            ui.selectable_value(&mut general.units, Units::Metric, "metric");
        });
        row(ui, "3D model folder", "Folder with your .glb / .obj models (or the data folder containing 'meshes')", |ui| {
            if ui.button("Choose…").clicked() {
                *pick_model_folder = true;
            }
            let found = if user_models == 0 { "no models found".to_owned() } else { format!("{user_models} models") };
            ui.add(egui::Label::new(egui::RichText::new(format!("{found} in {user_data}")).weak()).truncate());
        });
        row(ui, "Background", "Colour of space behind the globe", |ui| color(ui, &mut general.background));
        row(ui, "Anti-aliasing", "4× multisampling of the 3D view: smoother edges for a little GPU time", |ui| {
            ui.checkbox(&mut general.antialiasing, "");
            restart(ui);
        });
        row(
            ui,
            "Keep one frame in",
            "Thins recordings as they load: 1 keeps everything. Raise only for files too large for memory. Applies to the next file opened.",
            |ui| {
                ui.add(egui::DragValue::new(&mut loading.keep_every_nth_frame).range(1..=100));
            },
        );
    });
    if do_reset {
        reset(general, loading);
    }
}

fn camera(ui: &mut egui::Ui, settings: &mut Settings) {
    let camera = &mut settings.camera;
    let mut do_reset = false;
    section(ui, "Camera", false, || do_reset = true, |ui| {
        row(ui, "Field of view", "Vertical field of view of the 3D view", |ui| {
            ui.add(egui::Slider::new(&mut camera.fov_deg, 20.0..=100.0).suffix("°"));
        });
        row(ui, "Mouse orbit speed", "Degrees of orbit per pixel dragged, in tracking and free modes", |ui| {
            ui.add(egui::Slider::new(&mut camera.orbit_sensitivity, 0.05..=1.0).suffix(" °/px"));
        });
        row(ui, "Keyboard orbit speed", "Degrees per second when orbiting with the arrow keys", |ui| {
            ui.add(egui::Slider::new(&mut camera.key_orbit_speed, 10.0..=180.0).suffix(" °/s"));
        });
        row(ui, "Tracking distance", "How far from an object the tracking camera starts", |ui| {
            ui.add(egui::Slider::new(&mut camera.tracking_distance_m, 15.0..=5000.0).logarithmic(true).suffix(" m"));
        });
        row(ui, "Free camera speed", "Starting speed of the free camera; the mouse wheel changes it in flight", |ui| {
            ui.add(egui::Slider::new(&mut camera.free_speed_mps, 10.0..=100_000.0).logarithmic(true).suffix(" m/s"));
        });
    });
    if do_reset {
        *camera = Default::default();
    }
}

fn objects(ui: &mut egui::Ui, settings: &mut Settings) {
    let objects = &mut settings.objects;
    let mut do_reset = false;
    section(ui, "Objects and labels", false, || do_reset = true, |ui| {
        row(ui, "Smallest size on screen", "Models never shrink below this radius, so objects stay visible from any distance", |ui| {
            ui.add(egui::Slider::new(&mut objects.min_pixel_radius, 2.0..=40.0).suffix(" px"));
        });
        row(ui, "Labels", "Name tags next to objects in the 3D view", |ui| {
            ui.checkbox(&mut objects.show_labels, "");
        });
        row(ui, "Most labels shown", "Labels are drawn for at most this many objects, nearest first", |ui| {
            ui.add(egui::DragValue::new(&mut objects.max_labels).range(0..=5000).speed(5));
        });
        row(ui, "Primary–secondary line", "Colour of the line joining the two selected objects", |ui| {
            color(ui, &mut objects.relation_line_color);
        });
    });
    if do_reset {
        // Which kinds of object are shown belongs to the View panel.
        *objects = crate::config::Objects { visible_categories: objects.visible_categories, ..Default::default() };
    }
}

fn trails(ui: &mut egui::Ui, settings: &mut Settings) {
    let trails = &mut settings.trails;
    let mut do_reset = false;
    section(ui, "Trails", false, || do_reset = true, |ui| {
        row(ui, "Most points per trail", "Longer trails are thinned to about this many points", |ui| {
            ui.add(egui::DragValue::new(&mut trails.max_points).range(16..=20_000).speed(8));
        });
        row(ui, "Altitude line spacing", "Seconds of flight between the vertical lines dropped to the ground", |ui| {
            ui.add(egui::Slider::new(&mut trails.altitude_line_spacing_s, 0.25..=10.0).logarithmic(true).suffix(" s"));
        });
        row(ui, "Ribbon trails", "Aircraft leave a band in the plane of their wings, which shows rolls", |ui| {
            ui.checkbox(&mut trails.ribbons, "");
        });
        row(ui, "Ribbon width", "1 is about a wingspan", |ui| {
            ui.add(egui::Slider::new(&mut trails.ribbon_width, 0.25..=4.0).logarithmic(true).suffix("×"));
        });
        row(ui, "Ribbon opacity", "How solid ribbons are at their most visible", |ui| {
            ui.add(egui::Slider::new(&mut trails.ribbon_opacity, 0.05..=1.0));
        });
    });
    if do_reset {
        // Whether and how long trails are drawn belongs to the View panel.
        let keep = (trails.show, trails.past_seconds, trails.future_seconds, trails.show_altitude_lines);
        *trails = Default::default();
        (trails.show, trails.past_seconds, trails.future_seconds, trails.show_altitude_lines) = keep;
    }
}

fn lighting(ui: &mut egui::Ui, settings: &mut Settings) {
    let lighting = &mut settings.lighting;
    let mut do_reset = false;
    section(ui, "Sun and lighting", false, || do_reset = true, |ui| {
        row(ui, "Sun lighting", "Light the globe and objects from the sun; off shows everything fully lit", |ui| {
            ui.checkbox(&mut lighting.enabled, "");
        });
        row(ui, "Night-side brightness", "Light that reaches surfaces facing away from the sun", |ui| {
            ui.add(egui::Slider::new(&mut lighting.ambient, 0.0..=1.0));
        });
        row(ui, "Sun at recorded time", "Place the sun where it really was at the recording's date and time", |ui| {
            ui.checkbox(&mut lighting.follow_recording_time, "");
        });
        row(ui, "Fixed sun longitude", "Where the sun is overhead when not following the recording (or it has no date)", |ui| {
            ui.add(egui::Slider::new(&mut lighting.sun_longitude_deg, -180.0..=180.0).suffix("°"));
        });
        row(ui, "Fixed sun latitude", "Where the sun is overhead when not following the recording (or it has no date)", |ui| {
            ui.add(egui::Slider::new(&mut lighting.sun_latitude_deg, -23.5..=23.5).suffix("°"));
        });
    });
    if do_reset {
        *lighting = Default::default();
    }
}

fn atmosphere(ui: &mut egui::Ui, settings: &mut Settings) {
    let earth = &mut settings.earth;
    let mut do_reset = false;
    section(ui, "Atmosphere and world map", false, || do_reset = true, |ui| {
        row(ui, "Atmosphere", "Sky, haze and the blue rim of the planet", |ui| {
            ui.checkbox(&mut earth.show_atmosphere, "");
        });
        row(ui, "Amount of air", "1 is the real atmosphere; less is clearer, more is hazier", |ui| {
            ui.add(egui::Slider::new(&mut earth.atmosphere_strength, 0.0..=3.0));
        });
        row(ui, "Sky brightness", "Brightness of the sky and haze", |ui| {
            ui.add(egui::Slider::new(&mut earth.atmosphere_exposure, 0.2..=3.0));
        });
        row(ui, "World map image", "An equirectangular image to use instead of the bundled world map; empty for the bundled one", |ui| {
            text(ui, &mut earth.texture, "bundled map");
            restart(ui);
        });
        row(ui, "World map width", "The map is scaled down to at most this many pixels across", |ui| {
            egui::ComboBox::from_id_salt("map_width").selected_text(earth.texture_max_width.to_string()).show_ui(ui, |ui| {
                for width in [2048, 4096, 8192, 16384] {
                    ui.selectable_value(&mut earth.texture_max_width, width, width.to_string());
                }
            });
            restart(ui);
        });
    });
    if do_reset {
        *earth = Default::default();
    }
}

fn terrain(ui: &mut egui::Ui, settings: &mut Settings) {
    let terrain = &mut settings.terrain;
    let mut do_reset = false;
    section(ui, "Terrain", false, || do_reset = true, |ui| {
        row(ui, "Download tiles", "Fetch terrain and imagery over the internet; off uses only what is already cached", |ui| {
            ui.checkbox(&mut terrain.online, "");
        });
        row(ui, "Finest detail level", "Deepest zoom level streamed: 12 is about 30 m per height sample", |ui| {
            ui.add(egui::Slider::new(&mut terrain.max_zoom, 4..=15));
        });
        row(ui, "Sharpness", "Screen pixels a terrain sample may cover before more detail loads: lower is sharper and downloads more", |ui| {
            ui.add(egui::Slider::new(&mut terrain.pixels_per_sample, 0.5..=8.0).logarithmic(true).suffix(" px"));
        });
        row(ui, "Height exaggeration", "Multiplier applied to terrain heights; 1 is true to scale", |ui| {
            ui.add(egui::Slider::new(&mut terrain.vertical_exaggeration, 0.5..=5.0).suffix("×"));
        });
        row(ui, "Simultaneous downloads", "How many tiles are fetched at once", |ui| {
            ui.add(egui::Slider::new(&mut terrain.download_threads, 1..=16));
        });
        row(ui, "Elevation server", "Address of Terrarium-encoded elevation tiles, with {z}, {x} and {y} placeholders", |ui| {
            text(ui, &mut terrain.url_template, "https://…/{z}/{x}/{y}.png");
        });
        row(ui, "Cache folder", "Where downloaded tiles are kept; empty for the usual place. Never emptied automatically.", |ui| {
            text(ui, &mut terrain.cache_dir, "default location");
        });
    });
    if do_reset {
        let imagery = terrain.clone();
        *terrain = crate::config::Terrain {
            // The Ground choice belongs to the View panel, the rest to Imagery.
            enabled: imagery.enabled,
            imagery: imagery.imagery,
            elevation_colors: imagery.elevation_colors,
            imagery_url_template: imagery.imagery_url_template,
            imagery_max_zoom: imagery.imagery_max_zoom,
            imagery_relief: imagery.imagery_relief,
            imagery_brightness: imagery.imagery_brightness,
            imagery_saturation: imagery.imagery_saturation,
            imagery_gamma: imagery.imagery_gamma,
            imagery_attribution: imagery.imagery_attribution,
            ..Default::default()
        };
    }
}

fn imagery(ui: &mut egui::Ui, settings: &mut Settings) {
    let terrain = &mut settings.terrain;
    let mut do_reset = false;
    section(ui, "Satellite imagery", false, || do_reset = true, |ui| {
        row(ui, "Brightness", "1 is as served", |ui| {
            ui.add(egui::Slider::new(&mut terrain.imagery_brightness, 0.3..=2.0));
        });
        row(ui, "Saturation", "Colour strength; 1 is as served, 0 is grey", |ui| {
            ui.add(egui::Slider::new(&mut terrain.imagery_saturation, 0.0..=2.0));
        });
        row(ui, "Contrast curve", "Below 1 lifts dark areas such as forest; 1 is as served", |ui| {
            ui.add(egui::Slider::new(&mut terrain.imagery_gamma, 0.4..=1.6));
        });
        row(ui, "Relief shading", "How strongly hill shading is laid over the imagery", |ui| {
            ui.add(egui::Slider::new(&mut terrain.imagery_relief, 0.0..=1.0));
        });
        row(ui, "Imagery server", "Address of Web-Mercator JPEG or PNG tiles, with {z}, {x} and {y} placeholders. Mind the server's terms of use.", |ui| {
            text(ui, &mut terrain.imagery_url_template, "https://…/{z}/{y}/{x}.jpg");
        });
        row(ui, "Server's finest level", "Deepest zoom level the imagery server has; closer views enlarge it", |ui| {
            ui.add(egui::Slider::new(&mut terrain.imagery_max_zoom, 4..=20));
        });
        row(ui, "Credit line", "Shown in the corner of the 3D view while imagery is on", |ui| {
            text(ui, &mut terrain.imagery_attribution, "");
        });
    });
    if do_reset {
        let defaults = crate::config::Terrain::default();
        terrain.imagery_url_template = defaults.imagery_url_template;
        terrain.imagery_max_zoom = defaults.imagery_max_zoom;
        terrain.imagery_relief = defaults.imagery_relief;
        terrain.imagery_brightness = defaults.imagery_brightness;
        terrain.imagery_saturation = defaults.imagery_saturation;
        terrain.imagery_gamma = defaults.imagery_gamma;
        terrain.imagery_attribution = defaults.imagery_attribution;
    }
}

fn grid(ui: &mut egui::Ui, settings: &mut Settings) {
    let (grid, borders) = (&mut settings.grid, &mut settings.borders);
    let mut do_reset = false;
    section(ui, "Grid and borders", false, || do_reset = true, |ui| {
        row(ui, "Grid spacing", "Degrees between lines of latitude and of longitude", |ui| {
            ui.add(egui::DragValue::new(&mut grid.lat_step_deg).range(1.0..=45.0).suffix("° lat"));
            ui.add(egui::DragValue::new(&mut grid.lon_step_deg).range(1.0..=45.0).suffix("° lon"));
            restart(ui);
        });
        row(ui, "Grid colour", "Colour of the grid, and of the equator", |ui| {
            color(ui, &mut grid.color);
            color(ui, &mut grid.equator_color);
            restart(ui);
        });
        row(ui, "Fine grid spacing", "A finer grid that appears when the camera is low", |ui| {
            ui.add(egui::DragValue::new(&mut grid.fine_step_deg).range(0.1..=10.0).speed(0.1).suffix("°"));
            color(ui, &mut grid.fine_color);
            restart(ui);
        });
        row(ui, "Fine grid below", "Camera altitude under which the fine grid is shown", |ui| {
            let mut km = grid.fine_below_altitude_m / 1000.0;
            if ui.add(egui::Slider::new(&mut km, 10.0..=10_000.0).logarithmic(true).suffix(" km")).changed() {
                grid.fine_below_altitude_m = km * 1000.0;
            }
        });
        row(ui, "Border colour", "Colour of country borders", |ui| {
            color(ui, &mut borders.color);
            restart(ui);
        });
    });
    if do_reset {
        // Whether they are shown belongs to the View panel.
        *grid = crate::config::Grid { show: grid.show, ..Default::default() };
        *borders = crate::config::Borders { show: borders.show, ..Default::default() };
    }
}
