//! Application state and the per-frame update loop.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use acmi_core::{geo, ObjectHandle};
use acmi_io::{LoadJob, LoadOptions, RttClient, RttConfig, RttMessage};
use acmi_render::acmi_terrain::{Imagery, TileLoader, TileSource};
use acmi_render::wgpu;
use acmi_render::{
    Atmosphere, Camera, EarthImage, Frame, GridStyle, LineVertex, MeshLibrary, Renderer, RendererConfig, TerrainConfig,
    TerrainStats,
};
use eframe::egui_wgpu;
use glam::DVec3;

use crate::camera_controller::{CameraController, CameraMode, TrackTarget};
use crate::config::Settings;
use crate::palette::Palette;
use crate::scene::{Scene, SceneInputs};
use crate::session::{Session, Source};
use crate::ui::{self, UiState};
use crate::{export, sun, Args};

pub struct App {
    pub(crate) settings: Settings,
    settings_path: Option<PathBuf>,
    pub(crate) session: Session,
    pub(crate) palette: Palette,
    pub(crate) meshes: MeshLibrary,
    assets: PathBuf,
    /// Folder the user's own models and colours were read from.
    pub(crate) user_data: PathBuf,
    /// Number of model files found in [`Self::user_data`].
    pub(crate) user_models: usize,

    render_state: Option<egui_wgpu::RenderState>,
    renderer: Option<Renderer>,
    /// The renderer's output as an egui texture.
    scene_texture: Option<egui::TextureId>,
    pub(crate) scene: Scene,
    pub(crate) camera: Camera,
    pub(crate) controller: CameraController,

    pub(crate) load_job: Option<LoadJob>,
    pub(crate) rtt: Option<RttClient>,
    earth_image: Option<Receiver<image::ImageResult<EarthImage>>>,

