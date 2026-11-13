//! Contents of the graph panels: chosen flight quantities plotted against
//! time, one strip per quantity. A panel follows the primary object, the
//! secondary one, or overlays both for comparison.

use acmi_core::time::format_duration;
use acmi_core::Object;

use super::side_panel::DERIVED_COLOR;
use super::track_label;
use crate::app::App;
use crate::config::{GraphSettings, Units};
use crate::quantity::{Dimension, PairQuantity, Quantity, Source};

/// Which selected objects a graph panel plots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    Primary,
    Secondary,
    /// Primary and secondary drawn over each other on shared axes.
    Both,
}

/// Time spans offered for the horizontal axis; 0 is the objects' whole life.
const WINDOWS: [(f64, &str); 5] =
    [(0.0, "Whole flight"), (600.0, "10 min"), (120.0, "2 min"), (30.0, "30 s"), (10.0, "10 s")];

/// Values averaged into each plotted point, spread over the time it covers.
const BIN_TAPS: usize = 5;
const STRIP_MIN_HEIGHT: f32 = 96.0;
const STRIP_MIN_WIDTH: f32 = 170.0;
/// One colour per quantity when a single object is plotted.
const COLORS: [egui::Color32; 6] = [
    egui::Color32::from_rgb(92, 178, 255),
    egui::Color32::from_rgb(255, 176, 66),
    egui::Color32::from_rgb(120, 220, 130),
    egui::Color32::from_rgb(240, 110, 130),
    egui::Color32::from_rgb(190, 150, 255),
    egui::Color32::from_rgb(90, 215, 215),
];
/// One colour per object when two are compared.
const PRIMARY_COLOR: egui::Color32 = egui::Color32::from_rgb(92, 178, 255);
const SECONDARY_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 176, 66);

/// What one strip plots.
#[derive(Clone, Copy)]
enum Plot {
    /// The same quantity for each object, one line per object.
    Each(Quantity),
    /// One line for a quantity between the two objects.
    Pair(PairQuantity),
}

impl Plot {
    fn label(self) -> &'static str {
        match self {
            Self::Each(quantity) => quantity.label(),
            Self::Pair(quantity) => quantity.label(),
        }
    }

    fn dimension(self) -> Dimension {
        match self {
            Self::Each(quantity) => quantity.dimension(),
            Self::Pair(quantity) => quantity.dimension(),
        }
    }
}

/// One line of a strip: its colour (`None` for the strip's own) and its
/// value in SI units at a time.
struct Line<'a> {
    color: Option<egui::Color32>,
    /// Estimated rather than recorded: drawn dashed.
    derived: bool,
    sample: Box<dyn Fn(f64) -> Option<f64> + 'a>,
}

/// One plotted object.
struct Curve<'a> {
    object: &'a Object,
    /// `None`: use the strip's own colour.
    color: Option<egui::Color32>,
}

