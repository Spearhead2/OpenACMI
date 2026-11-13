// Coloured line segments: grid, borders, trails and overlays.

struct LineIn {
    @location(0) high: vec3<f32>,
    @location(1) low: vec3<f32>,
    // sRGB colour with straight alpha.
    @location(2) color: vec4<f32>,
};

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    // Negative where a point on the Earth's surface is beyond the horizon.
    @location(1) horizon: f32,
};

fn line_common(in: LineIn) -> LineOut {
    var out: LineOut;
    out.clip = globals.view_proj * vec4<f32>(rte(in.high, in.low), 1.0);
    out.color = vec4<f32>(srgb_to_linear(in.color.rgb), in.color.a);
    out.horizon = 1.0;
    return out;
}

@vertex
fn vs_line(in: LineIn) -> LineOut {
    return line_common(in);
}

// Lines lying on the surface are drawn without a depth test (they would
// z-fight with the globe), so the far side is culled analytically instead:
// a surface point P is visible from eye E when P·E >= R².
@vertex
fn vs_surface_line(in: LineIn) -> LineOut {
    var out = line_common(in);
    out.horizon = dot(normalize(in.high + in.low), globals.eye_unit.xyz) - 1.0;
    return out;
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    if (in.horizon < 0.0) {
        discard;
    }
    return in.color;
}