    pub(crate) ui: UiState,
    window_title: String,
    args: Args,
    /// Frames drawn since the scene last changed for a reason other than time.
    pub(crate) settled_frames: u32,
    /// The settings the running terrain layer was started with. Tiles are
    /// shaped and coloured as they load, so a change means starting over.
    terrain_applied: Option<crate::config::Terrain>,
    /// A slider is being dragged or text typed: not the moment to restart
    /// anything expensive over a half-made change.
    editing: bool,
    /// Frames drawn since startup.
    frame_count: u64,
    started: std::time::Instant,
    screenshot_done: Arc<AtomicBool>,
    screenshot_requested: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, args: Args) -> Self {
        let settings_path = Settings::default_path();
        let settings = settings_path.as_deref().map(Settings::load).unwrap_or_default();
        let assets = find_assets_dir();
        let user_data = user_data_dir(&settings);
        log::info!("assets: {}, user data: {}", assets.display(), user_data.display());

        let (palette, meshes, user_models) = load_content(&assets, &user_data);

        let render_state = cc.wgpu_render_state.clone();
        let mut earth_image = None;
        let renderer = render_state.as_ref().map(|state| {
            let borders = std::fs::read(assets.join("borders.bin")).unwrap_or_else(|err| {
                log::warn!("country borders unavailable: {err}");
                Vec::new()
            });
            let config = RendererConfig {
                msaa_samples: if settings.general.antialiasing { 4 } else { 1 },
                grid: GridStyle {
                    lat_step_deg: settings.grid.lat_step_deg,
                    lon_step_deg: settings.grid.lon_step_deg,
                    color: settings.grid.color.0,
                    equator_color: settings.grid.equator_color.0,
                    fine_step_deg: settings.grid.fine_step_deg,
                    fine_color: settings.grid.fine_color.0,
                },
                border_color: settings.borders.color.0,
            };
            earth_image = Some(spawn_earth_loader(&settings, &assets, Renderer::max_texture_width(&state.device), &cc.egui_ctx));
            Renderer::new(&state.device, &state.queue, &config, &borders)
        });
        if renderer.is_none() {
            log::error!("no wgpu render state: the 3D view is unavailable");
        }

        let mut app = Self {
            controller: CameraController::new(&settings.camera),
            settings,
            settings_path,
            session: Session::default(),
            palette,
            meshes,
            assets,
            user_data,
            user_models,
            render_state,
            renderer,
            scene_texture: None,
            scene: Scene::default(),
            camera: Camera::default(),
            load_job: None,
            rtt: None,
            earth_image,
            ui: UiState::default(),
            window_title: String::new(),
            args,
            settled_frames: 0,
            terrain_applied: None,
            editing: false,
            frame_count: 0,
            started: std::time::Instant::now(),
            screenshot_done: Arc::new(AtomicBool::new(false)),
            screenshot_requested: false,
        };
        app.ui.preferences_open = app.args.preferences;
        ui::preferences::EXPAND_ALL.store(app.args.preferences, Ordering::Relaxed);
        if let Some(path) = app.args.file.clone() {
            app.load_file(path);
        }
        app
    }

    // ── Files ────────────────────────────────────────────────────────────────

    pub(crate) fn load_file(&mut self, path: PathBuf) {
        self.disconnect();
        // Absolute, so the recent-files entry works from any directory.
        let path = std::path::absolute(&path).unwrap_or(path);
        let options = LoadOptions { keep_every_nth: self.settings.loading.keep_every_nth_frame.max(1) };
        self.load_job = Some(LoadJob::spawn(path, options));
    }

    pub(crate) fn open_file_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Open ACMI recording")
            .add_filter("ACMI recordings", &["acmi", "zip", "txt"])
            .add_filter("All files", &["*"])
            .pick_file();
        if let Some(path) = picked {
            self.load_file(path);
        }
    }

    /// Asks for the folder holding the user's models and switches to it.
    pub(crate) fn model_folder_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Folder with 3D models (.glb / .obj)");
        if self.user_data.is_dir() {
            dialog = dialog.set_directory(&self.user_data);
        }
        if let Some(folder) = dialog.pick_folder() {
            self.set_user_data_dir(folder);
        }
    }

    /// Reads models and colours from `folder` from now on, and remembers it.
    pub(crate) fn set_user_data_dir(&mut self, folder: PathBuf) {
        let (palette, meshes, user_models) = load_content(&self.assets, &folder);
        (self.palette, self.meshes, self.user_models) = (palette, meshes, user_models);
        self.settings.general.user_data_dir = folder.to_string_lossy().into_owned();
        self.user_data = folder;
        // Objects already on screen pick their models again.
        self.scene.forget_models();
        self.settled_frames = 0;
        // Saved at once rather than on exit, so a crash cannot lose it.
        self.save_settings();
    }

    pub(crate) fn save_file_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Save recording")
            .add_filter("Compressed ACMI (*.zip.acmi)", &["zip.acmi"])
            .add_filter("ACMI text (*.acmi)", &["acmi"])
            .set_file_name("recording.zip.acmi")
            .save_file();
        let Some(path) = picked else { return };
        let compress = path.to_string_lossy().to_lowercase().ends_with(".zip.acmi");
        match acmi_io::save_file(&self.session.recording, &path, compress) {
            Ok(()) => log::info!("saved {}", path.display()),
            Err(err) => self.ui.error = Some(format!("Could not save {}:\n{err}", path.display())),
        }
    }

    pub(crate) fn export_telemetry_dialog(&mut self) {
        let Some(handle) = self.session.primary else { return };
        let name = self.session.recording.object(handle).name().replace(['/', '\\', ':'], "-");
        let picked = rfd::FileDialog::new()
            .set_title("Export telemetry of the primary object")
            .add_filter("CSV", &["csv"])
            .set_file_name(format!("{}.csv", if name.is_empty() { "telemetry" } else { &name }))
            .save_file();
        if let Some(path) = picked {
            if let Err(err) = export::telemetry_csv(&self.session.recording, handle, &path) {
                self.ui.error = Some(format!("Could not export to {}:\n{err}", path.display()));
            }
        }
    }

    // ── Real-time telemetry ──────────────────────────────────────────────────

    pub(crate) fn connect(&mut self) {
        let connection = &self.settings.connection;
        self.load_job = None;
        self.rtt = Some(RttClient::connect(RttConfig {
            host: connection.host.trim().to_owned(),
            port: connection.port,
            client_name: connection.client_name.clone(),
            password: connection.password.clone(),
        }));
    }

    pub(crate) fn disconnect(&mut self) {
        self.rtt = None;
        self.session.timeline.follow_live = false;
    }

    // ── Selection ────────────────────────────────────────────────────────────

    /// World pose of the primary object at the displayed time.
    pub(crate) fn track_target(&self) -> Option<TrackTarget> {
        let object = self.session.recording.get(self.session.primary?)?;
        let pose = object.pose_at(self.session.timeline.time)?;
        let position = pose.world_position();
        Some(TrackTarget {
            position,
            rotation: geo::orientation(position, pose.heading as f64, pose.pitch as f64, pose.roll as f64),
        })
    }

    pub(crate) fn select_primary(&mut self, handle: Option<ObjectHandle>) {
        self.session.primary = handle;
        // Picking something from the globe view is a request to look at it.
        if handle.is_some() && self.controller.mode() == CameraMode::Global {
            self.set_camera_mode(CameraMode::Tracking);
        }
    }

    pub(crate) fn set_camera_mode(&mut self, mode: CameraMode) {
        self.controller.set_mode(mode, &self.camera, &self.settings.camera);
    }

    /// Points the global camera at the area the recording takes place in.
    fn frame_recording(&mut self) {
        let recording = &self.session.recording;
        let positions: Vec<DVec3> =
            recording.objects().iter().filter(|o| !o.track.is_empty()).take(2000).map(|o| o.track.world_position(0)).collect();
        if positions.is_empty() {
            return;
        }
        let centre = positions.iter().sum::<DVec3>() / positions.len() as f64;
        let (lat, lon, _) = geo::world_to_geo(centre);
        let extent = positions.iter().map(|p| p.distance(centre)).fold(0.0, f64::max);
        self.controller.focus_on(lat, lon, (extent * 2.5).clamp(50_000.0, 6.0e6));
        self.set_camera_mode(CameraMode::Global);
    }

    // ── Background work ──────────────────────────────────────────────────────

    fn poll_background(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.load_job {
            match job.poll() {
                Some(Ok(loaded)) => {
                    let path = job.path().to_owned();
                    self.settings.recent_files.add(&path);
                    self.save_settings();
                    self.session.open(loaded.recording, Source::File(path));
                    self.load_job = None;
                    self.frame_recording();
                    self.apply_startup_args();
                }
                Some(Err(err)) => {
                    self.ui.error = Some(format!("Could not load {}:\n{err}", job.path().display()));
                    self.load_job = None;
                }
                None => ctx.request_repaint(),
            }
        }

        if let Some(rtt) = &self.rtt {
            let mut received = false;
            while let Some(message) = rtt.try_recv() {
                received = true;
                match message {
                    RttMessage::StreamStarted => {
                        let label = format!("{}:{}", rtt.config().host, rtt.config().port);
                        self.session.begin_live(label, self.settings.connection.clear_on_connect);
                    }
                    RttMessage::Lines(lines) => self.session.feed_live(&lines),
                }
            }
            if received {
                self.settled_frames = 0;
            }
            // Keep polling the socket thread even when the UI is otherwise idle.
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }

        if let Some(receiver) = &self.earth_image {
            if let Ok(result) = receiver.try_recv() {
                match (result, &self.render_state, &mut self.renderer) {
                    (Ok(image), Some(state), Some(renderer)) => renderer.set_earth_texture(&state.device, &state.queue, &image),
                    (Err(err), ..) => log::warn!("Earth map unavailable: {err}"),
                    _ => {}
                }
                self.earth_image = None;
                self.settled_frames = 0;
            }
        }
    }

    /// Applies `--seek` / `--track` / `--secondary` once the startup file has loaded.
    fn apply_startup_args(&mut self) {
        if let Some(time) = self.args.seek.take() {
            self.session.timeline.seek(time, self.session.recording.time_range());
        }
        if let Some(id) = self.args.track.take() {
            let handle = self.session.recording.find(id);
            self.select_primary(handle);
        }
        if let Some(handle) = self.args.edit.take().and_then(|id| self.session.recording.find(id)) {
            self.ui.edit_object = ui::edit_object::EditObject::open(self, handle);
        }
        if let Some(id) = self.args.secondary.take() {
            self.session.secondary = self.session.recording.find(id);
        }
    }

    // ── Terrain ──────────────────────────────────────────────────────────────

    /// Ground height in metres at a location, where terrain is loaded and shown.
    pub(crate) fn terrain_height(&self, latitude: f64, longitude: f64) -> Option<f64> {
        if !self.settings.terrain.enabled {
            return None;
        }
        self.renderer.as_ref()?.terrain_height(latitude, longitude).map(f64::from)
    }

    pub(crate) fn terrain_stats(&self) -> Option<TerrainStats> {
        self.renderer.as_ref()?.terrain_stats().filter(|_| self.settings.terrain.enabled)
    }

    /// Whether terrain for the current view is still streaming in.
    pub(crate) fn terrain_busy(&self) -> bool {
        self.settings.terrain.enabled && self.renderer.as_ref().is_some_and(Renderer::terrain_busy)
    }

    pub(crate) fn save_settings(&self) {
        if let Some(path) = &self.settings_path {
            self.settings.save(path);
        }
    }

    // ── Rendering ────────────────────────────────────────────────────────────

    /// Renders the 3D view at `size` (in points) and returns the texture to show.
    pub(crate) fn render_scene(&mut self, size: egui::Vec2, pixels_per_point: f32) -> Option<egui::TextureId> {
        let state = self.render_state.as_ref()?;
        let renderer = self.renderer.as_mut()?;
        let (device, queue) = (&state.device, &state.queue);

        // Terrain starts on first use, so a disabled layer costs nothing.
        let terrain = &self.settings.terrain;
        if renderer.has_terrain() && !self.editing && self.terrain_applied.as_ref() != Some(terrain) {
            renderer.disable_terrain();
        }
        if terrain.enabled && !renderer.has_terrain() {
            let cache_dir = terrain.resolved_cache_dir();
            match &cache_dir {
                Some(dir) => log::info!("terrain cache: {}", dir.display()),
                None => log::warn!("no cache directory available: terrain tiles will not be kept"),
            }
            let source = TileSource::new(terrain.url_template.clone(), cache_dir, terrain.online);
            let config = TerrainConfig {
                max_zoom: terrain.max_zoom,
                pixels_per_texel: terrain.pixels_per_sample.clamp(0.5, 8.0),
                exaggeration: terrain.vertical_exaggeration.clamp(0.1, 20.0),
                ..Default::default()
            };
            let imagery = terrain.imagery.then(|| {
                let cache_dir = terrain.resolved_imagery_cache_dir();
                if let Some(dir) = &cache_dir {
                    log::info!("imagery cache: {}", dir.display());
                }
                let source = TileSource::new(terrain.imagery_url_template.clone(), cache_dir, terrain.online);
                Imagery::new(source, terrain.imagery_max_zoom.clamp(1, 20), terrain.imagery_relief).with_tone(
                    terrain.imagery_brightness,
                    terrain.imagery_saturation,
                    terrain.imagery_gamma,
                )
            });
            self.terrain_applied = Some(terrain.clone());
            let loader = TileLoader::new(source, imagery, terrain.palette(), terrain.download_threads.clamp(1, 16));
            renderer.enable_terrain(device, queue, loader, config);
        }

        let pixels = (size * pixels_per_point).round();
        if renderer.resize(device, pixels.x.max(1.0) as u32, pixels.y.max(1.0) as u32) {
            let view = renderer.color_view()?;
            let mut egui_renderer = state.renderer.write();
            match self.scene_texture {
                Some(id) => egui_renderer.update_egui_texture_from_wgpu_texture(device, view, wgpu::FilterMode::Linear, id),
                None => self.scene_texture = Some(egui_renderer.register_native_texture(device, view, wgpu::FilterMode::Linear)),
            }
        }

        let time = self.session.timeline.time;
        let recording = &self.session.recording;
        let hidden = self.session.primary.filter(|_| self.controller.mode() == CameraMode::Cockpit);
        let inputs = SceneInputs {
            recording,
            generation: self.session.generation(),
            time,
            camera: &self.camera,
            viewport_height: size.y,
            palette: &self.palette,
            settings: &self.settings,
            meshes: &self.meshes,
            hidden,
        };
        self.scene.update(&inputs, renderer, device, queue);

        // Line between the primary and secondary objects.
        let mut overlay = Vec::new();
        if let (Some(a), Some(b)) = (self.session.primary, self.session.secondary) {
            let position = |h: ObjectHandle| recording.get(h)?.pose_at(time).map(|pose| pose.world_position());
            if let (Some(a), Some(b), true) = (position(a), position(b), a != b) {
                let color = self.settings.objects.relation_line_color.0;
                overlay.extend([LineVertex::new(a, color), LineVertex::new(b, color)]);
            }
        }

        let lighting = &self.settings.lighting;
        let (sun_lat, sun_lon) = match recording.absolute_time(time).filter(|_| lighting.follow_recording_time) {
            Some(utc) => sun::subsolar_point(utc),
            None => (lighting.sun_latitude_deg as f64, lighting.sun_longitude_deg as f64),
        };
        let earth = &self.settings.earth;
        renderer.render(
            device,
            queue,
            &Frame {
                camera: &self.camera,
                clear_color: self.settings.general.background.rgb_f32(),
                sun_direction: sun::direction(sun_lat, sun_lon),
                ambient: lighting.ambient.clamp(0.0, 1.0),
                lighting: lighting.enabled,
                show_grid: self.settings.grid.show,
                show_fine_grid: self.camera.eye.length() - geo::EARTH_RADIUS_M < self.settings.grid.fine_below_altitude_m,
                show_borders: self.settings.borders.show,
                show_trails: self.settings.trails.show,
                show_terrain: self.settings.terrain.enabled,
                atmosphere: earth
                    .show_atmosphere
                    .then_some(Atmosphere { density: earth.atmosphere_strength, exposure: earth.atmosphere_exposure }),
                instances: &self.scene.instances,
                overlay_lines: &overlay,
            },
        );
        self.scene_texture
    }

    // ── Frame ────────────────────────────────────────────────────────────────

    fn handle_global_input(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.first().map(|file| file.path().to_owned()));
        if let Some(path) = dropped.filter(|p| !p.as_os_str().is_empty()) {
            self.load_file(path);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.open_file_dialog();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) && !self.session.recording.is_empty() {
            self.save_file_dialog();
        }
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let session = self.session.title();
        let title = if session.is_empty() { "OpenACMI".to_owned() } else { format!("OpenACMI - {session}") };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    /// `--screenshot`: once nothing is loading any more, capture the window and quit.
    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        const SETTLE_FRAMES: u32 = 10;
        let Some(path) = self.args.screenshot.clone() else { return };
        if self.screenshot_done.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        ctx.request_repaint();
        let busy = self.load_job.is_some() || self.earth_image.is_some() || self.terrain_busy();
        self.frame_count += 1;
        self.settled_frames = if busy { 0 } else { self.settled_frames + 1 };
        if self.settled_frames >= SETTLE_FRAMES && !self.screenshot_requested {
            self.screenshot_requested = true;
            log::info!("scene settled after {} frames in {:.1?}", self.frame_count, self.started.elapsed());
            let done = Arc::clone(&self.screenshot_done);
            let wake = ctx.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |image| {
                let (width, height) = (image.width() as u32, image.height() as u32);
                match image::save_buffer(&path, image.as_raw(), width, height, image::ColorType::Rgba8) {
                    Ok(()) => log::info!("screenshot saved to {}", path.display()),
                    Err(err) => log::error!("cannot save screenshot: {err}"),
                }
                done.store(true, Ordering::Relaxed);
                wake.request_repaint();
            })));
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.editing = ctx.input(|input| input.pointer.any_down()) || ctx.egui_wants_keyboard_input();
        self.poll_background(&ctx);
        self.handle_global_input(&ctx);

        egui::Panel::top("menu").show(ui, |ui| ui::menu::show(self, ui));
        if self.settings.panels.track_selection {
            egui::Panel::top("tracks").show(ui, |ui| ui::toolbar::show(self, ui));
        }
        egui::Panel::bottom("timeline").show(ui, |ui| ui::timeline_bar::show(self, ui));
        ui::panels::show_docks(self, ui);
        egui::CentralPanel::no_frame().show(ui, |ui| ui::viewport::show(self, ui));
        ui::dialogs::show(self, &ctx);

        self.update_window_title(&ctx);
        self.handle_screenshot(&ctx);
    }

    fn on_exit(&mut self) {
        self.save_settings();
    }
}

