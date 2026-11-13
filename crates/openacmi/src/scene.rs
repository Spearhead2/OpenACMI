//! Translates the recording at the displayed time into render data:
//! model instances, trail geometry and the list of visible objects used for
//! labels and picking.

use std::hash::{Hash, Hasher};

use acmi_core::geo;
use acmi_core::{known, Category, Object, ObjectHandle, Recording, TagSet};
use acmi_render::wgpu;
use acmi_render::{Camera, LineVertex, MeshId, MeshLibrary, ModelInstance, Renderer, RibbonVertex};
use glam::DVec3;
use rustc_hash::{FxHashMap, FxHasher};

use crate::config::{Rgba, Settings, Trails};
use crate::palette::{ObjectColors, Palette};

/// An object that exists at the displayed time and whose category is shown.
#[derive(Debug, Clone, Copy)]
pub struct VisibleObject {
    pub handle: ObjectHandle,
    pub position: DVec3,
}

pub struct SceneInputs<'a> {
    pub recording: &'a Recording,
    /// See [`crate::session::Session::generation`].
    pub generation: u64,
    pub time: f64,
    pub camera: &'a Camera,
    pub viewport_height: f32,
    pub palette: &'a Palette,
    pub settings: &'a Settings,
    pub meshes: &'a MeshLibrary,
    /// Object whose model is not drawn (the one the cockpit camera sits in).
    pub hidden: Option<ObjectHandle>,
}

/// Everything the trail geometry depends on; trails are rebuilt only when it changes.
#[derive(Debug, Clone, PartialEq)]
struct TrailKey {
    generation: u64,
    revision: u64,
    time: u64,
    visible: [bool; Category::COUNT],
    settings: (u32, u32, usize, bool, u32, bool, u32, u32),
}

#[derive(Default)]
pub struct Scene {
    pub instances: Vec<ModelInstance>,
    pub visible: Vec<VisibleObject>,
    trails: Vec<LineVertex>,
    ribbons: Vec<RibbonVertex>,
    trail_key: Option<TrailKey>,
    /// Model of each object, by handle. Rebuilt when identities change.
    object_meshes: Vec<Option<MeshId>>,
    object_meshes_key: Option<(u64, u64)>,
    /// (shape, name, tags) hash → model, so re-resolving is a lookup.
    mesh_cache: FxHashMap<u64, Option<MeshId>>,
}

impl Scene {
    /// Rebuilds per-frame data and uploads trails when they changed.
    pub fn update(&mut self, inputs: &SceneInputs<'_>, renderer: &mut Renderer, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.resolve_meshes(inputs, renderer, device);
        self.build_instances(inputs, renderer);

        let trails = &inputs.settings.trails;
        let key = TrailKey {
            generation: inputs.generation,
            revision: inputs.recording.revision(),
            time: inputs.time.to_bits(),
            visible: inputs.settings.objects.visible_categories,
            settings: (
                trails.past_seconds.to_bits(),
                trails.future_seconds.to_bits(),
                trails.max_points,
                trails.show_altitude_lines,
                trails.altitude_line_spacing_s.to_bits(),
                trails.ribbons,
                trails.ribbon_width.to_bits(),
                trails.ribbon_opacity.to_bits(),
            ),
        };
        if self.trail_key.as_ref() != Some(&key) {
            self.trail_key = Some(key);
            self.build_trails(inputs, renderer);
            renderer.set_trails(device, queue, &self.trails, &self.ribbons);
        }
    }

    /// Makes every object look its model up again, after the model library changed.
    pub fn forget_models(&mut self) {
        self.mesh_cache.clear();
        self.object_meshes_key = None;
    }

    /// Number of trail line vertices currently on the GPU.
    pub fn trail_vertex_count(&self) -> usize {
        self.trails.len()
    }

