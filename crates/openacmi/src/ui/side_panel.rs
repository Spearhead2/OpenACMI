//! Contents of the telemetry, session and event log panels. Where they are
//! shown is up to [`super::panels`].

use acmi_core::time::{format_duration, format_utc};
use acmi_core::{known, ObjectHandle, PropKind, Unit};

use super::{format, track_label};
use crate::app::App;
use crate::config::Units;
use crate::quantity::{Quantity, Reading};
use crate::session::Session;

pub(super) fn telemetry(ui: &mut egui::Ui, app: &App, handle: Option<ObjectHandle>, units: Units) {
    let session = &app.session;
    let Some(object) = handle.and_then(|h| session.recording.get(h)) else {
        ui.weak("No object selected.\nPick one in the track selector or click it in the 3D view.");
        return;
    };
    let time = session.timeline.time;
    let recording = &session.recording;
    let number = |prop, unit| object.number_at(prop, PropKind::Number(unit), time).map(f64::from);

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let mut rows = Rows::new(ui, "telemetry");

        rows.section("Identity");
        rows.text("ID", format!("{:x}", object.id));
        for (label, prop) in [
            ("Name", known::NAME),
            ("Call sign", known::CALL_SIGN),
            ("Pilot", known::PILOT),
            ("Group", known::GROUP),
            ("Coalition", known::COALITION),
            ("Country", known::COUNTRY),
            ("Type", known::TYPE),
        ] {
            rows.optional(label, Some(object.text_at(prop, time)).filter(|v| !v.is_empty()).map(str::to_owned));
        }
        // The file the object is drawn with, so a wrong or missing match shows.
        let model = app.meshes.resolve(object.text(known::SHAPE), object.name(), object.tags());
        let model = model.and_then(|path| path.file_name()).map(|name| name.to_string_lossy().into_owned());
        let generic = model.as_ref().is_some_and(|name| name.to_ascii_lowercase().starts_with("core."));
        rows.optional("3D model", model.map(|name| if generic { format!("{name} (generic)") } else { name }));

        if !object.is_active_at(time) {
            rows.section("Not in the world at this time");
            rows.finish();
            return;
        }
        let Some(pose) = object.pose_at(time) else { return rows.finish() };

        rows.section("Position");
        rows.text("Latitude", format::latitude(pose.lat));
        rows.text("Longitude", format::longitude(pose.lon));
        rows.text("Altitude MSL", format::altitude(pose.alt, units));
        // Recorded AGL when the file has it, else height above the streamed terrain.
        match number(known::AGL, Unit::Meters) {
            Some(agl) => rows.text("Altitude AGL", format::altitude(agl, units)),
            None => {
                let agl = app.terrain_height(pose.lat, pose.lon).map(|ground| format::altitude(pose.alt - ground, units));
                rows.estimate("Altitude AGL", agl, true, "Estimated: altitude above the streamed terrain elevation here.");
            }
        }

        rows.section("Attitude");
        rows.text("Heading", format::heading(pose.heading as f64));
        rows.text("Pitch", format!("{:+.1}°", pose.pitch));
        rows.text("Roll", format!("{:+.1}°", pose.roll));
        // Recorded where the file has it, otherwise estimated and marked so.
        let mut any_derived = false;
        let mut quantity = |rows: &mut Rows<'_>, label: &str, quantity: Quantity, text: &dyn Fn(f64) -> String| {
            let reading: Option<Reading> = quantity.read(object, time);
            let derived = reading.is_some_and(|reading| reading.is_derived());
            any_derived |= derived;
            rows.estimate(label, reading.map(|reading| text(reading.value)), derived, quantity.derivation().unwrap_or(""));
        };
        quantity(&mut rows, "AOA", Quantity::Aoa, &|v| format!("{v:.1}°"));

        rows.section("Motion");
        let kinematics = object.track.kinematics(time);
        quantity(&mut rows, "IAS", Quantity::Ias, &|v| format::speed(v, units));
        quantity(&mut rows, "CAS", Quantity::Cas, &|v| format::speed(v, units));
        quantity(&mut rows, "TAS", Quantity::Tas, &|v| format::speed(v, units));
        rows.optional("Ground speed", kinematics.map(|k| format::speed(k.speed, units)));
        quantity(&mut rows, "Mach", Quantity::Mach, &|v| format!("{v:.3}"));
        rows.optional("Vertical speed", kinematics.map(|k| format::vertical_speed(k.vertical_speed, units)));
        rows.optional("Turn rate", kinematics.map(|k| format!("{:+.1} °/s", k.turn_rate)));
        quantity(&mut rows, "G", Quantity::G, &|v| format!("{v:.2} G"));
        if any_derived {
            rows.note("≈ DERIVED: not in the recording. Estimated from the flight path, standard atmosphere, no wind. Hover a value for how.");
        }

        // Whatever else the recording carries for this object.
        const SHOWN_ABOVE: [acmi_core::PropId; 15] = [
            known::NAME,
            known::CALL_SIGN,
            known::PILOT,
            known::GROUP,
            known::COALITION,
            known::COUNTRY,
            known::TYPE,
            known::AGL,
            known::AOA,
            known::IAS,
            known::CAS,
            known::TAS,
            known::MACH,
            known::VERTICAL_G_FORCE,
            known::HDG,
        ];
        let mut other: Vec<(&str, String)> = Vec::new();
        for (prop, series) in object.number_props().filter(|(prop, _)| !SHOWN_ABOVE.contains(prop)) {
            let kind = recording.props.kind(prop);
            if let Some(value) = series.sample(time, kind) {
                let text = match kind {
                    PropKind::Number(unit) => format::property(value, unit, units),
                    PropKind::Flag => if value != 0.0 { "yes" } else { "no" }.to_owned(),
                    _ => format!("{value:.0}"),
                };
                other.push((recording.props.name(prop), text));
            }
        }
        for (prop, series) in object.text_props().filter(|(prop, _)| !SHOWN_ABOVE.contains(prop)) {
            let value = series.at(time);
            if !value.is_empty() {
                // Target references are object ids: show who they are.
                let named = acmi_core::ObjectId::from_str_radix(value, 16)
                    .ok()
                    .filter(|_| recording.props.name(prop).contains("Target") || prop == known::PARENT)
                    .and_then(|id| recording.find(id))
                    .map(|target| track_label(recording, target));
                other.push((recording.props.name(prop), named.unwrap_or_else(|| value.to_owned())));
            }
        }
        if !other.is_empty() {
            rows.section("Other properties");
            other.sort_unstable_by_key(|(name, _)| *name);
            for (name, value) in other {
                rows.text(name, value);
            }
        }
        rows.finish();
    });
}

