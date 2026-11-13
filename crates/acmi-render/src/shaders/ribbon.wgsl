// Ribbon trails: a flat band along an aircraft's path, lying in the plane of
// its wings, so that rolls show as twists. The upper face is dark, the lower
// face light, and both edges carry a thin outline in the object's own colour.

// The band never gets narrower than this on screen, so distant trails stay
// visible; from far away it reads as a line.
const MIN_HALF_WIDTH_PX: f32 = 1.25;
// Width of the outline along each edge, in pixels.
const OUTLINE_PX: f32 = 1.4;
// Face shades relative to the object's colour.
const TOP_SHADE: f32 = 0.28;
const BOTTOM_TINT: f32 = 0.45;
const FACE_OPACITY: f32 = 0.8;

struct RibbonIn {
    // Centre line of the trail.
    @location(0) high: vec3<f32>,
    @location(1) low: vec3<f32>,
    // Unit vector along the right wing, in world axes.
    @location(2) side: vec3<f32>,
    // Half the real width in metres.
    @location(3) half_width: f32,
    // -1 on the left edge, +1 on the right edge.
    @location(4) across: f32,
    // sRGB colour with straight alpha.
    @location(5) color: vec4<f32>,
};

struct RibbonOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) across: f32,
};

@vertex
fn vs_ribbon(in: RibbonIn) -> RibbonOut {
    let centre = rte(in.high, in.low);
    // globals.params.z: metres per pixel at one metre from the eye.
    let half_width = max(in.half_width, MIN_HALF_WIDTH_PX * length(centre) * globals.params.z);
    var out: RibbonOut;
    out.clip = globals.view_proj * vec4<f32>(centre + in.side * (in.across * half_width), 1.0);
    out.color = vec4<f32>(srgb_to_linear(in.color.rgb), in.color.a);
    out.across = in.across;
    return out;
}

@fragment
fn fs_ribbon(in: RibbonOut, @builtin(front_facing) from_above: bool) -> @location(0) vec4<f32> {
    // Triangles wind so that the front face is the aircraft's upper side.
    let top = in.color.rgb * TOP_SHADE;
    let bottom = mix(in.color.rgb, vec3<f32>(1.0), BOTTOM_TINT);
    let face = select(bottom, top, from_above);
    // `across` runs from -1 to 1 over the width; its screen-space rate of
    // change turns a width in pixels into that scale.
    let outline = smoothstep(1.0 - fwidth(in.across) * OUTLINE_PX, 1.0 - fwidth(in.across) * (OUTLINE_PX - 1.0), abs(in.across));
    let rgb = mix(face, in.color.rgb, outline);
    return vec4<f32>(rgb, in.color.a * mix(FACE_OPACITY, 1.0, outline));
}
