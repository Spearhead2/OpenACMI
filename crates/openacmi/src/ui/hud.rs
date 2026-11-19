//! A head-up display drawn over the cockpit view: an imitation of the
//! symbology a fighter's HUD puts in front of the pilot, built from the
//! recording's data for the primary object.
//!
//! The whole display is drawn on one flat sheet, the [`Glass`], fixed to
//! the aircraft a little way ahead of the eye and square to the nose, just
//! as the real thing is a pane of glass bolted to the airframe. Every line
//! and every piece of text is laid out on that sheet and then seen through
//! the camera. Turning the head, rolling or zooming therefore moves the
//! display as one rigid object: nothing shifts against anything else, and
//! text turns and scales with the lines around it.
//!
//! Positions on the glass are in *HUD units*, with the boresight (where
//! the nose points) at the origin, x to the right wing and y downwards.
//! One unit is a fixed angle ([`RADIANS_PER_UNIT`], a pixel of a 900-pixel
//! view at a 45° field of view), so the display has a real angular size
//! and zooming in magnifies it together with the world.
//!
//! The pitch ladder, the flight path marker and the target box are
//! *conformal*: each is placed where the line of sight to the real thing
//! pierces the glass, so it lies over what it stands for from the pilot's
//! seat. Tapes and readouts are simply positioned on the glass.
//!
//! To add a readout, give it a place in [`paint`]; values come from
//! [`Quantity::read`], which also says when a value is an estimate (shown
//! with a leading `≈`).

use acmi_core::geo::{self, LocalFrame, METERS_PER_NM};
use acmi_core::{derive, Category, Object};
use glam::{DVec3, Vec2};

use crate::app::App;
use crate::config::Units;
use crate::quantity::{Dimension, PairQuantity, Quantity};

/// Angle one HUD unit subtends at the boresight (as a tangent).
const RADIANS_PER_UNIT: f64 = 0.000_920_5;
/// Cosine of the widest angle from the view's centre at which a point of
/// the glass is still drawn (about 75 degrees).
const MIN_FACING: f64 = 0.26;
/// Tallest text drawn, in screen pixels.
const MAX_TEXT_PIXELS: f32 = 160.0;
/// How far along a line of sight a point is taken to find it on screen.
const PROJECTION_DISTANCE_M: f64 = 1.0e7;
/// Half-size of the square the conformal symbols are confined to, in units.
const FIELD: f32 = 300.0;
/// Degrees of pitch ladder shown either side of the current pitch.
const LADDER_RANGE_DEG: f64 = 13.5;
/// Radius of a weapon's reticle circle, in units (about 10 degrees).
const SEEKER_RADIUS: f32 = 190.0;
/// Radius of the open middle its crosshair leaves.
const SEEKER_OPEN_RADIUS: f32 = 70.0;
/// Distance of a weapon's speed and altitude readouts from the nose.
const SEEKER_READOUT_X: f32 = 250.0;
/// A point on the glass, in HUD units.
type P = egui::Pos2;

