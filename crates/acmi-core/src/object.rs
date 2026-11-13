//! Objects and their time series.
//!
//! An [`Object`] is one ACMI object (aircraft, missile, building, …). Its
//! motion lives in a [`Track`], a structure-of-arrays history that is cheap
//! to bisect and interpolate. Every other property is stored as a sparse
//! series holding only the instants at which the recording set it, so the
//! state of any object can be reconstructed at any time.

use crate::geo;
use crate::props::{known, PropId, PropKind, Unit};
use crate::tags::{Category, TagSet};

/// Object identifier as written in the file (hexadecimal, up to 64 bits).
pub type ObjectId = u64;

// ─────────────────────────────────────────────────────────────────────────────
// Track
// ─────────────────────────────────────────────────────────────────────────────

/// Fields of one `T=` update. Absent fields keep their previous value.
///
/// Longitude and latitude are absolute degrees: the parser has already added
/// the recording's reference point.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TransformUpdate {
    pub lon: Option<f64>,
    pub lat: Option<f64>,
    pub alt: Option<f32>,
    pub roll: Option<f32>,
    pub pitch: Option<f32>,
    pub yaw: Option<f32>,
}

/// Position and attitude at one instant. Never contains NaN.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub lon: f64,
    pub lat: f64,
    /// Metres above mean sea level.
    pub alt: f64,
    /// Degrees, positive right wing down.
    pub roll: f32,
    /// Degrees, positive nose up.
    pub pitch: f32,
    /// Degrees clockwise from true north, `[0, 360)`.
    pub heading: f32,
}

impl Pose {
    pub fn world_position(&self) -> glam::DVec3 {
        geo::geo_to_world(self.lat, self.lon, self.alt)
    }
}

/// Motion derived from neighbouring track samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Kinematics {
    /// 3D speed over the ground, m/s.
    pub speed: f64,
    /// Climb rate, m/s.
    pub vertical_speed: f64,
    /// Rate of change of course, °/s, positive turning right.
    pub turn_rate: f64,
}

/// Time over which [`Track::kinematics`] averages, centred on the moment asked for.
pub const KINEMATICS_WINDOW_S: f64 = 2.0;
/// Samples further apart than this are not averaged across.
const KINEMATICS_MAX_GAP_S: f64 = 5.0;

/// Transform history of an object.
///
/// All columns have the same length, except `orient` which stays empty for
/// objects that never report an attitude (most static and many ground units).
#[derive(Debug, Clone)]
pub struct Track {
    time: Vec<f64>,
    lon: Vec<f64>,
    lat: Vec<f64>,
    alt: Vec<f32>,
    /// `[roll, pitch, yaw]`, NaN where unknown.
    orient: Vec<[f32; 3]>,
    /// Running state that partial updates are merged into.
    cur_lon: f64,
    cur_lat: f64,
    cur_alt: f32,
    cur_orient: [f32; 3],
}

impl Default for Track {
    fn default() -> Self {
        Self {
            time: Vec::new(),
            lon: Vec::new(),
            lat: Vec::new(),
            alt: Vec::new(),
            orient: Vec::new(),
            cur_lon: f64::NAN,
            cur_lat: f64::NAN,
            cur_alt: f32::NAN,
            cur_orient: [f32::NAN; 3],
        }
    }
}

impl Track {
    #[inline]
    pub fn len(&self) -> usize {
        self.time.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.time.is_empty()
    }

    #[inline]
    pub fn times(&self) -> &[f64] {
        &self.time
    }

    pub fn first_time(&self) -> Option<f64> {
        self.time.first().copied()
    }

    pub fn last_time(&self) -> Option<f64> {
        self.time.last().copied()
    }

    /// `(lat, lon, alt)` of sample `i`.
    #[inline]
    pub fn position(&self, i: usize) -> (f64, f64, f64) {
        (self.lat[i], self.lon[i], self.alt[i] as f64)
    }

    /// World-space position of sample `i`.
    #[inline]
    pub fn world_position(&self, i: usize) -> glam::DVec3 {
        geo::geo_to_world(self.lat[i], self.lon[i], self.alt[i] as f64)
    }

