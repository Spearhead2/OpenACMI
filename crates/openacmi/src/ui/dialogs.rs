//! Floating windows: the telemetry connection dialog, preferences and error messages.

use acmi_io::RttStatus;

use crate::app::App;

pub fn show(app: &mut App, ctx: &egui::Context) {
    connection(app, ctx);
    super::preferences::show(app, ctx);
    super::edit_object::show(app, ctx);
    error(app, ctx);
}

fn connection(app: &mut App, ctx: &egui::Context) {
    if !app.ui.connection_open {
        return;
    }
    let mut open = true;
    let mut connect = false;
    let mut disconnect = false;

    egui::Window::new("Real-Time Telemetry").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
        let connected = app.rtt.is_some();
        let settings = &mut app.settings.connection;
        ui.add_enabled_ui(!connected, |ui| {
            egui::Grid::new("connection_fields").num_columns(2).show(ui, |ui| {
                ui.label("Host");
                ui.text_edit_singleline(&mut settings.host);
                ui.end_row();
                ui.label("Port");
                ui.add(egui::DragValue::new(&mut settings.port).range(1..=65535));
                ui.end_row();
                ui.label("Client name");
                ui.text_edit_singleline(&mut settings.client_name);
                ui.end_row();
                ui.label("Password");
                ui.add(egui::TextEdit::singleline(&mut settings.password).password(true));
                ui.end_row();
            });
            ui.checkbox(&mut settings.clear_on_connect, "Clear current session on connect");
        });
        ui.separator();

        let (dot, text) = match app.rtt.as_ref().map(|client| client.status()) {
            None | Some(RttStatus::Stopped) => (egui::Color32::GRAY, "Idle".to_owned()),
            Some(RttStatus::Connecting) => (egui::Color32::from_rgb(255, 217, 51), "Connecting…".to_owned()),
            Some(RttStatus::Handshaking) => (egui::Color32::from_rgb(255, 217, 51), "Handshaking…".to_owned()),
            Some(RttStatus::Streaming { host_name }) => {
                (egui::Color32::from_rgb(51, 255, 102), format!("Streaming from {host_name}"))
            }
            Some(RttStatus::Reconnecting(reason)) => (egui::Color32::from_rgb(255, 153, 51), format!("Retrying: {reason}")),
        };
        ui.horizontal(|ui| {
            ui.colored_label(dot, "●");
            ui.label(text);
        });
        if let Some(client) = &app.rtt {
            ui.weak(format!("{:.1} kB received", client.bytes_received() as f64 / 1000.0));
        }

        ui.separator();
        ui.horizontal(|ui| {
            let valid = !settings.host.trim().is_empty();
            connect = ui.add_enabled(!connected && valid, egui::Button::new("Connect")).clicked();
            disconnect = ui.add_enabled(connected, egui::Button::new("Disconnect")).clicked();
        });
    });

    if connect {
        if app.settings.connection.client_name.trim().is_empty() {
            app.settings.connection.client_name = "OpenACMI".to_owned();
        }
        app.save_settings();
        app.connect();
    }
    if disconnect {
        app.disconnect();
    }
    app.ui.connection_open = open;
}

fn error(app: &mut App, ctx: &egui::Context) {
    let Some(message) = app.ui.error.clone() else { return };
    egui::Window::new("Error").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(
        ctx,
        |ui| {
            ui.label(message);
            ui.add_space(8.0);
            if ui.button("OK").clicked() {
                app.ui.error = None;
            }
        },
    );
}