/// Draws the HUD of the primary object over `rect`, the 3D view.
pub fn paint(app: &App, painter: &egui::Painter, rect: egui::Rect) {
    let settings = &app.settings.hud;
    let recording = &app.session.recording;
    let time = app.session.timeline.time;
    let Some(object) = app.session.primary.and_then(|handle| recording.get(handle)) else { return };
    let Some(pose) = object.pose_at(time).filter(|_| object.is_active_at(time)) else { return };

    let position = pose.world_position();
    let frame = LocalFrame::at(position);
    let attitude = geo::orientation(position, pose.heading as f64, pose.pitch as f64, pose.roll as f64);
    let glass = Glass {
        painter: painter.with_clip_rect(rect),
        camera: &app.camera,
        rect,
        nose: attitude * DVec3::NEG_Z,
        right: attitude * DVec3::X,
        up: attitude * DVec3::Y,
        color: settings.color.to_egui(),
        clip: None,
    };
    // Looking away from the nose altogether: nothing of the glass is in view.
    if glass.screen(P::ZERO).is_none() {
        return;
    }
    // Unit vector at an azimuth and elevation, in degrees, from the aircraft.
    let direction = |azimuth: f64, elevation: f64| {
        let (az, el) = (azimuth.to_radians(), elevation.to_radians());
        frame.north * (el.cos() * az.cos()) + frame.east * (el.cos() * az.sin()) + frame.up * el.sin()
    };
    // The 2D layout can be made larger or smaller; angles cannot.
    let scale = settings.scale.clamp(0.4, 3.0);
    let at = |x: f32, y: f32| egui::pos2(x * scale, y * scale);
    let units = app.settings.general.units;
    let (heading, pitch, roll) = (pose.heading as f64, pose.pitch as f64, pose.roll as f64);

    // ── Conformal symbols, confined to the HUD's field ──────────────────────
    let field = egui::Rect::from_center_size(P::ZERO, egui::Vec2::splat(FIELD * 2.0));
    let read = |quantity: Quantity| {
        let reading = quantity.read(object, time)?;
        let (factor, _) = quantity.dimension().display(units);
        Some((reading.value * factor, reading.is_derived()))
    };
    // Airspeed: indicated where there is one, else calibrated.
    let speed = read(Quantity::Ias).or_else(|| read(Quantity::Cas));
    let (_, speed_unit) = Dimension::Speed.display(units);
    let altitude = read(Quantity::AltitudeMsl);
    let (_, altitude_unit) = Dimension::Altitude.display(units);

    // Flight path marker: where the object is actually going.
    let velocity = derive::motion(&object.track, time).map(|motion| motion.velocity).filter(|v| v.length() > 10.0);
    if let Some(marker) = velocity.and_then(|velocity| glass.toward(velocity)).filter(|marker| field.contains(*marker)) {
        glass.circle(marker, 7.0);
        glass.line(marker + egui::vec2(-20.0, 0.0), marker + egui::vec2(-7.0, 0.0));
        glass.line(marker + egui::vec2(7.0, 0.0), marker + egui::vec2(20.0, 0.0));
        glass.line(marker + egui::vec2(0.0, -7.0), marker + egui::vec2(0.0, -15.0));
    }

    // Target designator: a box on the secondary object, or a line towards it.
    let target = app.session.secondary.and_then(|handle| recording.get(handle)).filter(|target| target.is_active_at(time));
    if let Some(to_target) = target.and_then(|target| target.pose_at(time)).map(|pose| pose.world_position() - app.camera.eye) {
        match glass.toward(to_target).filter(|point| field.contains(*point)) {
            Some(point) => glass.rect(egui::Rect::from_center_size(point, egui::Vec2::splat(30.0))),
            None => {
                // Out of the field: point the way to turn, by where the
                // target lies around the nose.
                let way = egui::vec2(to_target.dot(glass.right) as f32, -to_target.dot(glass.up) as f32).normalized();
                if way.length() > 0.5 {
                    glass.line(P::ZERO + way * 14.0, P::ZERO + way * 60.0);
                }
            }
        }
    }

    // A weapon gets a seeker's view instead of a pilot's: a reticle around
    // the nose and the two numbers that matter.
    if object.category() == Category::Weapon {
        reticle(&glass, scale);
        // Mach is the number that matters for a missile; airspeed goes under it.
        readout(&glass, at(-SEEKER_READOUT_X, 0.0), read(Quantity::Mach), 2, "MACH", scale);
        let airspeed = match speed {
            Some((value, derived)) => format!("{}{value:.0} {}", if derived { "≈" } else { "" }, speed_unit.to_uppercase()),
            None => format!("--- {}", speed_unit.to_uppercase()),
        };
        glass.text(at(-SEEKER_READOUT_X, 17.0), egui::Align2::CENTER_TOP, airspeed, 12.0 * scale);
        readout(&glass, at(SEEKER_READOUT_X, 0.0), altitude, 0, &altitude_unit.to_uppercase(), scale);
        return;
    }

    let conformal = Glass { clip: Some(field), ..glass.clone() };
    pitch_ladder(&conformal, &direction, heading, pitch);

    // Gun cross: where the nose points.
    for (from, to) in [((-9.0, 0.0), (-3.0, 0.0)), ((3.0, 0.0), (9.0, 0.0)), ((0.0, -9.0), (0.0, -3.0)), ((0.0, 3.0), (0.0, 9.0))] {
        glass.line(egui::pos2(from.0, from.1), egui::pos2(to.0, to.1));
    }

    // ── Tapes and readouts ──────────────────────────────────────────────────
    heading_tape(&glass, at(0.0, -245.0), heading, scale);
    bank_scale(&glass, at(0.0, 60.0), roll, scale);
    // Estimates carry a leading ≈.
    let shown = |value: Option<(f64, bool)>, text: &dyn Fn(f64) -> String| match value {
        Some((value, derived)) => format!("{}{}", if derived { "≈" } else { "" }, text(value)),
        None => "---".to_owned(),
    };
    let line_of_text = |x: f32, y: f32, text: String| {
        glass.text(at(x, y), egui::Align2::LEFT_TOP, text, 13.0 * scale);
    };

    tape(&glass, at(-250.0, 0.0), speed, scale, Tape { tick: 10.0, label_every: 5, side: -1.0, divide_labels_by: 10.0 });
    glass.text(at(-250.0, -118.0), egui::Align2::CENTER_BOTTOM, speed_unit.to_uppercase(), 11.0 * scale);

    tape(&glass, at(250.0, 0.0), altitude, scale, Tape { tick: 100.0, label_every: 5, side: 1.0, divide_labels_by: 1000.0 });
    glass.text(at(250.0, -118.0), egui::Align2::CENTER_BOTTOM, altitude_unit.to_uppercase(), 11.0 * scale);

    // Height above the ground: recorded, else from the streamed terrain.
    let (altitude_factor, _) = Dimension::Altitude.display(units);
    let above_ground = read(Quantity::AltitudeAgl)
        .or_else(|| app.terrain_height(pose.lat, pose.lon).map(|ground| ((pose.alt - ground) * altitude_factor, true)));
    line_of_text(215.0, 128.0, format!("R {}", shown(above_ground, &|v| format!("{v:.0}"))));
    line_of_text(215.0, 146.0, format!("VS {}", shown(read(Quantity::VerticalSpeed), &|v| format!("{v:+.0}"))));

    // Lower left: Mach, load factor and angle of attack.
    line_of_text(-285.0, 128.0, format!("M {}", shown(read(Quantity::Mach), &|v| format!("{v:.2}"))));
    line_of_text(-285.0, 146.0, format!("G {}", shown(read(Quantity::G), &|v| format!("{v:.1}"))));
    line_of_text(-285.0, 164.0, format!("α {}", shown(read(Quantity::Aoa), &|v| format!("{v:.1}"))));

    // Lower right: the target's range and closure.
    if let Some(target) = target {
        let range = PairQuantity::Distance.sample(object, target, time);
        let closure = PairQuantity::ClosureRate.sample(object, target, time);
        let (speed_factor, _) = Dimension::Speed.display(units);
        let range_text = match (range, units) {
            (Some(range), Units::Aviation) => format!("{:.1} NM", range / METERS_PER_NM),
            (Some(range), Units::Metric) => format!("{:.1} KM", range / 1000.0),
            (None, _) => "---".to_owned(),
        };
        line_of_text(215.0, 172.0, format!("F {range_text}"));
        line_of_text(215.0, 190.0, format!("VC {}", closure.map_or("---".to_owned(), |closure| format!("{:+.0}", closure * speed_factor))));
    }
}