    /// Recorded `[roll, pitch, yaw]` of sample `i` (NaN where unknown).
    #[inline]
    pub fn recorded_orientation(&self, i: usize) -> [f32; 3] {
        self.orient.get(i).copied().unwrap_or([f32::NAN; 3])
    }

    /// Whether any sample carries attitude data.
    #[inline]
    pub fn has_orientation(&self) -> bool {
        !self.orient.is_empty()
    }

    /// Index of the last sample at or before `t` (0 when `t` precedes the track).
    #[inline]
    pub fn index_at(&self, t: f64) -> usize {
        self.time.partition_point(|&x| x <= t).saturating_sub(1)
    }

    /// Merges an update and records a sample at `t`.
    ///
    /// Returns `false` while the object still has no usable position.
    pub(crate) fn push(&mut self, t: f64, update: &TransformUpdate) -> bool {
        if let Some(v) = update.lon {
            self.cur_lon = v;
        }
        if let Some(v) = update.lat {
            self.cur_lat = v;
        }
        if let Some(v) = update.alt {
            self.cur_alt = v;
        }
        for (slot, value) in self.cur_orient.iter_mut().zip([update.roll, update.pitch, update.yaw]) {
            if let Some(v) = value {
                *slot = v;
            }
        }
        if self.cur_lon.is_nan() || self.cur_lat.is_nan() {
            return false;
        }

        let alt = if self.cur_alt.is_nan() { 0.0 } else { self.cur_alt };
        let has_orient = self.cur_orient.iter().any(|v| !v.is_nan());
        if has_orient && self.orient.is_empty() {
            self.orient.resize(self.time.len(), [f32::NAN; 3]);
        }
        let store_orient = has_orient || !self.orient.is_empty();

        match self.time.last().copied() {
            // Several updates within one frame: the last one wins.
            Some(last) if t == last => {
                let i = self.time.len() - 1;
                self.lon[i] = self.cur_lon;
                self.lat[i] = self.cur_lat;
                self.alt[i] = alt;
                if store_orient {
                    self.orient[i] = self.cur_orient;
                }
            }
            // Out-of-order frame (rare; some live sources): keep the track sorted.
            Some(last) if t < last => {
                let i = self.time.partition_point(|&x| x <= t);
                self.time.insert(i, t);
                self.lon.insert(i, self.cur_lon);
                self.lat.insert(i, self.cur_lat);
                self.alt.insert(i, alt);
                if store_orient {
                    self.orient.insert(i, self.cur_orient);
                }
            }
            _ => {
                self.time.push(t);
                self.lon.push(self.cur_lon);
                self.lat.push(self.cur_lat);
                self.alt.push(alt);
                if store_orient {
                    self.orient.push(self.cur_orient);
                }
            }
        }
        true
    }

    /// Interpolated pose at `t`, clamped to the ends of the track.
    ///
    /// Attitude that the recording does not provide is derived from the
    /// direction of travel, so position-only objects still point where they
    /// are going.
    pub fn sample(&self, t: f64) -> Option<Pose> {
        let n = self.len();
        if n == 0 {
            return None;
        }
        let i = self.index_at(t);
        let (lon, lat, alt, mut rpy);
        if i + 1 >= n || t <= self.time[i] {
            lon = self.lon[i];
            lat = self.lat[i];
            alt = self.alt[i] as f64;
            rpy = self.recorded_orientation(i);
        } else {
            let (t0, t1) = (self.time[i], self.time[i + 1]);
            let f = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
            // Take the short way across the antimeridian.
            let dlon = geo::angle_delta_deg(self.lon[i], self.lon[i + 1]);
            lon = wrap_longitude(self.lon[i] + dlon * f);
            lat = self.lat[i] + (self.lat[i + 1] - self.lat[i]) * f;
            alt = self.alt[i] as f64 + (self.alt[i + 1] - self.alt[i]) as f64 * f;
            let (a, b) = (self.recorded_orientation(i), self.recorded_orientation(i + 1));
            rpy =
                [lerp_angle_or_hold(a[0], b[0], f, true), lerp_or_hold(a[1], b[1], f), lerp_angle_or_hold(a[2], b[2], f, false)];
        }

        if rpy[2].is_nan() || (rpy[1].is_nan() && !self.has_orientation()) {
            let (course, climb) = self.course_at(i).unwrap_or((0.0, 0.0));
            if rpy[2].is_nan() {
                rpy[2] = course;
            }
            if rpy[1].is_nan() && !self.has_orientation() {
                rpy[1] = climb;
            }
        }
        Some(Pose { lon, lat, alt, roll: nan_to_zero(rpy[0]), pitch: nan_to_zero(rpy[1]), heading: nan_to_zero(rpy[2]) })
    }

