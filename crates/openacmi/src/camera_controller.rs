//! Turns user input into camera motion.
//!
//! Four modes, switchable at any time:
//!
//! * **Global**: orbits the Earth, always looking at its centre.
//! * **Tracking**: orbits the primary object.
//! * **Cockpit**: sits in the primary object and looks where it points.
//! * **Free**: a fly-through camera.

use acmi_core::geo::{self, LocalFrame, EARTH_RADIUS_M};
use acmi_render::camera::look_rotation;
use acmi_render::Camera;
use glam::{DQuat, DVec3, Vec2};

use crate::config::CameraSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraMode {
    Global,
    Tracking,
    Cockpit,
    Free,
}

impl CameraMode {
    pub const ALL: [CameraMode; 4] = [CameraMode::Global, CameraMode::Tracking, CameraMode::Cockpit, CameraMode::Free];

    pub fn label(self) -> &'static str {
        match self {
            CameraMode::Global => "Global orbit",
            CameraMode::Tracking => "Tracking",
            CameraMode::Cockpit => "Cockpit",
            CameraMode::Free => "Free flight",
        }
    }

    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|&m| m == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }
}

/// Input gathered from the 3D view for one frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct CameraInput {
    /// Pointer drag in pixels.
    pub drag: Vec2,
    /// Positive zooms in.
    pub zoom: f32,
    /// Keyboard axes in `-1..=1`: x = right, y = up/forward, z = ascend.
    pub keys: glam::Vec3,
    pub fast: bool,
    pub dt: f32,
    pub viewport_height: f32,
    /// Terrain height under the camera, metres above sea level.
    pub ground_height: f64,
}

/// Where the tracked object is this frame.
#[derive(Debug, Clone, Copy)]
pub struct TrackTarget {
    pub position: DVec3,
    pub rotation: DQuat,
}

const MIN_ALTITUDE_M: f64 = 200.0;
const MAX_ALTITUDE_M: f64 = EARTH_RADIUS_M * 6.0;
const MIN_TRACK_DISTANCE_M: f64 = 15.0;
const MAX_TRACK_DISTANCE_M: f64 = 2.0e6;

#[derive(Debug, Clone)]
pub struct CameraController {
    mode: CameraMode,
    // Global: the point above which the camera hovers.
    longitude: f64,
    latitude: f64,
    altitude: f64,
    // Tracking (orbit angles) and cockpit (head angles), degrees.
    orbit_yaw: f64,
    orbit_pitch: f64,
    orbit_distance: f64,
    // Free flight.
    free_position: DVec3,
    free_yaw: f64,
    free_pitch: f64,
    free_speed: f64,
}

impl CameraController {
    pub fn new(settings: &CameraSettings) -> Self {
        Self {
            mode: CameraMode::Global,
            longitude: 20.0,
            latitude: 25.0,
            altitude: EARTH_RADIUS_M * 1.6,
            orbit_yaw: 0.0,
            orbit_pitch: 20.0,
            orbit_distance: settings.tracking_distance_m,
            free_position: geo::geo_to_world(25.0, 20.0, EARTH_RADIUS_M),
            free_yaw: 0.0,
            free_pitch: -30.0,
            free_speed: settings.free_speed_mps,
        }
    }

    pub fn mode(&self) -> CameraMode {
        self.mode
    }

    /// Switches mode, carrying the current view over where that makes sense.
    pub fn set_mode(&mut self, mode: CameraMode, camera: &Camera, settings: &CameraSettings) {
        if mode == self.mode {
            return;
        }
        match mode {
            CameraMode::Global => {
                let (lat, lon, alt) = geo::world_to_geo(camera.eye);
                self.latitude = lat.clamp(-89.5, 89.5);
                self.longitude = lon;
                self.altitude = alt.clamp(MIN_ALTITUDE_M, MAX_ALTITUDE_M);
            }
            CameraMode::Tracking => {
                self.orbit_yaw = 0.0;
                self.orbit_pitch = 20.0;
                self.orbit_distance = settings.tracking_distance_m;
            }
            CameraMode::Cockpit => {
                self.orbit_yaw = 0.0;
                self.orbit_pitch = 0.0;
            }
            CameraMode::Free => {
                let frame = LocalFrame::at(camera.eye);
                let forward = camera.forward();
                self.free_position = camera.eye;
                self.free_pitch = forward.dot(frame.up).clamp(-1.0, 1.0).asin().to_degrees();
                self.free_yaw = forward.dot(frame.east).atan2(forward.dot(frame.north)).to_degrees();
            }
        }
        self.mode = mode;
    }