pub(super) fn session_info(ui: &mut egui::Ui, session: &Session) {
    let recording = &session.recording;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let mut rows = Rows::new(ui, "session");
        rows.section("Recording");
        rows.text("Source", session.title());
        rows.text("Objects", recording.objects().len().to_string());
        rows.text("Events", recording.events().len().to_string());
        if let Some((start, end)) = recording.time_range() {
            rows.text("Duration", format_duration(end - start));
            rows.optional("Start (UTC)", recording.absolute_time(start).map(format_utc));
            rows.optional("End (UTC)", recording.absolute_time(end).map(format_utc));
        }
        rows.section("Properties");
        for (key, value) in recording.meta.iter() {
            rows.text(key, value.to_owned());
        }
        rows.finish();
    });
}

pub(super) fn event_log(app: &mut App, ui: &mut egui::Ui) {
    ui.add(egui::TextEdit::singleline(&mut app.ui.event_filter).hint_text("Filter…").desired_width(f32::INFINITY));
    ui.add_space(2.0);

    let recording = &app.session.recording;
    let needle = app.ui.event_filter.to_lowercase();
    let describe = |event: &acmi_core::Event| {
        let mut text = String::new();
        for &id in &event.objects {
            let name = recording.find(id).map(|h| track_label(recording, h)).unwrap_or_else(|| format!("{id:x}"));
            text.push_str(&name);
            text.push_str("  ");
        }
        text.push_str(&event.text);
        text
    };
    let rows: Vec<(f64, &str, String)> = recording
        .events()
        .iter()
        .map(|event| (event.time, event.kind.name(), describe(event)))
        .filter(|(_, kind, text)| {
            needle.is_empty() || kind.to_lowercase().contains(&needle) || text.to_lowercase().contains(&needle)
        })
        .collect();
    if rows.is_empty() {
        ui.weak("No events.");
        return;
    }

    let start = recording.time_range().map_or(0.0, |(start, _)| start);
    let current = app.session.timeline.time;
    let row_height = ui.text_style_height(&egui::TextStyle::Body);
    let mut seek = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_height, rows.len(), |ui, range| {
        for (time, kind, text) in &rows[range] {
            let line = format!("{}  {kind}  {text}", format_duration(time - start));
            let passed = *time <= current;
            let label =
                egui::RichText::new(line).color(if passed { ui.visuals().text_color() } else { ui.visuals().weak_text_color() });
            if ui.add(egui::Button::selectable(false, label).truncate()).on_hover_text("Jump to this event").clicked() {
                seek = Some(*time);
            }
        }
    });
    if let Some(time) = seek {
        app.session.timeline.seek(time, app.session.recording.time_range());
    }
}

