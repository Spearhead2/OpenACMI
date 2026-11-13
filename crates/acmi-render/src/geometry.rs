//! CPU-side geometry: vertex formats and the static meshes of the globe.

use acmi_core::geo::{self, EARTH_RADIUS_M};
use bytemuck::{Pod, Zeroable};
use glam::DVec3;

/// A double-precision world position split into two `f32` parts per axis.
///
/// The GPU subtracts the equally split eye position part by part ("relative
/// to eye" rendering), which keeps millimetre accuracy at planetary
/// distances without rebuilding vertex buffers when the camera moves.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct RtePosition {
    pub high: [f32; 3],
    pub low: [f32; 3],
}

impl From<DVec3> for RtePosition {
    #[inline]
    fn from(p: DVec3) -> Self {
        // `high` is a multiple of 65536 (exact in f32 far beyond Earth scale);
        // `low` is the remainder in [0, 65536), resolved to a few millimetres.
        let high = (p / 65_536.0).floor() * 65_536.0;
        Self { high: high.as_vec3().to_array(), low: (p - high).as_vec3().to_array() }
    }
}

/// One end of a line segment. Colour is sRGB with straight alpha.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct LineVertex {
    pub position: RtePosition,
    pub color: [u8; 4],
}

impl LineVertex {
    #[inline]
    pub fn new(world: DVec3, color: [u8; 4]) -> Self {
        Self { position: world.into(), color }
    }
}

/// One corner of a ribbon trail: a point on the trail's centre line, pushed
/// out along `side` by the renderer. Colour is sRGB with straight alpha.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct RibbonVertex {
    pub position: RtePosition,
    /// Unit vector along the object's right wing, in world axes.
    pub side: [f32; 3],
    /// Half the ribbon's real width in metres; it is widened on screen when
    /// that would be too thin to see.
    pub half_width: f32,
    /// −1 for the left edge, +1 for the right edge.
    pub across: f32,
    pub color: [u8; 4],
}