/// The sheet the HUD is drawn on: fixed to the aircraft, square to its
/// nose, one unit of distance ahead of the eye. Drawing calls take points
/// on the sheet in HUD units and put them on screen through the camera.
#[derive(Clone)]
struct Glass<'a> {
    painter: egui::Painter,
    camera: &'a acmi_render::Camera,
    /// The 3D view on screen.
    rect: egui::Rect,
    // The aircraft's axes in the world.
    nose: DVec3,
    right: DVec3,
    up: DVec3,
    color: egui::Color32,
    /// Lines are cut off at this rectangle of the glass, and text outside
    /// it is left out.
    clip: Option<egui::Rect>,
}

impl Glass<'_> {
    const SHADOW: egui::Color32 = egui::Color32::from_black_alpha(120);

    /// Where a point of the glass appears on screen, if it is in front of the eye.
    fn screen(&self, point: P) -> Option<egui::Pos2> {
        let ray = self.nose + (self.right * point.x as f64 - self.up * point.y as f64) * RADIANS_PER_UNIT;
        // Seen at a grazing angle the glass stretches without limit across
        // the screen; past this it is treated as out of view.
        if ray.normalize().dot(self.camera.forward()) < MIN_FACING {
            return None;
        }
        let size = Vec2::new(self.rect.width(), self.rect.height());
        // Projected far along the ray: the camera does not project points
        // nearer than its near plane, which is tens of metres out at
        // altitude, and a point at the glass's own distance would fall
        // inside it as soon as the view turned away from the nose.
        self.camera.project(self.camera.eye + ray * PROJECTION_DISTANCE_M, size).map(|p| self.rect.min + egui::vec2(p.x, p.y))
    }

    /// Where the line of sight along `direction` pierces the glass; `None`
    /// for directions beside or behind it.
    fn toward(&self, direction: DVec3) -> Option<P> {
        let ahead = direction.dot(self.nose);
        if ahead <= direction.length() * 0.2 {
            return None;
        }
        let scale = 1.0 / (ahead * RADIANS_PER_UNIT);
        Some(egui::pos2((direction.dot(self.right) * scale) as f32, (-direction.dot(self.up) * scale) as f32))
    }

    /// Screen pixels per HUD unit around a point, and the angle on screen
    /// of the glass's x axis there: how to size and turn text.
    fn lay(&self, point: P) -> Option<(f32, f32)> {
        let (here, beside) = (self.screen(point)?, self.screen(point + egui::vec2(10.0, 0.0))?);
        let along = beside - here;
        let pixels = along.length() / 10.0;
        (pixels.is_finite() && pixels > 0.0).then(|| (pixels, along.angle()))
    }

    fn stroke_width(&self, point: P) -> f32 {
        // Zooming in magnifies the glass; strokes stop thickening before they
        // turn into bars.
        self.lay(point).map_or(1.0, |(pixels, _)| (1.6 * pixels).clamp(1.0, 3.2))
    }

    fn line(&self, from: P, to: P) {
        let Some((from, to)) = clip_segment(from, to, self.clip) else { return };
        let (Some(a), Some(b)) = (self.screen(from), self.screen(to)) else { return };
        let width = self.stroke_width(from);
        self.painter.line_segment([a, b], (width + 2.0, Self::SHADOW));
        self.painter.line_segment([a, b], (width, self.color));
    }

    /// A line of `dashes` dashes with gaps of the same length.
    fn dashed(&self, from: P, to: P, dashes: usize) {
        let pieces = (dashes * 2 - 1) as f32;
        for dash in 0..dashes {
            let (start, end) = (dash as f32 * 2.0 / pieces, (dash as f32 * 2.0 + 1.0) / pieces);
            self.line(from + (to - from) * start, from + (to - from) * end);
        }
    }

    /// A circle on the glass: an ellipse on screen when seen at an angle.
    fn circle(&self, centre: P, radius: f32) {
        // Enough sides for a large circle not to show its corners.
        let sides = if radius > 40.0 { 72 } else { 24 };
        let point = |i: usize| {
            let angle = i as f32 / sides as f32 * std::f32::consts::TAU;
            centre + egui::vec2(angle.cos(), angle.sin()) * radius
        };
        for i in 0..sides {
            self.line(point(i), point(i + 1));
        }
    }

    fn rect(&self, rect: egui::Rect) {
        let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
        for i in 0..4 {
            self.line(corners[i], corners[(i + 1) % 4]);
        }
    }

    /// Text of height `size` units anchored at `position`, turned and
    /// scaled to lie on the glass. Returns the space it takes, in units.
    fn text(&self, position: P, align: egui::Align2, text: impl ToString, size: f32) -> egui::Rect {
        let text = text.to_string();
        // Width of a monospace glyph is about 0.6 of its height.
        let extent = egui::vec2(text.chars().count() as f32 * size * 0.6, size * 1.15);
        let space = align.anchor_size(position, extent);
        if self.clip.is_some_and(|clip| !clip.contains(space.center())) {
            return space;
        }
        let (Some(anchor), Some((pixels, angle))) = (self.screen(position), self.lay(position)) else { return space };
        // Text too large to be read as text (deep zoom, or a steep angle)
        // is left out rather than rendered as a wall of pixels.
        let height = size * pixels;
        if height > MAX_TEXT_PIXELS {
            return space;
        }
        let font = egui::FontId::monospace(height.max(1.0));
        let galley = self.painter.layout_no_wrap(text, font, self.color);
        // The galley is drawn from its top-left corner, turned about it.
        let offset = egui::vec2(align.x().to_factor() * galley.size().x, align.y().to_factor() * galley.size().y);
        let rotation = egui::emath::Rot2::from_angle(angle);
        let corner = anchor - rotation * offset;
        let shape = |at: egui::Pos2, color: egui::Color32| {
            egui::epaint::TextShape::new(at, galley.clone(), color).with_override_text_color(color).with_angle(angle)
        };
        self.painter.add(shape(corner + egui::vec2(1.0, 1.0), Self::SHADOW));
        self.painter.add(shape(corner, self.color));
        space
    }
}