pub fn show(app: &mut App, ui: &mut egui::Ui, subject: Subject) {
    let units = app.settings.general.units;
    let settings = match subject {
        Subject::Primary => &mut app.settings.graphs.primary,
        Subject::Secondary => &mut app.settings.graphs.secondary,
        Subject::Both => &mut app.settings.graphs.comparison,
    };
    let recording = &app.session.recording;
    let (primary, secondary) = (app.session.primary, app.session.secondary);
    let wanted: &[(Option<acmi_core::ObjectHandle>, egui::Color32)] = match subject {
        Subject::Primary => &[(primary, PRIMARY_COLOR)],
        Subject::Secondary => &[(secondary, SECONDARY_COLOR)],
        Subject::Both => &[(primary, PRIMARY_COLOR), (secondary, SECONDARY_COLOR)],
    };
    let comparing = subject == Subject::Both;
    let found: Vec<(acmi_core::ObjectHandle, &Object, egui::Color32)> =
        wanted.iter().filter_map(|&(handle, color)| Some((handle?, recording.get(handle?)?, color))).collect();
    let curves: Vec<Curve<'_>> =
        found.iter().map(|&(_, object, color)| Curve { object, color: comparing.then_some(color) }).collect();

    ui.horizontal(|ui| {
        ui.menu_button("Plot…", |ui| quantity_menu(ui, settings, &curves, comparing)).response.on_hover_text("Choose what to plot");
        let current = WINDOWS.iter().find(|(span, _)| *span == settings.window_s).map_or("Custom", |(_, label)| *label);
        egui::ComboBox::from_id_salt("graph_window").selected_text(current).width(110.0).show_ui(ui, |ui| {
            for (span, label) in WINDOWS {
                ui.selectable_value(&mut settings.window_s, span, label);
            }
        });
        // Who is plotted; when comparing, in the colour of their line.
        for &(handle, _, color) in &found {
            let name = egui::RichText::new(track_label(recording, handle));
            let name = if comparing { name.color(color) } else { name.weak() };
            ui.add(egui::Label::new(name).truncate());
        }
    });
    ui.add_space(2.0);

    if curves.is_empty() {
        let who = match subject {
            Subject::Primary => "No primary object selected.",
            Subject::Secondary => "No secondary object selected.",
            Subject::Both => "Select a primary and a secondary object to compare them.",
        };
        ui.weak(format!("{who}\nPick one in the track selector or click it in the 3D view."));
        return;
    }
    let pairs: &[PairQuantity] = if comparing { &settings.pair_quantities } else { &[] };
    if settings.quantities.is_empty() && pairs.is_empty() {
        ui.weak("Nothing chosen: pick quantities from the Plot menu.");
        return;
    }
    let now = app.session.timeline.time;
    let Some(span) = time_span(&curves, now, settings.window_s) else {
        ui.weak("No recorded positions to plot.");
        return;
    };

    let origin = recording.time_range().map_or(0.0, |(start, _)| start);
    // What relates the two objects first, then each one's own numbers.
    let quantities: Vec<Plot> =
        pairs.iter().map(|&pair| Plot::Pair(pair)).chain(settings.quantities.iter().map(|&each| Plot::Each(each))).collect();
    let count = quantities.len() as f32;
    let strip = |index: usize, plot| Strip { plot, color: COLORS[index % COLORS.len()], span, now, origin, units };
    let mut seek = None;
    let spacing = ui.spacing().item_spacing;
    let (width, height) = (ui.available_width(), ui.available_height());
    // In a wide, low dock (top or bottom) the strips sit side by side.
    let side_by_side = count > 1.0 && width > height * 2.0 && (width + spacing.x) / count - spacing.x >= STRIP_MIN_WIDTH;
    if side_by_side {
        let size = egui::vec2((width + spacing.x) / count - spacing.x, height.max(STRIP_MIN_HEIGHT));
        ui.horizontal(|ui| {
            for (index, quantity) in quantities.into_iter().enumerate() {
                seek = strip(index, quantity).show(ui, &curves, size).or(seek);
            }
        });
    } else {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let size = egui::vec2(ui.available_width(), ((height + spacing.y) / count - spacing.y).max(STRIP_MIN_HEIGHT));
            for (index, quantity) in quantities.into_iter().enumerate() {
                seek = strip(index, quantity).show(ui, &curves, size).or(seek);
            }
        });
    }
    if let Some(time) = seek {
        app.session.timeline.seek(time, app.session.recording.time_range());
    }
}