/// A two-column name/value table with section headings.
struct Rows<'a> {
    ui: &'a mut egui::Ui,
    id: &'static str,
    section: usize,
    pending: Vec<Row>,
}

/// One line of a [`Rows`] table.
struct Row {
    label: String,
    value: String,
    /// Set for estimated values: how the estimate is made.
    derived: Option<&'static str>,
}

/// Colour of everything that marks a value as estimated rather than recorded.
pub(super) const DERIVED_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 184, 76);

impl<'a> Rows<'a> {
    fn new(ui: &'a mut egui::Ui, id: &'static str) -> Self {
        Self { ui, id, section: 0, pending: Vec::new() }
    }

    fn section(&mut self, title: &str) {
        self.flush();
        self.ui.add_space(6.0);
        self.ui.strong(title);
    }

    fn text(&mut self, label: &str, value: String) {
        self.pending.push(Row { label: label.to_owned(), value, derived: None });
    }

    /// A row whose value is an estimate when `derived`: it is then drawn in
    /// the derived colour with a tag, and explains itself on hover.
    fn estimate(&mut self, label: &str, value: Option<String>, derived: bool, how: &'static str) {
        let derived = (derived && value.is_some()).then_some(how);
        self.pending.push(Row { label: label.to_owned(), value: value.unwrap_or_else(|| format::NO_DATA.to_owned()), derived });
    }

    /// A line of explanation under the rows so far, in the derived colour.
    fn note(&mut self, text: &str) {
        self.flush();
        self.ui.add(egui::Label::new(egui::RichText::new(text).small().color(DERIVED_COLOR)).wrap());
    }

    /// A row that shows a placeholder when the recording has no such data.
    fn optional(&mut self, label: &str, value: Option<String>) {
        self.text(label, value.unwrap_or_else(|| format::NO_DATA.to_owned()));
    }

    fn finish(mut self) {
        self.flush();
    }

    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        self.section += 1;
        let rows = std::mem::take(&mut self.pending);
        egui::Grid::new((self.id, self.section)).num_columns(2).striped(true).min_col_width(110.0).show(self.ui, |ui| {
            for Row { label, value, derived } in rows {
                match derived {
                    Some(how) => {
                        ui.label(egui::RichText::new(label).color(DERIVED_COLOR)).on_hover_text(how);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("≈ {value}")).monospace().color(DERIVED_COLOR)).on_hover_text(how);
                            let tag = egui::RichText::new(" DERIVED ").small().strong().color(egui::Color32::BLACK);
                            ui.label(tag.background_color(DERIVED_COLOR)).on_hover_text(how);
                        });
                    }
                    None => {
                        ui.weak(label);
                        if value == format::NO_DATA {
                            ui.weak(value);
                        } else {
                            ui.add(egui::Label::new(egui::RichText::new(value).monospace()).wrap());
                        }
                    }
                }
                ui.end_row();
            }
        });
    }
}