    /// Speed, climb rate and turn rate around `t`.
    ///
    /// These are differences between recorded positions, averaged over
    /// [`KINEMATICS_WINDOW_S`]. A single pair of samples is not enough:
    /// recorders round timestamps (to 10 ms in DCS exports), which alone makes
    /// the speed between neighbouring samples jump by several percent.
    pub fn kinematics(&self, t: f64) -> Option<Kinematics> {
        let n = self.len();
        if n < 2 {
            return None;
        }
        let i = self.index_at(t).min(n - 2);
        // Widen from the segment holding `t` until the window is covered,
        // without running through gaps where the object was not recorded.
        let half = KINEMATICS_WINDOW_S * 0.5;
        let centre = t.clamp(self.time[i], self.time[i + 1]);
        let (mut first, mut last) = (i, i + 1);
        while first > 0 && self.time[first] > centre - half && self.time[first] - self.time[first - 1] <= KINEMATICS_MAX_GAP_S {
            first -= 1;
        }
        while last + 1 < n && self.time[last] < centre + half && self.time[last + 1] - self.time[last] <= KINEMATICS_MAX_GAP_S {
            last += 1;
        }
        let dt = self.time[last] - self.time[first];
        if dt <= 0.0 {
            return None;
        }
        // Along the path, so a turn does not read as a slowdown.
        let distance: f64 = (first..last).map(|j| self.world_position(j).distance(self.world_position(j + 1))).sum();
        let vertical_speed = (self.alt[last] - self.alt[first]) as f64 / dt;

        // Course change summed segment to segment, so more than half a turn
        // inside the window is still counted the right way round.
        let mut turn_rate = 0.0;
        let mut courses = (first..last).filter_map(|j| Some((self.segment_course(j)?.0 as f64, (self.time[j] + self.time[j + 1]) * 0.5)));
        if let Some((mut course, start)) = courses.next() {
            let (mut turned, mut end) = (0.0, start);
            for (next, time) in courses {
                turned += geo::angle_delta_deg(course, next);
                (course, end) = (next, time);
            }
            if end > start {
                turn_rate = turned / (end - start);
            } else if i >= 1 {
                // The window is a single segment: compare it with the one before.
                let span = (self.time[i + 1] - self.time[i - 1]) * 0.5;
                if let (Some((before, _)), true) = (self.segment_course(i - 1), span > 0.0) {
                    turn_rate = geo::angle_delta_deg(before as f64, course) / span;
                }
            }
        }
        Some(Kinematics { speed: distance / dt, vertical_speed, turn_rate })
    }

    /// `(course, climb angle)` in degrees of the segment starting at sample `i`,
    /// or `None` when the object does not move over it.
    fn segment_course(&self, i: usize) -> Option<(f32, f32)> {
        if i + 1 >= self.len() {
            return None;
        }
        let (lat0, lon0, alt0) = self.position(i);
        let (lat1, lon1, alt1) = self.position(i + 1);
        let horizontal = geo::great_circle_distance_m(lat0, lon0, lat1, lon1);
        let vertical = alt1 - alt0;
        if horizontal < 0.01 && vertical.abs() < 0.01 {
            return None;
        }
        let course = if horizontal < 0.01 { 0.0 } else { geo::initial_bearing_deg(lat0, lon0, lat1, lon1) };
        Some((course as f32, vertical.atan2(horizontal).to_degrees() as f32))
    }

    /// Course around sample `i`, looking back a few segments if stationary.
    fn course_at(&self, i: usize) -> Option<(f32, f32)> {
        let n = self.len();
        if n < 2 {
            return None;
        }
        let start = i.min(n - 2);
        (0..=start.min(8)).find_map(|back| self.segment_course(start - back))
    }
}

