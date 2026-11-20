//! OpenACMI: a flight-recording viewer for ACMI 2.x telemetry.
//!
//! This crate is the application shell: windowing, UI and the glue between
//! the data model (`acmi-core`), I/O (`acmi-io`) and the renderer
//! (`acmi-render`).

// No console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod camera_controller;
mod config;
mod export;
mod file_association;
mod palette;
mod quantity;
mod scene;
mod session;
mod sun;
mod timeline;
mod ui;
mod update;

use std::path::PathBuf;

/// Command-line options.
#[derive(Debug, Default)]
pub struct Args {
    /// Recording to open at startup.
    pub file: Option<PathBuf>,
    /// Development aid: save a picture of the window to this path once the
    /// scene has settled, then quit.
    pub screenshot: Option<PathBuf>,
    /// Development aid: time to seek to before taking the screenshot.
    pub seek: Option<f64>,
    /// Development aid: object id (hex) to select and track before the screenshot.
    pub track: Option<u64>,
    /// Development aid: object id (hex) to open the Edit Object window for.
    pub edit: Option<u64>,
    /// Development aid: start in the cockpit of the tracked object.
    pub cockpit: bool,
    /// Development aid: in the cockpit, look `yaw,pitch` degrees off the
    /// nose with the field of view scaled by a third number.
    pub look: Option<[f64; 3]>,
    /// Development aid: start with the Preferences window open.
    pub preferences: bool,
    /// Development aid: object id (hex) to select as the secondary object.
    pub secondary: Option<u64>,
    /// Development aid: check for updates as if this were another version.
    pub pretend_version: Option<update::Version>,
    /// Development aid: install an update as soon as one is found.
    pub update_now: bool,
}

impl Args {
    fn parse() -> Self {
        let mut args = Args::default();
        let mut raw = std::env::args().skip(1);
        while let Some(arg) = raw.next() {
            match arg.as_str() {
                "--screenshot" => args.screenshot = raw.next().map(PathBuf::from),
                "--seek" => args.seek = raw.next().and_then(|v| v.parse().ok()),
                "--preferences" => args.preferences = true,
                "--cockpit" => args.cockpit = true,
                "--pretend-version" => args.pretend_version = raw.next().and_then(|v| update::Version::parse(&v)),
                "--update-now" => args.update_now = true,
                "--look" => {
                    let numbers: Vec<f64> = raw.next().unwrap_or_default().split(',').filter_map(|v| v.parse().ok()).collect();
                    if let [yaw, pitch, scale] = numbers[..] {
                        args.look = Some([yaw, pitch, scale]);
                    }
                }
                "--edit" => args.edit = raw.next().and_then(|v| u64::from_str_radix(&v, 16).ok()),
                "--secondary" => args.secondary = raw.next().and_then(|v| u64::from_str_radix(&v, 16).ok()),
                "--track" => args.track = raw.next().and_then(|v| u64::from_str_radix(&v, 16).ok()),
                // For installers and scripts: change the association and exit.
                "--associate" | "--remove-association" => {
                    let result = if arg == "--associate" { file_association::register() } else { file_association::remove() };
                    if let Err(err) = &result {
                        eprintln!("cannot change the .acmi file association: {err}");
                    }
                    std::process::exit(result.is_err() as i32);
                }
                "-h" | "--help" => {
                    println!(
                        "Usage: openacmi [FILE.acmi]\n\nOpens an ACMI 2.x recording (plain or zipped).\n\n\
                         Options:\n  --associate           open .acmi files with this program (current user)\n  \
                         --remove-association  undo that"
                    );
                    std::process::exit(0);
                }
                _ => args.file = Some(PathBuf::from(arg)),
            }
        }
        args
    }
}

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .filter_module("wgpu_core", log::LevelFilter::Warn)
        .filter_module("wgpu_hal", log::LevelFilter::Warn)
        .filter_module("naga", log::LevelFilter::Warn)
        .init();

    let args = Args::parse();
    // The window and taskbar icon; the executable's own icon is set by build.rs.
    let icon = image::load_from_memory(include_bytes!("../../../assets/icon.png")).map(|image| {
        let image = image.into_rgba8();
        egui::IconData { width: image.width(), height: image.height(), rgba: image.into_raw() }
    });
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OpenACMI")
            .with_icon(icon.unwrap_or_default())
            .with_inner_size([1600.0, 900.0])
            .with_min_inner_size([800.0, 480.0])
            .with_drag_and_drop(true),
        // Where the window was, its size and whether it was maximised are
        // restored on the next start (eframe keeps them, with the sizes of
        // docks and positions of floating panels, beside the settings file).
        // Screenshots for development always use the default window.
        persist_window: args.screenshot.is_none(),
        ..Default::default()
    };
    eframe::run_native("OpenACMI", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, args)))))
}