/// Finds the bundled `assets` directory: `OPENACMI_ASSETS` if set, else the
/// nearest `assets` folder beside or above the executable. That is next to
/// it when installed, and the repository's when run through cargo (whose
/// executables sit in `target/<profile>/`).
fn find_assets_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("OPENACMI_ASSETS").map(PathBuf::from).filter(|dir| dir.is_dir()) {
        return dir;
    }
    let exe = std::env::current_exe().ok();
    let beside = exe.as_deref().and_then(Path::parent);
    beside
        .into_iter()
        .flat_map(Path::ancestors)
        .take(4)
        .map(|dir| dir.join("assets"))
        .find(|dir| dir.join("acmi_colors.json").is_file())
        .unwrap_or_else(|| PathBuf::from("assets"))
}

/// Loads colour schemes and indexes models: the bundled set, then the user's,
/// which override it. Returns the number of user model files found.
///
/// Models are looked for in `meshesobj` and `meshes` inside `user_data` and
/// directly in it, so either the data folder or a folder of models can be given.
fn load_content(assets: &Path, user_data: &Path) -> (Palette, MeshLibrary, usize) {
    let mut palette = Palette::default();
    palette.load_file(&assets.join("acmi_colors.json"));
    if user_data.join("acmi_colors.json").is_file() {
        palette.load_file(&user_data.join("acmi_colors.json"));
    }

    let mut meshes = MeshLibrary::default();
    let bundled = meshes.scan_dir(&assets.join("meshes"));
    let user = meshes.scan_dir(user_data) + meshes.scan_dir(&user_data.join("meshesobj")) + meshes.scan_dir(&user_data.join("meshes"));
    log::info!("models: {bundled} bundled, {user} from {}", user_data.display());
    (palette, meshes, user)
}