#[inline]
fn wrap_longitude(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

#[inline]
fn nan_to_zero(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v
    }
}

#[inline]
fn lerp_or_hold(a: f32, b: f32, f: f64) -> f32 {
    match (a.is_nan(), b.is_nan()) {
        (false, false) => a + (b - a) * f as f32,
        (false, true) => a,
        _ => b,
    }
}

/// Angular interpolation; `signed` selects `[-180, 180)` over `[0, 360)`.
#[inline]
fn lerp_angle_or_hold(a: f32, b: f32, f: f64, signed: bool) -> f32 {
    match (a.is_nan(), b.is_nan()) {
        (false, false) => {
            let v = geo::lerp_angle_deg(a as f64, b as f64, f);
            (if signed && v >= 180.0 { v - 360.0 } else { v }) as f32
        }
        (false, true) => a,
        _ => b,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Property series
// ─────────────────────────────────────────────────────────────────────────────

/// Sparse history of one numeric property.
#[derive(Debug, Clone, Default)]
pub struct NumSeries {
    times: Vec<f64>,
    values: Vec<f32>,
}

impl NumSeries {
    #[inline]
    pub fn len(&self) -> usize {
        self.times.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    pub fn points(&self) -> impl Iterator<Item = (f64, f32)> + '_ {
        self.times.iter().copied().zip(self.values.iter().copied())
    }

    pub fn latest(&self) -> Option<f32> {
        self.values.last().copied()
    }

    /// `dedupe` drops repeats of the current value; only valid for step-sampled kinds.
    fn push(&mut self, t: f64, value: f32, dedupe: bool) {
        match self.times.last().copied() {
            Some(last) if t == last => *self.values.last_mut().unwrap() = value,
            Some(last) if t < last => {
                let i = self.times.partition_point(|&x| x <= t);
                self.times.insert(i, t);
                self.values.insert(i, value);
            }
            Some(_) if dedupe && self.values.last() == Some(&value) => {}
            _ => {
                self.times.push(t);
                self.values.push(value);
            }
        }
    }

    /// Value at `t`. `None` before the first sample.
    pub fn sample(&self, t: f64, kind: PropKind) -> Option<f32> {
        let after = self.times.partition_point(|&x| x <= t);
        if after == 0 {
            return None;
        }
        let i = after - 1;
        let a = self.values[i];
        let PropKind::Number(unit) = kind else { return Some(a) };
        if after >= self.len() {
            return Some(a);
        }
        let b = self.values[after];
        let f = (t - self.times[i]) / (self.times[after] - self.times[i]);
        Some(match unit {
            Unit::Degrees => geo::lerp_angle_deg(a as f64, b as f64, f) as f32,
            _ => a + (b - a) * f as f32,
        })
    }
}

/// Sparse history of one text property.
#[derive(Debug, Clone, Default)]
pub struct TextSeries {
    points: Vec<(f64, Box<str>)>,
}

impl TextSeries {
    pub fn points(&self) -> impl Iterator<Item = (f64, &str)> + '_ {
        self.points.iter().map(|(t, v)| (*t, &**v))
    }

    pub fn latest(&self) -> &str {
        self.points.last().map_or("", |(_, v)| v)
    }

    pub fn at(&self, t: f64) -> &str {
        let after = self.points.partition_point(|(time, _)| *time <= t);
        // Before the first assignment, fall back to it: identity properties
        // (name, pilot, …) are usually written once, sometimes a frame late.
        self.points.get(after.saturating_sub(1)).map_or("", |(_, v)| v)
    }

    /// Returns `true` if the value changed.
    fn push(&mut self, t: f64, value: &str) -> bool {
        if self.latest() == value && !self.points.is_empty() {
            return false;
        }
        match self.points.last_mut() {
            Some((last, slot)) if *last == t => *slot = value.into(),
            _ => self.points.push((t, value.into())),
        }
        true
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Object
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Object {
    pub id: ObjectId,
    pub track: Track,
    tags: TagSet,
    category: Category,
    removed_at: Option<f64>,
    /// Object that fired this one, when the recording does not say (`Parent`).
    pub(crate) inferred_parent: Option<ObjectId>,
    // Small sorted-by-insertion maps: objects rarely carry more than a few
    // dozen properties, so a linear scan beats hashing.
    nums: Vec<(PropId, NumSeries)>,
    texts: Vec<(PropId, TextSeries)>,
}

impl Object {
    pub(crate) fn new(id: ObjectId) -> Self {
        Self {
            id,
            track: Track::default(),
            tags: TagSet::EMPTY,
            category: Category::Misc,
            removed_at: None,
            inferred_parent: None,
            nums: Vec::new(),
            texts: Vec::new(),
        }
    }

    #[inline]
    pub fn tags(&self) -> TagSet {
        self.tags
    }

    #[inline]
    pub fn category(&self) -> Category {
        self.category
    }

    /// Time at which the recording removed the object, if it did.
    #[inline]
    pub fn removed_at(&self) -> Option<f64> {
        self.removed_at
    }

    pub(crate) fn set_removed(&mut self, t: f64) {
        self.removed_at = Some(t);
    }

    /// Whether the object exists in the world at time `t`.
    #[inline]
    pub fn is_active_at(&self, t: f64) -> bool {
        match self.track.first_time() {
            Some(first) => t >= first && self.removed_at.is_none_or(|end| t <= end),
            None => false,
        }
    }

    /// Latest value of a text property (`""` when unset).
    pub fn text(&self, prop: PropId) -> &str {
        self.text_series(prop).map_or("", TextSeries::latest)
    }

    /// Value of a text property at time `t`.
    pub fn text_at(&self, prop: PropId, t: f64) -> &str {
        self.text_series(prop).map_or("", |s| s.at(t))
    }

    pub fn text_series(&self, prop: PropId) -> Option<&TextSeries> {
        self.texts.iter().find(|(id, _)| *id == prop).map(|(_, s)| s)
    }

    /// Value of a numeric property at time `t`, sampled according to `kind`.
    pub fn number_at(&self, prop: PropId, kind: PropKind, t: f64) -> Option<f32> {
        self.number_series(prop)?.sample(t, kind)
    }

    pub fn number_series(&self, prop: PropId) -> Option<&NumSeries> {
        self.nums.iter().find(|(id, _)| *id == prop).map(|(_, s)| s)
    }

    pub fn text_props(&self) -> impl Iterator<Item = (PropId, &TextSeries)> {
        self.texts.iter().map(|(id, s)| (*id, s))
    }

    pub fn number_props(&self) -> impl Iterator<Item = (PropId, &NumSeries)> {
        self.nums.iter().map(|(id, s)| (*id, s))
    }

    pub fn name(&self) -> &str {
        self.text(known::NAME)
    }

    pub fn type_str(&self) -> &str {
        self.text(known::TYPE)
    }

    /// Id of the launching/owning object: explicit `Parent`, else inferred.
    pub fn parent(&self) -> Option<ObjectId> {
        let explicit = self.text(known::PARENT);
        if explicit.is_empty() {
            self.inferred_parent
        } else {
            ObjectId::from_str_radix(explicit.trim(), 16).ok()
        }
    }

    /// Interpolated pose at `t`.
    ///
    /// When the track has no yaw, the `HDG` (true heading) property is used
    /// before falling back to the direction of travel.
    pub fn pose_at(&self, t: f64) -> Option<Pose> {
        let mut pose = self.track.sample(t)?;
        let i = self.track.index_at(t);
        if self.track.recorded_orientation(i)[2].is_nan() {
            if let Some(hdg) = self.number_at(known::HDG, PropKind::Number(Unit::Degrees), t) {
                pose.heading = hdg;
            }
        }
        Some(pose)
    }

    /// Makes `value` the property's value for the object's whole life,
    /// discarding its history; an empty value removes the property.
    /// Returns `true` if anything changed.
    pub(crate) fn replace_text(&mut self, prop: PropId, value: &str) -> bool {
        let existing = self.texts.iter().position(|(id, _)| *id == prop);
        let unchanged = match existing {
            Some(i) => self.texts[i].1.points.len() == 1 && self.texts[i].1.latest() == value,
            None => value.is_empty(),
        };
        if unchanged {
            return false;
        }
        if let Some(i) = existing {
            self.texts.remove(i);
        }
        if !value.is_empty() {
            let from = self.track.first_time().unwrap_or(0.0);
            self.texts.push((prop, TextSeries { points: vec![(from, value.into())] }));
        }
        if prop == known::TYPE {
            self.tags = TagSet::parse(value);
            self.category = Category::from_tags(self.tags);
        }
        true
    }

    /// Returns `true` if the value changed.
    pub(crate) fn set_text(&mut self, prop: PropId, t: f64, value: &str) -> bool {
        let series = match self.texts.iter().position(|(id, _)| *id == prop) {
            Some(i) => &mut self.texts[i].1,
            None => {
                self.texts.push((prop, TextSeries::default()));
                &mut self.texts.last_mut().unwrap().1
            }
        };
        let changed = series.push(t, value);
        if changed && prop == known::TYPE {
            self.tags = TagSet::parse(value);
            self.category = Category::from_tags(self.tags);
        }
        changed
    }

    pub(crate) fn set_number(&mut self, prop: PropId, kind: PropKind, t: f64, value: f32) {
        let series = match self.nums.iter().position(|(id, _)| *id == prop) {
            Some(i) => &mut self.nums[i].1,
            None => {
                self.nums.push((prop, NumSeries::default()));
                &mut self.nums.last_mut().unwrap().1
            }
        };
        series.push(t, value, !matches!(kind, PropKind::Number(_)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(lon: f64, lat: f64, alt: f32) -> TransformUpdate {
        TransformUpdate { lon: Some(lon), lat: Some(lat), alt: Some(alt), ..Default::default() }
    }

    #[test]
    fn track_interpolates_and_clamps() {
        let mut track = Track::default();
        assert!(track.push(0.0, &update(10.0, 20.0, 100.0)));
        assert!(track.push(10.0, &update(11.0, 20.0, 200.0)));

        let mid = track.sample(5.0).unwrap();
        assert!((mid.lon - 10.5).abs() < 1e-12 && (mid.alt - 150.0).abs() < 1e-6);
        // Travelling east with no recorded attitude: heading is derived.
        assert!((mid.heading - 90.0).abs() < 0.5);

        assert_eq!(track.sample(-5.0).unwrap().lon, 10.0);
        assert_eq!(track.sample(99.0).unwrap().lon, 11.0);
    }

    #[test]
    fn partial_updates_keep_previous_fields() {
        let mut track = Track::default();
        // Altitude alone is not a position yet.
        assert!(!track.push(0.0, &TransformUpdate { alt: Some(50.0), ..Default::default() }));
        assert!(track.push(1.0, &update(1.0, 2.0, 50.0)));
        assert!(track.push(2.0, &TransformUpdate { lat: Some(3.0), ..Default::default() }));
        assert_eq!(track.position(1), (3.0, 1.0, 50.0));
    }

    #[test]
    fn same_frame_updates_overwrite() {
        let mut track = Track::default();
        track.push(1.0, &update(1.0, 1.0, 0.0));
        track.push(1.0, &update(2.0, 2.0, 0.0));
        assert_eq!(track.len(), 1);
        assert_eq!(track.position(0).1, 2.0);
    }

    #[test]
    fn orientation_backfills_lazily() {
        let mut track = Track::default();
        track.push(0.0, &update(0.0, 0.0, 0.0));
        assert!(!track.has_orientation());
        track.push(1.0, &TransformUpdate { yaw: Some(350.0), ..update(0.0, 0.0, 0.0) });
        track.push(2.0, &TransformUpdate { yaw: Some(10.0), ..update(0.0, 0.0, 0.0) });
        assert!(track.recorded_orientation(0)[2].is_nan());
        let pose = track.sample(1.5).unwrap();
        assert!(pose.heading < 1.0 || pose.heading > 359.0, "wraps through north: {}", pose.heading);
    }

    #[test]
    fn interpolation_crosses_antimeridian() {
        let mut track = Track::default();
        track.push(0.0, &update(179.0, 0.0, 0.0));
        track.push(2.0, &update(-179.0, 0.0, 0.0));
        let lon = track.sample(1.0).unwrap().lon;
        assert!((lon.abs() - 180.0).abs() < 1e-9, "{lon}");
    }

    #[test]
    fn kinematics_from_track() {
        let mut track = Track::default();
        track.push(0.0, &update(0.0, 0.0, 1000.0));
        track.push(1.0, &update(0.0, 0.001, 1010.0));
        let k = track.kinematics(0.5).unwrap();
        assert!((k.vertical_speed - 10.0).abs() < 1e-3);
        assert!((k.speed - 111.7).abs() < 1.0, "{}", k.speed);
    }

    #[test]
    fn kinematics_smooth_out_timestamp_rounding() {
        // Steady 200 m/s climbing at 5 m/s, sampled every 0.37 s, with the
        // timestamps rounded to 10 ms as recorders do.
        let mut track = Track::default();
        let degrees_per_second = 200.0 / 111_195.0;
        for step in 0..60 {
            let time = step as f64 * 0.37 + if step % 2 == 0 { 0.0 } else { 0.004 };
            let recorded = (time * 100.0).round() / 100.0;
            track.push(recorded, &update(0.0, time * degrees_per_second, (1000.0 + 5.0 * time) as f32));
        }
        let speeds: Vec<f64> = (0..40).map(|i| track.kinematics(3.0 + i as f64 * 0.37).unwrap().speed).collect();
        let (low, high) = speeds.iter().fold((f64::MAX, f64::MIN), |(low, high), &v| (low.min(v), high.max(v)));
        assert!(high - low < 2.0, "speed wanders between {low:.1} and {high:.1}");
        assert!((low - 200.0).abs() < 3.0, "{low}");
        let k = track.kinematics(10.0).unwrap();
        assert!((k.vertical_speed - 5.0).abs() < 0.1, "{}", k.vertical_speed);
        assert!(k.turn_rate.abs() < 0.05, "{}", k.turn_rate);
    }

    #[test]
    fn kinematics_follow_a_turn_and_stop_at_gaps() {
        // A circle flown at 6 degrees per second, 150 m/s.
        let mut track = Track::default();
        let radius_deg = 150.0 / 6f64.to_radians() / 111_195.0;
        for step in 0..=120 {
            let time = step as f64 * 0.5;
            let angle = (6.0 * time).to_radians();
            track.push(time, &update(radius_deg * angle.cos(), radius_deg * angle.sin(), 3000.0));
        }
        let k = track.kinematics(30.0).unwrap();
        assert!((k.speed - 150.0).abs() < 1.5, "{}", k.speed);
        assert!((k.turn_rate.abs() - 6.0).abs() < 0.2, "{}", k.turn_rate);

        // Not recorded for a minute: the speed after it is not diluted by the gap.
        let mut track = Track::default();
        for (time, lat) in [(0.0, 0.0), (0.5, 0.001), (60.0, 0.002), (60.5, 0.003), (61.0, 0.004)] {
            track.push(time, &update(0.0, lat, 0.0));
        }
        let k = track.kinematics(60.6).unwrap();
        assert!((k.speed - 222.4).abs() < 2.0, "{}", k.speed);
    }

    #[test]
    fn numeric_series_sampling() {
        let mut series = NumSeries::default();
        series.push(0.0, 100.0, false);
        series.push(10.0, 200.0, false);
        assert_eq!(series.sample(-1.0, PropKind::Number(Unit::None)), None);
        assert_eq!(series.sample(5.0, PropKind::Number(Unit::None)), Some(150.0));
        assert_eq!(series.sample(5.0, PropKind::Index), Some(100.0));
        assert_eq!(series.sample(50.0, PropKind::Number(Unit::None)), Some(200.0));
    }

    #[test]
    fn type_updates_category() {
        let mut object = Object::new(0x1a);
        assert!(object.set_text(known::TYPE, 0.0, "Air+Rotorcraft"));
        assert!(!object.set_text(known::TYPE, 1.0, "Air+Rotorcraft"));
        assert_eq!(object.category(), Category::Aircraft);
        assert!(object.tags().contains(TagSet::ROTORCRAFT));
    }
}