/// Checkboxes for every quantity; those no plotted object has data for are marked.
fn quantity_menu(ui: &mut egui::Ui, settings: &mut GraphSettings, curves: &[Curve<'_>], comparing: bool) {
    if comparing {
        ui.weak("Between the two (secondary relative to primary)");
        for quantity in PairQuantity::ALL {
            let mut chosen = settings.pair_quantities.contains(&quantity);
            if ui.checkbox(&mut chosen, quantity.label()).changed() {
                if chosen {
                    settings.pair_quantities.push(quantity);
                    settings.pair_quantities.sort_by_key(|q| PairQuantity::ALL.iter().position(|candidate| candidate == q));
                } else {
                    settings.pair_quantities.retain(|q| *q != quantity);
                }
            }
        }
        ui.separator();
        ui.weak("Each object");
    }
    for quantity in Quantity::ALL {
        let mut chosen = settings.quantities.contains(&quantity);
        let sources: Vec<Option<Source>> = curves.iter().map(|curve| quantity.source(curve.object)).collect();
        let label = if curves.is_empty() || sources.contains(&Some(Source::Recorded)) {
            quantity.label().to_owned()
        } else if sources.contains(&Some(Source::Derived)) {
            format!("{}  ≈ derived", quantity.label())
        } else {
            format!("{} (no data)", quantity.label())
        };
        if ui.checkbox(&mut chosen, label).changed() {
            if chosen {
                settings.quantities.push(quantity);
                // Kept in the order of the list, whatever order they were ticked in.
                settings.quantities.sort_by_key(|q| Quantity::ALL.iter().position(|candidate| candidate == q));
            } else {
                settings.quantities.retain(|q| *q != quantity);
            }
        }
    }
}

/// The stretch of time on the horizontal axis: from the first object's
/// appearance to the last one's end, or `window` seconds centred on `now`.
fn time_span(curves: &[Curve<'_>], now: f64, window: f64) -> Option<(f64, f64)> {
    let life = |object: &Object| {
        let (first, last) = (object.track.first_time()?, object.track.last_time()?);
        Some((first, object.removed_at().map_or(last, |removed| removed.min(last).max(first))))
    };
    let (first, last) = curves
        .iter()
        .filter_map(|curve| life(curve.object))
        .reduce(|(first, last), (start, end)| (first.min(start), last.max(end)))?;
    if window > 0.0 {
        return Some((now - window * 0.5, now + window * 0.5));
    }
    // A single sample still needs an axis with some width.
    Some((first, last.max(first + 1.0)))
}

/// One quantity plotted over [`Strip::span`].
struct Strip {
    plot: Plot,
    color: egui::Color32,
    span: (f64, f64),
    now: f64,
    /// Start of the recording: times are shown relative to it.
    origin: f64,
    units: Units,
}

impl Strip {
    /// Draws the strip; returns the time the user clicked or dragged to.
    fn show(&self, ui: &mut egui::Ui, curves: &[Curve<'_>], size: egui::Vec2) -> Option<f64> {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        if !ui.is_rect_visible(rect) {
            return None;
        }
        let visuals = ui.visuals();
        let (weak, text) = (visuals.weak_text_color(), visuals.text_color());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, visuals.extreme_bg_color);

        let dimension = self.plot.dimension();
        let (factor, unit) = dimension.display(self.units);
        let font = egui::FontId::monospace(11.0);
        let lines: Vec<Line<'_>> = match (self.plot, curves) {
            (Plot::Each(quantity), _) => curves
                .iter()
                .map(|curve| Line {
                    color: curve.color,
                    derived: quantity.source(curve.object) == Some(Source::Derived),
                    sample: Box::new(move |time| quantity.sample(curve.object, time)),
                })
                .collect(),
            (Plot::Pair(quantity), [first, second]) => {
                let sample = Box::new(move |time| quantity.sample(first.object, second.object, time));
                vec![Line { color: None, derived: false, sample }]
            }
            (Plot::Pair(_), _) => Vec::new(),
        };
        let value_at = |line: &Line<'_>, time: f64| (line.sample)(time).map(|value| value * factor);
        // Lines in the objects' colours leave the title neutral.
        let title_color = if lines.iter().any(|line| line.color.is_some()) && lines.len() > 1 { text } else { self.color };
        let title = if unit.is_empty() || unit == self.plot.label() { self.plot.label().to_owned() } else { format!("{} ({unit})", self.plot.label()) };
        let title = painter.text(rect.left_top() + egui::vec2(6.0, 4.0), egui::Align2::LEFT_TOP, title, font.clone(), title_color);
        // Estimates say so, next to the title.
        let derived = lines.iter().filter(|line| line.derived).count();
        if derived > 0 {
            let tag = if derived == lines.len() { " DERIVED " } else { " PARTLY DERIVED " };
            let galley = painter.layout_no_wrap(tag.to_owned(), font.clone(), egui::Color32::BLACK);
            let badge = egui::Rect::from_min_size(egui::pos2(title.right() + 8.0, title.top()), galley.size());
            painter.rect_filled(badge, 2.0, DERIVED_COLOR);
            painter.galley(badge.min, galley, egui::Color32::BLACK);
            if let Plot::Each(quantity) = self.plot {
                let over_title = response.hover_pos().is_some_and(|pointer| pointer.y < rect.top() + 20.0);
                if over_title {
                    response.clone().on_hover_text(format!("{}\nDashed lines are estimates.", quantity.derivation().unwrap_or("")));
                }
            }
        }
        if lines.is_empty() {
            let hint = "select a primary and a secondary object";
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, hint, font, weak);
            return None;
        }

        let plot = egui::Rect::from_min_max(rect.min + egui::vec2(6.0, 20.0), rect.max - egui::vec2(6.0, 16.0));
        let (start, end) = self.span;
        let time_at = |x: f32| start + (end - start) * ((x - plot.left()) / plot.width()).clamp(0.0, 1.0) as f64;
        let x_at = |time: f64| plot.left() + plot.width() * ((time - start) / (end - start)) as f32;

        // One sample every other pixel is as fine as the line can show.
        let count = ((plot.width() / 2.0) as usize).clamp(16, 1000);
        // Each point is the mean over the stretch of time it stands for, so a
        // long flight squeezed into a narrow strip shows its trend instead of
        // whichever instants happened to be picked. Bearings cannot be averaged.
        let step = (end - start) / count as f64;
        let taps = if dimension == Dimension::Bearing { 1 } else { BIN_TAPS };
        let binned = |line: &Line<'_>, centre: f64| {
            let values = (0..taps).filter_map(|tap| value_at(line, centre + step * ((tap as f64 + 0.5) / taps as f64 - 0.5)));
            let (sum, found) = values.fold((0.0, 0u32), |(sum, found), value| (sum + value, found + 1));
            (found > 0).then(|| sum / found as f64)
        };
        let sampled: Vec<Vec<Option<f64>>> =
            lines.iter().map(|line| (0..=count).map(|i| binned(line, start + step * i as f64)).collect()).collect();
        let Some((low, high)) = value_range(sampled.iter().flatten().copied(), dimension) else {
            painter.text(plot.center(), egui::Align2::CENTER_CENTER, "not recorded in this time span", font, weak);
            return None;
        };
        let y_at = |value: f64| plot.bottom() - plot.height() * ((value - low) / (high - low)) as f32;

        // Scale lines with their values.
        let grid = egui::Stroke::new(1.0, weak.gamma_multiply(0.25));
        for (value, align) in [(high, egui::Align2::LEFT_TOP), ((low + high) * 0.5, egui::Align2::LEFT_CENTER), (low, egui::Align2::LEFT_BOTTOM)] {
            let y = y_at(value);
            painter.line_segment([egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)], grid);
            painter.text(egui::pos2(plot.left() + 2.0, y), align, number(value, high - low), font.clone(), weak);
        }

        // The curves, broken where there is no data and where a bearing wraps.
        let wraps = dimension == Dimension::Bearing;
        for (line, samples) in lines.iter().zip(&sampled) {
            let stroke = egui::Stroke::new(1.5, line.color.unwrap_or(self.color));
            let mut run: Vec<egui::Pos2> = Vec::new();
            let mut previous: Option<f64> = None;
            let flush = |run: &mut Vec<egui::Pos2>| {
                if run.len() > 1 && line.derived {
                    painter.extend(egui::Shape::dashed_line(run, stroke, 5.0, 3.0));
                } else if run.len() > 1 {
                    painter.add(egui::Shape::line(std::mem::take(run), stroke));
                }
                run.clear();
            };
            for (i, sample) in samples.iter().enumerate() {
                match *sample {
                    Some(value) => {
                        if wraps && previous.is_some_and(|last| (value - last).abs() > 180.0) {
                            flush(&mut run);
                        }
                        run.push(egui::pos2(plot.left() + plot.width() * i as f32 / count as f32, y_at(value)));
                        previous = Some(value);
                    }
                    None => {
                        flush(&mut run);
                        previous = None;
                    }
                }
            }
            flush(&mut run);
        }

        // Where playback is, and each object's value there.
        let reading = |value: Option<f64>| value.map_or("—".to_owned(), |value| number(value, high - low));
        let marked = |line: &Line<'_>, text: String| if line.derived { format!("≈{text}") } else { text };
        if (start..=end).contains(&self.now) {
            let x = x_at(self.now);
            painter.line_segment([egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())], egui::Stroke::new(1.0, text));
            for line in &lines {
                if let Some(value) = value_at(line, self.now) {
                    painter.circle_filled(egui::pos2(x, y_at(value)), 3.0, line.color.unwrap_or(self.color));
                }
            }
        }
        // Right-aligned, last object outermost.
        let mut right = rect.right() - 6.0;
        for line in lines.iter().rev() {
            let color = line.color.unwrap_or(text);
            let drawn = painter.text(
                egui::pos2(right, rect.top() + 4.0),
                egui::Align2::RIGHT_TOP,
                marked(line, reading(value_at(line, self.now))),
                font.clone(),
                color,
            );
            right = drawn.left() - 10.0;
        }

        // Time axis.
        let clock = |time: f64| format_duration(time - self.origin);
        painter.text(egui::pos2(plot.left(), rect.bottom() - 2.0), egui::Align2::LEFT_BOTTOM, clock(start), font.clone(), weak);
        painter.text(egui::pos2(plot.right(), rect.bottom() - 2.0), egui::Align2::RIGHT_BOTTOM, clock(end), font.clone(), weak);

        // Pointer: read values off the curves, click or drag to go there.
        let pointer = response.hover_pos().or(response.interact_pointer_pos())?;
        let time = time_at(pointer.x);
        let x = x_at(time);
        painter.line_segment([egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())], egui::Stroke::new(1.0, weak));
        let values: Vec<String> = lines.iter().map(|line| marked(line, reading(value_at(line, time)))).collect();
        let hint = format!("{}  {} {unit}", clock(time), values.join(" / "));
        let after = x <= plot.center().x;
        let (align, offset) = if after { (egui::Align2::LEFT_BOTTOM, 5.0) } else { (egui::Align2::RIGHT_BOTTOM, -5.0) };
        let anchor = egui::pos2(x + offset, rect.bottom() - 2.0);
        // Over the time labels, so on a backing of its own.
        let galley = painter.layout_no_wrap(hint, font, text);
        let backing = align.anchor_size(anchor, galley.size());
        painter.rect_filled(backing.expand(2.0), 2.0, visuals.extreme_bg_color);
        painter.galley(backing.min, galley, text);
        (response.clicked() || response.dragged()).then_some(time)
    }
}

