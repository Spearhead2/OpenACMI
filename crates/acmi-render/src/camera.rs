//! Camera state and projection maths.
//!
//! The world is Earth-sized and in metres, far beyond what `f32` can address
//! precisely, so the camera keeps its position in `f64` and everything is
//! rendered *relative to the eye*: the view matrix only rotates, and
//! positions reach the GPU already translated (see [`crate::RtePosition`]).
//!
//! Depth uses a reversed, infinite far plane (1 at the near plane, 0 at
//! infinity), which spreads precision evenly from metres to planetary scale.

use glam::{DMat3, DQuat, DVec3, Mat4, Vec2};

#[derive(Debug, Clone, Copy)]
pub struct Camera {
    /// Eye position in world space.
    pub eye: DVec3,
    /// Camera-to-world rotation. The camera looks down its local −Z, +Y is up.
    pub rotation: DQuat,
    /// Vertical field of view, radians.
    pub fov_y: f32,
    pub near: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { eye: DVec3::new(0.0, 0.0, 2.0e7), rotation: DQuat::IDENTITY, fov_y: 45f32.to_radians(), near: 1.0 }
    }
}

impl Camera {
    #[inline]
    pub fn forward(&self) -> DVec3 {
        self.rotation * DVec3::NEG_Z
    }

    #[inline]
    pub fn right(&self) -> DVec3 {
        self.rotation * DVec3::X
    }

    #[inline]
    pub fn up(&self) -> DVec3 {
        self.rotation * DVec3::Y
    }

    /// Points the camera at `target`, keeping `up` as close to screen-up as possible.
    pub fn look_at(&mut self, target: DVec3, up: DVec3) {
        if let Some(rotation) = look_rotation(target - self.eye, up) {
            self.rotation = rotation;
        }
    }

    /// Eye-relative view-projection matrix for a viewport of the given aspect ratio.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = Mat4::from_quat(self.rotation.conjugate().as_quat());
        // Right-handed, infinite far plane, reversed depth in [0, 1]:
        // z_ndc = near / distance.
        let f = 1.0 / (self.fov_y * 0.5).tan();
        let projection = Mat4::from_cols_array_2d(&[
            [f / aspect.max(1e-3), 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, 0.0, -1.0],
            [0.0, 0.0, self.near, 0.0],
        ]);
        projection * view
    }

    /// Projects a world position to viewport pixels (origin top-left).
    /// Returns `None` for points behind the camera.
    pub fn project(&self, world: DVec3, viewport: Vec2) -> Option<Vec2> {
        let local = self.rotation.conjugate() * (world - self.eye);
        if local.z >= -(self.near as f64) {
            return None;
        }
        let f = 1.0 / (self.fov_y as f64 * 0.5).tan();
        let aspect = (viewport.x / viewport.y) as f64;
        let ndc_x = local.x * f / aspect / -local.z;
        let ndc_y = local.y * f / -local.z;
        Some(Vec2::new(((ndc_x * 0.5 + 0.5) * viewport.x as f64) as f32, ((0.5 - ndc_y * 0.5) * viewport.y as f64) as f32))
    }

    /// World-space direction of the ray through a viewport pixel.
    pub fn pixel_ray(&self, pixel: Vec2, viewport: Vec2) -> DVec3 {
        let tan = (self.fov_y as f64 * 0.5).tan();
        let aspect = (viewport.x / viewport.y) as f64;
        let x = (pixel.x / viewport.x) as f64 * 2.0 - 1.0;
        let y = 1.0 - (pixel.y / viewport.y) as f64 * 2.0;
        (self.rotation * DVec3::new(x * tan * aspect, y * tan, -1.0)).normalize()
    }

    /// World metres covered by one pixel at `distance` from the eye.
    #[inline]
    pub fn meters_per_pixel(&self, distance: f64, viewport_height: f32) -> f64 {
        2.0 * distance * (self.fov_y as f64 * 0.5).tan() / viewport_height.max(1.0) as f64
    }
}

/// Rotation whose −Z axis points along `direction` with +Y towards `up`.
/// Returns `None` when the inputs are degenerate.
pub fn look_rotation(direction: DVec3, up: DVec3) -> Option<DQuat> {
    let forward = direction.try_normalize()?;
    let right = forward.cross(up).try_normalize()?;
    let true_up = right.cross(forward);
    Some(DQuat::from_mat3(&DMat3::from_cols(right, true_up, -forward)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_and_ray_are_inverse() {
        let mut camera = Camera { eye: DVec3::new(1.0e7, 2.0e6, 3.0e6), ..Default::default() };
        camera.look_at(DVec3::ZERO, DVec3::Y);
        let viewport = Vec2::new(1280.0, 720.0);

        let centre = camera.project(DVec3::ZERO, viewport).unwrap();
        assert!((centre - viewport * 0.5).length() < 1e-2);

        let pixel = Vec2::new(300.0, 500.0);
        let point = camera.eye + camera.pixel_ray(pixel, viewport) * 5.0e6;
        assert!((camera.project(point, viewport).unwrap() - pixel).length() < 1e-2);
        assert!(camera.project(camera.eye - camera.forward() * 10.0, viewport).is_none());
    }

    #[test]
    fn view_proj_matches_project() {
        let mut camera = Camera { eye: DVec3::new(0.0, 0.0, 100.0), ..Default::default() };
        camera.look_at(DVec3::new(5.0, 3.0, 0.0), DVec3::Y);
        let viewport = Vec2::new(800.0, 600.0);
        let world = DVec3::new(10.0, -4.0, -20.0);

        let clip = camera.view_proj(viewport.x / viewport.y) * (world - camera.eye).as_vec3().extend(1.0);
        let ndc = clip.truncate() / clip.w;
        let expected = Vec2::new((ndc.x * 0.5 + 0.5) * viewport.x, (0.5 - ndc.y * 0.5) * viewport.y);
        assert!((camera.project(world, viewport).unwrap() - expected).length() < 1e-2);
        assert!(ndc.z > 0.0 && ndc.z < 1.0, "reverse-Z depth in range: {}", ndc.z);
    }
}
