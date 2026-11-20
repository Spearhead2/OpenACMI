//! The tile cache as Preferences shows it: how much disk space the
//! downloaded terrain and imagery take, and a way to delete them.
//!
//! Counting a large cache means reading hundreds of thousands of directory
//! entries, and deleting it takes longer still, so both run on a thread of
//! their own and the window shows the result when it arrives.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use acmi_render::acmi_terrain::cache::{self, CacheUsage};
use eframe::egui;

use crate::config::Terrain;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum Status {
    /// Not looked at yet.
    #[default]
    Unknown,
    Measuring,
    Known(CacheUsage),
    Clearing,
}

/// State of the tile cache row in Preferences.
#[derive(Default)]
pub struct TileCache {
    status: Arc<Mutex<Status>>,
    /// The Clear button was pressed and waits to be confirmed.
    confirming: bool,
}

impl TileCache {
    /// Draws the size of the cache with its buttons.
    pub fn ui(&mut self, ui: &mut egui::Ui, terrain: &Terrain) {
        let status = *self.status.lock().unwrap();
        let busy = matches!(status, Status::Measuring | Status::Clearing);
        match status {
            Status::Unknown => self.run(ui.ctx(), terrain, Status::Measuring),
            Status::Measuring => {
                ui.spinner();
                ui.weak("measuring…");
            }
            Status::Clearing => {
                ui.spinner();
                ui.weak("deleting…");
            }
            Status::Known(usage) if usage.tiles == 0 => {
                ui.label("empty");
            }
            Status::Known(usage) => {
                ui.label(format!("{} in {} tiles", bytes_text(usage.bytes), thousands(usage.tiles)));
            }
        }

        let filled = matches!(status, Status::Known(usage) if usage.tiles > 0);
        if self.confirming && filled {
            ui.label("Delete them all?");
            if ui.button("Delete").clicked() {
                self.confirming = false;
                self.run(ui.ctx(), terrain, Status::Clearing);
            }
            if ui.button("Cancel").clicked() {
                self.confirming = false;
            }
            return;
        }
        self.confirming = false;
        if ui.add_enabled(filled, egui::Button::new("Clear…")).on_hover_text("Delete every downloaded terrain and imagery tile. They are downloaded again as needed.").clicked() {
            self.confirming = true;
        }
        if ui.add_enabled(!busy, egui::Button::new("Refresh")).on_hover_text("Measure the cache again").clicked() {
            self.run(ui.ctx(), terrain, Status::Measuring);
        }
        if let Some(folder) = terrain.resolved_cache_dir().filter(|folder| folder.is_dir()) {
            if ui.button("Open").on_hover_text(folder.display().to_string()).clicked() {
                open_folder(&folder);
            }
        }
    }

    /// Measures (`Status::Measuring`) or empties (`Status::Clearing`) the
    /// cache in the background.
    fn run(&mut self, ctx: &egui::Context, terrain: &Terrain, task: Status) {
        let folders = terrain.cache_folders();
        let (status, ctx) = (self.status.clone(), ctx.clone());
        *status.lock().unwrap() = task;
        std::thread::spawn(move || {
            let mut total = CacheUsage::default();
            for folder in &folders {
                if task == Status::Clearing {
                    let removed = cache::clear(folder);
                    log::info!("tile cache: removed {} tiles from {}", removed.tiles, folder.display());
                }
                // After clearing: what could not be deleted is still there.
                total += cache::usage(folder);
            }
            *status.lock().unwrap() = Status::Known(total);
            ctx.request_repaint();
        });
    }
}

/// Shows a folder in the system's file manager.
fn open_folder(folder: &PathBuf) {
    let program = if cfg!(windows) {
        "explorer.exe"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(err) = std::process::Command::new(program).arg(folder).spawn() {
        log::warn!("cannot open {}: {err}", folder.display());
    }
}

/// A size in the largest unit that keeps it above 1.
fn bytes_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// A count with commas between the thousands.
fn thousands(count: u64) -> String {
    let digits = count.to_string();
    let mut text = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_counts_read_naturally() {
        assert_eq!(bytes_text(0), "0 bytes");
        assert_eq!(bytes_text(1023), "1023 bytes");
        assert_eq!(bytes_text(1536), "1.5 KB");
        assert_eq!(bytes_text(48 * 1024 * 1024), "48 MB");
        assert_eq!(bytes_text(3 * 1024 * 1024 * 1024 + 300 * 1024 * 1024), "3.3 GB");
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1234567), "1,234,567");
    }
}
