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
mod palette;
mod quantity;
mod scene;
mod session;
mod sun;
mod timeline;
mod ui;

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
    /// Development aid: start with the Preferences window open.
    pub preferences: bool,
    /// Development aid: object id (hex) to select as the secondary object.
    pub secondary: Option<u64>,
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
                "--edit" => args.edit = raw.next().and_then(|v| u64::from_str_radix(&v, 16).ok()),
                "--secondary" => args.secondary = raw.next().and_then(|v| u64::from_str_radix(&v, 16).ok()),
                "--track" => args.track = raw.next().and_then(|v| u64::from_str_radix(&v, 16).ok()),
                "-h" | "--help" => {
                    println!("Usage: openacmi [FILE.acmi]\n\nOpens an ACMI 2.x recording (plain or zipped).");
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
