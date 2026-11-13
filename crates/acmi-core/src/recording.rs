//! The in-memory representation of one ACMI session.

use rustc_hash::FxHashMap;

use crate::event::Event;
use crate::geo;
use crate::object::{Object, ObjectId};
use crate::props::PropRegistry;
use crate::tags::{Category, TagSet};

/// Stable index of an object within its [`Recording`].
///
/// Handles stay valid for the life of the recording: objects are never
/// deleted, only marked as removed at a point in time, so history can be
/// scrubbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectHandle(pub(crate) u32);

impl ObjectHandle {
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Global (object id 0) properties.
#[derive(Debug, Clone, Default)]
pub struct Metadata {
    /// Every global property in file order, e.g. `Title`, `Author`, `DataSource`.
    props: Vec<(Box<str>, String)>,
    /// Degrees added to every longitude in the file.
    pub reference_longitude: f64,
    /// Degrees added to every latitude in the file.
    pub reference_latitude: f64,
    /// `ReferenceTime` as seconds since the Unix epoch; frame times are relative to it.
    pub reference_time: Option<f64>,
}

impl Metadata {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.props.iter().find(|(k, _)| &**k == key).map(|(_, v)| v.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.props.iter().map(|(k, v)| (&**k, v.as_str()))
    }

    pub fn title(&self) -> &str {
        self.get("Title").unwrap_or("")
    }

    /// Sets a global property, keeping the typed reference fields in sync.
    pub fn set(&mut self, key: &str, value: &str) {
        match key {
            "ReferenceLongitude" => self.reference_longitude = value.trim().parse().unwrap_or(0.0),
            "ReferenceLatitude" => self.reference_latitude = value.trim().parse().unwrap_or(0.0),
            "ReferenceTime" => self.reference_time = crate::time::parse_iso8601(value),
            _ => {}
        }
        match self.props.iter_mut().find(|(k, _)| &**k == key) {
            Some((_, slot)) => value.clone_into(slot),
            None => self.props.push((key.into(), value.to_owned())),
        }
    }
}

/// Everything known about a session.
#[derive(Debug, Clone, Default)]
pub struct Recording {
    pub meta: Metadata,
    pub props: PropRegistry,
    pub(crate) objects: Vec<Object>,
    /// Id → most recent object carrying that id.
    pub(crate) index: FxHashMap<ObjectId, ObjectHandle>,
    pub(crate) events: Vec<Event>,
    pub(crate) time_range: Option<(f64, f64)>,
    pub(crate) revision: u64,
}

impl Recording {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn objects(&self) -> &[Object] {
        &self.objects
    }

    /// Objects paired with their handles.
    pub fn iter(&self) -> impl Iterator<Item = (ObjectHandle, &Object)> {
        self.objects.iter().enumerate().map(|(i, o)| (ObjectHandle(i as u32), o))
    }

    #[inline]
    pub fn object(&self, handle: ObjectHandle) -> &Object {
        &self.objects[handle.index()]
    }

    pub fn get(&self, handle: ObjectHandle) -> Option<&Object> {
        self.objects.get(handle.index())
    }

