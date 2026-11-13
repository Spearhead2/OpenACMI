//! The Edit Object window: change who an object is (name, pilot, type,
//! coalition, colour…) and which 3D model it is drawn with.
//!
//! Edits replace the property for the whole recording, take effect at once
//! and are written out by File › Save As. The model is stored in the
//! object's `Shape` property, the same one recordings use to name a model.

use acmi_core::{known, ObjectHandle, PropId};

use super::track_label;
use crate::app::App;

/// Editable identity properties, in the order shown: `(label, property, hint)`.
const FIELDS: [(&str, PropId, &str); 8] = [
    ("Name", known::NAME, "aircraft or unit type, e.g. F-16C"),
    ("Call sign", known::CALL_SIGN, ""),
    ("Pilot", known::PILOT, ""),
    ("Group", known::GROUP, ""),
    ("Coalition", known::COALITION, ""),
    ("Country", known::COUNTRY, "two-letter code, e.g. us"),
    ("Type", known::TYPE, "tags joined by +, e.g. Air+FixedWing"),
    ("Color", known::COLOR, "a colour scheme, e.g. Blue"),
];

/// Values being edited for one object.
pub struct EditObject {
    handle: ObjectHandle,
    /// One per entry of [`FIELDS`].
    values: [String; FIELDS.len()],
    /// File name of the chosen model; empty picks one from name and type.
    shape: String,
    model_filter: String,
}

impl EditObject {
    /// Starts editing `handle` from its present values.
    pub fn open(app: &App, handle: ObjectHandle) -> Option<Self> {
        let object = app.session.recording.get(handle)?;
        Some(Self {
            handle,
            values: FIELDS.map(|(_, prop, _)| object.text(prop).to_owned()),
            shape: object.text(known::SHAPE).to_owned(),
            model_filter: String::new(),
        })
    }
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut edit) = app.ui.edit_object.take() else { return };
    // The recording may have been closed or replaced underneath the window.
    if app.session.recording.get(edit.handle).is_none() {
        return;
    }
    let title = format!("Edit Object: {}", track_label(&app.session.recording, edit.handle));
    let (mut open, mut done, mut browse) = (true, false, false);
    egui::Window::new("Edit Object")
        .id(egui::Id::unique("edit_object"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_pos(ctx.content_rect().center() - egui::vec2(220.0, 200.0))
        .show(ctx, |ui| {
            ui.add(egui::Label::new(egui::RichText::new(&title).strong()).truncate());
            ui.weak(format!("ID {:x}. Changes apply to the whole recording.", app.session.recording.object(edit.handle).id));
            ui.separator();

            egui::Grid::new("edit_object_fields").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                for ((label, prop, hint), value) in FIELDS.iter().zip(&mut edit.values) {
                    ui.label(*label);
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(value).hint_text(*hint).desired_width(260.0));
                        if *prop == known::COLOR {
                            color_choices(ui, app, value);
                        }
                    });
                    ui.end_row();
                }
                ui.label("3D model").on_hover_text("The model file this object is drawn with");
                ui.horizontal(|ui| {
                    model_choices(ui, app, &mut edit);
                    browse = ui.button("Browse…").on_hover_text("Use a .glb or .obj file from anywhere").clicked();
                });
                ui.end_row();
            });

            // What the object would be drawn with, were the edits applied.
            let tags = acmi_core::TagSet::parse(&edit.values[6]);
            let resolved = app.meshes.resolve(&edit.shape, &edit.values[0], tags);
            let resolved = resolved.and_then(|path| path.file_name()).map(|name| name.to_string_lossy().into_owned());
            ui.weak(match resolved {
                Some(name) if edit.shape.is_empty() => format!("Drawn with {name}, chosen from the name and type."),
                Some(name) => format!("Drawn with {name}."),
                None => "Drawn without a model.".to_owned(),
            });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Apply").clicked() {
                    apply(app, &edit);
                }
                if ui.button("OK").clicked() {
                    apply(app, &edit);
                    done = true;
                }
                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
        });

    if browse {
        let picked = rfd::FileDialog::new().set_title("3D model").add_filter("3D models", &["glb", "obj"]).pick_file();
        if let Some(name) = picked.and_then(|path| app.meshes.add_file(&path)) {
            edit.shape = name;
        }
    }
    if open && !done {
        app.ui.edit_object = Some(edit);
    }
}

/// A menu of the colour schemes the palette knows.
fn color_choices(ui: &mut egui::Ui, app: &App, value: &mut String) {
    egui::ComboBox::from_id_salt("edit_object_color").selected_text("").width(18.0).show_ui(ui, |ui| {
        for name in app.palette.names() {
            let swatch = app.palette.display_color(name).to_egui();
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 2.0, swatch);
                if ui.selectable_label(value == name, name).clicked() {
                    *value = name.to_owned();
                }
            });
        }
    });
}

/// A searchable menu of every model in the library, plus "automatic".
fn model_choices(ui: &mut egui::Ui, app: &App, edit: &mut EditObject) {
    let shown = if edit.shape.is_empty() { "Automatic (by name and type)" } else { edit.shape.as_str() };
    egui::ComboBox::from_id_salt("edit_object_model").selected_text(shown.to_owned()).width(260.0).height(320.0).show_ui(ui, |ui| {
        ui.add(egui::TextEdit::singleline(&mut edit.model_filter).hint_text("Filter…").desired_width(f32::INFINITY));
        if ui.selectable_label(edit.shape.is_empty(), "Automatic (by name and type)").clicked() {
            edit.shape.clear();
        }
        ui.separator();
        let needle = edit.model_filter.to_lowercase();
        for name in app.meshes.file_names() {
            if !needle.is_empty() && !name.to_lowercase().contains(&needle) {
                continue;
            }
            if ui.selectable_label(edit.shape.eq_ignore_ascii_case(name), name).clicked() {
                edit.shape = name.to_owned();
            }
        }
    });
}

/// Writes the edited values into the recording.
fn apply(app: &mut App, edit: &EditObject) {
    let recording = &mut app.session.recording;
    for ((_, prop, _), value) in FIELDS.iter().zip(&edit.values) {
        recording.edit_text(edit.handle, *prop, value.trim());
    }
    recording.edit_text(edit.handle, known::SHAPE, edit.shape.trim());
    // Redraw even while paused: models, labels and trails follow the edit.
    app.settled_frames = 0;
}