/// Folder holding the user's own models and colour schemes.
fn user_data_dir(settings: &Settings) -> PathBuf {
    if !settings.general.user_data_dir.is_empty() {
        return PathBuf::from(&settings.general.user_data_dir);
    }
    if let Some(dir) = std::env::var_os("OPENACMI_DATA_DIR") {
        return PathBuf::from(dir);
    }
    std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("data"))).unwrap_or_else(|| PathBuf::from("data"))
}

/// Decodes the Earth map on a worker thread; the UI is woken when it is ready.
fn spawn_earth_loader(
    settings: &Settings,
    assets: &Path,
    device_limit: u32,
    ctx: &egui::Context,
) -> Receiver<image::ImageResult<EarthImage>> {
    let path = match settings.earth.texture.as_str() {
        "" => assets.join("textures/world_map.png"),
        custom => PathBuf::from(custom),
    };
    let max_width = settings.earth.texture_max_width.min(device_limit);
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .name("earth-texture".into())
        .spawn(move || {
            let started = std::time::Instant::now();
            let result = acmi_render::load_earth_image(&path, max_width);
            if let Ok(image) = &result {
                log::info!("Earth map {}x{} ready in {:.2?}", image.width, image.height, started.elapsed());
            }
            let _ = tx.send(result);
            ctx.request_repaint();
        })
        .expect("failed to spawn texture thread");
    rx
}
