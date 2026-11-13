//! Contents of the "View" panel: camera mode, what is visible and how trails
//! are drawn. Where it is shown is up to [`super::panels`].

use acmi_core::Category;

use crate::app::App;
use crate::camera_controller::CameraMode;
use crate::config::{Ground, Units};

pub(super) fn contents(app: &mut App, ui: &mut egui::Ui) {
    ui.strong("Camera");
    let mut mode = app.controller.mode();
    egui::ComboBox::from_id_salt("camera_mode").selected_text(mode.label()).width(170.0).show_ui(ui, |ui| {
        for candidate in CameraMode::ALL {
            ui.selectable_value(&mut mode, candidate, candidate.label());
        }
    });
    if mode != app.controller.mode() {
        app.set_camera_mode(mode);
    }
    ui.add(egui::Slider::new(&mut app.settings.camera.fov_deg, 20.0..=100.0).text("FOV").suffix("°"));

    ui.separator();
    ui.strong("Ground");
    let mut ground = app.settings.terrain.ground();
    egui::ComboBox::from_id_salt("ground")
        .selected_text(ground.label())
        .width(170.0)
        .show_ui(ui, |ui| {
            for candidate in Ground::all() {
                ui.selectable_value(&mut ground, candidate, candidate.label());
            }
        })
        .response
        .on_hover_text("Satellite imagery and elevation are streamed from the internet and cached on disk");
    if ground != app.settings.terrain.ground() {
        app.settings.terrain.set_ground(ground);
    }

    ui.separator();
    ui.strong("Show");
    let settings = &mut app.settings;
    for category in Category::ALL {
        ui.checkbox(&mut settings.objects.visible_categories[category.index()], category.label());
    }
    ui.add_space(4.0);
    ui.checkbox(&mut settings.objects.show_labels, "Labels");
    ui.checkbox(&mut settings.grid.show, "Latitude / longitude grid");
    ui.checkbox(&mut settings.borders.show, "Country borders");
    ui.checkbox(&mut settings.earth.show_atmosphere, "Atmosphere");
    ui.checkbox(&mut settings.lighting.enabled, "Sun lighting");
    ui.add_enabled(
        settings.lighting.enabled,
        egui::Checkbox::new(&mut settings.lighting.follow_recording_time, "Sun at recorded time"),
    );

    ui.separator();
    ui.strong("Trails");
    let trails = &mut settings.trails;
    ui.checkbox(&mut trails.show, "Show trails");
    ui.add_enabled_ui(trails.show, |ui| {
        ui.add(
            egui::Slider::new(&mut trails.past_seconds, 0.0..=600.0)
                .text("Past")
                .suffix(" s")
                .logarithmic(true)
                .smallest_positive(1.0),
        );
        ui.add(
            egui::Slider::new(&mut trails.future_seconds, 0.0..=600.0)
                .text("Future")
                .suffix(" s")
                .logarithmic(true)
                .smallest_positive(1.0),
        );
        ui.checkbox(&mut trails.show_altitude_lines, "Altitude lines");
        ui.add_enabled(
            trails.show_altitude_lines,
            egui::Slider::new(&mut trails.altitude_line_spacing_s, 0.25..=10.0).text("Spacing").suffix(" s").logarithmic(true),
        )
        .on_hover_text("Seconds of flight between altitude lines");
        ui.checkbox(&mut trails.ribbons, "Ribbon trails for aircraft")
            .on_hover_text("A band in the plane of the wings, dark on top and light underneath, that shows rolls");
        ui.add_enabled_ui(trails.ribbons, |ui| {
            ui.add(egui::Slider::new(&mut trails.ribbon_width, 0.25..=4.0).text("Width").suffix("×").logarithmic(true))
                .on_hover_text("Ribbon width; 1 is about a wingspan");
            let mut percent = trails.ribbon_opacity * 100.0;
            if ui.add(egui::Slider::new(&mut percent, 5.0..=100.0).text("Opacity").suffix(" %").fixed_decimals(0)).changed() {
                trails.ribbon_opacity = percent / 100.0;
            }
        });
    });

    ui.separator();
    ui.horizontal(|ui| {
        ui.strong("Units");
        ui.selectable_value(&mut settings.general.units, Units::Aviation, "ft / kt / nm");
        ui.selectable_value(&mut settings.general.units, Units::Metric, "metric");
    });
}