    fn resolve_meshes(&mut self, inputs: &SceneInputs<'_>, renderer: &mut Renderer, device: &wgpu::Device) {
        let key = (inputs.generation, inputs.recording.revision());
        if self.object_meshes_key == Some(key) {
            return;
        }
        self.object_meshes_key = Some(key);
        self.object_meshes.clear();
        for object in inputs.recording.objects() {
            let (shape, name, tags) = (object.text(known::SHAPE), object.name(), object.tags());
            let mut hasher = FxHasher::default();
            (shape, name, tags).hash(&mut hasher);
            let mesh = *self.mesh_cache.entry(hasher.finish()).or_insert_with(|| {
                let path = inputs.meshes.resolve(shape, name, tags);
                log::debug!("model for {name:?} ({tags:?}): {:?}", path.map(|p| p.file_name().unwrap_or_default()));
                renderer.load_mesh(device, path?)
            });
            self.object_meshes.push(mesh);
        }
    }

    fn build_instances(&mut self, inputs: &SceneInputs<'_>, renderer: &Renderer) {
        self.instances.clear();
        self.visible.clear();
        let objects = &inputs.settings.objects;

        for (handle, object) in inputs.recording.iter() {
            if !objects.visible_categories[object.category().index()] || !object.is_active_at(inputs.time) {
                continue;
            }
            let Some(pose) = object.pose_at(inputs.time) else { continue };
            let position = pose.world_position();
            let rotation = geo::orientation(position, pose.heading as f64, pose.pitch as f64, pose.roll as f64);
            self.visible.push(VisibleObject { handle, position });

            let Some(mesh) = self.object_meshes.get(handle.index()).copied().flatten() else { continue };
            if inputs.hidden == Some(handle) {
                continue;
            }
            // Enlarge distant models so they never shrink below a few pixels.
            let min_radius_px = objects.min_pixel_radius
                * match object.category() {
                    Category::Static | Category::Navaid => 0.4,
                    Category::Weapon => 0.7,
                    _ => 1.0,
                };
            let distance = position.distance(inputs.camera.eye);
            let meters_per_pixel = inputs.camera.meters_per_pixel(distance, inputs.viewport_height);
            let scale = (min_radius_px as f64 * meters_per_pixel / renderer.mesh_radius(mesh).max(0.01) as f64).max(1.0);

            self.instances.push(ModelInstance {
                mesh,
                position,
                rotation: rotation.as_quat(),
                scale: scale as f32,
                color: inputs.palette.for_object(object).display.rgba_f32(),
            });
        }
    }

    fn build_trails(&mut self, inputs: &SceneInputs<'_>, renderer: &Renderer) {
        self.trails.clear();
        self.ribbons.clear();
        let trails = &inputs.settings.trails;
        if !trails.show || (trails.past_seconds <= 0.0 && trails.future_seconds <= 0.0) {
            return;
        }
        for (handle, object) in inputs.recording.iter() {
            let category = object.category();
            let moves = !matches!(category, Category::Static | Category::Navaid) && object.track.len() >= 2;
            if moves && inputs.settings.objects.visible_categories[category.index()] && object.is_active_at(inputs.time) {
                // Aeroplanes and helicopters roll, and a ribbon shows it;
                // for everything else a line says all there is to say.
                let rolls = category == Category::Aircraft && !object.tags().intersects(TagSet::PARACHUTIST.union(TagSet::HUMAN));
                let ribbon = (trails.ribbons && rolls).then(|| {
                    let mesh = self.object_meshes.get(handle.index()).copied().flatten();
                    let size = mesh.map_or(RIBBON_DEFAULT_SIZE_M, |mesh| renderer.mesh_radius(mesh));
                    size * RIBBON_HALF_WIDTH_PER_SIZE * trails.ribbon_width.max(0.0)
                });
                let colors = inputs.palette.for_object(object);
                append_trail(&mut self.trails, &mut self.ribbons, object, colors, inputs.time, trails, ribbon);
            }
        }
    }
}

