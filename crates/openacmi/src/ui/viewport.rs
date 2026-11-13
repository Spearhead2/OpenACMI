//! The 3D view: input handling, the rendered scene and everything drawn on
//! top of it (labels, selection markers, status text).

use acmi_core::geo::{self, EARTH_RADIUS_M};
use acmi_core::ObjectHandle;
use glam::{Vec2, Vec3};

use super::{format, panels};
use crate::app::App;
use crate::camera_controller::{CameraInput, CameraMode};

/// How close (in points) a click must be to an object to select it.
const PICK_RADIUS: f32 = 14.0;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
    let ctx = ui.ctx().clone();
    let size = Vec2::new(rect.width().max(1.0), rect.height().max(1.0));

    // ── Time ─────────────────────────────────────────────────────────────────
    let dt = ctx.input(|i| i.unstable_dt).min(0.1);
    handle_keys(app, &ctx);
    let range = app.session.recording.time_range();
    app.session.timeline.advance(dt as f64, range);
    if app.session.timeline.playing || app.session.timeline.follow_live {
        ctx.request_repaint();
    }

    // ── Camera ───────────────────────────────────────────────────────────────
    let input = gather_camera_input(app, &ctx, &response, dt, size.y);
    let target = app.track_target();
    app.controller.update(&mut app.camera, &input, target, &app.settings.camera);

    // ── Scene ────────────────────────────────────────────────────────────────
    let painter = ui.painter_at(rect);
    match app.render_scene(rect.size(), ctx.pixels_per_point()) {
        Some(texture) => {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(texture, rect, uv, egui::Color32::WHITE);
        }
        None => {
            painter.rect_filled(rect, 0.0, egui::Color32::BLACK);
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "3D view unavailable (no compatible graphics adapter)",
                egui::FontId::proportional(16.0),
                egui::Color32::GRAY,
            );
        }
    }

    // ── Overlays ─────────────────────────────────────────────────────────────
    let to_screen = |world| app.camera.project(world, size).map(|p| rect.min + egui::vec2(p.x, p.y));
    let pointer = response.hover_pos();
    let hovered = pointer.and_then(|pointer| pick(app, pointer, &to_screen));

    app.ui.labels.refresh(&app.session);
    if app.settings.objects.show_labels {
        paint_labels(app, &painter, rect, &to_screen);
    }
    for (handle, color) in [
        (app.session.secondary, egui::Color32::from_rgb(255, 204, 51)),
        (app.session.primary, egui::Color32::WHITE),
        (hovered, egui::Color32::from_white_alpha(140)),
    ] {
        let position = handle.and_then(|h| app.scene.visible.iter().find(|v| v.handle == h)).and_then(|v| to_screen(v.position));
        if let Some(position) = position {
            painter.circle_stroke(position, PICK_RADIUS, (1.5, color));
        }
    }
    paint_status(app, &painter, rect, &ctx);
    paint_progress(app, &painter, rect);
    if app.terrain_busy() {
        // Keep drawing while terrain streams in, so it appears without input.
        ctx.request_repaint();
    }

    // ── Picking ──────────────────────────────────────────────────────────────
    if response.clicked() {
        let secondary = ctx.input(|i| i.modifiers.ctrl || i.modifiers.shift);
        match (hovered, secondary) {
            (Some(handle), false) => app.select_primary(Some(handle)),
            (Some(handle), true) => app.session.secondary = Some(handle),
            (None, _) => {
                // Clicking the globe reports the coordinates under the pointer.
                app.ui.clicked_geo = pointer.and_then(|pointer| {
                    let pixel = Vec2::new(pointer.x - rect.min.x, pointer.y - rect.min.y);
                    let direction = app.camera.pixel_ray(pixel, size);
                    let distance = geo::ray_sphere(app.camera.eye, direction, EARTH_RADIUS_M)?;
                    let (lat, lon, _) = geo::world_to_geo(app.camera.eye + direction * distance);
                    Some((lat, lon))
                });
            }
        }
    }
    if hovered.is_some() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    // ── Context menu ─────────────────────────────────────────────────────────
    // Right-clicking an object opens a menu for it; elsewhere, nothing.
    if response.secondary_clicked() {
        app.ui.context_object = hovered;
    }
    if let Some(handle) = app.ui.context_object.filter(|&handle| app.session.recording.get(handle).is_some()) {
        response.context_menu(|ui| {
            ui.weak(super::track_label(&app.session.recording, handle));
            ui.separator();
            if ui.button("Edit Object…").clicked() {
                app.ui.edit_object = super::edit_object::EditObject::open(app, handle);
                ui.close();
            }
            ui.separator();
            if ui.button("Select as Primary").clicked() {
                app.select_primary(Some(handle));
                ui.close();
            }
            if ui.button("Select as Secondary").clicked() {
                app.session.secondary = Some(handle);
                ui.close();
            }
        });
    }

    panels::show_floating(app, &ctx, rect);
}

