//! Bottom bar: transport controls and the time slider.

use acmi_core::time::{format_duration, format_utc};
use acmi_core::EventKind;

use crate::app::App;
use crate::timeline::SPEEDS;

const LIVE_COLOR: egui::Color32 = egui::Color32::from_rgb(51, 255, 102);
const PAUSED_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 140, 26);

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let is_live = app.session.is_live();
    // Separate borrows: the clock is edited while the recording is read.
    let (timeline, recording) = (&mut app.session.timeline, &app.session.recording);
    let range = recording.time_range();
    let (start, end) = range.unwrap_or((0.0, 0.0));

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_enabled_ui(range.is_some(), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if transport_button(ui, Icon::ToStart, false).on_hover_text("Go to start").clicked() {
                timeline.seek(start, range);
            }
            if transport_button(ui, Icon::Slower, false).on_hover_text("Slower").clicked() {
                timeline.step_speed(-1);
            }
            let play_icon = if timeline.playing { Icon::Pause } else { Icon::Play };
            if transport_button(ui, play_icon, true).on_hover_text("Play / pause (Space)").clicked() {
                timeline.toggle_play(range);
            }
            if transport_button(ui, Icon::Faster, false).on_hover_text("Faster").clicked() {
                timeline.step_speed(1);
            }
            if transport_button(ui, Icon::ToEnd, false).on_hover_text("Go to end").clicked() {
                timeline.seek(end, range);
            }
            ui.spacing_mut().item_spacing.x = 8.0;

            egui::ComboBox::from_id_salt("playback_speed").selected_text(speed_label(timeline.speed)).width(52.0).show_ui(
                ui,
                |ui| {
                    for speed in SPEEDS {
                        ui.selectable_value(&mut timeline.speed, speed, speed_label(speed));
                    }
                },
            );

            // Elapsed / total, with the wall-clock time when the recording has a date.
            let clock = format!("{} / {}", format_duration(timeline.time - start), format_duration(end - start));
            let utc = recording.absolute_time(timeline.time).map(format_utc);
            let response = ui.monospace(clock);
            if let Some(utc) = &utc {
                response.on_hover_text(format!("{utc} UTC"));
            }

            if is_live {
                let (text, color) = if timeline.follow_live { ("● LIVE", LIVE_COLOR) } else { ("⏸ PAUSED", PAUSED_COLOR) };
                let button = egui::Button::new(egui::RichText::new(text).color(color).strong());
                if ui.add(button).on_hover_text("Follow the live stream").clicked() {
                    timeline.follow_live = !timeline.follow_live;
                    timeline.playing = false;
                }
            }

            // The slider takes whatever width is left.
            let mut time = timeline.time;
            ui.spacing_mut().slider_width = (ui.available_width() - 12.0).max(80.0);
            let slider = ui.add(egui::Slider::new(&mut time, start..=end.max(start + 1e-6)).show_value(false));
            if slider.changed() {
                timeline.seek(time, range);
            }
            paint_event_marks(ui, slider.rect, recording.events(), start, end);
        });
    });
    ui.add_space(2.0);
}

/// What a transport button shows.
#[derive(Clone, Copy, PartialEq)]
enum Icon {
    ToStart,
    Slower,
    Play,
    Pause,
    Faster,
    ToEnd,
}

/// A round transport button with its icon drawn as shapes, so it is crisp at
/// any scale and does not depend on which glyphs the UI font happens to have.
/// The `primary` one (play / pause) is larger and filled with the accent colour.
fn transport_button(ui: &mut egui::Ui, icon: Icon, primary: bool) -> egui::Response {
    let diameter = if primary { 30.0 } else { 26.0 };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(diameter, 30.0), egui::Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let visuals = ui.visuals();
    let enabled = ui.is_enabled();
    let (hovered, pressed) = (response.hovered(), response.is_pointer_button_down_on());
    let accent = visuals.selection.bg_fill;
    let (fill, ink) = match (primary, enabled) {
        (_, false) => (egui::Color32::TRANSPARENT, visuals.widgets.noninteractive.fg_stroke.color.gamma_multiply(0.4)),
        (true, true) => {
            let fill = if pressed { accent.gamma_multiply(0.8) } else if hovered { brighten(accent, 1.2) } else { accent };
            (fill, egui::Color32::WHITE)
        }
        (false, true) => {
            let fill = if pressed {
                visuals.widgets.active.weak_bg_fill
            } else if hovered {
                visuals.widgets.hovered.weak_bg_fill
            } else {
                egui::Color32::TRANSPARENT
            };
            (fill, if hovered { visuals.strong_text_color() } else { visuals.text_color() })
        }
    };
    let painter = ui.painter();
    let center = rect.center();
    painter.circle_filled(center, diameter * 0.5 - 1.0, fill);

    // Icons are laid out in a square of side 2 around the origin.
    let unit = if primary { 5.5 } else { 5.0 };
    let at = |x: f32, y: f32| center + egui::vec2(x, y) * unit;
    let triangle = |tip: f32, base: f32| {
        painter.add(egui::Shape::convex_polygon(vec![at(base, -1.0), at(tip, 0.0), at(base, 1.0)], ink, egui::Stroke::NONE));
    };
    let bar = |left: f32, right: f32| {
        painter.rect_filled(egui::Rect::from_two_pos(at(left, -1.0), at(right, 1.0)), 1.0, ink);
    };
    match icon {
        // Nudged right: a triangle's visual centre is left of its box's.
        Icon::Play => triangle(1.05, -0.65),
        Icon::Pause => {
            bar(-0.8, -0.2);
            bar(0.2, 0.8);
        }
        Icon::Faster => {
            triangle(0.0, -1.1);
            triangle(1.1, 0.0);
        }
        Icon::Slower => {
            triangle(0.0, 1.1);
            triangle(-1.1, 0.0);
        }
        Icon::ToEnd => {
            triangle(0.55, -0.95);
            bar(0.6, 0.95);
        }
        Icon::ToStart => {
            triangle(-0.55, 0.95);
            bar(-0.95, -0.6);
        }
    }
    response
}

fn brighten(color: egui::Color32, factor: f32) -> egui::Color32 {
    let channel = |value: u8| (value as f32 * factor).min(255.0) as u8;
    egui::Color32::from_rgb(channel(color.r()), channel(color.g()), channel(color.b()))
}

fn speed_label(speed: f64) -> String {
    if speed < 1.0 {
        format!("×{speed}")
    } else {
        format!("×{speed:.0}")
    }
}

/// Small ticks above the slider where notable events happened.
fn paint_event_marks(ui: &egui::Ui, rect: egui::Rect, events: &[acmi_core::Event], start: f64, end: f64) {
    const MAX_MARKS: usize = 2000;
    if end <= start || events.len() > MAX_MARKS {
        return;
    }
    // The slider's handle travels inside the rect by half its height on each side.
    let inset = rect.height() * 0.5;
    let painter = ui.painter();
    for event in events {
        let color = match event.kind {
            EventKind::Destroyed => egui::Color32::from_rgb(255, 90, 90),
            EventKind::Bookmark => egui::Color32::from_rgb(255, 210, 60),
            EventKind::TakenOff | EventKind::Landed => egui::Color32::from_rgb(120, 190, 255),
            _ => continue,
        };
        let fraction = ((event.time - start) / (end - start)) as f32;
        let x = egui::lerp((rect.left() + inset)..=(rect.right() - inset), fraction);
        painter.line_segment([egui::pos2(x, rect.top() - 1.0), egui::pos2(x, rect.top() + 4.0)], (1.5, color));
    }
}