/// The part of a segment inside `clip`, if any (Liang–Barsky).
fn clip_segment(from: P, to: P, clip: Option<egui::Rect>) -> Option<(P, P)> {
    let Some(clip) = clip else { return Some((from, to)) };
    let delta = to - from;
    let (mut enter, mut leave) = (0.0f32, 1.0f32);
    for (step, start, low, high) in [(delta.x, from.x, clip.left(), clip.right()), (delta.y, from.y, clip.top(), clip.bottom())] {
        if step.abs() < 1e-6 {
            if start < low || start > high {
                return None;
            }
            continue;
        }
        let (a, b) = ((low - start) / step, (high - start) / step);
        enter = enter.max(a.min(b));
        leave = leave.min(a.max(b));
    }
    (enter <= leave).then(|| (from + delta * enter, from + delta * leave))
}

/// Pitch bars every 5 degrees around the horizon line, each lying at its
/// true elevation: solid above the horizon, dashed below, with ticks at the
/// outer ends pointing towards the horizon.
fn pitch_ladder(glass: &Glass<'_>, direction: &impl Fn(f64, f64) -> DVec3, heading: f64, pitch: f64) {
    for step in -17..=17 {
        let elevation = step as f64 * 5.0;
        if (elevation - pitch).abs() > LADDER_RANGE_DEG {
            continue;
        }
        // Constant width on the glass: meridians converge away from the horizon.
        let widen = 1.0 / elevation.to_radians().cos();
        let (inner, outer) = if step == 0 { (2.5, 13.0) } else { (2.5, 7.0) };
        for side in [-1.0, 1.0] {
            let point = |azimuth: f64, lift: f64| glass.toward(direction(heading + side * azimuth * widen, elevation + lift));
            let (Some(near), Some(far)) = (point(inner, 0.0), point(outer, 0.0)) else { continue };
            if step < 0 {
                glass.dashed(near, far, 3);
            } else {
                glass.line(near, far);
            }
            if step != 0 {
                if let Some(tick) = point(outer, -0.8 * elevation.signum()) {
                    glass.line(far, tick);
                }
                let outward = (far - near).normalized() * 14.0;
                glass.text(far + outward, egui::Align2::CENTER_CENTER, elevation.abs() as i32, 12.0);
            }
        }
    }
}

