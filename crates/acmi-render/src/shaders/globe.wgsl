// The Earth and its atmosphere.

@group(1) @binding(0) var earth_texture: texture_2d<f32>;
@group(1) @binding(1) var earth_sampler: sampler;

struct SphereIn {
    @location(0) high: vec3<f32>,
    @location(1) low: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct SphereOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    // From the eye to the fragment.
    @location(2) view: vec3<f32>,
};

@vertex
fn vs_sphere(in: SphereIn) -> SphereOut {
    var out: SphereOut;
    let relative = rte(in.high, in.low);
    out.clip = globals.view_proj * vec4<f32>(relative, 1.0);
    out.normal = normalize(in.high + in.low);
    out.uv = in.uv;
    out.view = relative;
    return out;
}

const OCEAN: vec3<f32> = vec3<f32>(0.012, 0.045, 0.11);

@fragment
fn fs_globe(in: SphereOut) -> @location(0) vec4<f32> {
    let normal = normalize(in.normal);
    let texel = textureSample(earth_texture, earth_sampler, in.uv).rgb;
    let albedo = mix(OCEAN, texel, globals.params.x);

    return shade_ground(albedo, normal, in.view);
}

// ── Atmosphere ───────────────────────────────────────────────────────────────
//
// Single scattering, integrated numerically along the view ray: Rayleigh
// (air; blue sky, red sunsets) and Mie (haze; the pale horizon and the glow
// around the sun), each with an exponential density profile. Lengths are in
// kilometres. The same function gives the sky, the glow around the limb seen
// from space, and the haze in front of distant ground, so the three always
// agree and nothing ends at a mesh boundary.
//
// Cost: VIEW_STEPS x LIGHT_STEPS exponentials per pixel, no textures.

const ATMOSPHERE_KM: f32 = 80.0;
// Scale heights: density falls by e every this many kilometres.
const RAYLEIGH_KM: f32 = 8.0;
const MIE_KM: f32 = 1.2;
// Scattering per kilometre at sea level.
const RAYLEIGH: vec3<f32> = vec3<f32>(0.0058, 0.0135, 0.0331);
const MIE: f32 = 0.004;
const MIE_EXTINCTION: f32 = 0.0044;
const MIE_ANISOTROPY: f32 = 0.76;
// Ground in full sun is drawn at its albedo, which corresponds to an irradiance of pi;
// a little more makes up for the multiple scattering this model leaves out.
const SUN_INTENSITY: f32 = 4.5;
const GROUND_STEPS: i32 = 8;
const SKY_STEPS: i32 = 16;
const LIGHT_STEPS: i32 = 4;
const PI: f32 = 3.14159265;

struct Scatter {
    // Light scattered towards the eye along the ray (tone-mapped).
    light: vec3<f32>,
    // Fraction of the light from behind that gets through.
    transmittance: vec3<f32>,
};

// Distances at which a ray enters and leaves a sphere around the Earth's
// centre; `x > y` when it misses.
fn sphere_span(origin: vec3<f32>, dir: vec3<f32>, radius: f32) -> vec2<f32> {
    let distance = length(origin);
    let b = dot(origin, dir);
    // Factored to stay accurate near the surface.
    let c = (distance - radius) * (distance + radius);
    let discriminant = b * b - c;
    if (discriminant < 0.0) {
        return vec2<f32>(1.0, -1.0);
    }
    let root = sqrt(discriminant);
    return vec2<f32>(-b - root, -b + root);
}

// Rayleigh and Mie density at a point, relative to sea level.
fn air_density(point: vec3<f32>, radius: f32) -> vec2<f32> {
    let height = max(length(point) - radius, 0.0);
    return exp(vec2<f32>(-height / RAYLEIGH_KM, -height / MIE_KM)) * globals.atmosphere.x;
}

// The eye in kilometres from the Earth's centre, kept above the surface.
fn eye_km() -> vec3<f32> {
    let radius = globals.atmosphere.z;
    let eye = globals.eye_unit.xyz * radius;
    return normalize(eye) * max(length(eye), radius + 0.002);
}