    /// Moves the global camera above a location.
    pub fn focus_on(&mut self, latitude: f64, longitude: f64, altitude: f64) {
        self.latitude = latitude.clamp(-89.5, 89.5);
        self.longitude = longitude;
        self.altitude = altitude.clamp(MIN_ALTITUDE_M, MAX_ALTITUDE_M);
    }

    /// Applies one frame of input and writes the resulting pose to `camera`.
    pub fn update(&mut self, camera: &mut Camera, input: &CameraInput, target: Option<TrackTarget>, settings: &CameraSettings) {
        camera.fov_y = settings.fov_deg.clamp(10.0, 120.0).to_radians();
        let key_step = (settings.key_orbit_speed * input.dt) as f64;
        let zoom = 0.9f64.powf(input.zoom as f64 + input.keys.z as f64 * input.dt as f64 * 10.0);

        match (self.mode, target) {
            (CameraMode::Tracking, Some(target)) => {
                let sensitivity = settings.orbit_sensitivity as f64;
                self.orbit_yaw += input.drag.x as f64 * sensitivity + input.keys.x as f64 * key_step;
                self.orbit_pitch =
                    (self.orbit_pitch + input.drag.y as f64 * sensitivity + input.keys.y as f64 * key_step).clamp(-85.0, 89.0);
                self.orbit_distance = (self.orbit_distance * zoom).clamp(MIN_TRACK_DISTANCE_M, MAX_TRACK_DISTANCE_M);

                let frame = LocalFrame::at(target.position);
                let (yaw, pitch) = (self.orbit_yaw.to_radians(), self.orbit_pitch.to_radians());
                // Yaw 0 puts the camera south of the target, looking north.
                let horizontal = -frame.north * yaw.cos() - frame.east * yaw.sin();
                camera.eye = target.position + (horizontal * pitch.cos() + frame.up * pitch.sin()) * self.orbit_distance;
                camera.look_at(target.position, frame.up);
            }
            (CameraMode::Cockpit, Some(target)) => {
                let sensitivity = settings.orbit_sensitivity as f64;
                self.orbit_yaw = (self.orbit_yaw + input.drag.x as f64 * sensitivity).clamp(-170.0, 170.0);
                self.orbit_pitch = (self.orbit_pitch - input.drag.y as f64 * sensitivity).clamp(-80.0, 80.0);
                camera.eye = target.position;
                camera.rotation = target.rotation
                    * DQuat::from_rotation_y(-self.orbit_yaw.to_radians())
                    * DQuat::from_rotation_x(self.orbit_pitch.to_radians());
            }
            (CameraMode::Free, _) => self.update_free(camera, input, settings),
            // Global, or a target-relative mode with nothing to follow.
            _ => {
                self.altitude = (self.altitude * zoom).clamp(input.ground_height + MIN_ALTITUDE_M, MAX_ALTITUDE_M);
                // Dragging moves the ground under the pointer at any zoom level.
                let degrees_per_pixel =
                    camera.meters_per_pixel(self.altitude, input.viewport_height).to_degrees() / EARTH_RADIUS_M;
                let key_degrees = key_step * (self.altitude / EARTH_RADIUS_M).min(1.0);
                self.longitude -= input.drag.x as f64 * degrees_per_pixel / self.latitude.to_radians().cos().max(0.05)
                    - input.keys.x as f64 * key_degrees;
                self.latitude = (self.latitude + input.drag.y as f64 * degrees_per_pixel + input.keys.y as f64 * key_degrees)
                    .clamp(-89.5, 89.5);
                self.longitude = (self.longitude + 180.0).rem_euclid(360.0) - 180.0;

                camera.eye = geo::geo_to_world(self.latitude, self.longitude, self.altitude);
                camera.look_at(DVec3::ZERO, DVec3::Y);
            }
        }
        // Keep the near plane as far out as the scene allows: better depth
        // precision high up, no clipping when close to an object.
        let height = camera.eye.length() - EARTH_RADIUS_M - input.ground_height;
        let mut near = (height.abs() * 0.01).clamp(0.5, 500.0);
        // An aircraft followed from 30 m at 7 km up would otherwise sit in
        // front of the near plane and vanish.
        if let (CameraMode::Tracking, Some(target)) = (self.mode, target) {
            near = near.min(camera.eye.distance(target.position) * 0.05).max(0.2);
        }
        camera.near = near as f32;
    }