/// A strip of headings across the top, the current one boxed in the middle.
fn heading_tape(glass: &Glass<'_>, centre: P, heading: f64, scale: f32) {
    const UNITS_PER_DEGREE: f32 = 6.0;
    const HALF_SPAN_DEG: f64 = 25.0;
    let first = ((heading - HALF_SPAN_DEG) / 5.0).ceil() as i32;
    let last = ((heading + HALF_SPAN_DEG) / 5.0).floor() as i32;
    for step in first..=last {
        let degrees = step as f64 * 5.0;
        let x = centre.x + (degrees - heading) as f32 * UNITS_PER_DEGREE * scale;
        let major = step % 2 == 0;
        glass.line(egui::pos2(x, centre.y), egui::pos2(x, centre.y - if major { 10.0 } else { 5.0 } * scale));
        if major {
            let tens = (degrees.rem_euclid(360.0) / 10.0).round() as i32 % 36;
            glass.text(egui::pos2(x, centre.y - 13.0 * scale), egui::Align2::CENTER_BOTTOM, format!("{tens:02}"), 12.0 * scale);
        }
    }
    // Caret, and the exact heading in a box under the strip.
    glass.line(centre + egui::vec2(0.0, 3.0) * scale, centre + egui::vec2(-6.0, 12.0) * scale);
    glass.line(centre + egui::vec2(0.0, 3.0) * scale, centre + egui::vec2(6.0, 12.0) * scale);
    let shown = format!("{:03.0}", heading.rem_euclid(360.0) % 360.0);
    let text = glass.text(centre + egui::vec2(0.0, 17.0) * scale, egui::Align2::CENTER_TOP, shown, 14.0 * scale);
    glass.rect(text.expand2(egui::vec2(5.0, 2.0) * scale));
}

