// Shared by every pipeline (prepended to each shader at build time).

struct Globals {
    // Rotation-only view times projection: positions are eye-relative.
    view_proj: mat4x4<f32>,
    // Eye position split into two floats per axis (see RtePosition).
    eye_high: vec4<f32>,
    eye_low: vec4<f32>,
    // xyz: unit vector towards the sun, w: ambient light level.
    sun: vec4<f32>,
    // xyz: eye position divided by the Earth radius.
    eye_unit: vec4<f32>,
    // x: atmosphere density relative to the real one (0 = no atmosphere),
    // y: exposure of the scattered light, z: Earth radius in kilometres.
    atmosphere: vec4<f32>,
    // x: 1 when the Earth texture is loaded, y: 1 when lighting is enabled,
    // z: metres per pixel at one metre from the eye.
    params: vec4<f32>,
    // World direction of the view ray through NDC (x, y):
    // sky_forward + x * sky_right + y * sky_up (not normalised).
    sky_right: vec4<f32>,
    sky_up: vec4<f32>,
    sky_forward: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

// Eye-relative position from a double-precision position split in two.
// Subtracting the large parts first keeps the result accurate to millimetres.
fn rte(high: vec3<f32>, low: vec3<f32>) -> vec3<f32> {
    return (high - globals.eye_high.xyz) + (low - globals.eye_low.xyz);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(c, vec3<f32>(2.2));
}