impl RibbonVertex {
    /// Appends the band between two cross-sections of a trail, each given as
    /// `(centre, side, colour)`, as two triangles. They wind counter-clockwise
    /// seen from above the wings when going from `from` to `to`, which is how
    /// the renderer tells the upper face from the lower.
    pub fn push_band(out: &mut Vec<RibbonVertex>, half_width: f32, from: (DVec3, DVec3, [u8; 4]), to: (DVec3, DVec3, [u8; 4])) {
        let corner = |(centre, side, color): (DVec3, DVec3, [u8; 4]), across: f32| RibbonVertex {
            position: centre.into(),
            side: side.as_vec3().to_array(),
            half_width,
            across,
            color,
        };
        let (left_0, right_0, left_1, right_1) = (corner(from, -1.0), corner(from, 1.0), corner(to, -1.0), corner(to, 1.0));
        out.extend([left_0, right_0, left_1, right_0, right_1, left_1]);
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct SphereVertex {
    pub position: RtePosition,
    pub uv: [f32; 2],
}

/// Latitude/longitude sphere with equirectangular texture coordinates
/// (`u` 0→1 from 180°W to 180°E, `v` 0→1 from north to south pole).
/// Triangles wind counter-clockwise seen from outside.
pub fn uv_sphere(radius: f64, segments: u32, rings: u32) -> (Vec<SphereVertex>, Vec<u32>) {
    let mut vertices = Vec::with_capacity(((segments + 1) * (rings + 1)) as usize);
    for ring in 0..=rings {
        let v = ring as f64 / rings as f64;
        for segment in 0..=segments {
            let u = segment as f64 / segments as f64;
            let world = geo::geo_to_world(90.0 - 180.0 * v, -180.0 + 360.0 * u, radius - EARTH_RADIUS_M);
            vertices.push(SphereVertex { position: world.into(), uv: [u as f32, v as f32] });
        }
    }

    let mut indices = Vec::with_capacity((segments * rings * 6) as usize);
    let stride = segments + 1;
    for ring in 0..rings {
        for segment in 0..segments {
            let a = ring * stride + segment;
            let (b, c, d) = (a + 1, a + stride, a + stride + 1);
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    (vertices, indices)
}

/// Appearance of the latitude/longitude grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridStyle {
    pub lat_step_deg: f64,
    pub lon_step_deg: f64,
    pub color: [u8; 4],
    pub equator_color: [u8; 4],
    /// Spacing of the fine grid shown when the camera is close to the ground.
    pub fine_step_deg: f64,
    pub fine_color: [u8; 4],
}

/// Parallels and meridians on the surface, as a line list.
pub fn grid_lines(style: &GridStyle) -> Vec<LineVertex> {
    graticule(style.lat_step_deg, style.lon_step_deg, style.color, style.equator_color, None)
}

/// The fine grid, without the lines the coarse grid already draws.
pub fn fine_grid_lines(style: &GridStyle) -> Vec<LineVertex> {
    let step = style.fine_step_deg;
    graticule(step, step, style.fine_color, style.fine_color, Some((style.lat_step_deg, style.lon_step_deg)))
}

/// `skip` gives `(lat, lon)` spacings whose multiples are left out.
fn graticule(
    lat_step: f64,
    lon_step: f64,
    line_color: [u8; 4],
    equator_color: [u8; 4],
    skip: Option<(f64, f64)>,
) -> Vec<LineVertex> {
    const SEGMENT_DEG: f64 = 1.0;
    let is_multiple = |value: f64, step: f64| ((value / step).round() * step - value).abs() < 1e-6;
    let mut lines = Vec::new();
    let mut polyline = |points: &mut dyn Iterator<Item = DVec3>, color: [u8; 4]| {
        let mut previous: Option<DVec3> = None;
        for point in points {
            if let Some(previous) = previous {
                lines.push(LineVertex::new(previous, color));
                lines.push(LineVertex::new(point, color));
            }
            previous = Some(point);
        }
    };

    let lat_step = lat_step.max(0.25);
    let lon_step = lon_step.max(0.25);
    let steps = |span: f64| (span / SEGMENT_DEG) as u32;

    let mut lat = -90.0 + lat_step;
    while lat < 90.0 - 1e-9 {
        if !skip.is_some_and(|(coarse, _)| is_multiple(lat, coarse)) {
            let color = if lat.abs() < 1e-9 { equator_color } else { line_color };
            polyline(&mut (0..=steps(360.0)).map(|i| geo::geo_to_world(lat, -180.0 + i as f64 * SEGMENT_DEG, 0.0)), color);
        }
        lat += lat_step;
    }
    let mut lon = -180.0;
    while lon < 180.0 - 1e-9 {
        if !skip.is_some_and(|(_, coarse)| is_multiple(lon, coarse)) {
            polyline(&mut (0..=steps(180.0)).map(|i| geo::geo_to_world(-90.0 + i as f64 * SEGMENT_DEG, lon, 0.0)), line_color);
        }
        lon += lon_step;
    }
    lines
}

/// Decodes the bundled country-border data into a line list.
///
/// Layout (little endian): `u32 ring_count`, then per ring `u32 point_count`
/// followed by `point_count × (f32 lon, f32 lat)`. Rings are closed.
pub fn border_lines(data: &[u8], color: [u8; 4]) -> Vec<LineVertex> {
    fn read_u32(data: &[u8], offset: &mut usize) -> Option<u32> {
        let bytes = data.get(*offset..*offset + 4)?;
        *offset += 4;
        Some(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    let mut lines = Vec::new();
    let mut offset = 0;
    let Some(ring_count) = read_u32(data, &mut offset) else { return lines };
    for _ in 0..ring_count {
        let Some(point_count) = read_u32(data, &mut offset) else { break };
        let Some(raw) = data.get(offset..offset + point_count as usize * 8) else { break };
        offset += raw.len();
        if point_count < 2 {
            continue;
        }
        let point = |i: usize| {
            let lon = f32::from_le_bytes(raw[i * 8..i * 8 + 4].try_into().unwrap());
            let lat = f32::from_le_bytes(raw[i * 8 + 4..i * 8 + 8].try_into().unwrap());
            geo::geo_to_world(lat as f64, lon as f64, 0.0)
        };
        let count = point_count as usize;
        for i in 0..count {
            lines.push(LineVertex::new(point(i), color));
            lines.push(LineVertex::new(point((i + 1) % count), color));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rte_split_is_exact_enough() {
        let world = DVec3::new(6_371_123.456_789, -4_000_000.25, 12.5);
        let rte = RtePosition::from(world);
        let rebuilt = DVec3::from_array(rte.high.map(f64::from)) + DVec3::from_array(rte.low.map(f64::from));
        assert!((rebuilt - world).length() < 0.01, "{}", (rebuilt - world).length());
        assert!(rte.low.iter().all(|&l| (0.0..65_536.0).contains(&l)));
    }

    #[test]
    fn sphere_topology() {
        let (vertices, indices) = uv_sphere(EARTH_RADIUS_M, 8, 4);
        assert_eq!(vertices.len(), 9 * 5);
        assert_eq!(indices.len(), 8 * 4 * 6);
        assert!(indices.iter().all(|&i| (i as usize) < vertices.len()));
    }

    #[test]
    fn borders_decode_closed_rings() {
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&3u32.to_le_bytes());
        for (lon, lat) in [(0f32, 0f32), (1.0, 0.0), (1.0, 1.0)] {
            data.extend_from_slice(&lon.to_le_bytes());
            data.extend_from_slice(&lat.to_le_bytes());
        }
        let lines = border_lines(&data, [255; 4]);
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[5].position, lines[0].position, "ring closes on its first point");
        // Truncated input must not panic.
        assert!(border_lines(&data[..10], [255; 4]).is_empty());
    }

    #[test]
    fn grid_has_equator() {
        let style = GridStyle {
            lat_step_deg: 30.0,
            lon_step_deg: 30.0,
            color: [1, 1, 1, 255],
            equator_color: [9, 9, 9, 255],
            fine_step_deg: 10.0,
            fine_color: [5, 5, 5, 255],
        };
        let lines = grid_lines(&style);
        assert!(lines.len().is_multiple_of(2) && !lines.is_empty());
        assert!(lines.iter().any(|v| v.color == style.equator_color));

        // 5 parallels + 12 meridians coarse; 17 + 36 at 10 degrees, minus the shared ones.
        let fine = fine_grid_lines(&style);
        assert_eq!(lines.len(), (5 * 360 + 12 * 180) * 2);
        assert_eq!(fine.len(), ((17 - 5) * 360 + (36 - 12) * 180) * 2);
    }
}