/// Half-width of a ribbon as a fraction of the model's bounding radius:
/// somewhat narrower than the wingspan, so the ribbon does not hide the model.
const RIBBON_HALF_WIDTH_PER_SIZE: f32 = 0.33;
/// Seconds of flight over which a ribbon fades in behind (and ahead of) the aircraft.
const RIBBON_FADE_IN_S: f64 = 0.6;
/// Cross-sections added through that fade so it is a smooth ramp.
const RIBBON_FADE_IN_STEPS: usize = 4;
/// Bounding radius assumed for an aircraft drawn without a model.
const RIBBON_DEFAULT_SIZE_M: f32 = 9.0;

/// Appends the trail of one object: line segments, or with `ribbon` (its
/// half-width in metres) a band lying in the plane of the object's wings.
/// Altitude lines are line segments either way.
///
/// The trail fades out over `past_seconds` behind the object and over
/// `future_seconds` ahead of it, and ends exactly at the object's
/// interpolated position rather than at the last recorded sample.
fn append_trail(
    out: &mut Vec<LineVertex>,
    ribbons: &mut Vec<RibbonVertex>,
    object: &Object,
    colors: &ObjectColors,
    time: f64,
    settings: &Trails,
    ribbon: Option<f32>,
) {
    /// Shells are numerous and fast; a short streak is enough.
    const SHELL_POINTS: usize = 4;

    let track = &object.track;
    let times = track.times();
    let Some(head) = object.pose_at(time).map(|pose| pose.world_position()) else { return };
    let current = track.index_at(time);
    let is_shell = object.tags().contains(TagSet::SHELL);

    let (past, future) = (settings.past_seconds as f64, if is_shell { 0.0 } else { settings.future_seconds as f64 });
    let mut first = times.partition_point(|&t| t < time - past).min(current);
    if is_shell {
        first = first.max(current.saturating_sub(SHELL_POINTS));
    }
    let last = times.partition_point(|&t| t <= time + future).saturating_sub(1).max(current);

    // A ribbon takes the colour the model is drawn in.
    let rgb = if ribbon.is_some() { colors.display.0 } else { colors.trail.0 };
    let color = |alpha: f64| [rgb[0], rgb[1], rgb[2], (alpha.clamp(0.0, 1.0) * 255.0) as u8];
    // Direction of the right wing when the object was at a point of its trail.
    let side_at = |position: DVec3, at: f64| {
        let pose = object.pose_at(at);
        let (heading, pitch, roll) = pose.map_or((0.0, 0.0, 0.0), |pose| (pose.heading as f64, pose.pitch as f64, pose.roll as f64));
        geo::orientation(position, heading, pitch, roll) * DVec3::X
    };
    // One stretch of trail between two `(position, time, opacity)` points.
    type Point = (DVec3, f64, f64);
    let segment = |out: &mut Vec<LineVertex>, ribbons: &mut Vec<RibbonVertex>, from: Point, to: Point| match ribbon {
        Some(half_width) => {
            let section = |(position, at, alpha): Point| (position, side_at(position, at), color(alpha));
            RibbonVertex::push_band(ribbons, half_width, section(from), section(to));
        }
        None => {
            out.push(LineVertex::new(from.0, color(from.2)));
            out.push(LineVertex::new(to.0, color(to.2)));
        }
    };
    // A line is solid at the object and fades evenly with age. A ribbon is
    // see-through, eases out with age, and also eases in over its first
    // moments so that it does not start as a hard edge at the aircraft.
    let ease = |x: f64| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    let opacity = (settings.ribbon_opacity as f64).clamp(0.0, 1.0);
    let fade = |age: f64, span: f64| match ribbon {
        _ if span <= 0.0 => 0.0,
        Some(_) => opacity * ease(1.0 - age / span) * ease(age / RIBBON_FADE_IN_S.min(span * 0.5)),
        None => 1.0 - age / span,
    };
    let past_alpha = |t: f64| fade(time - t, past);
    let future_alpha = |t: f64| fade(t - time, future);
    let head_alpha = if ribbon.is_some() { 0.0 } else { 1.0 };
    // Thin out very dense trails, always keeping the newest point.
    let stride = ((last - first + 1) / settings.max_points.max(2)).max(1);

    // Behind the object: first → current → head.
    if past > 0.0 && times[current] <= time {
        let indices = (first..current).step_by(stride).chain(std::iter::once(current));
        let mut points: Vec<Point> = indices.map(|index| (track.world_position(index), times[index], past_alpha(times[index]))).collect();
        if ribbon.is_some() {
            // Extra cross-sections through the fade-in, which is shorter
            // than the time between samples of many recordings.
            for step in 1..=RIBBON_FADE_IN_STEPS {
                let at = time - RIBBON_FADE_IN_S * step as f64 / RIBBON_FADE_IN_STEPS as f64;
                if let Some(pose) = object.pose_at(at).filter(|_| at > times[first]) {
                    points.push((pose.world_position(), at, past_alpha(at)));
                }
            }
            points.sort_by(|a, b| a.1.total_cmp(&b.1));
        }
        points.push((head, time, head_alpha));
        for pair in points.windows(2) {
            segment(out, ribbons, pair[0], pair[1]);
        }

        // Altitude lines: where the object was, projected onto the ground.
        let wants_altitude_lines =
            settings.show_altitude_lines && object.category() != Category::Weapon && !object.tags().contains(TagSet::DECOY);
        if wants_altitude_lines {
            let tail_rgb = colors.trail_tails.0;
            // One line each time the flight passes a multiple of the spacing,
            // so the lines stay where they are as the trail moves on.
            let spacing = (settings.altitude_line_spacing_s as f64).max(0.05);
            let slot = |t: f64| (t / spacing).floor();
            let mut last_slot = if first > 0 { slot(times[first - 1]) } else { f64::NEG_INFINITY };
            for index in (first..=current).step_by(stride) {
                let this_slot = slot(times[index]);
                let (lat, lon, alt) = track.position(index);
                if this_slot == last_slot || alt.abs() < 1.0 {
                    continue;
                }
                last_slot = this_slot;
                let alpha = (past_alpha(times[index]).clamp(0.0, 1.0) * 0.3 * 255.0) as u8;
                let tail_color = Rgba([tail_rgb[0], tail_rgb[1], tail_rgb[2], alpha]).0;
                out.push(LineVertex::new(track.world_position(index), tail_color));
                out.push(LineVertex::new(geo::geo_to_world(lat, lon, 0.0), tail_color));
            }
        }
    }

    // Ahead of the object: head → current+1 → last.
    if future > 0.0 && last > current {
        let mut previous = (head, time, head_alpha);
        for index in (current + 1..=last).step_by(stride) {
            let point = (track.world_position(index), times[index], future_alpha(times[index]));
            segment(out, ribbons, previous, point);
            previous = point;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acmi_core::Parser;

    fn recording() -> Recording {
        let mut text = String::from("FileType=text/acmi/tacview\nFileVersion=2.2\n");
        for second in 0..=20 {
            text += &format!("#{second}\n1,T={}|10|3000,Type=Air+FixedWing,Color=Blue\n", 10.0 + second as f64 * 0.01);
        }
        let mut recording = Recording::new();
        Parser::new().feed_text(&mut recording, &text, |_| {}).unwrap();
        recording
    }

    #[test]
    fn trail_covers_the_requested_window() {
        let recording = recording();
        let object = &recording.objects()[0];
        let colors = *Palette::default().for_object(object);
        let settings = Trails { past_seconds: 5.0, future_seconds: 3.0, show_altitude_lines: false, ..Default::default() };

        let (mut lines, mut ribbons) = (Vec::new(), Vec::new());
        append_trail(&mut lines, &mut ribbons, object, &colors, 10.5, &settings, None);
        assert!(ribbons.is_empty());
        // Past: samples 6..=10 give 4 segments plus one to the head.
        // Future: head → 11 → 12 → 13 gives 3 segments.
        assert_eq!(lines.len(), (4 + 1 + 3) * 2);
        let head = LineVertex::new(object.pose_at(10.5).unwrap().world_position(), [0; 4]).position;
        assert_eq!(lines[9].position, head);
        assert_eq!(lines[9].color[3], 255, "opaque at the object");
        assert!(lines[0].color[3] < 40, "faded at the old end");
        assert!(lines.last().unwrap().color[3] < 60, "faded at the future end");
    }

    #[test]
    fn altitude_lines_reach_the_ground() {
        let recording = recording();
        let object = &recording.objects()[0];
        let colors = *Palette::default().for_object(object);
        let settings = Trails { past_seconds: 10.0, altitude_line_spacing_s: 2.0, ..Default::default() };

        let (mut with_lines, mut without, mut ribbons) = (Vec::new(), Vec::new(), Vec::new());
        append_trail(&mut with_lines, &mut ribbons, object, &colors, 20.0, &settings, None);
        let bare = Trails { show_altitude_lines: false, ..settings.clone() };
        append_trail(&mut without, &mut ribbons, object, &colors, 20.0, &bare, None);
        // Samples 10, 12, …, 20 → 6 vertical lines.
        assert_eq!(with_lines.len() - without.len(), 6 * 2);

        // They stay put as time moves on: half a second later the same six.
        let mut later = Vec::new();
        append_trail(&mut later, &mut ribbons, object, &colors, 20.0, &Trails { past_seconds: 9.5, ..settings.clone() }, None);
        let ground = |lines: &[LineVertex]| lines.iter().rev().step_by(2).take(5).map(|vertex| vertex.position).collect::<Vec<_>>();
        assert_eq!(ground(&later), ground(&with_lines));
    }

    #[test]
    fn aircraft_trails_can_be_ribbons() {
        let recording = recording();
        let object = &recording.objects()[0];
        let colors = *Palette::default().for_object(object);
        let settings = Trails { past_seconds: 5.0, future_seconds: 3.0, show_altitude_lines: false, ..Default::default() };

        let (mut lines, mut ribbons) = (Vec::new(), Vec::new());
        append_trail(&mut lines, &mut ribbons, object, &colors, 10.5, &settings, Some(6.0));
        // The 8 stretches of the line trail plus 4 through the fade-in behind
        // the aircraft, two triangles each, and no lines.
        assert!(lines.is_empty());
        assert_eq!(ribbons.len(), (8 + 4) * 6);
        let first = ribbons[0];
        assert_eq!((first.across, ribbons[1].across, first.half_width), (-1.0, 1.0, 6.0));
        // In the colour the model is drawn with, fading like the line does.
        assert_eq!(first.color[..3], colors.display.0[..3]);
        assert!(first.color[3] < 40);
        // See-through throughout, and fading in from nothing at the aircraft.
        let most = ribbons.iter().map(|vertex| vertex.color[3]).max().unwrap();
        assert!(most > 40 && most < (settings.ribbon_opacity * 255.0) as u8 + 1, "{most}");
        let at_head: Vec<u8> = ribbons.iter().filter(|vertex| vertex.position == LineVertex::new(object.pose_at(10.5).unwrap().world_position(), [0; 4]).position).map(|vertex| vertex.color[3]).collect();
        assert!(!at_head.is_empty() && at_head.iter().all(|&alpha| alpha == 0));
        // Flying east with wings level: the right wing points south.
        let side = DVec3::from(first.side.map(f64::from));
        let frame = geo::LocalFrame::at(object.pose_at(6.0).unwrap().world_position());
        assert!(side.dot(-frame.north) > 0.99, "{side:?}");
    }
}
