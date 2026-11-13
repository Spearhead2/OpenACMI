//! Main menu bar.

use crate::app::App;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| file_menu(app, ui));
        ui.menu_button("Real-Time Telemetry", |ui| {
            if ui.button("Connect to Data Stream…").clicked() {
                app.ui.connection_open = true;
                ui.close();
            }
            if ui.add_enabled(app.rtt.is_some(), egui::Button::new("Disconnect")).clicked() {
                app.disconnect();
                ui.close();
            }
        });
        ui.menu_button("View", |ui| {
            super::panels::menu(app, ui);
            ui.separator();
            let panels = &mut app.settings.panels;
            ui.checkbox(&mut panels.track_selection, "Primary / Secondary Track Selection");
            ui.checkbox(&mut panels.render_info, "Render Information");
        });
    });
}

fn file_menu(app: &mut App, ui: &mut egui::Ui) {
    let has_data = !app.session.recording.is_empty();
    if ui.add(egui::Button::new("Open…").shortcut_text("Ctrl+O")).clicked() {
        ui.close();
        app.open_file_dialog();
    }
    if ui.add_enabled(has_data, egui::Button::new("Save As…").shortcut_text("Ctrl+S")).clicked() {
        ui.close();
        app.save_file_dialog();
    }
    let can_export = has_data && app.session.primary.is_some();
    if ui
        .add_enabled(can_export, egui::Button::new("Export Telemetry (CSV)…"))
        .on_disabled_hover_text("Select a primary object first")
        .clicked()
    {
        ui.close();
        app.export_telemetry_dialog();
    }
    if ui.add_enabled(has_data, egui::Button::new("Close")).clicked() {
        ui.close();
        app.disconnect();
        app.session.close();
    }

    ui.separator();
    ui.menu_button("Recent Files", |ui| {
        if app.settings.recent_files.paths.is_empty() {
            ui.weak("No recent files");
        }
        let mut chosen = None;
        for path in &app.settings.recent_files.paths {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let exists = path.is_file();
            let hover = if exists { path.display().to_string() } else { "File not found".to_owned() };
            let button = ui.add_enabled(exists, egui::Button::new(name));
            if button.on_hover_text(&hover).on_disabled_hover_text(&hover).clicked() {
                chosen = Some(path.clone());
            }
        }
        if let Some(path) = chosen {
            ui.close();
            app.load_file(path);
        }
    });

    ui.separator();
    let models = match app.user_models {
        0 => "No models of your own are loaded: aircraft are drawn as generic shapes.".to_owned(),
        count => format!("{count} models from {}", app.user_data.display()),
    };
    if ui.button("Preferences…").clicked() {
        ui.close();
        app.ui.preferences_open = true;
    }
    if ui
        .button("3D Model Folder…")
        .on_hover_text(format!("{models}\nChoose the folder holding your .glb / .obj models, or the data folder containing 'meshes'."))
        .clicked()
    {
        ui.close();
        app.model_folder_dialog();
    }

    ui.separator();
    if ui.button("Quit").clicked() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