/// Vertical extent for `samples`, padded so the curves do not touch the edges.
fn value_range(samples: impl Iterator<Item = Option<f64>>, dimension: Dimension) -> Option<(f64, f64)> {
    let mut values = samples.flatten();
    let first = values.next()?;
    if dimension == Dimension::Bearing {
        return Some((0.0, 360.0));
    }
    let (low, high) = values.fold((first, first), |(low, high), value| (low.min(value), high.max(value)));
    // A flat line gets a band around it rather than a zero-height axis.
    let pad = ((high - low) * 0.08).max(high.abs().max(low.abs()) * 0.01).max(0.5);
    Some((low - pad, high + pad))
}

/// `value` with as many decimals as an axis spanning `extent` calls for.
fn number(value: f64, extent: f64) -> String {
    match extent {
        e if e >= 50.0 => format!("{value:.0}"),
        e if e >= 5.0 => format!("{value:.1}"),
        _ => format!("{value:.2}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_padded_and_bearings_fixed() {
        let samples = [Some(100.0), None, Some(300.0)];
        let (low, high) = value_range(samples.into_iter(), Dimension::Altitude).unwrap();
        assert!(low < 100.0 && high > 300.0 && high - low < 250.0);
        assert_eq!(value_range(samples.into_iter(), Dimension::Bearing), Some((0.0, 360.0)));
        // Constant data still has height; no data has no range.
        let (low, high) = value_range([Some(0.0), Some(0.0)].into_iter(), Dimension::G).unwrap();
        assert!(high - low >= 1.0);
        assert_eq!(value_range([None, None].into_iter(), Dimension::Speed), None);
    }

    #[test]
    fn decimals_follow_the_axis_extent() {
        assert_eq!(number(12345.678, 4000.0), "12346");
        assert_eq!(number(12.345, 20.0), "12.3");
        assert_eq!(number(0.8567, 0.4), "0.86");
    }
}
