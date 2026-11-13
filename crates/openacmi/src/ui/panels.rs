//! Where the information panels live: docked as tabs on any side of the 3D
//! view, or floating over it as windows.
//!
//! Every panel is listed in [`PanelId`] and has a [`crate::config::PanelSlot`]
//! in the settings saying whether it is shown and where. This module draws
//! the two docks and the floating windows; what is *inside* a panel comes
//! from [`contents`].

use super::{graph, side_panel, view_options};
use crate::app::App;
use crate::config::{Dock, PanelId};

/// Distance from an edge of the 3D view within which dropping a floating
/// window docks it there.
const DOCK_ZONE: f32 = 36.0;

/// Draws the docks around the 3D view. Call before the central panel.
///
/// The side docks come first and so run the full height; the top and bottom
/// ones fit between them, directly above and below the 3D view.
pub fn show_docks(app: &mut App, ui: &mut egui::Ui) {
    for side in Dock::EDGES {
        let tabs = app.settings.panels.docked(side);
        if tabs.is_empty() {
            continue;
        }
        let panel = match side {
            Dock::Left => egui::Panel::left("dock_left"),
            Dock::Right => egui::Panel::right("dock_right"),
            Dock::Top => egui::Panel::top("dock_top"),
            _ => egui::Panel::bottom("dock_bottom"),
        };
        let (default, min) = if matches!(side, Dock::Left | Dock::Right) { (330.0, 240.0) } else { (230.0, 130.0) };
        panel.resizable(true).default_size(default).min_size(min).show(ui, |ui| dock(app, ui, side, &tabs));
    }
}

/// One dock: a tab per panel, the panel menu, then the active panel.
fn dock(app: &mut App, ui: &mut egui::Ui, side: Dock, tabs: &[PanelId]) {
    let mut active = app.settings.panels.active_tab(side).filter(|tab| tabs.contains(tab)).unwrap_or(tabs[0]);
    let mut moved = None;
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The menu first, so it keeps its place when the tabs need the room.
            ui.menu_button("…", |ui| moved = placement_menu(ui, active, side)).response.on_hover_text("Move or close this panel");
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true), |ui| {
                for &tab in tabs {
                    ui.selectable_value(&mut active, tab, tab.title());
                }
            });
        });
    });
    ui.separator();
    app.settings.panels.set_active_tab(side, active);

    ui.push_id(active, |ui| match active {
        // The view options have no scroll area of their own.
        PanelId::View => {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| contents(app, ui, active));
        }
        _ => contents(app, ui, active),
    });
    if let Some(change) = moved {
        apply(app, active, change);
    }
}

/// Draws every floating panel as a window over `viewport`, the 3D view.
pub fn show_floating(app: &mut App, ctx: &egui::Context, viewport: egui::Rect) {
    let floating: Vec<PanelId> =
        PanelId::ALL.into_iter().filter(|&id| app.settings.panels.slot(id).is(Dock::Floating)).collect();
    for (index, id) in floating.into_iter().enumerate() {
        let mut open = true;
        let mut moved = None;
        let offset = index as f32 * 28.0;
        let mut window = egui::Window::new(id.title()).id(egui::Id::unique(("floating_panel", id))).open(&mut open).collapsible(true);
        window = match id {
            // Sized by its contents, in its familiar corner.
            PanelId::View => {
                window.default_pos(viewport.right_top() + egui::vec2(-10.0, 10.0 + offset)).pivot(egui::Align2::RIGHT_TOP).resizable(false)
            }
            _ => window
                .default_pos(viewport.left_top() + egui::vec2(10.0 + offset, 10.0 + offset))
                .default_size([340.0, 460.0])
                .min_size([240.0, 160.0])
                .resizable(true),
        };
        let shown = window.show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.weak("Dock");
                for side in Dock::EDGES {
                    let hint = format!("Dock at the {} of the 3D view", side.label().to_lowercase());
                    if ui.small_button(side.label()).on_hover_text(hint).clicked() {
                        moved = Some(Change::Dock(side));
                    }
                }
                ui.weak("or drag to an edge");
            });
            ui.separator();
            ui.push_id(id, |ui| contents(app, ui, id));
        });

        // Dragging the window to an edge of the 3D view docks it there.
        if let Some(response) = shown.map(|shown| shown.response) {
            let pointer = ctx.input(|input| input.pointer.interact_pos());
            let zone = pointer.and_then(|pointer| drop_zone(viewport, pointer));
            if let (true, Some((_, strip))) = (response.dragged(), zone) {
                let accent = ctx.global_style().visuals.selection.bg_fill;
                ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("dock_zone")))
                    .rect_filled(strip, 0.0, accent.gamma_multiply(0.45));
            }
            if let (true, Some((side, _))) = (response.drag_stopped(), zone) {
                moved = Some(Change::Dock(side));
            }
        }

        if !open {
            moved = Some(Change::Close);
        }
        if let Some(change) = moved {
            apply(app, id, change);
        }
    }
}