    fn update_free(&mut self, camera: &mut Camera, input: &CameraInput, settings: &CameraSettings) {
        let sensitivity = settings.orbit_sensitivity as f64 * 0.6;
        self.free_yaw += input.drag.x as f64 * sensitivity;
        self.free_pitch = (self.free_pitch - input.drag.y as f64 * sensitivity).clamp(-89.0, 89.0);
        // The wheel changes how fast the camera flies.
        self.free_speed = (self.free_speed * 1.1f64.powf(input.zoom as f64)).clamp(1.0, 5.0e6);

        let frame = LocalFrame::at(self.free_position);
        let (yaw, pitch) = (self.free_yaw.to_radians(), self.free_pitch.to_radians());
        let heading = frame.north * yaw.cos() + frame.east * yaw.sin();
        let forward = heading * pitch.cos() + frame.up * pitch.sin();
        let right = forward.cross(frame.up).normalize_or_zero();

        let step = self.free_speed * input.dt as f64 * if input.fast { 4.0 } else { 1.0 };
        self.free_position +=
            (right * input.keys.x as f64 + forward * input.keys.y as f64 + frame.up * input.keys.z as f64) * step;
        // Never fly through the ground.
        let min_radius = EARTH_RADIUS_M + input.ground_height + 2.0;
        if self.free_position.length() < min_radius {
            self.free_position = self.free_position.normalize_or(DVec3::Z) * min_radius;
        }

        camera.eye = self.free_position;
        if let Some(rotation) = look_rotation(forward, frame.up) {
            camera.rotation = rotation;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> CameraInput {
        CameraInput { dt: 1.0 / 60.0, viewport_height: 720.0, ..Default::default() }
    }

    #[test]
    fn global_looks_at_earth_centre() {
        let settings = CameraSettings::default();
        let mut controller = CameraController::new(&settings);
        let mut camera = Camera::default();
        controller.focus_on(45.0, 10.0, 1.0e6);
        controller.update(&mut camera, &input(), None, &settings);
        let (lat, lon, alt) = geo::world_to_geo(camera.eye);
        assert!((lat - 45.0).abs() < 1e-9 && (lon - 10.0).abs() < 1e-9 && (alt - 1.0e6).abs() < 1e-3);
        assert!(camera.forward().abs_diff_eq(-camera.eye.normalize(), 1e-9));
    }

    #[test]
    fn tracking_keeps_distance_and_aim() {
        let settings = CameraSettings::default();
        let mut controller = CameraController::new(&settings);
        let mut camera = Camera::default();
        controller.set_mode(CameraMode::Tracking, &camera, &settings);
        let position = geo::geo_to_world(30.0, 40.0, 5000.0);
        let target = TrackTarget { position, rotation: DQuat::IDENTITY };
        controller.update(&mut camera, &CameraInput { drag: Vec2::new(40.0, -10.0), ..input() }, Some(target), &settings);
        assert!((camera.eye.distance(position) - settings.tracking_distance_m).abs() < 1e-6);
        assert!(camera.forward().abs_diff_eq((position - camera.eye).normalize(), 1e-9));
    }

    #[test]
    fn mode_switches_preserve_the_view() {
        let settings = CameraSettings::default();
        let mut controller = CameraController::new(&settings);
        let mut camera = Camera::default();
        controller.update(&mut camera, &input(), None, &settings);
        let (eye, forward) = (camera.eye, camera.forward());

        controller.set_mode(CameraMode::Free, &camera, &settings);
        controller.update(&mut camera, &input(), None, &settings);
        assert!(camera.eye.abs_diff_eq(eye, 1e-3));
        // Straight down is clamped to 89°, hence the loose tolerance.
        assert!(camera.forward().abs_diff_eq(forward, 0.02), "{:?} vs {forward:?}", camera.forward());

        controller.set_mode(CameraMode::Global, &camera, &settings);
        controller.update(&mut camera, &input(), None, &settings);
        assert!(camera.eye.abs_diff_eq(eye, 1e-3));
    }
}
