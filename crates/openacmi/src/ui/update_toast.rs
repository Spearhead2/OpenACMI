//! The update notice: a small card in the corner of the 3D view that says
//! a new version is out, follows its installation and offers the restart.

use eframe::egui;

use crate::app::App;
use crate::update::{self, Release, Status};

/// What the user asked for with a button of the notice.
enum Action {
    Install(Release),
    Open(String),
    Dismiss,
    Restart,
}

/// Draws the notice over `view`, the 3D view, if there is anything to say.
pub fn show(app: &mut App, ctx: &egui::Context, view: egui::Rect) {
    let status = app.updater.status();
    // A check the user did not ask for stays quiet unless it finds something.
    let quiet = !app.ui.update_check_requested;
    let silent = match &status {
        Status::Idle => true,
        Status::Checking | Status::UpToDate | Status::CheckFailed(_) => quiet,
        _ => false,
    };
    if silent {
        return;
    }

    let mut action = None;
    egui::Area::new(egui::Id::unique("update_notice"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .fixed_pos(view.right_bottom() - egui::vec2(14.0, 34.0))
        .show(ctx, |ui| {
            egui::Frame::window(ui.style()).inner_margin(12.0).show(ui, |ui| {
                ui.set_max_width(330.0);
                action = card(ui, &status, app.updater.current_version());
            });
        });

    match action {
        Some(Action::Install(release)) => app.updater.install(ctx, release),
        Some(Action::Open(url)) => ctx.open_url(egui::OpenUrl::new_tab(url)),
        Some(Action::Dismiss) => {
            app.updater.dismiss();
            app.ui.update_check_requested = false;
        }
        Some(Action::Restart) => app.restart(ctx),
        None => {}
    }
}

fn card(ui: &mut egui::Ui, status: &Status, current: update::Version) -> Option<Action> {
    let mut action = None;
    match status {
        Status::Idle => {}
        Status::Checking => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Checking for updates…");
            });
        }
        Status::UpToDate => {
            ui.strong("OpenACMI is up to date");
            ui.label(format!("Version {current} is the latest."));
            ui.add_space(6.0);
            if ui.button("OK").clicked() {
                action = Some(Action::Dismiss);
            }
        }
        Status::CheckFailed(message) => {
            ui.strong("Could not check for updates");
            ui.label(message);
            ui.add_space(6.0);
            if ui.button("Dismiss").clicked() {
                action = Some(Action::Dismiss);
            }
        }
        Status::Available(release) => {
            ui.strong(format!("OpenACMI {} is available", release.version));
            ui.label(format!("You have version {current}."));
            let blocked = match (&release.package, update::cannot_install()) {
                (None, _) => Some("this release has no download that can be installed automatically"),
                (Some(_), reason) => reason,
            };
            if let Some(reason) = blocked {
                ui.weak(format!("Get it from the release page: {reason}."));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if blocked.is_none() {
                    let update = ui.button("Update").on_hover_text("Download and install it; you choose when to restart");
                    if update.clicked() {
                        action = Some(Action::Install(release.clone()));
                    }
                }
                let page = if blocked.is_none() { "Release notes" } else { "Open release page" };
                if ui.button(page).on_hover_text(&release.page).clicked() {
                    action = Some(Action::Open(release.page.clone()));
                }
                if ui.button("Dismiss").on_hover_text("Hide this until the next start").clicked() {
                    action = Some(Action::Dismiss);
                }
            });
        }
        Status::Downloading { release, received } => {
            ui.strong(format!("Updating to OpenACMI {}", release.version));
            let total = release.package.as_ref().map_or(0, |package| package.bytes);
            let megabytes = |bytes: u64| bytes as f32 / (1024.0 * 1024.0);
            let fraction = if total > 0 { (*received as f32 / total as f32).min(1.0) } else { 0.0 };
            ui.add(egui::ProgressBar::new(fraction).desired_width(300.0).text(format!("{:.1} of {:.1} MB", megabytes(*received), megabytes(total))));
            ui.weak("You can keep working.");
        }
        Status::Installed(release) => {
            ui.strong(format!("OpenACMI {} is installed", release.version));
            ui.label("Restart to start using it.");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Restart now").clicked() {
                    action = Some(Action::Restart);
                }
                if ui.button("Release notes").on_hover_text(&release.page).clicked() {
                    action = Some(Action::Open(release.page.clone()));
                }
            });
            ui.weak("Or carry on: it starts the next time you open OpenACMI.");
        }
        Status::Failed { release, message } => {
            ui.strong(format!("The update to {} did not install", release.version));
            ui.label(message);
            ui.weak("Nothing was changed. You can download it yourself from the release page.");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Open release page").on_hover_text(&release.page).clicked() {
                    action = Some(Action::Open(release.page.clone()));
                }
                if ui.button("Dismiss").clicked() {
                    action = Some(Action::Dismiss);
                }
            });
        }
    }
    action
}

/// The About window.
pub fn about_window(app: &mut App, ctx: &egui::Context) {
    if !app.ui.about_open {
        return;
    }
    let mut open = true;
    egui::Window::new("About OpenACMI")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.heading("OpenACMI");
            ui.label(format!("Version {}", update::Version::current()));
            ui.add_space(4.0);
            ui.label(env!("CARGO_PKG_DESCRIPTION"));
            ui.add_space(6.0);
            ui.hyperlink_to("github.com/Spearhead2/OpenACMI", "https://github.com/Spearhead2/OpenACMI");
            ui.weak("Open source under the MIT licence.");
            ui.separator();
            updates(app, ui);
        });
    app.ui.about_open &= open;
}

/// The update controls shared by the About menu and the About window.
pub fn updates(app: &mut App, ui: &mut egui::Ui) {
    ui.checkbox(&mut app.settings.general.check_for_updates, "Check for updates at startup")
        .on_hover_text("Ask GitHub for the latest release each time OpenACMI starts. Nothing is installed without asking.");
    if ui.add_enabled(!app.updater.busy(), egui::Button::new("Check for Updates Now")).clicked() {
        app.ui.update_check_requested = true;
        app.updater.dismiss();
        app.updater.check(ui.ctx());
        ui.close();
    }
}
