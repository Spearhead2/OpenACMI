//! Primary / secondary track selection bar.

use acmi_core::ObjectHandle;

use super::{format, TrackList};
use crate::app::App;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    app.ui.tracks.refresh(&app.session);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label("Primary");
        if let Some(choice) = track_combo(ui, "primary_track", app.session.primary, &app.ui.tracks, &mut app.ui.primary_filter) {
            app.select_primary(choice);
        }
        if ui.button("⇄").on_hover_text("Swap primary and secondary").clicked() {
            app.session.swap_selection();
        }
        if let Some(choice) =
            track_combo(ui, "secondary_track", app.session.secondary, &app.ui.tracks, &mut app.ui.secondary_filter)
        {
            app.session.secondary = choice;
        }
        ui.label("Secondary");

        // Range / bearing / altitude difference from primary to secondary.
        if let (Some(a), Some(b)) = (app.session.primary, app.session.secondary) {
            let units = app.settings.general.units;
            if let Some((range, bearing, delta_alt)) = app.session.recording.relation(a, b, app.session.timeline.time) {
                ui.separator();
                ui.monospace(format!(
                    "{}  /  {}  /  Δalt {}",
                    format::distance(range, units),
                    format::heading(bearing),
                    format::altitude(delta_alt, units)
                ));
            }
        }
    });
}

/// A drop-down with a filter box. Returns the new selection when it changes.
fn track_combo(
    ui: &mut egui::Ui,
    id: &str,
    current: Option<ObjectHandle>,
    tracks: &TrackList,
    filter: &mut String,
) -> Option<Option<ObjectHandle>> {
    const NONE: &str = "— None —";
    let selected_text = current.and_then(|handle| tracks.label(handle)).unwrap_or(NONE);
    let mut choice = None;

    egui::ComboBox::from_id_salt(id)
        .selected_text(selected_text)
        .width(300.0)
        .height(460.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            ui.add(egui::TextEdit::singleline(filter).hint_text("Filter…").desired_width(f32::INFINITY));
            if ui.selectable_label(current.is_none(), NONE).clicked() {
                choice = Some(None);
            }
            ui.separator();

            let needle = filter.to_lowercase();
            let rows: Vec<_> = tracks.items.iter().filter(|item| needle.is_empty() || item.search.contains(&needle)).collect();
            let row_height = ui.text_style_height(&egui::TextStyle::Button);
            egui::ScrollArea::vertical().max_height(380.0).show_rows(ui, row_height, rows.len(), |ui, range| {
                for item in &rows[range] {
                    let text = if item.depth > 0 { format!("    ↳ {}", item.label) } else { item.label.clone() };
                    if ui.selectable_label(current == Some(item.handle), text).clicked() {
                        choice = Some(Some(item.handle));
                    }
                }
            });
            if choice.is_some() {
                ui.close();
            }
        });
    choice
}