/// Keyboard shortcuts that are not camera movement.
fn handle_keys(app: &mut App, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    let range = app.session.recording.time_range();
    let (space, cycle, back, forward, home, end) = ctx.input(|i| {
        (
            i.key_pressed(egui::Key::Space),
            i.key_pressed(egui::Key::C),
            i.key_pressed(egui::Key::Comma),
            i.key_pressed(egui::Key::Period),
            i.key_pressed(egui::Key::Home),
            i.key_pressed(egui::Key::End),
        )
    });
    if space {
        app.session.timeline.toggle_play(range);
    }
    if cycle {
        let next = app.controller.mode().next();
        app.set_camera_mode(next);
    }
    if back || forward {
        step_time(app, if forward { 1 } else { -1 });
    }
    if let (Some((start, finish)), true) = (range, home || end) {
        app.session.timeline.seek(if home { start } else { finish }, range);
    }
}

/// Moves to the previous/next recorded sample of the primary object, or by
/// one second when nothing is selected.
fn step_time(app: &mut App, direction: i32) {
    let time = app.session.timeline.time;
    let samples =
        app.session.primary.and_then(|h| app.session.recording.get(h)).map(|o| o.track.times()).filter(|t| !t.is_empty());
    let target = match samples {
        Some(times) => {
            let index = if direction > 0 {
                times.partition_point(|&t| t <= time).min(times.len() - 1)
            } else {
                times.partition_point(|&t| t < time).saturating_sub(1)
            };
            times[index]
        }
        None => time + direction as f64,
    };
    app.session.timeline.playing = false;
    app.session.timeline.seek(target, app.session.recording.time_range());
}

fn gather_camera_input(
    app: &mut App,
    ctx: &egui::Context,
    response: &egui::Response,
    dt: f32,
    viewport_height: f32,
) -> CameraInput {
    let (eye_latitude, eye_longitude, _) = geo::world_to_geo(app.camera.eye);
    let ground_height = app.terrain_height(eye_latitude, eye_longitude).unwrap_or(0.0);
    let mut input = CameraInput { dt, viewport_height, ground_height, ..Default::default() };
    let shift = ctx.input(|i| i.modifiers.shift);
    let delta = response.drag_delta();

    if response.dragged_by(egui::PointerButton::Middle) {
        // Scrub through time.
        let seconds_per_point = (app.session.recording.duration() / 2000.0).max(0.02);
        let time = app.session.timeline.time + delta.x as f64 * seconds_per_point;
        app.session.timeline.seek(time, app.session.recording.time_range());
    } else if response.dragged_by(egui::PointerButton::Secondary) && shift {
        // Move the sun by hand.
        let lighting = &mut app.settings.lighting;
        lighting.follow_recording_time = false;
        lighting.sun_longitude_deg = (lighting.sun_longitude_deg - delta.x * 0.3 + 540.0).rem_euclid(360.0) - 180.0;
        lighting.sun_latitude_deg = (lighting.sun_latitude_deg + delta.y * 0.3).clamp(-60.0, 60.0);
    } else if response.dragged() {
        input.drag = Vec2::new(delta.x, delta.y);
    }

    if response.contains_pointer() {
        // One wheel notch scrolls about 50 points.
        input.zoom = ctx.input(|i| i.smooth_scroll_delta.y) / 50.0;
    }

    if !ctx.egui_wants_keyboard_input() {
        let free = app.controller.mode() == CameraMode::Free;
        input.fast = shift;
        input.keys = ctx.input(|i| {
            let axis = |positive: &[egui::Key], negative: &[egui::Key]| {
                let down = |keys: &[egui::Key]| keys.iter().any(|&key| i.key_down(key)) as i32 as f32;
                down(positive) - down(negative)
            };
            use egui::Key::*;
            Vec3::new(
                axis(&[D, ArrowRight], &[A, ArrowLeft]),
                axis(&[W, ArrowUp], &[S, ArrowDown]),
                if free { axis(&[E], &[Q]) } else { axis(&[Plus, Equals], &[Minus]) },
            )
        });
        if input.keys != Vec3::ZERO {
            ctx.request_repaint();
        }
    }
    input
}