    /// Looks up the most recent object with this id.
    pub fn find(&self, id: ObjectId) -> Option<ObjectHandle> {
        self.index.get(&id).copied()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Events in chronological order.
    #[inline]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// `(first, last)` time at which the recording holds data.
    #[inline]
    pub fn time_range(&self) -> Option<(f64, f64)> {
        self.time_range
    }

    pub fn duration(&self) -> f64 {
        self.time_range.map_or(0.0, |(start, end)| end - start)
    }

    /// Counter that changes whenever the object list or an object's identity
    /// (name, type, pilot, …) changes. Lets UI code cache derived lists.
    #[inline]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Changes a text property of an object (its name, type, pilot, model…)
    /// for the whole recording, as an edit by the user rather than something
    /// the recording said: the property's history is replaced by the one
    /// value, and an empty value removes it. Returns `true` if it changed.
    pub fn edit_text(&mut self, handle: ObjectHandle, prop: crate::PropId, value: &str) -> bool {
        let changed = self.objects.get_mut(handle.index()).is_some_and(|object| object.replace_text(prop, value));
        if changed {
            self.revision += 1;
        }
        changed
    }

    /// Absolute UTC time (Unix seconds) of a frame time, if the recording has a reference.
    pub fn absolute_time(&self, t: f64) -> Option<f64> {
        self.meta.reference_time.map(|reference| reference + t)
    }

    pub(crate) fn extend_time_range(&mut self, t: f64) {
        self.time_range = Some(match self.time_range {
            Some((start, end)) => (start.min(t), end.max(t)),
            None => (t, t),
        });
    }

    /// Returns the live object with this id, creating it if needed.
    ///
    /// Some exporters recycle ids after removing an object; an update for a
    /// removed id therefore starts a new object rather than resurrecting the
    /// old one.
    pub(crate) fn obtain(&mut self, id: ObjectId, t: f64) -> ObjectHandle {
        if let Some(&handle) = self.index.get(&id) {
            let recycled = self.objects[handle.index()].removed_at().is_some_and(|end| t > end);
            if !recycled {
                return handle;
            }
        }
        let handle = ObjectHandle(u32::try_from(self.objects.len()).expect("more than u32::MAX objects"));
        self.objects.push(Object::new(id));
        self.index.insert(id, handle);
        self.revision += 1;
        handle
    }

    pub(crate) fn push_event(&mut self, event: Event) {
        self.extend_time_range(event.time);
        let at = self.events.partition_point(|e| e.time <= event.time);
        self.events.insert(at, event);
    }

    /// Guesses which object launched a weapon that has just appeared: the
    /// closest platform at that moment, within a plausible distance.
    pub(crate) fn infer_parent(&self, weapon: ObjectHandle) -> Option<ObjectId> {
        const MAX_DISTANCE_M: f64 = 3_000.0;
        // Cheap reject before any trigonometry: ~5.5 km of latitude.
        const MAX_LAT_DELTA_DEG: f64 = 0.05;

        let subject = &self.objects[weapon.index()];
        let last = subject.track.len().checked_sub(1)?;
        let t = subject.track.times()[last];
        let (lat, _, _) = subject.track.position(last);
        let origin = subject.track.world_position(last);

        let mut best: Option<(f64, ObjectId)> = None;
        for (i, candidate) in self.objects.iter().enumerate() {
            let is_platform = matches!(candidate.category(), Category::Aircraft | Category::Ground | Category::Ship)
                || candidate.tags().contains(TagSet::ANTI_AIRCRAFT);
            if i == weapon.index() || !is_platform || !candidate.is_active_at(t) {
                continue;
            }
            // During sequential parsing the candidate's newest sample is "now".
            let newest = candidate.track.len() - 1;
            if (candidate.track.position(newest).0 - lat).abs() > MAX_LAT_DELTA_DEG {
                continue;
            }
            let distance = candidate.track.world_position(newest).distance(origin);
            if distance <= MAX_DISTANCE_M && best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, candidate.id));
            }
        }
        best.map(|(_, id)| id)
    }

    /// Great-circle distance, bearing and altitude difference from `a` to `b`
    /// at time `t`: `(metres, degrees true, metres)`.
    pub fn relation(&self, a: ObjectHandle, b: ObjectHandle, t: f64) -> Option<(f64, f64, f64)> {
        let pa = self.get(a)?.pose_at(t)?;
        let pb = self.get(b)?.pose_at(t)?;
        Some((
            geo::great_circle_distance_m(pa.lat, pa.lon, pb.lat, pb.lon),
            geo::initial_bearing_deg(pa.lat, pa.lon, pb.lat, pb.lon),
            pb.alt - pa.alt,
        ))
    }
}

#[cfg(test)]
mod edit_tests {
    use crate::{known, Category, Parser, Recording};

    #[test]
    fn edits_replace_a_property_for_the_whole_recording() {
        let mut recording = Recording::new();
        let text = "FileType=text/acmi/tacview
FileVersion=2.2
#0
1,T=1|2|300,Type=Air+FixedWing,Name=F-16C,Pilot=Viper
                    #5
1,T=1.1|2|300,Pilot=Cobra
";
        Parser::new().feed_text(&mut recording, text, |_| {}).unwrap();
        let handle = recording.find(1).unwrap();
        let before = recording.revision();

        assert!(recording.edit_text(handle, known::NAME, "MiG-29"));
        assert!(recording.edit_text(handle, known::PILOT, "Bandit"));
        assert!(recording.edit_text(handle, known::TYPE, "Sea+Watercraft"));
        assert!(recording.edit_text(handle, known::SHAPE, "Watercraft.Frigate.glb"));
        let object = recording.object(handle);
        assert_eq!(object.name(), "MiG-29");
        // The whole history, not just from now on.
        assert_eq!((object.text_at(known::PILOT, 0.0), object.text_at(known::PILOT, 9.0)), ("Bandit", "Bandit"));
        assert_eq!(object.category(), Category::Ship);
        assert_eq!(object.text(known::SHAPE), "Watercraft.Frigate.glb");
        // Caches keyed on the revision notice.
        assert!(recording.revision() > before);

        // The same value again is no change; an empty one removes the property.
        let settled = recording.revision();
        assert!(!recording.edit_text(handle, known::NAME, "MiG-29"));
        assert_eq!(recording.revision(), settled);
        assert!(recording.edit_text(handle, known::SHAPE, ""));
        assert_eq!(recording.object(handle).text(known::SHAPE), "");
        assert!(recording.object(handle).text_series(known::SHAPE).is_none());

        // Saving writes the edited identity.
        let saved = crate::writer::to_acmi_string(&recording);
        assert!(saved.contains("Name=MiG-29") && saved.contains("Pilot=Bandit") && !saved.contains("Viper"), "{saved}");
    }
}