/// The dock a window dropped at `pointer` would join, and the strip to highlight.
fn drop_zone(viewport: egui::Rect, pointer: egui::Pos2) -> Option<(Dock, egui::Rect)> {
    if !viewport.contains(pointer) {
        return None;
    }
    let column = |left: f32| egui::Rect::from_min_size(egui::pos2(left, viewport.top()), egui::vec2(DOCK_ZONE, viewport.height()));
    let row = |top: f32| egui::Rect::from_min_size(egui::pos2(viewport.left(), top), egui::vec2(viewport.width(), DOCK_ZONE));
    // Sides first: they own the corners.
    if pointer.x <= viewport.left() + DOCK_ZONE {
        Some((Dock::Left, column(viewport.left())))
    } else if pointer.x >= viewport.right() - DOCK_ZONE {
        Some((Dock::Right, column(viewport.right() - DOCK_ZONE)))
    } else if pointer.y <= viewport.top() + DOCK_ZONE {
        Some((Dock::Top, row(viewport.top())))
    } else if pointer.y >= viewport.bottom() - DOCK_ZONE {
        Some((Dock::Bottom, row(viewport.bottom() - DOCK_ZONE)))
    } else {
        None
    }
}

/// Something the user asked to do with a panel.
#[derive(Clone, Copy)]
enum Change {
    Dock(Dock),
    Close,
}

fn apply(app: &mut App, id: PanelId, change: Change) {
    let panels = &mut app.settings.panels;
    match change {
        Change::Dock(dock) => {
            panels.slot_mut(id).dock = dock;
            // A panel that was just docked is the one to look at.
            panels.set_active_tab(dock, id);
        }
        Change::Close => panels.slot_mut(id).visible = false,
    }
}

/// Menu entries that move the panel `id`, currently at `current`.
fn placement_menu(ui: &mut egui::Ui, id: PanelId, current: Dock) -> Option<Change> {
    let mut change = None;
    ui.weak(id.title());
    for dock in Dock::EDGES {
        if ui.add_enabled(dock != current, egui::Button::new(format!("Dock {}", dock.label().to_lowercase()))).clicked() {
            change = Some(Change::Dock(dock));
            ui.close();
        }
    }
    if ui.button("Undock (floating window)").clicked() {
        change = Some(Change::Dock(Dock::Floating));
        ui.close();
    }
    ui.separator();
    if ui.button("Close").on_hover_text("Bring it back from the View menu").clicked() {
        change = Some(Change::Close);
        ui.close();
    }
    change
}

/// The View menu's entries: show or hide each panel and choose where it sits.
pub fn menu(app: &mut App, ui: &mut egui::Ui) {
    for id in PanelId::ALL {
        ui.horizontal(|ui| {
            let slot = app.settings.panels.slot_mut(id);
            ui.checkbox(&mut slot.visible, id.title());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Right to left, so listed in reverse.
                for dock in [Dock::Floating, Dock::Bottom, Dock::Top, Dock::Right, Dock::Left] {
                    let hint = if dock == Dock::Floating { "Floating window" } else { "Docked at this side of the 3D view" };
                    if ui.selectable_label(slot.dock == dock, dock.label()).on_hover_text(hint).clicked() {
                        slot.dock = dock;
                        slot.visible = true;
                    }
                }
            });
        });
    }
    if ui.button("Reset Panel Layout").clicked() {
        app.settings.panels.reset_layout();
        ui.close();
    }
}

/// Draws what is inside the panel `id`.
fn contents(app: &mut App, ui: &mut egui::Ui, id: PanelId) {
    let units = app.settings.general.units;
    match id {
        PanelId::Telemetry1 => side_panel::telemetry(ui, app, app.session.primary, units),
        PanelId::Telemetry2 => side_panel::telemetry(ui, app, app.session.secondary, units),
        PanelId::Session => side_panel::session_info(ui, &app.session),
        PanelId::Events => side_panel::event_log(app, ui),
        PanelId::View => view_options::contents(app, ui),
        PanelId::Graph1 => graph::show(app, ui, graph::Subject::Primary),
        PanelId::Graph2 => graph::show(app, ui, graph::Subject::Secondary),
        PanelId::Compare => graph::show(app, ui, graph::Subject::Both),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_dock_only_at_the_edges_of_the_view() {
        let viewport = egui::Rect::from_min_size(egui::pos2(300.0, 50.0), egui::vec2(1000.0, 700.0));
        let side = |x, y| drop_zone(viewport, egui::pos2(x, y)).map(|(dock, _)| dock);
        assert_eq!(side(310.0, 400.0), Some(Dock::Left));
        assert_eq!(side(1290.0, 400.0), Some(Dock::Right));
        assert_eq!(side(800.0, 400.0), None);
        assert_eq!(side(800.0, 60.0), Some(Dock::Top));
        assert_eq!(side(800.0, 740.0), Some(Dock::Bottom));
        // Corners belong to the sides.
        assert_eq!(side(310.0, 60.0), Some(Dock::Left));
        // Outside the view (menu, track selectors, other docks) is not a drop target.
        assert_eq!(side(310.0, 20.0), None);
    }
}