/// The visible object nearest to `pointer`, if within picking distance.
fn pick(app: &App, pointer: egui::Pos2, to_screen: &impl Fn(glam::DVec3) -> Option<egui::Pos2>) -> Option<ObjectHandle> {
    app.scene
        .visible
        .iter()
        .filter_map(|object| Some((object.handle, to_screen(object.position)?.distance(pointer))))
        .filter(|(_, distance)| *distance <= PICK_RADIUS)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(handle, _)| handle)
}

fn paint_labels(app: &App, painter: &egui::Painter, rect: egui::Rect, to_screen: &impl Fn(glam::DVec3) -> Option<egui::Pos2>) {
    /// Offset of the label from its object, with a leader line between them.
    const OFFSET: egui::Vec2 = egui::vec2(16.0, -12.0);
    let font = egui::FontId::proportional(11.5);
    let eye = app.camera.eye;
    // Beyond the horizon means hidden behind the Earth.
    let horizon = |position: glam::DVec3| position.dot(eye) >= EARTH_RADIUS_M * EARTH_RADIUS_M * 0.999;

    // Nearest first, so they win when labels compete for space.
    let mut candidates: Vec<_> = app
        .scene
        .visible
        .iter()
        .filter(|object| horizon(object.position))
        .filter_map(|object| Some((object, app.ui.labels.get(object.handle)?, to_screen(object.position)?)))
        .filter(|(_, _, anchor)| rect.expand(40.0).contains(*anchor))
        .collect();
    candidates.sort_unstable_by(|a, b| a.0.position.distance_squared(eye).total_cmp(&b.0.position.distance_squared(eye)));

    let selected = [app.session.primary, app.session.secondary];
    let mut placed: Vec<egui::Rect> = Vec::new();
    for (object, text, anchor) in candidates {
        let is_selected = selected.contains(&Some(object.handle));
        if !is_selected && placed.len() >= app.settings.objects.max_labels {
            continue;
        }
        let colors = app.palette.for_object(app.session.recording.object(object.handle));
        let galley = painter.layout_no_wrap(text.to_owned(), font.clone(), colors.label_text.to_egui());
        let corner = anchor + OFFSET;
        let text_rect = egui::Rect::from_min_size(egui::pos2(corner.x, corner.y - galley.size().y), galley.size());
        let frame = text_rect.expand2(egui::vec2(4.0, 2.0));
        // Labels never overlap: the nearer object keeps its label.
        if !is_selected && placed.iter().any(|other| other.intersects(frame)) {
            continue;
        }
        placed.push(frame);

        painter.line_segment([anchor, frame.left_bottom()], (1.0, colors.label_line.to_egui()));
        painter.rect_filled(frame, 3.0, colors.label_background.with_alpha(215).to_egui());
        let border_width = if is_selected { 2.0 } else { 1.0 };
        painter.rect_stroke(frame, 3.0, (border_width, colors.label_border.to_egui()), egui::StrokeKind::Outside);
        painter.galley(text_rect.min, galley, egui::Color32::BLACK);
    }
}