/// Bank angle: fixed ticks on an arc below the boresight and a pointer
/// that swings with the wings.
fn bank_scale(glass: &Glass<'_>, centre: P, roll: f64, scale: f32) {
    const RADIUS: f32 = 150.0;
    // A point on the arc `angle` degrees from straight down, at a radius.
    let on_arc = |angle: f64, radius: f32| {
        let radians = angle.to_radians() as f32;
        centre + egui::vec2(-radians.sin(), radians.cos()) * radius * scale
    };
    for (angle, length) in [(0.0, 14.0), (10.0, 7.0), (20.0, 7.0), (30.0, 14.0), (45.0, 7.0), (60.0, 14.0)] {
        for side in if angle == 0.0 { &[1.0][..] } else { &[-1.0, 1.0][..] } {
            glass.line(on_arc(angle * side, RADIUS), on_arc(angle * side, RADIUS + length));
        }
    }
    // The pointer stops at the end of the scale rather than leaving it.
    let shown = roll.clamp(-65.0, 65.0);
    let (tip, left, right) = (on_arc(shown, RADIUS - 3.0), on_arc(shown - 3.0, RADIUS - 16.0), on_arc(shown + 3.0, RADIUS - 16.0));
    glass.line(tip, left);
    glass.line(tip, right);
    glass.line(left, right);
}

/// The seeker reticle of a weapon: a large circle about the nose with a
/// crosshair reaching in from it, the middle left open for the target.
fn reticle(glass: &Glass<'_>, scale: f32) {
    let (radius, open) = (SEEKER_RADIUS * scale, SEEKER_OPEN_RADIUS * scale);
    glass.circle(P::ZERO, radius);
    for way in [egui::vec2(1.0, 0.0), egui::vec2(-1.0, 0.0), egui::vec2(0.0, 1.0), egui::vec2(0.0, -1.0)] {
        glass.line(P::ZERO + way * open, P::ZERO + way * radius);
    }
}

/// A boxed number with its unit above it.
fn readout(glass: &Glass<'_>, centre: P, value: Option<(f64, bool)>, decimals: usize, unit: &str, scale: f32) {
    let text = match value {
        Some((value, derived)) => format!("{}{value:.decimals$}", if derived { "≈" } else { "" }),
        None => "---".to_owned(),
    };
    let label = glass.text(centre, egui::Align2::CENTER_CENTER, text, 16.0 * scale);
    glass.rect(egui::Rect::from_center_size(centre, egui::vec2(label.width().max(48.0 * scale) + 10.0 * scale, 24.0 * scale)));
    glass.text(centre + egui::vec2(0.0, -16.0) * scale, egui::Align2::CENTER_BOTTOM, unit, 11.0 * scale);
}