// Scattering along the ray from `origin` towards `dir` (unit), up to `limit` km.
fn scatter(origin: vec3<f32>, dir: vec3<f32>, limit: f32, steps: i32) -> Scatter {
    var result: Scatter;
    result.light = vec3<f32>(0.0);
    result.transmittance = vec3<f32>(1.0);
    if (globals.atmosphere.x <= 0.0) {
        return result;
    }
    let radius = globals.atmosphere.z;
    let top = radius + ATMOSPHERE_KM;
    let span = sphere_span(origin, dir, top);
    let start = max(span.x, 0.0);
    let end = min(span.y, limit);
    if (end <= start) {
        return result;
    }

    // Without sun lighting the whole sky is lit from straight above.
    let lit = globals.params.y > 0.5;
    let sun = globals.sun.xyz;
    let step = (end - start) / f32(steps);
    var depth = vec2<f32>(0.0);
    var rayleigh = vec3<f32>(0.0);
    var mie = vec3<f32>(0.0);

    for (var i = 0; i < steps; i++) {
        let point = origin + dir * (start + (f32(i) + 0.5) * step);
        let density = air_density(point, radius) * step;
        depth += density * 0.5;

        let to_sun = select(normalize(point), sun, lit);
        // In the Earth's shadow no sunlight arrives.
        let ground = sphere_span(point, to_sun, radius);
        if (ground.x > ground.y || ground.y < 0.0) {
            let light_step = sphere_span(point, to_sun, top).y / f32(LIGHT_STEPS);
            var light_depth = vec2<f32>(0.0);
            for (var j = 0; j < LIGHT_STEPS; j++) {
                light_depth += air_density(point + to_sun * ((f32(j) + 0.5) * light_step), radius) * light_step;
            }
            let path = depth + light_depth;
            let arriving = exp(-(RAYLEIGH * path.x + MIE_EXTINCTION * path.y));
            rayleigh += arriving * density.x;
            mie += arriving * density.y;
        }
        depth += density * 0.5;
    }

    let cos_sun = select(0.0, dot(dir, sun), lit);
    let g = MIE_ANISOTROPY;
    let rayleigh_phase = 3.0 / (16.0 * PI) * (1.0 + cos_sun * cos_sun);
    let mie_phase = (1.0 - g * g) / (4.0 * PI * pow(1.0 + g * g - 2.0 * g * cos_sun, 1.5));
    let radiance = SUN_INTENSITY * (rayleigh * RAYLEIGH * rayleigh_phase + mie * MIE * mie_phase);

    result.light = vec3<f32>(1.0) - exp(-radiance * globals.atmosphere.y);
    result.transmittance = exp(-(RAYLEIGH * depth.x + MIE_EXTINCTION * depth.y));
    return result;
}

// Exponent applied to the sine of the sun's elevation; 1 is physical.
const LOW_SUN_LIFT: f32 = 0.6;

// Sunlight on the ground, seen through the air between it and the eye.
fn shade_ground(albedo: vec3<f32>, normal: vec3<f32>, view: vec3<f32>) -> vec4<f32> {
    let ambient = globals.sun.w;
    // Physically the light on level ground falls with the sine of the sun's
    // elevation, which leaves mornings and evenings too dark to read a map
    // by. LOW_SUN_LIFT flattens that: full daylight is unchanged, a sun 10
    // degrees up gives about half of it instead of a sixth, and the
    // terminator stays where it is.
    let diffuse = pow(max(dot(normal, globals.sun.xyz), 0.0), LOW_SUN_LIFT);
    let light = mix(1.0, ambient + (1.0 - ambient) * diffuse, globals.params.y);
    let distance = length(view);
    let air = scatter(eye_km(), view / max(distance, 1e-3), distance * 0.001, GROUND_STEPS);
    return vec4<f32>(albedo * light * air.transmittance + air.light, 1.0);
}

// A streamed terrain tile: its texture is the elevation-derived ground colour.
// Lighting uses the sphere normal; relief shading is baked into the texture.
@fragment
fn fs_terrain(in: SphereOut) -> @location(0) vec4<f32> {
    let texel = textureSample(earth_texture, earth_sampler, in.uv).rgb;
    return shade_ground(texel, normalize(in.normal), in.view);
}

// Terrain tiles are Web-Mercator and end at this latitude (as sin).
const MERCATOR_LIMIT: f32 = 0.99627;
// Typical ice-sheet elevation where the southern tiles end, metres.
const ANTARCTIC_PLATEAU: f32 = 2800.0;
const POLAR_SEA: vec3<f32> = vec3<f32>(0.012, 0.045, 0.13);
const POLAR_ICE: vec3<f32> = vec3<f32>(0.89, 0.91, 0.95);

// The plain globe mesh, used only beyond the Mercator limit: open sea over
// the north pole, raised ice over the south.
@vertex
fn vs_polar(in: SphereIn) -> SphereOut {
    var out: SphereOut;
    let normal = normalize(in.high + in.low);
    let lift = normal * select(0.0, ANTARCTIC_PLATEAU, normal.y < 0.0);
    let relative = rte(in.high, in.low) + lift;
    out.clip = globals.view_proj * vec4<f32>(relative, 1.0);
    out.normal = normal;
    out.uv = in.uv;
    out.view = relative;
    return out;
}

@fragment
fn fs_polar(in: SphereOut) -> @location(0) vec4<f32> {
    let normal = normalize(in.normal);
    if (abs(normal.y) < MERCATOR_LIMIT) {
        discard;
    }
    let albedo = select(POLAR_ICE, POLAR_SEA, normal.y > 0.0);
    return shade_ground(albedo, normal, in.view);
}

// The sky: one triangle covering the screen at the far plane, so it shows
// only where no ground was drawn. Output is premultiplied: the scattered
// light, over whatever is behind (space) dimmed by the air.
struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) dir: vec3<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) index: u32) -> SkyOut {
    var out: SkyOut;
    let ndc = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u)) * 2.0 - 1.0;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.dir = globals.sky_forward.xyz + ndc.x * globals.sky_right.xyz + ndc.y * globals.sky_up.xyz;
    return out;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let air = scatter(eye_km(), normalize(in.dir), 1e9, SKY_STEPS);
    let through = (air.transmittance.r + air.transmittance.g + air.transmittance.b) / 3.0;
    return vec4<f32>(air.light, 1.0 - through);
}