/// Status lines in the corners of the view.
fn paint_status(app: &App, painter: &egui::Painter, rect: egui::Rect, ctx: &egui::Context) {
    let font = egui::FontId::monospace(12.0);
    let color = egui::Color32::from_white_alpha(200);
    let units = app.settings.general.units;
    let shadowed = |pos: egui::Pos2, align: egui::Align2, text: String| {
        painter.text(pos + egui::vec2(1.0, 1.0), align, &text, font.clone(), egui::Color32::from_black_alpha(200));
        painter.text(pos, align, text, font.clone(), color);
    };

    let (lat, lon, altitude) = geo::world_to_geo(app.camera.eye);
    let mut left = format!(
        "{}   camera {} {} at {}",
        app.controller.mode().label(),
        format::latitude(lat),
        format::longitude(lon),
        format::altitude(altitude, units)
    );
    if let Some((lat, lon)) = app.ui.clicked_geo {
        left.push_str(&format!("\nclicked {} {}", format::latitude(lat), format::longitude(lon)));
        if let Some(elevation) = app.terrain_height(lat, lon) {
            left.push_str(&format!("  elevation {}", format::altitude(elevation, units)));
        }
    }
    shadowed(rect.left_bottom() + egui::vec2(10.0, -8.0), egui::Align2::LEFT_BOTTOM, left);

    let terrain = &app.settings.terrain;
    if terrain.enabled && terrain.imagery && !terrain.imagery_attribution.is_empty() {
        let credit = format!("Imagery: {}", terrain.imagery_attribution);
        let small = egui::FontId::proportional(10.0);
        let pos = rect.right_bottom() + egui::vec2(-8.0, -6.0);
        painter.text(
            pos + egui::vec2(1.0, 1.0),
            egui::Align2::RIGHT_BOTTOM,
            &credit,
            small.clone(),
            egui::Color32::from_black_alpha(200),
        );
        painter.text(pos, egui::Align2::RIGHT_BOTTOM, credit, small, egui::Color32::from_white_alpha(170));
    }

    if app.session.recording.is_empty() && app.load_job.is_none() {
        painter.text(
            rect.center_bottom() - egui::vec2(0.0, 60.0),
            egui::Align2::CENTER_BOTTOM,
            "Open a recording with File › Open, or drop an .acmi file here",
            egui::FontId::proportional(15.0),
            egui::Color32::from_white_alpha(170),
        );
    }

    if app.user_models == 0 && !app.session.recording.is_empty() {
        painter.text(
            rect.center_top() + egui::vec2(0.0, 10.0),
            egui::Align2::CENTER_TOP,
            "Showing generic shapes: choose your model folder with File › 3D Model Folder…",
            egui::FontId::proportional(14.0),
            egui::Color32::from_rgb(255, 200, 90),
        );
    }

    if app.settings.panels.render_info {
        let frame_time = ctx.input(|i| i.stable_dt);
        let mut info = format!(
            "{:.0} fps\n{} objects shown\n{} models drawn\n{} trail segments",
            1.0 / frame_time.max(1e-4),
            app.scene.visible.len(),
            app.scene.instances.len(),
            app.scene.trail_vertex_count() / 2
        );
        if let Some(terrain) = app.terrain_stats() {
            info.push_str(&format!(
                "\n{} terrain tiles drawn, {} loaded, {} pending",
                terrain.drawn, terrain.loaded, terrain.pending
            ));
        }
        shadowed(rect.left_top() + egui::vec2(10.0, 8.0), egui::Align2::LEFT_TOP, info);
    }
}

fn paint_progress(app: &App, painter: &egui::Painter, rect: egui::Rect) {
    let Some(job) = &app.load_job else { return };
    let bar = egui::Rect::from_center_size(
        rect.center_bottom() - egui::vec2(0.0, 30.0),
        egui::vec2((rect.width() * 0.5).min(480.0), 18.0),
    );
    painter.rect_filled(bar, 4.0, egui::Color32::from_black_alpha(190));
    let mut filled = bar.shrink(2.0);
    filled.set_width(filled.width() * job.progress().clamp(0.0, 1.0));
    painter.rect_filled(filled, 3.0, egui::Color32::from_rgb(70, 140, 255));
    let name = job.path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    painter.text(
        bar.center(),
        egui::Align2::CENTER_CENTER,
        format!("Loading {name}  {:.0}%", job.progress() * 100.0),
        egui::FontId::proportional(12.0),
        egui::Color32::WHITE,
    );
}