/// Layout of a vertical tape.
struct Tape {
    /// Value between ticks, in display units.
    tick: f64,
    /// A label on every this-many ticks.
    label_every: i64,
    /// −1: scale on the right of the box (a tape on the left of the HUD);
    /// +1: the mirror image.
    side: f32,
    /// Labels show the value divided by this (speed in tens, altitude in thousands).
    divide_labels_by: f64,
}

/// A moving vertical scale with the current value boxed at its centre.
fn tape(glass: &Glass<'_>, centre: P, value: Option<(f64, bool)>, scale: f32, layout: Tape) {
    const HALF_HEIGHT: f32 = 105.0;
    const UNITS_PER_TICK: f32 = 13.0;
    let text = match value {
        Some((value, derived)) => format!("{}{value:.0}", if derived { "≈" } else { "" }),
        None => "---".to_owned(),
    };
    // The box, on the side away from the scale.
    let box_centre = centre + egui::vec2(layout.side * 34.0, 0.0) * scale;
    let label = glass.text(box_centre, egui::Align2::CENTER_CENTER, text, 15.0 * scale);
    glass.rect(egui::Rect::from_center_size(box_centre, egui::vec2(label.width().max(48.0 * scale) + 10.0 * scale, 22.0 * scale)));

    let Some((value, _)) = value else { return };
    let scale_x = centre.x - layout.side * 8.0 * scale;
    let window = egui::Rect::from_center_size(centre, egui::vec2(160.0, HALF_HEIGHT * 2.0) * scale);
    let clipped = Glass { clip: Some(window), ..glass.clone() };
    let visible = (HALF_HEIGHT / UNITS_PER_TICK) as f64 * layout.tick;
    let first = ((value - visible) / layout.tick).ceil() as i64;
    let last = ((value + visible) / layout.tick).floor() as i64;
    for step in first..=last {
        let tick_value = step as f64 * layout.tick;
        let y = centre.y - ((tick_value - value) / layout.tick) as f32 * UNITS_PER_TICK * scale;
        let major = step.rem_euclid(layout.label_every) == 0;
        let length = if major { 10.0 } else { 5.0 } * scale;
        clipped.line(egui::pos2(scale_x, y), egui::pos2(scale_x - layout.side * length, y));
        // Labels would collide with the box in the middle.
        if major && (y - centre.y).abs() > 16.0 * scale {
            let scaled = tick_value / layout.divide_labels_by;
            let label = if layout.divide_labels_by >= 1000.0 { format!("{scaled:.1}") } else { format!("{scaled:.0}") };
            let align = if layout.side < 0.0 { egui::Align2::LEFT_CENTER } else { egui::Align2::RIGHT_CENTER };
            clipped.text(egui::pos2(scale_x - layout.side * 14.0 * scale, y), align, label, 11.0 * scale);
        }
    }
}

/// Whether `object` is something a HUD makes sense for.
pub fn applies_to(object: &Object) -> bool {
    matches!(object.category(), Category::Aircraft | Category::Weapon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_are_cut_at_the_clip_rectangle() {
        let clip = Some(egui::Rect::from_min_max(egui::pos2(-10.0, -10.0), egui::pos2(10.0, 10.0)));
        let cut = |from: (f32, f32), to: (f32, f32)| clip_segment(egui::pos2(from.0, from.1), egui::pos2(to.0, to.1), clip);
        // Crossing the whole rectangle: both ends are cut.
        assert_eq!(cut((-30.0, 0.0), (30.0, 0.0)), Some((egui::pos2(-10.0, 0.0), egui::pos2(10.0, 0.0))));
        // Wholly inside: untouched. Wholly outside: nothing.
        assert_eq!(cut((-5.0, 5.0), (5.0, -5.0)), Some((egui::pos2(-5.0, 5.0), egui::pos2(5.0, -5.0))));
        assert_eq!(cut((20.0, -30.0), (20.0, 30.0)), None);
        assert_eq!(cut((-30.0, 20.0), (30.0, 40.0)), None);
        // No rectangle, no cutting.
        assert!(clip_segment(egui::pos2(-99.0, 0.0), egui::pos2(99.0, 0.0), None).is_some());
    }
}
